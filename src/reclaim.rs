//! What a deleted thread leaves on disk, taken back: its session directory (helpers'
//! worktrees, merge trials, patches), the sandbox it was given, and a thread worktree no thread
//! is left to use.
//!
//! A thread is deleted in more than one place — `orochi threads delete`, the window, and the
//! retention window inside `Activity::open` — so this does not follow deletions. It compares
//! what is on disk with the threads that still exist, and takes only what belongs to none of
//! them and that no running process holds. Calling it again is always safe.
use crate::{activity::Activity, config::Config, sandbox, workspaces, worktrees};
use anyhow::Result;
use serde::Serialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Serialize)]
pub struct Reclaimed {
    /// Session directories removed, each with whatever was still in it.
    pub sessions: Vec<PathBuf>,
    pub sandboxes: Vec<String>,
    /// Sandboxes of deleted threads left because the VM was stopped; the next call with
    /// `start_vm` takes them.
    pub sandboxes_waiting: usize,
    pub worktrees: Vec<worktrees::Removed>,
    pub errors: Vec<String>,
}
impl Reclaimed {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
            && self.sandboxes.is_empty()
            && self.sandboxes_waiting == 0
            && self.worktrees.is_empty()
            && self.errors.is_empty()
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![];
        for name in &self.sandboxes {
            lines.push(format!("deleted sandbox {name}"));
        }
        if self.sandboxes_waiting > 0 {
            lines.push(format!(
                "{} sandbox{} of deleted conversations wait for the sandbox VM; `orochi workspaces prune` deletes them",
                self.sandboxes_waiting,
                if self.sandboxes_waiting == 1 { "" } else { "es" }
            ));
        }
        for session in &self.sessions {
            lines.push(format!("removed {}", session.display()));
        }
        for worktree in &self.worktrees {
            lines.push(worktree.describe());
        }
        for error in &self.errors {
            lines.push(format!("could not reclaim: {error}"));
        }
        lines
    }
}

/// Takes back everything no thread in `activity` owns and no process holds. `start_vm` lets it
/// start the sandbox VM to delete sandboxes; without it they wait for the VM's next use.
pub fn reclaim(
    config: &Config,
    data: &Path,
    activity: &Activity,
    start_vm: bool,
) -> Result<Reclaimed> {
    let crate::activity::Places { threads, roots } = activity.places()?;
    let keys: HashSet<String> = threads
        .iter()
        .map(|(id, _)| crate::context::hash(&[id.as_bytes()]))
        .collect();
    let cwds: Vec<PathBuf> = threads
        .iter()
        .map(|(_, cwd)| canonical(Path::new(cwd)))
        .collect();
    let mut reclaimed = Reclaimed::default();

    // Every checkout a session directory can be in: the projects, the places threads work,
    // and the thread worktrees of each repository.
    let mut mains: Vec<PathBuf> = roots
        .iter()
        .chain(threads.iter().map(|(_, cwd)| cwd))
        .filter_map(|root| worktrees::main_checkout(Path::new(root)).ok())
        .collect();
    mains.sort();
    mains.dedup();
    let thread_worktrees: Vec<PathBuf> = mains.iter().flat_map(|m| worktrees::list(m)).collect();
    let mut checkouts: Vec<PathBuf> = roots
        .iter()
        .map(|r| canonical(Path::new(r)))
        .chain(cwds.iter().cloned())
        .chain(mains.iter().cloned())
        .chain(thread_worktrees.iter().cloned())
        .collect();
    checkouts.sort();
    checkouts.dedup();

    // A session directory nobody's thread names and nobody holds, held here while it goes.
    let mut orphans: Vec<(PathBuf, workspaces::Live)> = vec![];
    let sessions: Vec<PathBuf> = checkouts
        .iter()
        .flat_map(|c| workspaces::sessions_dirs(&config.workspaces, c))
        .collect();
    for dir in &sessions {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let directory = entry.path();
            let named = directory
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| keys.contains(n));
            if named || !directory.is_dir() {
                continue;
            }
            if let Ok(Some(live)) = workspaces::hold(&directory) {
                orphans.push((canonical(&directory), live));
            }
        }
    }

    let orphaned: HashSet<PathBuf> = orphans.iter().map(|(d, _)| d.clone()).collect();
    let gone = |project: &sandbox::Project| {
        let root = canonical(&project.root);
        if orphaned.contains(&root) {
            return true;
        }
        // A sandbox whose directory is already gone: nothing holds what is not there.
        let named = root
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| keys.contains(n));
        !named && !project.root.exists()
    };
    match sandbox::ops::retire(&config.sandbox, data, &gone, start_vm) {
        Ok(retired) => {
            reclaimed.sandboxes = retired.removed;
            reclaimed.sandboxes_waiting = retired.waiting;
        }
        Err(error) => reclaimed.errors.push(format!("{error:#}")),
    }

    for (directory, live) in orphans {
        let taken = take_session(&directory);
        drop(live);
        match taken.and_then(|()| Ok(std::fs::remove_dir_all(&directory)?)) {
            Ok(()) => reclaimed.sessions.push(directory),
            Err(error) => reclaimed
                .errors
                .push(format!("{}: {error:#}", directory.display())),
        }
    }

    for worktree in thread_worktrees {
        if cwds.iter().any(|cwd| cwd.starts_with(&worktree)) {
            continue;
        }
        // Still on disk because a session in it is held: its process holds the worktree too.
        if workspaces::sessions_dirs(&config.workspaces, &worktree)
            .iter()
            .any(|dir| std::fs::read_dir(dir).is_ok_and(|mut e| e.next().is_some()))
        {
            continue;
        }
        match worktrees::remove(&worktree) {
            Ok(Some(removed)) => reclaimed.worktrees.push(removed),
            Ok(None) => {}
            Err(error) => reclaimed
                .errors
                .push(format!("{}: {error:#}", worktree.display())),
        }
    }
    Ok(reclaimed)
}

/// A session directory's helper worktrees and merge trials, removed through git so the
/// repository forgets them; the directory itself goes after.
fn take_session(directory: &Path) -> Result<()> {
    let manifest = directory.join("workspaces.json");
    let Ok(bytes) = std::fs::read(&manifest) else {
        return Ok(());
    };
    let Ok(recorded) = serde_json::from_slice::<workspaces::Workspaces>(&bytes) else {
        return Ok(());
    };
    if !recorded.source.is_dir() {
        return Ok(());
    }
    let mut state = workspaces::Workspaces::open(&recorded.source, directory)?;
    for name in state.names() {
        state.remove(&name)?;
    }
    let _ = crate::merge::sweep_trials(&recorded.source, directory);
    Ok(())
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
