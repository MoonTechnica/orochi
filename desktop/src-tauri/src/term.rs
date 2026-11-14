//! A shell in a tab (`docs/conversation-tabs-design.md` §3.4).
//!
//! A PTY started in the conversation's working directory, with the user's own login shell.
//! Only the user's keystrokes reach it: no agent holds a handle to it, and nothing the
//! repository contains contributes to its environment or command line. What it prints goes to
//! the page and nowhere else, so there is no scrollback to keep or to delete.
use anyhow::{Context, Result, bail, ensure};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

/// Terminals open at once in the whole window; the page keeps fewer per conversation.
pub const LIMIT: usize = 8;

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: Option<u32>,
}

#[derive(Default)]
pub struct Terminals {
    next: AtomicU64,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Opened {
    pub id: String,
    pub shell: String,
}

/// The user's shell: `$SHELL` when it is an absolute path that exists, else a system one.
fn shell() -> PathBuf {
    std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.exists())
        .or_else(|| {
            ["/bin/zsh", "/bin/bash", "/bin/sh"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
        })
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

impl Terminals {
    /// Starts `program` (the user's shell when none) in `cwd`. `data` is called from a reader
    /// thread with what the shell printed, in order; `exit` once, with the terminal's id and the exit code when
    /// there is one, after the last byte has been delivered.
    pub fn open(
        &self,
        cwd: &Path,
        program: Option<(&Path, &[&str])>,
        size: (u16, u16),
        data: impl Fn(Vec<u8>) + Send + 'static,
        exit: impl FnOnce(String, Option<i32>) + Send + 'static,
    ) -> Result<Opened> {
        ensure!(cwd.is_dir(), "{} is not a directory", cwd.display());
        ensure!(
            self.sessions.lock().unwrap().len() < LIMIT,
            "{LIMIT} terminals are open; close one first"
        );
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: size.1.max(2),
                cols: size.0.max(2),
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("a pseudo-terminal could not be opened")?;
        let (path, args): (PathBuf, Vec<&str>) = match program {
            Some((path, args)) => (path.to_path_buf(), args.to_vec()),
            // A login shell, so a window opened from Finder gets the PATH a terminal has.
            None => (shell(), vec!["-l"]),
        };
        let mut command = CommandBuilder::new(&path);
        command.args(&args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "orochi");
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("{} could not be started", path.display()))?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let pid = child.process_id();
        let id = format!("term-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        self.sessions.lock().unwrap().insert(
            id.clone(),
            Session {
                master: pair.master,
                writer,
                child,
                pid,
            },
        );
        let sessions = Arc::clone(&self.sessions);
        let key = id.clone();
        std::thread::spawn(move || {
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => data(buffer[..n].to_vec()),
                }
            }
            // The shell is gone (or its terminal was closed): reap it for its code. Output
            // can end a moment before the process does, so it is asked twice.
            let code = |sessions: &Mutex<HashMap<String, Session>>| {
                sessions
                    .lock()
                    .unwrap()
                    .get_mut(&key)
                    .and_then(|s| s.child.try_wait().ok().flatten())
                    .map(|status| status.exit_code() as i32)
            };
            let status = code(&sessions).or_else(|| {
                std::thread::sleep(Duration::from_millis(50));
                code(&sessions)
            });
            exit(key, status);
        });
        Ok(Opened {
            id,
            shell: path
                .file_name()
                .map_or_else(|| "shell".into(), |n| n.to_string_lossy().into_owned()),
        })
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let mut map = self.sessions.lock().unwrap();
        let session = map
            .get_mut(id)
            .with_context(|| format!("no terminal {id}"))?;
        session.writer.write_all(bytes)?;
        session.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let map = self.sessions.lock().unwrap();
        let session = map.get(id).with_context(|| format!("no terminal {id}"))?;
        session.master.resize(PtySize {
            rows: rows.max(2),
            cols: cols.max(2),
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Ends a terminal and everything it started: the whole process group is hung up, and
    /// killed if it does not go.
    pub fn close(&self, id: &str) -> Result<()> {
        let session = self.sessions.lock().unwrap().remove(id);
        let Some(mut session) = session else {
            bail!("no terminal {id}");
        };
        hang_up(&mut session);
        Ok(())
    }

    pub fn close_all(&self) {
        let all: Vec<_> = self.sessions.lock().unwrap().drain().collect();
        for (_, mut session) in all {
            hang_up(&mut session);
        }
    }

    pub fn count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

fn hang_up(session: &mut Session) {
    #[cfg(unix)]
    if let Some(pid) = session.pid {
        // The shell leads its own session and process group (the PTY makes it so), so a
        // negative pid reaches whatever it started too.
        unsafe {
            libc::killpg(pid as i32, libc::SIGHUP);
        }
        for _ in 0..20 {
            if session.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
    }
    let _ = session.child.kill();
    let _ = session.child.wait();
}
