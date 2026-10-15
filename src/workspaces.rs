//! Worktrees owned by one Orochi conversation, and the rule that none of them outlives it.
//!
//! A writing helper gets a detached worktree of its own under `.orochi/sessions/<key>/`,
//! started from a snapshot of the tree as it was when the helper was launched. The lead that
//! started it closes it (`close`): its changes are merged into the user's tree three-way,
//! opened as a pull request on a branch, or dropped — and in every case the worktree goes.
//! Whatever could not reach the tree (a conflict, a failed push, a conversation that ended
//! with the worktree still open) is kept as a patch file beside the manifest, which is small
//! and is not a worktree. `close_all` is the sweep at the end of a conversation; `prune` is
//! the same for every session directory whose process is gone (`hold` is how a live one says
//! it is not).
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// What becomes of a worktree's changes when it is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// Into the user's tree, file by file, three-way against the starting snapshot.
    Merge,
    /// A branch pushed to the remote and a pull request opened with `gh`.
    Pr,
    /// Kept only as a patch file.
    Discard,
}
impl Finish {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "merge" => Some(Self::Merge),
            "pr" | "pull-request" | "pull_request" => Some(Self::Pr),
            "discard" | "drop" | "close" => Some(Self::Discard),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Pr => "pr",
            Self::Discard => "discard",
        }
    }
}

/// What closing a worktree may need from outside the repository.
#[derive(Debug, Clone, Default)]
pub struct Tools {
    /// The GitHub CLI, where there is one (`setup::search_path`).
    pub gh: Option<PathBuf>,
    /// What a pull request's branch is named after, before the workspace name.
    pub branch_prefix: String,
}

