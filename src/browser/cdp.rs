//! The Chrome DevTools Protocol over a pipe (`--remote-debugging-pipe`).
//!
//! Chrome reads commands from file descriptor 3 and writes replies and events to 4, each
//! message a JSON text ended by a NUL byte. There is no port, so nothing else on the machine
//! can attach to the browser: whoever holds these two ends holds Chrome.
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::unix::pipe,
    sync::{broadcast, oneshot},
};

/// How long one command may take before the browser is taken to be stuck.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Event {
    pub method: String,
    pub params: Value,
    pub session: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Cdp {
    writer: tokio::sync::Mutex<pipe::Sender>,
    pending: Pending,
    next: AtomicU64,
    events: broadcast::Sender<Event>,
    closed: Arc<AtomicBool>,
}

fn pipe_pair() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` is two ints, as `pipe` wants.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        bail!("pipe: {}", std::io::Error::last_os_error());
    }
    // SAFETY: both are fresh descriptors nothing else owns.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// A descriptor of at least 10, close-on-exec, so it can neither be 3 or 4 nor leak into the
/// browser except through the `dup2` below.
fn high(fd: OwnedFd) -> Result<OwnedFd> {
    // SAFETY: `fd` is open for the whole call.
    let moved = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if moved < 0 {
        bail!("fcntl: {}", std::io::Error::last_os_error());
    }
    // SAFETY: a fresh descriptor `fcntl` just made.
    Ok(unsafe { OwnedFd::from_raw_fd(moved) })
}

impl Cdp {
    /// Starts `program` with the two pipes as its descriptors 3 and 4.
    pub fn launch(
        program: &Path,
        args: &[String],
        stderr: Stdio,
    ) -> Result<(Self, tokio::process::Child)> {
        use std::os::unix::process::CommandExt as _;
        let (child_reads, parent_writes) = pipe_pair()?;
        let (parent_reads, child_writes) = pipe_pair()?;
        // Every end is close-on-exec, the parent's too: a copy of the write end of the browser's
        // own input left open in the browser would keep it from ever seeing the broker go.
        let (child_reads, child_writes) = (high(child_reads)?, high(child_writes)?);
        let (parent_writes, parent_reads) = (high(parent_writes)?, high(parent_reads)?);
        let (read_fd, write_fd) = (child_reads.as_raw_fd(), child_writes.as_raw_fd());
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .kill_on_drop(true);
        // SAFETY: only async-signal-safe calls (`dup2`) between fork and exec.
        unsafe {
            command.as_std_mut().pre_exec(move || {
                if libc::dup2(read_fd, 3) < 0 || libc::dup2(write_fd, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .spawn()
            .with_context(|| format!("{} could not be started", program.display()))?;
        drop((child_reads, child_writes));
        let sender = pipe::Sender::from_owned_fd(parent_writes)?;
        let mut receiver = pipe::Receiver::from_owned_fd(parent_reads)?;
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(1024);
        let closed = Arc::new(AtomicBool::new(false));
        let (pending_reader, events_reader, closed_reader) =
            (pending.clone(), events.clone(), closed.clone());
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 64 * 1024];
            let mut message: Vec<u8> = Vec::new();
            loop {
                let n = match receiver.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for byte in &buffer[..n] {
                    if *byte != 0 {
                        message.push(*byte);
                        continue;
                    }
                    if let Ok(value) = serde_json::from_slice::<Value>(&message) {
                        route(value, &pending_reader, &events_reader);
                    }
                    message.clear();
                }
            }
            closed_reader.store(true, Ordering::SeqCst);
            for (_, waiting) in pending_reader.lock().unwrap().drain() {
                let _ = waiting.send(Err("the browser ended".into()));
            }
        });
        Ok((
            Self {
                writer: tokio::sync::Mutex::new(sender),
                pending,
                next: AtomicU64::new(1),
                events,
                closed,
            },
            child,
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// One command, and its answer. A protocol error is an `Err` carrying the browser's words.
    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        if self.is_closed() {
            bail!("the browser ended");
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut message = json!({"id": id, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&message)?;
        bytes.push(0);
        {
            let mut writer = self.writer.lock().await;
            if let Err(error) = writer.write_all(&bytes).await {
                self.pending.lock().unwrap().remove(&id);
                bail!("the browser could not be written to: {error}");
            }
        }
        match tokio::time::timeout(TIMEOUT, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(anyhow!("{method}: {error}")),
            Ok(Err(_)) => bail!("the browser ended"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("{method} did not answer in {} seconds", TIMEOUT.as_secs())
            }
        }
    }
}

fn route(value: Value, pending: &Pending, events: &broadcast::Sender<Event>) {
    if let Some(id) = value["id"].as_u64() {
        if let Some(waiting) = pending.lock().unwrap().remove(&id) {
            let answer = match value.get("error") {
                Some(error) => Err(error["message"]
                    .as_str()
                    .unwrap_or("the browser refused")
                    .to_owned()),
                None => Ok(value["result"].clone()),
            };
            let _ = waiting.send(answer);
        }
    } else if let Some(method) = value["method"].as_str() {
        let _ = events.send(Event {
            method: method.to_owned(),
            params: value["params"].clone(),
            session: value["sessionId"].as_str().map(str::to_owned),
        });
    }
}
