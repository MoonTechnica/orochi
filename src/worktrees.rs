//! Worktrees Orochi makes for threads to work in, and the rule that one lasts exactly as long
//! as the threads that use it.
//!
//! A thread started with `--worktree` (or the window's worktree toggle) works in
//! `<repository>/.orochi/worktrees/<name>`, on its own branch. Only Orochi makes these and
//! only Orochi works in them, so every user is one it can see: a thread whose `cwd` lies inside
//! one, or a process holding it (`hold`, a shared lock beside it) while it runs there. When no
//! thread and no process is left, `reclaim` removes it — a worktree is a whole checkout, and
//! left alone they fill the disk. A worktree some other tool made is never one of these.
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Where a repository's thread worktrees live, under its main checkout.
pub fn base(main: &Path) -> PathBuf {
    main.join(".orochi/worktrees")
}

/// The main checkout of the repository `root` is in, whichever worktree `root` is in.
pub fn main_checkout(root: &Path) -> Result<PathBuf> {
    let common = PathBuf::from(git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    ensure!(
        common.file_name().is_some_and(|n| n == ".git"),
        "a worktree needs a repository with a checkout, not a bare one"
    );
    Ok(common
        .parent()
        .context("the repository has no parent directory")?
        .canonicalize()?)
}

/// A name for a new worktree that none of the repository's has.
pub fn fresh_name(main: &Path) -> String {
    loop {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let name = format!("t-{}", &id[..6]);
        if !base(main).join(&name).exists() {
            return name;
        }
    }
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A thread worktree named `name` for the repository `root` is in, on the branch
/// `<prefix><name>` (taken as it is when it exists, else started at `root`'s HEAD). An existing
/// worktree of that name is joined, not made again. Returns the directory in it that
/// corresponds to `root`, so a subdirectory stays a subdirectory.
pub fn create(root: &Path, name: &str, prefix: &str) -> Result<PathBuf> {
    ensure!(
        valid_name(name),
        "a worktree name is lowercase letters, digits and `-`, at most 64 characters"
    );
    let root = root.canonicalize()?;
    let main = main_checkout(&root)?;
    let toplevel = PathBuf::from(git(&root, &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    let relative = root.strip_prefix(&toplevel).unwrap_or(Path::new(""));
    let path = base(&main).join(name);
    if !path.exists() {
        exclude_orochi(&root)?;
        std::fs::create_dir_all(base(&main))?;
        let branch = format!("{prefix}{name}");
        let target = path.to_str().context("non-UTF8 worktree path")?;
        let exists = git_ok(
            &main,
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )?;
        if exists {
            git(&main, &["worktree", "add", target, &branch])?;
        } else {
            let head = git(&root, &["rev-parse", "HEAD"])
                .context("a worktree starts from a commit; this repository has none yet")?;
            git(&main, &["worktree", "add", "-b", &branch, target, &head])?;
        }
    } else {
        ensure!(
            path.join(".git").exists(),
            "{} exists and is not a worktree",
            path.display()
        );
    }
    // Joining an empty path would add a trailing separator, and the thread's `cwd` with it.
    let path = path.canonicalize()?;
    let inside = if relative.as_os_str().is_empty() {
        path
    } else {
        path.join(relative)
    };
    std::fs::create_dir_all(&inside)?;
    Ok(inside)
}

/// The thread worktree `path` lies in, if it lies in one.
pub fn owning(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| {
            p.parent()
                .is_some_and(|parent| parent.ends_with(".orochi/worktrees"))
                && p.join(".git").is_file()
        })
        .map(Path::to_path_buf)
}

/// Every thread worktree of the repository whose main checkout is `main`.
pub fn list(main: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base(main)) else {
        return vec![];
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(valid_name)
                && p.join(".git").is_file()
        })
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    found.sort();
    found
}

/// A process working in a thread worktree says so for as long as it runs: a shared lock
/// beside it, so several may hold it and `reclaim`, which needs it exclusively, waits for all.
pub struct Held {
    _file: std::fs::File,
}
fn lock_file(worktree: &Path) -> Result<std::fs::File> {
    let name = worktree
        .file_name()
        .and_then(|n| n.to_str())
        .context("a worktree has a name")?;
    let parent = worktree.parent().context("a worktree has a parent")?;
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(format!(".{name}.live")))?)
}
pub fn hold(worktree: &Path) -> Result<Held> {
    let file = lock_file(worktree)?;
    file.lock_shared()?;
    Ok(Held { _file: file })
}

/// What removing a worktree left behind: its branch, unless it was merged and so deleted, and
/// a ref holding what it had not committed. A branch costs no disk worth counting; a checkout
/// does.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Removed {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub branch_deleted: bool,
    pub kept: Option<String>,
}
impl Removed {
    pub fn describe(&self) -> String {
        let mut line = format!("removed worktree {}", self.path.display());
        match (&self.branch, self.branch_deleted) {
            (Some(branch), true) => {
                line.push_str(&format!("; branch {branch} was merged and is gone"))
            }
            (Some(branch), false) => line.push_str(&format!("; branch {branch} is kept")),
            (None, _) => {}
        }
        if let Some(kept) = &self.kept {
            line.push_str(&format!("; uncommitted changes are kept as {kept}"));
        }
        line
    }
}

/// Removes `worktree` if nothing holds it, keeping its uncommitted changes as a ref and its
/// branch unless the branch is merged. `None` when a process is still working in it.
pub fn remove(worktree: &Path) -> Result<Option<Removed>> {
    let file = lock_file(worktree)?;
    if file.try_lock().is_err() {
        return Ok(None);
    }
    let name = worktree
        .file_name()
        .and_then(|n| n.to_str())
        .context("a worktree has a name")?
        .to_owned();
    let main = main_checkout(worktree)?;
    let mut removed = Removed {
        path: worktree.to_path_buf(),
        ..Default::default()
    };
    removed.branch = git(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    let dirty = !git(
        worktree,
        &["status", "--porcelain", "--", ".", ":(exclude).orochi"],
    )
    .unwrap_or_default()
    .is_empty();
    if dirty {
        let commit = crate::workspaces::snapshot(worktree)?;
        let reference = format!("refs/orochi/kept/{name}");
        git(&main, &["update-ref", &reference, &commit])?;
        removed.kept = Some(reference);
    }
    let target = worktree.to_str().context("non-UTF8 worktree path")?;
    if !git_ok(&main, &["worktree", "remove", "--force", "--force", target])? {
        std::fs::remove_dir_all(worktree)?;
    }
    let _ = git_ok(&main, &["worktree", "prune"]);
    if let Some(branch) = &removed.branch {
        removed.branch_deleted = git_ok(&main, &["branch", "-d", branch])?;
    }
    drop(file);
    let _ = std::fs::remove_file(base(&main).join(format!(".{name}.live")));
    Ok(Some(removed))
}

/// `.orochi/` stays out of `git status` in every checkout of the repository, its worktrees
/// included: it holds checkouts, not source.
fn exclude_orochi(root: &Path) -> Result<()> {
    let common = PathBuf::from(git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    let path = common.join("info/exclude");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current
        .lines()
        .any(|l| matches!(l.trim(), ".orochi" | ".orochi/" | "/.orochi/"))
    {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("info has a parent"))?;
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".orochi/\n");
    std::fs::write(&path, next)?;
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        bail!(
            "git {}: {}",
            args[0],
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
fn git_ok(root: &Path, args: &[&str]) -> Result<bool> {
    Ok(Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?
        .success())
}