/// What became of one closed worktree. Nothing in it is task text: paths and a branch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Closed {
    pub name: String,
    pub how: String,
    /// Paths the worktree changed against its starting snapshot.
    pub changed: Vec<String>,
    /// Paths written into the user's tree.
    pub merged: Vec<String>,
    /// Paths changed on both sides whose lines overlap, or deleted on one side.
    pub conflicts: Vec<String>,
    /// Where the change is kept when it did not reach the tree or a pull request.
    pub patch: Option<PathBuf>,
    pub branch: Option<String>,
    pub pr: Option<String>,
    /// What went wrong on the way, where something did.
    pub error: Option<String>,
}
impl Closed {
    /// One paragraph for the lead or the user.
    pub fn describe(&self) -> String {
        let mut text = format!("workspace `{}` closed", self.name);
        if self.changed.is_empty() {
            text.push_str(": it changed nothing");
        } else if !self.merged.is_empty() {
            text.push_str(&format!(
                ": {} merged into the working tree ({})",
                files(self.merged.len()),
                self.merged.join(", ")
            ));
        } else if let Some(pr) = &self.pr {
            text.push_str(&format!(": pull request {pr}"));
        } else if let Some(branch) = &self.branch {
            text.push_str(&format!(": branch `{branch}`"));
        }
        if !self.conflicts.is_empty() {
            text.push_str(&format!(
                "; conflicts in {}, so nothing was written",
                self.conflicts.join(", ")
            ));
        }
        if let Some(error) = &self.error {
            text.push_str(&format!("; {error}"));
        }
        if let Some(patch) = &self.patch {
            text.push_str(&format!(
                "; its changes are kept at {} (git apply --3way)",
                patch.display()
            ));
        }
        text
    }
}
fn files(n: usize) -> String {
    if n == 1 {
        "1 file".into()
    } else {
        format!("{n} files")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspaces {
    pub source: PathBuf,
    pub directory: PathBuf,
    pub workspaces: Vec<Workspace>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root: PathBuf,
    pub base: String,
    /// What the helper was for, which names its commit and pull request.
    #[serde(default)]
    pub title: String,
}

pub fn directory(root: &Path, id: &str) -> PathBuf {
    root.join(".orochi/sessions")
        .join(crate::context::hash(&[id.as_bytes()]))
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(git_bytes(root, args)?)?.trim().to_owned())
}
fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    ensure!(
        output.status.success(),
        "git {}: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}
/// Whether a git command that answers with its exit status succeeded.
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
const IDENTITY: [&str; 4] = [
    "-c",
    "user.name=Orochi",
    "-c",
    "user.email=orochi@localhost",
];
const SNAPSHOT_EXCLUDES: [&str; 2] = [":(glob,exclude)**/.orochi", ":(glob,exclude)**/.orochi/**"];

/// The sign that a conversation is still using its session directory: a lock on `live` in
/// it, held for as long as the process is. `prune` closes only what nobody holds.
pub struct Live {
    _file: std::fs::File,
}
pub fn hold(directory: &Path) -> Result<Option<Live>> {
    std::fs::create_dir_all(directory)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("live"))?;
    Ok(file.try_lock().is_ok().then_some(Live { _file: file }))
}
impl Workspaces {
    pub fn open(source: &Path, directory: &Path) -> Result<Self> {
        let source = source.canonicalize()?;
        std::fs::create_dir_all(directory)?;
        let directory = directory.canonicalize()?;
        let path = directory.join("workspaces.json");
        if path.exists() {
            let state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
            ensure!(
                state.source == source && state.directory == directory,
                "session workspace source changed"
            );
            return Ok(state);
        }
        let state = Self {
            source,
            directory,
            workspaces: vec![],
        };
        state.save()?;
        Ok(state)
    }
    fn save(&self) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(self)?)?;
        file.persist(self.directory.join("workspaces.json"))?;
        Ok(())
    }
    /// Snapshot tracked edits and nonignored new files using a private index. The user's HEAD,
    /// branch and index are untouched. Every worker starts with the state visible at launch.
    pub fn create(&mut self, name: &str) -> Result<PathBuf> {
        self.create_titled(name, "")
    }
    pub fn create_titled(&mut self, name: &str, title: &str) -> Result<PathBuf> {
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "invalid workspace name"
        );
        let mut ordinal = 1;
        let root = loop {
            let label = if ordinal == 1 {
                name.to_owned()
            } else {
                format!("{name}-{ordinal}")
            };
            let root = self.directory.join(&label);
            if !root.exists() {
                break (label, root);
            }
            ordinal += 1;
        };
        let repository = PathBuf::from(git(&self.source, &["rev-parse", "--show-toplevel"])?);
        let relative = self.source.strip_prefix(&repository)?;
        let head = git(&self.source, &["rev-parse", "HEAD"])
            .context("writable helpers require a Git repository with a commit")?;
        let index_dir = tempfile::tempdir_in(&self.directory)?;
        let index = index_dir.path().join("index");
        let indexed = |args: &[&str]| -> Result<String> {
            let output = Command::new("git")
                .current_dir(&repository)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .output()?;
            ensure!(
                output.status.success(),
                "cannot snapshot workspace: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_owned())
        };
        indexed(&["read-tree", &head])?;
        indexed(&[
            "add",
            "-A",
            "--",
            ".",
            ":(glob,exclude)**/.orochi",
            ":(glob,exclude)**/.orochi/**",
        ])?;
        let tree = indexed(&["write-tree"])?;
        let base = git(
            &self.source,
            &[
                "-c",
                "user.name=Orochi",
                "-c",
                "user.email=orochi@localhost",
                "commit-tree",
                &tree,
                "-p",
                &head,
                "-m",
                "Orochi workspace starting state",
            ],
        )?;
        git(
            &self.source,
            &[
                "worktree",
                "add",
                "--detach",
                root.1.to_str().context("non-UTF8 workspace path")?,
                &base,
            ],
        )?;
        self.workspaces.push(Workspace {
            name: root.0,
            root: root.1.join(relative),
            base,
            title: title.trim().chars().take(80).collect(),
        });
        self.save()?;
        Ok(root.1.join(relative))
    }
    pub fn git_common(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(git(
            &self.source,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?))
    }
    pub fn names(&self) -> Vec<String> {
        self.workspaces.iter().map(|w| w.name.clone()).collect()
    }
    /// The worktree itself (`root` is the directory inside it the session works in).
    fn worktree(&self, workspace: &Workspace) -> PathBuf {
        self.directory.join(&workspace.name)
    }

    /// Closes one worktree: what it changed goes where `how` says, and the worktree goes
    /// whatever happens. `title` names the commit and the pull request.
    pub fn close(&mut self, name: &str, how: Finish, tools: &Tools, title: &str) -> Result<Closed> {
        let index = self
            .workspaces
            .iter()
            .position(|w| w.name == name)
            .with_context(|| format!("no workspace named `{name}` in this conversation"))?;
        let worktree = self.worktree(&self.workspaces[index]);
        let base = self.workspaces[index].base.clone();
        let mut closed = Closed {
            name: name.to_owned(),
            how: how.word().to_owned(),
            ..Default::default()
        };
        if !worktree.join(".git").exists() {
            closed.error = Some("the worktree was already gone".into());
            self.remove(name)?;
            return Ok(closed);
        }
        let title = [title, self.workspaces[index].title.as_str()]
            .into_iter()
            .map(str::trim)
            .find(|t| !t.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Orochi workspace {name}"));
        let Some(head) = snapshot(&worktree, &title)? else {
            self.remove(name)?;
            return Ok(closed);
        };
        closed.changed = changed(&worktree, &base, &head)?
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        let keep = |closed: &mut Closed, this: &Self| match this
            .write_patch(&worktree, &base, &head, name)
        {
            Ok(path) => closed.patch = Some(path),
            Err(error) => closed.error = Some(format!("{error:#}")),
        };
        match how {
            Finish::Discard => keep(&mut closed, self),
            Finish::Merge => match self.merge(&worktree, &base, &head, name) {
                Ok((merged, conflicts)) if conflicts.is_empty() => closed.merged = merged,
                Ok((_, conflicts)) => {
                    closed.conflicts = conflicts;
                    keep(&mut closed, self);
                }
                Err(error) => {
                    closed.error = Some(format!("{error:#}"));
                    keep(&mut closed, self);
                }
            },
            Finish::Pr => {
                if let Err(error) =
                    self.pull_request(&worktree, &base, &head, name, &title, tools, &mut closed)
                {
                    closed.error = Some(format!("{error:#}"));
                }
                if closed.branch.is_none() {
                    keep(&mut closed, self);
                }
            }
        }
        self.remove(name)?;
        Ok(closed)
    }

    /// Takes the worktree away and forgets it. The directory is removed whatever is in it;
    /// `close` has already kept what should be kept.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let Some(index) = self.workspaces.iter().position(|w| w.name == name) else {
            return Ok(());
        };
        let worktree = self.worktree(&self.workspaces[index]);
        if worktree.exists() {
            let path = worktree.to_str().context("non-UTF8 workspace path")?;
            if !git_ok(&self.source, &["worktree", "remove", "--force", path])? {
                // Not registered as a worktree any more: take the directory itself.
                std::fs::remove_dir_all(&worktree)?;
            }
        }
        let _ = git_ok(&self.source, &["worktree", "prune"]);
        self.workspaces.remove(index);
        self.save()
    }

    /// The sweep: every worktree still open is closed, its changes kept as a patch. Nothing
    /// is merged unasked, and nothing is lost.
    pub fn close_all(&mut self, tools: &Tools) -> Result<Vec<Closed>> {
        let mut closed = vec![];
        for name in self.names() {
            match self.close(&name, Finish::Discard, tools, "") {
                Ok(mut result) => {
                    result.how = "closed".into();
                    closed.push(result);
                }
                Err(error) => {
                    // Keep going: one worktree that cannot be closed must not leave the rest.
                    closed.push(Closed {
                        name,
                        how: "closed".into(),
                        error: Some(format!("{error:#}")),
                        ..Default::default()
                    });
                }
            }
        }
        Ok(closed)
    }

    /// The change from the starting snapshot, as a patch file beside the manifest.
    fn write_patch(&self, worktree: &Path, base: &str, head: &str, name: &str) -> Result<PathBuf> {
        let bytes = git_bytes(worktree, &["diff", "--binary", base, head])?;
        let mut path = self.directory.join(format!("{name}.patch"));
        let mut ordinal = 2;
        while path.exists() {
            path = self.directory.join(format!("{name}-{ordinal}.patch"));
            ordinal += 1;
        }
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Three-way, file by file: each path the worktree changed, against what it started from
    /// and what the user's tree has now. All or nothing: a single conflict writes nothing.
    fn merge(
        &self,
        worktree: &Path,
        base: &str,
        head: &str,
        name: &str,
    ) -> Result<(Vec<String>, Vec<String>)> {
        use crate::collaboration::{Change, Content, Entry, commit, entry, merge_text};
        let top = PathBuf::from(git(&self.source, &["rev-parse", "--show-toplevel"])?);
        let mut changes = vec![];
        let mut conflicts = vec![];
        for (status, path) in changed(worktree, base, head)? {
            let target = top.join(&path);
            let theirs = match status.as_str() {
                "D" => None,
                _ => match entry(&worktree.join(&path))? {
                    Some(Entry::File { executable, .. }) => {
                        Some((std::fs::read(worktree.join(&path))?, executable))
                    }
                    _ => {
                        conflicts.push(path);
                        continue;
                    }
                },
            };
            let before = match status.as_str() {
                "A" => None,
                _ => Some(git_bytes(worktree, &["show", &format!("{base}:{path}")])?),
            };
            let expected = entry(&target)?;
            let ours = match expected {
                None => None,
                Some(Entry::File { .. }) => Some(std::fs::read(&target)?),
                Some(Entry::Unsupported) => {
                    conflicts.push(path);
                    continue;
                }
            };
            if ours.as_deref() == theirs.as_ref().map(|(bytes, _)| bytes.as_slice()) {
                continue;
            }
            let content = |theirs: Option<(Vec<u8>, bool)>| match theirs {
                Some((bytes, executable)) => Content::Bytes(bytes, executable),
                None => Content::Delete,
            };
            if ours == before {
                changes.push(Change {
                    path: PathBuf::from(&path),
                    content: content(theirs),
                    expected,
                });
                continue;
            }
            let (Some(ours), Some(before), Some((theirs, executable))) = (ours, before, theirs)
            else {
                conflicts.push(path);
                continue;
            };
            let texts = (
                String::from_utf8(ours),
                String::from_utf8(before),
                String::from_utf8(theirs),
            );
            let (Ok(ours), Ok(before), Ok(theirs)) = texts else {
                conflicts.push(path);
                continue;
            };
            let (merged, count) = merge_text(&ours, &before, &theirs, ["yours", "base", name])?;
            if count > 0 {
                conflicts.push(path);
                continue;
            }
            changes.push(Change {
                path: PathBuf::from(&path),
                content: Content::Bytes(merged.into_bytes(), executable),
                expected,
            });
        }
        if !conflicts.is_empty() {
            return Ok((vec![], conflicts));
        }
        let written = commit(&top, &changes.iter().collect::<Vec<_>>())?;
        Ok((
            written.iter().map(|p| p.display().to_string()).collect(),
            vec![],
        ))
    }

    /// The change replayed onto the user's current HEAD on a branch of its own, pushed, and
    /// opened as a pull request. A branch that could not be pushed or opened stays local and
    /// is named in the result; the worktree goes either way.
    #[allow(clippy::too_many_arguments)]
    fn pull_request(
        &self,
        worktree: &Path,
        base: &str,
        head: &str,
        name: &str,
        title: &str,
        tools: &Tools,
        closed: &mut Closed,
    ) -> Result<()> {
        let user_head = git(&self.source, &["rev-parse", "HEAD"])?;
        let user_branch = git(&self.source, &["branch", "--show-current"]).unwrap_or_default();
        git(worktree, &["checkout", "-q", "--detach", &user_head])?;
        let patch = self.directory.join(format!(".{name}.replay.patch"));
        std::fs::write(
            &patch,
            git_bytes(worktree, &["diff", "--binary", base, head])?,
        )?;
        let replayed = git_ok(
            worktree,
            &[
                "apply",
                "--3way",
                "--index",
                patch.to_str().context("non-UTF8 path")?,
            ],
        )?;
        let _ = std::fs::remove_file(&patch);
        if !replayed {
            git(worktree, &["reset", "-q", "--hard", head])?;
            bail!("its changes do not replay onto the current HEAD");
        }
        if git_ok(worktree, &["diff", "--cached", "--quiet"])? {
            bail!("the current HEAD already has its changes");
        }
        // The user's own identity signs the commit where the repository has one; Orochi's
        // only where it has none.
        if !git_ok(worktree, &["commit", "-q", "-m", title])? {
            let mut args = IDENTITY.to_vec();
            args.extend(["commit", "-q", "-m", title]);
            git(worktree, &args)?;
        }
        let branch = {
            let wanted = format!("{}{name}", tools.branch_prefix);
            let mut branch = wanted.clone();
            let mut ordinal = 2;
            while git_ok(
                &self.source,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )? {
                branch = format!("{wanted}-{ordinal}");
                ordinal += 1;
            }
            branch
        };
        git(worktree, &["checkout", "-q", "-b", &branch])?;
        closed.branch = Some(branch.clone());
        let remote = git(&self.source, &["remote"])?
            .lines()
            .next()
            .map(str::to_owned)
            .context("the repository has no remote to push to")?;
        git(worktree, &["push", "-q", "-u", &remote, &branch])?;
        let gh = tools.gh.as_ref().context(
            "`gh` is not installed (`orochi setup` puts it in), so no pull request was opened",
        )?;
        let body = format!(
            "Opened by Orochi from the worktree of helper `{name}`.\n\nChanged: {}",
            closed.changed.join(", ")
        );
        let mut command = Command::new(gh);
        command
            .current_dir(worktree)
            .args([
                "pr", "create", "--head", &branch, "--title", title, "--body", &body,
            ])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(std::process::Stdio::null());
        if !user_branch.is_empty() {
            command.args(["--base", &user_branch]);
        }
        let output = command.output().context("running gh")?;
        ensure!(
            output.status.success(),
            "gh pr create: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
                .trim()
        );
        let url = String::from_utf8_lossy(&output.stdout)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        closed.pr = Some(url);
        Ok(())
    }
}

