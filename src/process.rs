//! Agent process supervision that survives a forced Orochi exit.
//!
//! ACP agents are launched through `orochi --internal-supervise <lease> <command>...`.
//! The supervisor leads the agent's process group, keeps a durable lease listing the agent
//! and every descendant it has observed (including ones that left the group), and
//! terminates them when its owner disappears. Leases left behind when the supervisor was
//! killed as well are reaped by the next Orochi invocation. Every process is identified by
//! (pid, start time), so a recycled PID is never signalled.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const SUPERVISE_FLAG: &str = "--internal-supervise";
const GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Identity {
    pub pid: i32,
    pub start: u64,
}

#[derive(Debug, Clone, Copy)]
struct Proc {
    id: Identity,
    ppid: i32,
    pgid: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub owner: Identity,
    pub supervisor: Identity,
    pub agent: Option<Identity>,
    pub descendants: Vec<Identity>,
}

#[cfg(target_os = "macos")]
fn info(pid: i32) -> Option<Proc> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (read == size).then(|| Proc {
        id: Identity {
            pid,
            start: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        },
        ppid: info.pbi_ppid as i32,
        pgid: info.pbi_pgid as i32,
    })
}

#[cfg(target_os = "macos")]
fn pids() -> Vec<i32> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return vec![];
    }
    let mut buffer = vec![0 as libc::pid_t; count as usize + 64];
    let bytes = (buffer.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    let count = unsafe { libc::proc_listallpids(buffer.as_mut_ptr().cast(), bytes) };
    buffer.truncate(count.max(0) as usize);
    buffer
}

#[cfg(target_os = "linux")]
fn info(pid: i32) -> Option<Proc> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name may contain spaces and parentheses; fields resume after the last ')'.
    let fields: Vec<&str> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some(Proc {
        id: Identity {
            pid,
            start: fields.get(19)?.parse().ok()?,
        },
        ppid: fields.get(1)?.parse().ok()?,
        pgid: fields.get(2)?.parse().ok()?,
    })
}

#[cfg(target_os = "linux")]
fn pids() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn info(_: i32) -> Option<Proc> {
    None
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn pids() -> Vec<i32> {
    vec![]
}

pub fn identity(pid: i32) -> Option<Identity> {
    info(pid).map(|p| p.id)
}
pub fn alive(id: Identity) -> bool {
    identity(id.pid) == Some(id)
}

/// This process as an owner records itself: a pid plus the start time that tells a reused pid
/// apart. Stored by the mailbox for a peer and by the activity store for a host.
pub fn owner_identity() -> anyhow::Result<(i64, i64)> {
    #[cfg(unix)]
    {
        use anyhow::Context;
        let me = identity(std::process::id() as i32).context("cannot identify this process")?;
        Ok((i64::from(me.pid), me.start as i64))
    }
    #[cfg(not(unix))]
    {
        Ok((i64::from(std::process::id()), 0))
    }
}

/// Whether the process that recorded `(pid, start)` is still the one running under that pid.
pub fn owner_alive(pid: i64, start: i64) -> bool {
    #[cfg(unix)]
    {
        alive(Identity {
            pid: pid as i32,
            start: start as u64,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, start);
        true
    }
}

fn snapshot() -> Vec<Proc> {
    pids().into_iter().filter_map(info).collect()
}

/// Current descendants of `root` plus members of process group `group`.
fn family(processes: &[Proc], roots: &[i32], group: Option<i32>) -> BTreeSet<Identity> {
    let mut children: BTreeMap<i32, Vec<&Proc>> = BTreeMap::new();
    for p in processes {
        children.entry(p.ppid).or_default().push(p);
    }
    let mut found = BTreeSet::new();
    let mut queue: Vec<i32> = roots.to_vec();
    while let Some(pid) = queue.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if found.insert(child.id) {
                queue.push(child.id.pid);
            }
        }
    }
    if let Some(group) = group {
        found.extend(processes.iter().filter(|p| p.pgid == group).map(|p| p.id));
    }
    found
}

fn signal(targets: &BTreeSet<Identity>, signal: libc::c_int) {
    let me = std::process::id() as i32;
    for id in targets.iter().filter(|id| id.pid != me && alive(**id)) {
        unsafe {
            libc::kill(id.pid, signal);
        }
    }
}

