//! A conversation with no terminal at either end.
//!
//! `orochi host --thread <id>` is the console's turn loop with the keyboard replaced by the
//! store: it claims the turns a client queued, runs them the way a chat turn runs, and leaves
//! the questions they raise open for that client to answer. It exists so that closing the app
//! does not stop the work, and so that a thread has exactly one owner whether that owner is a
//! terminal or not.
use crate::{
    activity::{Activity, Shared, TurnState, share},
    config::{Config, PermissionMode},
    policy::Registry,
    scheduler::{self, Continuation, RunOptions},
    storage::{Store, workspace_lock},
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
    // is gone has already released it.
    let host = activity
        .lock()
        .expect("activity store")
        .register_host(Some(thread), "headless")
        .context("this thread is already running in another process")?;
    let _guard = Release {
        activity: activity.clone(),
        host: host.clone(),
    };

    let policies = Registry::load(data)?;
    let repository = store.repository_id(&cwd)?;
    let _lock = workspace_lock(data, &repository, config.scheduler.shared_workspace)?;
    let idle = Duration::from_secs(config.activity.host_idle_secs);
    let mut last = Instant::now();
    let mut continuation: Option<Continuation> = None;
    loop {
        if let Some((kind, _payload)) = activity
            .lock()
            .expect("activity store")
            .take_control(thread)?
        {
            match kind.as_str() {
                // Every rule about what an interrupt means lives in `interrupt.rs`; raising
                // the real signal is how this host says it without inventing a second path.
                "interrupt" | "stop" => {
                    #[cfg(unix)]
                    unsafe {
                        libc::raise(libc::SIGINT);
                    }
                    if kind == "stop" {
                        return Ok(130);
                    }
                }
                "new" => continuation = None,
                _ => {}
            }
            last = Instant::now();
        }
        let queued = activity
            .lock()
            .expect("activity store")
            .next_queued(thread)?;
        let Some((turn, text)) = queued else {
            if last.elapsed() >= idle {
                return Ok(0);
            }
            activity.lock().expect("activity store").heartbeat(&host)?;
            tokio::time::sleep(POLL).await;
            continue;
        };
        if !activity
            .lock()
            .expect("activity store")
            .claim_turn(&turn, &host)?
        {
            continue;
        }
        let seat = {
            let db = activity.lock().expect("activity store");
            db.create_seat(&turn, 0, "implementer", true, false, None, None)
                .map(|seat| crate::activity::SeatRef {
                    thread: thread.to_owned(),
                    turn: turn.clone(),
                    seat,
                    store: activity.clone(),
                })?
        };
        let resume = continuation
            .as_ref()
            .filter(|c| c.loadable)
            .map(|c| c.session.session_id.clone());
        let pinned = continuation.as_ref().map(|c| Overrides {
            agent: Some(c.session.agent.clone()),
            model: Some(c.session.model.clone()),
            reasoning: c.session.reasoning.clone(),
            mode: c.session.mode.clone(),
        });
        let code = scheduler::run_turn(
            config,
            &policies,
            &store,
            &cwd,
            RunOptions {
                task: text,
                descriptor: None,
                overrides: pinned.unwrap_or_else(|| overrides.clone()),
                dry_run: false,
                json: false,
                resume,
                permission,
                // A client renders progress from the store, so this is a chat turn in every
                // way except that nothing is watching stderr.
                interactive: true,
                attachments: vec![],
                peer: None,
                verify: true,
                read_only: false,
                place: None,
                seat: Some(seat.clone()),
                // Nobody here can answer a permission request; the question waits for the
                // client that asked for the work.
                answerer: crate::activity::recorder::Answerer::Store,
            },
            None,
            &mut continuation,
        )
        .await;
        let state = match &code {
            Ok(0) => TurnState::Completed,
            Ok(130) => TurnState::Interrupted,
            _ => TurnState::Failed,
        };
        let db = activity.lock().expect("activity store");
        db.turn_state(&turn, state)?;
        db.seat_state(
            &seat.seat,
            match state {
                TurnState::Completed => crate::activity::SeatState::Done,
                TurnState::Interrupted => crate::activity::SeatState::Cancelled,
                _ => crate::activity::SeatState::Failed,
            },
        )?;
        if let Some(current) = &continuation {
            db.set_continuation(
                thread,
                Some(
                    &serde_json::json!({
                        "agent": current.session.agent, "model": current.session.model,
                        "reasoning": current.session.reasoning, "mode": current.session.mode,
                        "session_id": current.session.session_id,
                        "loadable": current.loadable, "modes": current.modes,
                    })
                    .to_string(),
                ),
            )?;
        }
        drop(db);
        last = Instant::now();
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
