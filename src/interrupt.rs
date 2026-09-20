//! One interrupt for the whole process. A `ctrl_c()` future only hears an interrupt that
//! arrives while it is waiting, so work that was between two waits when the user stopped it —
//! stopping one agent before trying the next, merging a wave — would carry on. Every interrupt
//! is counted here instead; a run remembers the count it began with and stops at its next wait
//! once the count has moved.
use std::sync::{Mutex, OnceLock};
use tokio::{sync::watch, task::JoinHandle};

struct Latch {
    count: watch::Sender<u64>,
    listener: Mutex<Option<JoinHandle<()>>>,
}

fn latch() -> &'static Latch {
    static LATCH: OnceLock<Latch> = OnceLock::new();
    LATCH.get_or_init(|| Latch {
        count: watch::Sender::new(0),
        listener: Mutex::new(None),
    })
}

/// Starts counting interrupts on this runtime, if nothing is counting yet, and returns once
/// the handler is in place. A listener dies with the runtime it ran on, so a later runtime
/// (another test, say) starts its own.
pub async fn listen() {
    let ready = {
        let mut listener = latch().listener.lock().expect("interrupt listener");
        if listener.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let (ready, registered) = tokio::sync::oneshot::channel();
        *listener = Some(tokio::spawn(count(ready)));
        registered
    };
    let _ = ready.await;
}

async fn count(ready: tokio::sync::oneshot::Sender<()>) {
    #[cfg(unix)]
    let Ok(mut interrupts) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
    else {
        return;
    };
    let _ = ready.send(());
    loop {
        #[cfg(unix)]
        let heard = interrupts.recv().await.is_some();
        #[cfg(not(unix))]
        let heard = tokio::signal::ctrl_c().await.is_ok();
        if !heard {
            return;
        }
        latch().count.send_modify(|count| *count += 1);
    }
}

/// Where a run starts counting from.
#[derive(Debug, Clone, Copy)]
pub struct Since(u64);

/// The point a run starts from: interrupts before it do not stop it, every one after does.
pub fn mark() -> Since {
    Since(*latch().count.borrow())
}

impl Since {
    pub fn interrupted(self) -> bool {
        *latch().count.borrow() > self.0
    }
    /// Completes at once if the user interrupted since the mark, otherwise when they do.
    pub async fn wait(self) {
        // A library caller that never listened still gets a handler before it waits.
        listen().await;
        let mut count = latch().count.subscribe();
        let _ = count.wait_for(|count| *count > self.0).await;
    }
}
