//! A conversation with no terminal at either end.
//!
//! `orochi host --thread <id>` is the console's turn loop with the keyboard replaced by the
//! store: it claims the turns a client queued, runs them the way a chat turn runs, and leaves
//! the questions they raise open for that client to answer. It exists so that closing the app
//! does not stop the work, and so that a thread has exactly one owner whether that owner is a
//! terminal or not.
//!
//! There is no second turn loop here. A loop of its own would drift from the console's the
//! moment either changed — it did, and a message that seated two agents in a terminal ran
//! alone in the window. This reads rows and types them; everything that happens next is
//! `chat::Session`.
use crate::{
    activity::{Activity, Shared, share},
    chat::{Options, term::Key},
    config::{Config, PermissionMode},
    storage::Store,
    types::Overrides,
};
use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How often the queue and the control rows are looked at. A client's message should start
/// within a blink; nothing else here is time-critical.
const POLL: Duration = Duration::from_millis(100);

pub async fn run(config: &Config, config_path: &Path, data: &Path, thread: &str) -> Result<u8> {
    crate::keeper::remember_config(config_path);
    let store = Store::open(data)?;
    crate::activity::recorder::ASK_SECS.store(
        config.routines.ask_secs,
        std::sync::atomic::Ordering::Relaxed,
    );
    let activity = share(Activity::open(data, config.activity.retention_days)?);
    {
        let home = {
            let db = activity.lock().expect("activity store");
            db.resident_for(thread)
                .ok()
                .flatten()
                .map(|r| r.home)
                .unwrap_or_default()
        };
        if let Ok(mut policy) = crate::activity::recorder::ROUTINE_APPROVAL.lock() {
            *policy = (config.routines.approval.clone(), home);
        }
    }
    let (cwd, overrides, permission) = {
        let db = activity.lock().expect("activity store");
        let (cwd, overrides, permission) = db
            .thread_settings(thread)?
            .with_context(|| format!("no thread {thread}"))?;
        (
            PathBuf::from(cwd),
            serde_json::from_str::<Overrides>(&overrides).unwrap_or_default(),
            permission,
        )
    };
    anyhow::ensure!(
        cwd.is_dir(),
        "the thread's directory is gone: {}",
        cwd.display()
    );
    let permission = match permission.as_str() {
        "deny" => PermissionMode::Deny,
        "allow" => PermissionMode::Allow,
        // A headless host cannot ask a terminal, so `ask` means the client is asked instead.
        _ => PermissionMode::Ask,
    };

    // One owner per thread: the unique index refuses a second host, and a host whose process
    // is gone has already released it. Registered here rather than inside the session, because
    // being second is a reason not to start at all.
    let host = activity
        .lock()
        .expect("activity store")
        .register_host(Some(thread), "headless")
        .context("this thread is already running in another process")?;
    let _guard = Release {
        activity: activity.clone(),
        host: host.clone(),
    };

    let (keys, keyboard) = crate::chat::term::Keyboard::channel();
    let typist = tokio::spawn(feed(
        activity.clone(),
        thread.to_owned(),
        host.clone(),
        keys,
        Duration::from_secs(config.activity.host_idle_secs),
    ));
    let code = crate::chat::run_with(
        config,
        data,
        &store,
        &cwd,
        Options {
            overrides,
            host: Some(host),
            resume: None,
            permission,
            config_path: Some(config_path.to_path_buf()),
            thread: Some(thread.to_owned()),
        },
        Some(keyboard),
    )
    .await;
    typist.abort();
    code
}