/// Commits everything the worktree holds against what it started from, as Orochi, and
/// says the commit; `None` where it changed nothing.
fn snapshot(worktree: &Path, title: &str) -> Result<Option<String>> {
    let mut add = vec!["add", "-A", "--", "."];
    add.extend(SNAPSHOT_EXCLUDES);
    git(worktree, &add)?;
    if git_ok(worktree, &["diff", "--cached", "--quiet"])? {
        return Ok(None);
    }
    let mut args = IDENTITY.to_vec();
    args.extend(["commit", "-q", "--no-verify", "-m", title]);
    git(worktree, &args)?;
    Ok(Some(git(worktree, &["rev-parse", "HEAD"])?))
}

/// `(status, path)` for each path `head` differs from `base` in, relative to the repository.
fn changed(worktree: &Path, base: &str, head: &str) -> Result<Vec<(String, String)>> {
    Ok(git(
        worktree,
        &["diff", "--name-status", "--no-renames", base, head],
    )?
    .lines()
    .filter_map(|line| {
        let (status, path) = line.split_once('\t')?;
        Some((status.trim().to_owned(), path.trim().to_owned()))
    })
    .collect())
}

/// What one worktree has changed so far, for the report that hands it to the lead.
pub fn summary(root: &Path) -> Option<String> {
    let mut args = vec!["status", "--porcelain", "--untracked-files=all", "--", "."];
    args.extend(SNAPSHOT_EXCLUDES);
    let count = git(root, &args).ok()?.lines().count();
    Some(match count {
        0 => "no changes".to_owned(),
        n => format!("{} changed", files(n)),
    })
}

