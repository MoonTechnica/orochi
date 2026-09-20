//! Sends SIGINT to its own process, so it lives in a test binary of its own.
#![cfg(unix)]
use std::time::Duration;
use tokio::time::timeout;

/// Each `ctrl_c()` only hears an interrupt that arrives while it is waiting. Work that was
/// between two waits when the user pressed Esc — stopping one agent before trying the next,
/// merging a wave — must still hear it at its next wait, or it carries on after the user
/// stopped it. A run that begins after the interrupt is not stopped by it.
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupt_reaches_work_that_starts_waiting_after_it() {
    orochi::interrupt::listen().await;
    let run = orochi::interrupt::mark();
    assert!(!run.interrupted());
    unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
    // Nothing was waiting when it arrived; the run still hears it at its next wait.
    timeout(Duration::from_secs(5), async {
        while !run.interrupted() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the interrupt was lost");
    timeout(Duration::from_secs(1), run.wait())
        .await
        .expect("a later wait did not hear an earlier interrupt");

    let next = orochi::interrupt::mark();
    assert!(!next.interrupted());
    assert!(
        timeout(Duration::from_millis(200), next.wait())
            .await
            .is_err()
    );
}
