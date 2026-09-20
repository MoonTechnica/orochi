//! Workspace copies, three-way merges from a shared snapshot, and guarded writes.
//! Paths excluded from copies (repository metadata, credentials, caches) are never
//! compared, merged or written.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    process::Stdio,
};

const MAX_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FILES: usize = 20000;
const MAX_TEXT_MERGE: usize = 1024 * 1024;

fn excluded(name: &str) -> bool {
    name.starts_with(".env")
        || matches!(
            name,
            ".git" | ".orochi" | "target" | "node_modules" | ".venv" | "__pycache__"
        )
}

fn walk(source: &Path) -> impl Iterator<Item = walkdir::Result<walkdir::DirEntry>> {
    walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !excluded(&e.file_name().to_string_lossy()))
}

/// Copy regular project files; do not traverse symlinks or copy repository metadata,
/// credentials, build caches or our own output. Fail rather than silently truncate.
pub fn copy_workspace(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir(target)?;
    let mut total = 0u64;
    let mut count = 0usize;
    for entry in walk(source) {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }
        ensure!(
            !entry.file_type().is_symlink(),
            "workspace contains symlink: {}",
            entry.path().display()
        );
        let dest = target.join(entry.path().strip_prefix(source)?);
        if entry.file_type().is_dir() {
            std::fs::create_dir(&dest)?;
        } else {
            ensure!(
                entry.file_type().is_file(),
                "workspace contains a special file"
            );
            total = total.saturating_add(entry.metadata()?.len());
            count += 1;
            ensure!(
                total <= MAX_BYTES && count <= MAX_FILES,
                "collaboration workspace exceeds copy budget"
            );
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    File {
        hash: [u8; 32],
        executable: bool,
    },
    /// Symlinks and special files are never merged or replaced.
    Unsupported,
}

pub type Tree = BTreeMap<PathBuf, Entry>;

fn executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn entry(path: &Path) -> Result<Option<Entry>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok((!metadata.is_dir()).then_some(Entry::Unsupported));
    }
    Ok(Some(Entry::File {
        hash: Sha256::digest(std::fs::read(path)?).into(),
        executable: executable(&metadata),
    }))
}

pub fn scan(root: &Path) -> Result<Tree> {
    let mut tree = Tree::new();
    let mut total = 0u64;
    for item in walk(root) {
        let item = item?;
        if item.depth() == 0 || item.file_type().is_dir() {
            continue;
        }
        let relative = item.path().strip_prefix(root)?.to_path_buf();
        if item.file_type().is_file() {
            total = total.saturating_add(item.metadata()?.len());
            ensure!(
                total <= MAX_BYTES && tree.len() < MAX_FILES,
                "workspace exceeds comparison budget"
            );
        }
        if let Some(entry) = entry(item.path())? {
            tree.insert(relative, entry);
        }
    }
    Ok(tree)
}

