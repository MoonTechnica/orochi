//! Merging a conversation's worktree back into the user's tree, in two steps the lead asks
//! for and Orochi performs (`docs/manager-design.md` §4.5): a trial that merges, checks and
//! reports, in a worktree of its own; then an apply that re-snapshots the user's tree and writes
//! only the merged difference, with every target checked against what the merge was computed
//! from. The user's HEAD, branch and index are never touched. An applied trial closes the
//! helper's worktree and its own, as `finish_workspace` closes one: no worktree outlives the
//! conversation, and a trial left behind is swept with the conversation's worktrees.
use crate::{
    collaboration::workspace::{self, Change, Content},
    types::CheckResult,
    workspaces::{Workspace, snapshot},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Where a trial stands. `Resolving` and `Applying` are the console's; the rest are the git's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Clean,
    Conflicts,
    Resolving,
    Failed,
    Applying,
    Applied,
    Declined,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    pub id: String,
    pub workspace: String,
    pub n: usize,
    pub state: State,
    /// The trial's own worktree, where the merged tree is checked out and checks run.
    pub root: PathBuf,
    /// That worktree's top, which is what `git worktree remove` takes.
    #[serde(default)]
    pub worktree: PathBuf,
    pub ours: String,
    pub theirs: String,
    pub base: String,
    pub merged: String,
    /// Paths the merge would change in the user's tree.
    pub files: Vec<String>,
    /// Paths both sides changed since the base: what a clean merge cannot vouch for.
    pub overlap: Vec<String>,
    pub conflicts: Vec<String>,
    pub checks: Vec<CheckResult>,
    /// Why it failed, in one line, when it did.
    pub error: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "git {}: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Merges `workspace` into the user's tree at `source` as it is now, in a worktree under
/// `directory`, without running any check. `n` numbers the trials of one workspace.
pub fn trial(source: &Path, workspace: &Workspace, directory: &Path, n: usize) -> Result<Trial> {
    let ours = snapshot(source).context("cannot snapshot the working tree")?;
    let theirs = snapshot(&workspace.root).context("cannot snapshot the workspace")?;
    let base = workspace.base.clone();
    let merge_base = format!("--merge-base={base}");
    let output = Command::new("git")
        .current_dir(source)
        .args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &merge_base,
            &ours,
            &theirs,
        ])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = lines(&stdout).into_iter();
    let (merged, conflicts) = match output.status.code() {
        Some(0) => (out.next().context("merge-tree wrote no tree")?, vec![]),
        Some(1) => (
            out.next().context("merge-tree wrote no tree")?,
            out.collect::<Vec<_>>(),
        ),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("unknown option") || stderr.contains("usage:") {
                bail!("git 2.40 or newer is needed for merge trials");
            }
            bail!("git merge-tree: {}", stderr.trim());
        }
    };
    let changed = |from: &str, to: &str| -> Result<Vec<String>> {
        Ok(lines(&git(
            source,
            &["diff", "--name-only", "--no-renames", from, to],
        )?))
    };
    let files = changed(&ours, &merged)?;
    let mine = changed(&base, &ours)?;
    let overlap: Vec<String> = changed(&base, &theirs)?
        .into_iter()
        .filter(|path| mine.contains(path))
        .collect();
    let commit = git(
        source,
        &[
            "-c",
            "user.name=Orochi",
            "-c",
            "user.email=orochi@localhost",
            "commit-tree",
            &merged,
            "-p",
            &ours,
            "-m",
            &format!("Orochi merge trial of {}", workspace.name),
        ],
    )?;
    let trials = directory.join("trials");
    std::fs::create_dir_all(&trials)?;
    let root = trials.join(format!("{}-{n}", workspace.name));
    ensure!(!root.exists(), "trial {} already exists", root.display());
    git(
        source,
        &[
            "worktree",
            "add",
            "--detach",
            root.to_str().context("non-UTF8 trial path")?,
            &commit,
        ],
    )?;
    let relative = relative_cwd(source)?;
    Ok(Trial {
        id: format!("{}-{n}", workspace.name),
        workspace: workspace.name.clone(),
        n,
        state: if conflicts.is_empty() {
            State::Clean
        } else {
            State::Conflicts
        },
        root: root.join(relative),
        worktree: root,
        ours,
        theirs,
        base,
        merged,
        files,
        overlap,
        conflicts,
        checks: vec![],
        error: None,
    })
}

/// `source` below its repository's top, so a trial started from a subdirectory runs there.
fn relative_cwd(source: &Path) -> Result<PathBuf> {
    let repository = PathBuf::from(git(source, &["rev-parse", "--show-toplevel"])?);
    Ok(source
        .canonicalize()?
        .strip_prefix(&repository)
        .map(Path::to_path_buf)
        .unwrap_or_default())
}