/// Types what the client wrote. A queued turn is a message; a control is the key or the
/// command the same request is at a terminal, so both ends reach the one implementation of
/// what each means. Letting the sender drop ends the session, which is how a host leaves.
async fn feed(
    activity: Shared,
    thread: String,
    host: String,
    keys: tokio::sync::mpsc::UnboundedSender<Key>,
    idle: Duration,
) {
    let mut last = Instant::now();
    let line = |keys: &tokio::sync::mpsc::UnboundedSender<Key>, text: &str| {
        keys.send(Key::Paste(text.to_owned())).is_ok() && keys.send(Key::Enter).is_ok()
    };
    loop {
        let control = activity
            .lock()
            .expect("activity store")
            .take_control(&thread);
        if let Ok(Some((kind, payload))) = control {
            // `stop` naming a seat, or every background seat, stops those and nothing else.
            let helpers = payload
                .as_deref()
                .filter(|_| kind == "stop")
                .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
                .and_then(|payload| {
                    if payload["background"] == "all" {
                        return Some(None);
                    }
                    let seat = payload["seat"].as_str()?.to_owned();
                    let role = activity
                        .lock()
                        .expect("activity store")
                        .connection()
                        .query_row("SELECT role FROM seats WHERE id=?1", [&seat], |r| {
                            r.get::<_, String>(0)
                        })
                        .ok()?;
                    Some(Some(role))
                });
            if let Some(name) = helpers {
                if keys.send(Key::Stop(name)).is_err() {
                    return;
                }
                last = Instant::now();
                continue;
            }
            let sent = match kind.as_str() {
                "interrupt" | "stop" => keys.send(Key::Interrupt).is_ok(),
                "new" => line(&keys, "/new"),
                // With a route, `reroute` pins it (`/model`); without one Orochi chooses again.
                "reroute" => match payload.as_deref().filter(|p| !p.trim().is_empty()) {
                    Some(route) => line(&keys, &format!("/model {route}")),
                    None => line(&keys, "/reroute"),
                },
                "set_permission" => {
                    line(&keys, &format!("/confirm {}", payload.unwrap_or_default()))
                }
                // The agent's own session modes are cycled, not named, so the request is the
                // same key a person presses.
                "set_mode" => keys.send(Key::ShiftTab).is_ok(),
                _ => true,
            };
            if !sent || kind == "stop" {
                return;
            }
            last = Instant::now();
        }
        let queued = activity
            .lock()
            .expect("activity store")
            .next_queued(&thread);
        match queued {
            Ok(Some((turn, text))) => {
                let claimed = activity
                    .lock()
                    .expect("activity store")
                    .claim_turn(&turn, &host);
                if !matches!(claimed, Ok(true)) {
                    continue;
                }
                let routine = activity
                    .lock()
                    .expect("activity store")
                    .turn_is_routine(&turn)
                    .unwrap_or(false);
                if keys
                    .send(Key::Queued {
                        turn,
                        text,
                        routine,
                    })
                    .is_err()
                {
                    return;
                }
                last = Instant::now();
            }
            _ => {
                // Helpers still working keep the host: their results are a turn it has to run.
                // A request held in the store resumes here, so this host stays up for it.
                let held = activity
                    .lock()
                    .expect("activity store")
                    .held(&thread)
                    .ok()
                    .flatten()
                    .is_some();
                let working = activity
                    .lock()
                    .expect("activity store")
                    .live_seats()
                    .map(|seats| seats.iter().any(|s| s.thread_id == thread && s.background))
                    .unwrap_or(false)
                    // A request held for an agent's limit resumes here, so the host stays up.
                    || crate::chat::holding()
                    || held
                    // A question nobody has answered is a turn waiting on a person: ending the
                    // host would end the turn the answer was for.
                    || activity
                        .lock()
                        .expect("activity store")
                        .has_open_prompt(&thread)
                        .unwrap_or(false);
                if working {
                    last = Instant::now();
                }
                if last.elapsed() >= idle {
                    return;
                }
                let _ = activity.lock().expect("activity store").heartbeat(&host);
                tokio::time::sleep(POLL).await;
            }
        }
    }
}

/// Where a host's own stderr goes. A pipe nobody reads would stop it — or kill it, once the
/// reader went away — so it is a file, which lives as long as the host does.
pub fn log_path(data: &Path, thread: &str) -> PathBuf {
    data.join("hosts").join(format!("{thread}.log"))
}

/// Starts `orochi host` for a thread and makes sure it took. Spawning succeeds whatever the
/// binary then does, and one too old to know the subcommand rejects it and exits at once, so
/// the caller would report a host while the turn sat queued for ever with nothing said. A host
/// still running after a moment is a host; one that has already stopped is reported with what
/// it said on its way out. `global` are the arguments that name this Orochi's config and data
/// (`--config`, `--data-dir`), which the host needs to find the same store its caller wrote.
pub fn spawn(
    binary: &Path,
    global: &[std::ffi::OsString],
    data: &Path,
    thread: &str,
) -> Result<()> {
    let log = log_path(data, thread);
    std::fs::create_dir_all(log.parent().expect("a log has a folder"))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log)
        .with_context(|| format!("could not open {}", log.display()))?;
    let mut command = std::process::Command::new(binary);
    command
        .args(global)
        .args(["host", "--thread", thread])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(file);
    // Its own process group, so an interrupt aimed at whoever started it does not reach it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command
        .spawn()
        .with_context(|| format!("could not start {}", binary.display()))?;
    std::thread::sleep(Duration::from_millis(400));
    let Some(status) = child.try_wait()? else {
        return Ok(());
    };
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let said = said
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    anyhow::bail!(
        "{} could not host this thread ({status}){}{said}",
        binary.display(),
        if said.is_empty() { "" } else { ": " }
    )
}

/// Whether a host is running the thread now. A host whose process is gone has released it.
pub fn running(activity: &Activity, thread: &str) -> Result<bool> {
    activity.reap_hosts()?;
    Ok(activity.connection().query_row(
        "SELECT EXISTS(SELECT 1 FROM hosts WHERE thread_id=?1)",
        [thread],
        |r| r.get(0),
    )?)
}

/// A host that ends releases its thread, so the next message can start another.
struct Release {
    activity: Shared,
    host: String,
}
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self
            .activity
            .lock()
            .expect("activity store")
            .unregister_host(&self.host);
    }
}