/// SIGTERM, bounded grace, then SIGKILL. Returns how many identities were still running.
fn terminate(targets: &BTreeSet<Identity>) -> usize {
    let me = std::process::id() as i32;
    let running = targets
        .iter()
        .filter(|id| id.pid != me && alive(**id))
        .count();
    if running == 0 {
        return 0;
    }
    signal(targets, libc::SIGTERM);
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline && targets.iter().any(|id| id.pid != me && alive(*id)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    signal(targets, libc::SIGKILL);
    running
}

fn write(path: &Path, lease: &Lease) {
    let Some(parent) = path.parent() else { return };
    // Atomic replacement: a reader sees the previous or the new complete lease.
    if let Ok(mut temp) = tempfile::NamedTempFile::new_in(parent)
        && serde_json::to_writer(&mut temp, lease).is_ok()
    {
        let _ = temp.persist(path);
    }
}

fn read(path: &Path) -> Option<Lease> {
    if path.metadata().ok()?.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Entry point for `orochi --internal-supervise <lease> <command> [args...]`.
pub fn supervise(args: &[OsString]) -> ExitCode {
    let [lease_path, command, rest @ ..] = args else {
        eprintln!("orochi: invalid supervisor arguments");
        return ExitCode::from(2);
    };
    let lease_path = PathBuf::from(lease_path);
    let me = std::process::id() as i32;
    let owner_pid = unsafe { libc::getppid() };
    let (Some(owner), Some(supervisor)) = (identity(owner_pid), identity(me)) else {
        eprintln!("orochi: cannot identify supervised processes");
        return ExitCode::from(2);
    };
    let group = unsafe { libc::getpgrp() };
    let mut lease = Lease {
        owner,
        supervisor,
        agent: None,
        descendants: vec![],
    };
    // Record the lease before the agent exists, so no agent can outlive an unrecorded owner.
    write(&lease_path, &lease);
    let mut child = match std::process::Command::new(command).args(rest).spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!(
                "orochi: failed to spawn {}: {error}",
                command.to_string_lossy()
            );
            let _ = std::fs::remove_file(&lease_path);
            return ExitCode::from(127);
        }
    };
    let agent_pid = child.id() as i32;
    lease.agent = identity(agent_pid);
    write(&lease_path, &lease);
    let mut tracked: BTreeSet<Identity> = lease.agent.into_iter().collect();
    let mut refreshed = Instant::now();
    let code = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break status.code().unwrap_or_else(|| {
                128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0)
            });
        }
        if unsafe { libc::getppid() } != owner_pid {
            // Owner exited without stopping us (for example SIGKILL): stop the agent tree.
            unsafe {
                libc::signal(libc::SIGTERM, libc::SIG_IGN);
            }
            tracked.extend(family(&snapshot(), &[agent_pid, me], Some(group)));
            terminate(&tracked);
            let _ = child.try_wait();
            let _ = std::fs::remove_file(&lease_path);
            return ExitCode::from(1);
        }
        if refreshed.elapsed() >= Duration::from_millis(500) {
            refreshed = Instant::now();
            tracked.retain(|id| alive(*id));
            tracked.extend(family(&snapshot(), &[agent_pid, me], Some(group)));
            tracked.remove(&supervisor);
            let descendants: Vec<_> = tracked
                .iter()
                .copied()
                .filter(|id| Some(*id) != lease.agent)
                .collect();
            if descendants != lease.descendants {
                lease.descendants = descendants;
                write(&lease_path, &lease);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // The agent exited on its own; do not leave its background descendants running.
    tracked.extend(family(&snapshot(), &[me], Some(group)));
    tracked.remove(&supervisor);
    terminate(&tracked);
    let _ = std::fs::remove_file(&lease_path);
    ExitCode::from(code.clamp(0, 255) as u8)
}

struct Supervision {
    executable: PathBuf,
    leases: PathBuf,
}
static SUPERVISION: OnceLock<Supervision> = OnceLock::new();

/// Enable supervised launches for this process (the Orochi binary only).
pub fn enable(executable: PathBuf, leases: PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(&leases)?;
    let _ = SUPERVISION.set(Supervision { executable, leases });
    Ok(())
}

/// Wrap an agent command when supervision is enabled. Returns the lease to release on stop.
pub fn wrap(command: &mut String, args: &mut Vec<String>) -> Option<PathBuf> {
    let supervision = SUPERVISION.get()?;
    let lease = supervision
        .leases
        .join(format!("{}.json", uuid::Uuid::new_v4()));
    let mut wrapped = vec![
        SUPERVISE_FLAG.to_owned(),
        lease.to_string_lossy().into_owned(),
        std::mem::take(command),
    ];
    wrapped.append(args);
    *args = wrapped;
    *command = supervision.executable.to_string_lossy().into_owned();
    Some(lease)
}

/// Called after the ACP connection (and with it the agent's process group) was dropped.
pub fn release(lease: &Path) {
    if let Some(lease) = read(lease) {
        let targets = lease
            .descendants
            .iter()
            .chain(lease.agent.iter())
            .chain(std::iter::once(&lease.supervisor))
            .copied()
            .collect();
        signal(&targets, libc::SIGKILL);
    }
    let _ = std::fs::remove_file(lease);
}

/// Terminate processes recorded by Orochi runs that no longer exist. Returns the number of
/// processes that were still running.
pub fn reap(leases: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(leases) else {
        return 0;
    };
    let mut reaped = 0;
    let mut processes = None;
    for entry in entries.flatten().take(4096) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(lease) = read(&path) else {
            // Leases are replaced atomically, so an unreadable one is not in progress.
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if alive(lease.owner) {
            continue;
        }
        let mut targets: BTreeSet<Identity> = lease
            .descendants
            .iter()
            .chain(lease.agent.iter())
            .copied()
            .collect();
        let processes = processes.get_or_insert_with(snapshot);
        let mut roots: Vec<i32> = targets
            .iter()
            .filter(|id| alive(**id))
            .map(|id| id.pid)
            .collect();
        let group = alive(lease.supervisor).then_some(lease.supervisor.pid);
        if let Some(group) = group {
            roots.push(group);
            targets.insert(lease.supervisor);
        }
        // Only verified live members contribute their current descendants.
        targets.extend(family(processes, &roots, group));
        reaped += terminate(&targets);
        let _ = std::fs::remove_file(&path);
    }
    reaped
}