/// Closes every worktree of every session under `root` that no process holds, and removes a
/// session directory left with nothing in it. What was closed is reported per session.
pub fn prune(root: &Path, tools: &Tools) -> Result<Vec<(PathBuf, Vec<Closed>)>> {
    let sessions = root.join(".orochi/sessions");
    let mut report = vec![];
    let Ok(entries) = std::fs::read_dir(&sessions) else {
        return Ok(report);
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let directory = entry.path();
        let manifest = directory.join("workspaces.json");
        if !manifest.is_file() {
            continue;
        }
        // Held means a conversation is using it right now; its own end sweeps it.
        let Some(_live) = hold(&directory)? else {
            continue;
        };
        let state: Workspaces = match serde_json::from_slice(&std::fs::read(&manifest)?) {
            Ok(state) => state,
            Err(_) => continue,
        };
        let mut state = match Workspaces::open(&state.source, &directory) {
            Ok(state) => state,
            Err(error) => {
                report.push((
                    directory,
                    vec![Closed {
                        how: "skipped".into(),
                        error: Some(format!("{error:#}")),
                        ..Default::default()
                    }],
                ));
                continue;
            }
        };
        let closed = state.close_all(tools)?;
        let leftovers = std::fs::read_dir(&directory)?
            .filter_map(|e| e.ok())
            .filter(|e| !matches!(e.file_name().to_str(), Some("workspaces.json" | "live")))
            .count();
        let removed = leftovers == 0;
        if removed {
            drop(_live);
            let _ = std::fs::remove_dir_all(&directory);
        }
        if !closed.is_empty() || removed {
            report.push((directory, closed));
        }
    }
    Ok(report)
}