/// After a resolution seat worked in the trial's worktree: the markers that remain, and the
/// tree as it is now, which is what an apply would write.
pub fn rechecked(trial: &mut Trial) -> Result<()> {
    let remaining: Vec<String> = trial
        .conflicts
        .iter()
        .filter(|path| workspace::has_markers(&trial.root.join(path)))
        .cloned()
        .collect();
    if !remaining.is_empty() {
        trial.conflicts = remaining;
        trial.state = State::Conflicts;
        return Ok(());
    }
    // What the seat left is the merged result now; its snapshot is what gets applied.
    let resolved = snapshot(&trial.root)?;
    trial.merged = git(&trial.root, &["rev-parse", &format!("{resolved}^{{tree}}")])?;
    trial.files = lines(&git(
        &trial.root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            &trial.ours,
            &trial.merged,
        ],
    )?);
    trial.conflicts.clear();
    trial.state = State::Clean;
    Ok(())
}

/// Writes the trial's merged difference into the user's tree. The tree is snapshotted again
/// first: if it moved since the trial, nothing is written and the trial is `Stale`. Once
/// written, the helper's worktree (in the manifest under `directory`) and the trial's are
/// closed: what they held is in the tree now.
pub fn apply(source: &Path, directory: &Path, trial: &mut Trial) -> Result<Vec<PathBuf>> {
    ensure!(
        trial.state == State::Clean,
        "only a clean trial can be applied; trial {} is {:?}",
        trial.id,
        trial.state
    );
    let now = snapshot(source)?;
    let same = git(
        source,
        &[
            "rev-parse",
            &format!("{now}^{{tree}}"),
            &format!("{}^{{tree}}", trial.ours),
        ],
    )?;
    let mut trees = same.lines();
    if trees.next() != trees.next() {
        trial.state = State::Stale;
        bail!("the working tree changed since the trial; run merge_trial again");
    }
    let status = git(
        source,
        &[
            "diff",
            "--name-status",
            "--no-renames",
            "-z",
            &trial.ours,
            &trial.merged,
        ],
    )?;
    let mut fields = status.split('\0').filter(|f| !f.is_empty());
    let mut changes = vec![];
    while let (Some(kind), Some(path)) = (fields.next(), fields.next()) {
        let relative = PathBuf::from(path);
        let expected = workspace::entry(&source.join(&relative))?;
        let content = match kind.chars().next() {
            Some('D') => Content::Delete,
            _ => {
                let spec = format!("{}:{path}", trial.merged);
                let blob = Command::new("git")
                    .current_dir(source)
                    .args(["cat-file", "blob", &spec])
                    .output()?;
                ensure!(blob.status.success(), "cannot read {path} from the merge");
                let mode = git(source, &["ls-tree", &trial.merged, "--", path])?;
                Content::Bytes(blob.stdout, mode.starts_with("100755"))
            }
        };
        changes.push(Change {
            path: relative,
            content,
            expected,
        });
    }
    let refs: Vec<&Change> = changes.iter().collect();
    let written = workspace::commit(source, &refs)?;
    trial.state = State::Applied;
    trial.files = written.iter().map(|p| p.display().to_string()).collect();
    // The change is in the tree; the worktrees that carried it are closed, the helper's
    // through the manifest so the conversation forgets it too.
    if let Ok(mut work) = crate::workspaces::Workspaces::open(source, directory) {
        let _ = work.remove(&trial.workspace);
    }
    let _ = remove_worktree(source, &trial.worktree);
    Ok(written)
}

fn remove_worktree(source: &Path, worktree: &Path) -> Result<()> {
    if !worktree.exists() {
        return Ok(());
    }
    let path = worktree.to_str().context("non-UTF8 trial path")?;
    if git(source, &["worktree", "remove", "--force", path]).is_err() {
        std::fs::remove_dir_all(worktree)?;
    }
    let _ = git(source, &["worktree", "prune"]);
    Ok(())
}

/// Every trial worktree under `directory`, closed: the conversation's end, `/new` and a
/// resumed thread sweep them as they sweep the helpers' worktrees.
pub fn sweep_trials(source: &Path, directory: &Path) -> Result<usize> {
    let trials = directory.join("trials");
    let Ok(entries) = std::fs::read_dir(&trials) else {
        return Ok(0);
    };
    let mut count = 0;
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            remove_worktree(source, &entry.path())?;
            count += 1;
        }
    }
    Ok(count)
}
