//! A shell in a tab, without a window: over a real PTY, with `sh`
//! (`docs/conversation-tabs-design.md` §3.4).
use orochi_desktop::term::{LIMIT, Terminals};
use std::{
    path::Path,
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

struct Run {
    output: Receiver<Vec<u8>>,
    exit: Receiver<Option<i32>>,
    seen: String,
}

impl Run {
    /// Reads until `needle` has been printed, or fails after a few seconds.
    fn until(&mut self, needle: &str) -> &str {
        let end = Instant::now() + Duration::from_secs(8);
        while !self.seen.contains(needle) {
            let wait = end.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(wait) {
                Ok(bytes) => self.seen.push_str(&String::from_utf8_lossy(&bytes)),
                Err(_) => panic!("never saw {needle:?} in {:?}", self.seen),
            }
        }
        &self.seen
    }
    fn code(&self) -> Option<i32> {
        self.exit
            .recv_timeout(Duration::from_secs(8))
            .expect("the shell ended")
    }
}

fn start(terminals: &Terminals, cwd: &Path, program: &str, args: &[&str]) -> (String, Run) {
    let (data_tx, output) = channel();
    let (exit_tx, exit) = channel();
    let opened = terminals
        .open(
            cwd,
            Some((Path::new(program), args)),
            (80, 24),
            move |bytes| {
                let _ = data_tx.send(bytes);
            },
            move |_, code| {
                let _ = exit_tx.send(code);
            },
        )
        .unwrap();
    (
        opened.id,
        Run {
            output,
            exit,
            seen: String::new(),
        },
    )
}

#[test]
fn a_shell_prints_through_the_terminal_and_reports_how_it_ended() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let (_, mut run) = start(
        &terminals,
        dir.path(),
        "/bin/sh",
        &["-c", "printf 'hi %s' \"$TERM\"; exit 3"],
    );
    run.until("hi xterm-256color");
    assert_eq!(run.code(), Some(3));
}

#[test]
fn what_is_typed_reaches_the_shell_and_is_echoed_back() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let (id, mut run) = start(&terminals, dir.path(), "/bin/cat", &[]);
    terminals.write(&id, b"typed words\n").unwrap();
    run.until("typed words");
    terminals.close(&id).unwrap();
    assert!(
        terminals.write(&id, b"x").is_err(),
        "a closed terminal takes nothing"
    );
}

#[test]
fn it_starts_in_the_conversations_folder_and_nowhere_else() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let real = std::fs::canonicalize(dir.path()).unwrap();
    let (_, mut run) = start(&terminals, dir.path(), "/bin/sh", &["-c", "pwd"]);
    run.until(real.to_str().unwrap());
    let missing = dir.path().join("gone");
    let refused = terminals.open(&missing, None, (80, 24), |_| {}, |_, _| {});
    assert!(
        refused.is_err(),
        "a folder that is not there starts nothing"
    );
}

#[test]
fn the_size_follows_the_tab() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let (id, mut run) = start(
        &terminals,
        dir.path(),
        "/bin/sh",
        &["-c", "sleep 1; stty size"],
    );
    terminals.resize(&id, 100, 30).unwrap();
    run.until("30 100");
}

#[cfg(unix)]
#[test]
fn closing_a_terminal_ends_what_it_started_too() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let (id, mut run) = start(
        &terminals,
        dir.path(),
        "/bin/sh",
        &["-c", "sleep 300 & echo child=$!; wait"],
    );
    let seen = run.until("child=").to_string();
    let after = seen.split("child=").nth(1).unwrap();
    let pid: i32 = after.split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "the child is running");
    terminals.close(&id).unwrap();
    // Gone, or a zombie waiting for its parent to be reaped: neither is still running.
    let running = || {
        let ps = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&ps.stdout).trim().to_string();
        !(state.is_empty() || state.starts_with('Z'))
    };
    let end = Instant::now() + Duration::from_secs(5);
    while running() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(), "the child outlived its terminal");
}

#[test]
fn only_so_many_terminals_are_open_at_once() {
    let terminals = Terminals::default();
    let dir = tempfile::tempdir().unwrap();
    let mut ids = vec![];
    for _ in 0..LIMIT {
        ids.push(start(&terminals, dir.path(), "/bin/cat", &[]).0);
    }
    assert_eq!(terminals.count(), LIMIT);
    let again = || {
        terminals.open(
            dir.path(),
            Some((Path::new("/bin/cat"), &[])),
            (80, 24),
            |_| {},
            |_, _| {},
        )
    };
    assert!(again().unwrap_err().to_string().contains("close one first"));
    terminals.close(&ids[0]).unwrap();
    assert!(again().is_ok());
    terminals.close_all();
    assert_eq!(terminals.count(), 0);
}