/// Paths that differ between two trees.
pub fn changed(base: &Path, other: &Path) -> Result<Vec<PathBuf>> {
    let (before, after) = (scan(base)?, scan(other)?);
    Ok(before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub path: PathBuf,
    pub sources: Vec<String>,
    pub reason: String,
    /// Conflict markers were written to the path.
    pub markers: bool,
}

#[derive(Debug)]
pub enum Content {
    Bytes(Vec<u8>, bool),
    Delete,
}

#[derive(Debug)]
pub struct Change {
    pub path: PathBuf,
    pub content: Content,
    /// State of `path` in the target when the merge was computed.
    pub expected: Option<Entry>,
}

#[derive(Debug, Default)]
pub struct Merge {
    pub changes: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    /// Marker text for conflicts that can be edited in place.
    pub marked: Vec<Change>,
}

fn read_text(path: Option<&Path>) -> Option<String> {
    let Some(path) = path else {
        return Some(String::new());
    };
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() <= MAX_TEXT_MERGE)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

/// Line-based merge through `git merge-file`, run outside any repository.
/// Returns the merged text and the number of conflicts.
pub fn merge_text(
    ours: &str,
    base: &str,
    theirs: &str,
    labels: [&str; 3],
) -> Result<(String, usize)> {
    let temp = tempfile::tempdir()?;
    let paths = ["ours", "base", "theirs"].map(|n| temp.path().join(n));
    for (path, text) in paths.iter().zip([ours, base, theirs]) {
        std::fs::write(path, text)?;
    }
    let output = std::process::Command::new("git")
        .args(["merge-file", "-p", "--diff3"])
        .args(labels.iter().flat_map(|l| ["-L", l]))
        .args(&paths)
        .current_dir(temp.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .context("git is required for line merges")?;
    match output.status.code() {
        Some(code @ 0..=127) => Ok((String::from_utf8(output.stdout)?, code as usize)),
        _ => bail!("git merge-file failed"),
    }
}

/// Apply the changes `theirs` made relative to `base` onto `ours`.
pub fn three_way(base: &Path, ours: &Path, theirs: &Path, labels: [&str; 3]) -> Result<Merge> {
    let (b_tree, o_tree, t_tree) = (scan(base)?, scan(ours)?, scan(theirs)?);
    let paths: BTreeSet<&PathBuf> = b_tree
        .keys()
        .chain(t_tree.keys())
        .filter(|p| b_tree.get(*p) != t_tree.get(*p))
        .collect();
    let mut merge = Merge::default();
    for path in paths {
        let (b, o, t) = (b_tree.get(path), o_tree.get(path), t_tree.get(path));
        if o == t {
            continue;
        }
        let conflict = |reason: &str, markers: bool| Conflict {
            path: path.clone(),
            sources: vec![labels[0].into(), labels[2].into()],
            reason: reason.into(),
            markers,
        };
        if [b, o, t].contains(&Some(&Entry::Unsupported)) {
            merge
                .conflicts
                .push(conflict("symlink or special file", false));
            continue;
        }
        if o == b {
            merge.changes.push(Change {
                path: path.clone(),
                content: match t {
                    Some(Entry::File { executable, .. }) => {
                        Content::Bytes(std::fs::read(theirs.join(path))?, *executable)
                    }
                    _ => Content::Delete,
                },
                expected: o.copied(),
            });
            continue;
        }
        let (Some(Entry::File { executable, .. }), Some(Entry::File { .. })) = (o, t) else {
            merge.conflicts.push(conflict(
                "modified on one side and deleted on the other",
                false,
            ));
            continue;
        };
        let texts = (
            read_text(Some(&ours.join(path))),
            read_text(b.map(|_| base.join(path)).as_deref()),
            read_text(Some(&theirs.join(path))),
        );
        let (Some(o_text), Some(b_text), Some(t_text)) = texts else {
            merge
                .conflicts
                .push(conflict("binary or oversized file", false));
            continue;
        };
        let (merged, conflicts) = merge_text(&o_text, &b_text, &t_text, labels)?;
        let change = Change {
            path: path.clone(),
            content: Content::Bytes(merged.into_bytes(), *executable),
            expected: o.copied(),
        };
        if conflicts == 0 {
            merge.changes.push(change);
        } else {
            merge.conflicts.push(conflict("overlapping edits", true));
            merge.marked.push(change);
        }
    }
    Ok(merge)
}

/// Join a relative path below `root`, refusing traversal and symlinked ancestors.
fn contained(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "unsafe relative path: {}",
        relative.display()
    );
    let mut current = root.to_path_buf();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(m) => ensure!(m.is_dir(), "{} is not a directory", current.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(root.join(relative))
}

/// Write changes after verifying each target still matches the merge input. Everything is
/// staged before any file is replaced, so a detected concurrent edit changes nothing.
pub fn commit(root: &Path, changes: &[&Change]) -> Result<Vec<PathBuf>> {
    let mut staged = vec![];
    for change in changes {
        let target = contained(root, &change.path)?;
        ensure!(
            entry(&target)? == change.expected,
            "{} changed while the result was being applied",
            change.path.display()
        );
        if let Content::Bytes(bytes, executable) = &change.content {
            let parent = target.parent().context("target has no parent")?;
            std::fs::create_dir_all(parent)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            std::io::Write::write_all(&mut temp, bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = match change.expected {
                    Some(Entry::File { .. }) => std::fs::metadata(&target)?.permissions().mode(),
                    _ => 0o644,
                };
                let mode = if *executable {
                    mode | 0o111
                } else {
                    mode & !0o111
                };
                temp.as_file()
                    .set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))?;
            }
            staged.push((target, Some(temp)));
        } else {
            staged.push((target, None));
        }
    }
    let mut written = vec![];
    for ((target, temp), change) in staged.into_iter().zip(changes) {
        match temp {
            Some(temp) => {
                temp.persist(&target)?;
            }
            None => std::fs::remove_file(&target)?,
        }
        written.push(change.path.clone());
    }
    Ok(written)
}

pub fn has_markers(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| {
        text.lines().any(|line| {
            line.starts_with("<<<<<<< ")
                || line.starts_with(">>>>>>> ")
                || line.starts_with("||||||| ")
        })
    })
}
