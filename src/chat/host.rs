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

pub async fn run(config: &Config, data: &Path, thread: &str) -> Result<u8> {
    let store = Store::open(data)?;
    let activity = share(Activity::open(data, config.activity.retention_days)?);
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
            let sent = match kind.as_str() {
                "interrupt" | "stop" => keys.send(Key::Interrupt).is_ok(),
                "new" => line(&keys, "/new"),
                "reroute" => line(&keys, "/reroute"),
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
                if keys.send(Key::Queued { turn, text }).is_err() {
                    return;
                }
                last = Instant::now();
            }
            _ => {
                if last.elapsed() >= idle {
                    return;
                }
                let _ = activity.lock().expect("activity store").heartbeat(&host);
                tokio::time::sleep(POLL).await;
            }
        }
    }
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
