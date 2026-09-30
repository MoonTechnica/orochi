//! The Files pane: the thread's working tree, read from the disk (`docs/files-pane-design.md`).
//!
//! This is the one part of the window that is not a query over the store, on purpose: the store
//! holds what was said and done, and the tree is the user's own disk. Nothing read here is ever
//! written anywhere. Every path goes through `within`, because the repository is not trusted and
//! a path can arrive from an agent's tool call.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// What the viewer reads of one file: the cap patches already have.
pub const READ_LIMIT: u64 = 512 * 1024;
/// An image larger than this is described rather than drawn.
const IMAGE_LIMIT: u64 = 2 * 1024 * 1024;
/// Status lines kept from one `git status`; past it the header says the state is partial.
const STATUS_LIMIT: usize = 20_000;
/// Entries listed from one directory.
const LIST_LIMIT: usize = 5_000;
/// Files considered by a name search when the tree is not a repository.
const WALK_LIMIT: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Conflict,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
}

impl Status {
    /// Which of two states a directory shows when both are under it.
    fn weight(self) -> u8 {
        match self {
            Status::Conflict => 6,
            Status::Modified => 5,
            Status::Added => 4,
            Status::Deleted => 3,
            Status::Renamed => 2,
            Status::Untracked => 1,
            Status::Ignored => 0,
        }
    }

    /// The two columns of `git status --porcelain`, as a person reads them.
    fn parse(x: u8, y: u8) -> Option<Self> {
        Some(match (x, y) {
            (b'!', b'!') => Status::Ignored,
            (b'?', b'?') => Status::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => Status::Conflict,
            (b'R' | b'C', _) => Status::Renamed,
            (b'A', _) => Status::Added,
            (b'D', _) | (_, b'D') => Status::Deleted,
            (b'M' | b'T', _) | (_, b'M' | b'T') => Status::Modified,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    /// Relative to the thread's tree, `/`-separated.
    pub path: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub status: Option<Status>,
    /// A directory: the strongest state of anything under it.
    pub within: Option<Status>,
    /// Deleted in the tree but still known to git: listed so the deletion is visible, never
    /// opened.
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileText {
    pub path: String,
    pub bytes: u64,
    pub modified_at: i64,
    pub text: String,
    pub lines: u64,
    pub truncated: bool,
    pub binary: bool,
    pub image: Option<String>,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub path: String,
    pub line: Option<u64>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Name,
    Content,
}

impl By {
    pub fn parse(by: &str) -> Result<Self> {
        Ok(match by {
            "name" => By::Name,
            "content" => By::Content,
            other => bail!("no search by {other}"),
        })
    }
}

/// The whole tree's git state, read once and folded up to every directory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TreeState {
    pub repository: bool,
    pub partial: bool,
    pub read_at: i64,
    #[serde(skip)]
    own: HashMap<String, Status>,
    #[serde(skip)]
    under: HashMap<String, Status>,
    #[serde(skip)]
    ignored: HashSet<String>,
}

impl TreeState {
    pub fn status(&self, path: &str) -> Option<Status> {
        if let Some(status) = self.own.get(path) {
            return Some(*status);
        }
        // Git names an ignored directory once; everything inside it is ignored with it.
        let mut at = path;
        loop {
            if self.ignored.contains(at) {
                return Some(Status::Ignored);
            }
            match at.rsplit_once('/') {
                Some((parent, _)) => at = parent,
                None => return None,
            }
        }
    }

    pub fn within(&self, path: &str) -> Option<Status> {
        self.under.get(path).copied()
    }

    /// Files git still knows that are gone from this directory.
    fn deleted_in(&self, dir: &str) -> Vec<String> {
        let mut gone: Vec<String> = self
            .own
            .iter()
            .filter(|(_, status)| **status == Status::Deleted)
            .filter_map(|(path, _)| {
                let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
                (parent == dir).then(|| name.to_owned())
            })
            .collect();
        gone.sort();
        gone
    }
}

/// A `git` that cannot be made to run anything by the repository it reads. `core.fsmonitor`
/// is a command a repository's own config may name, and `status` would start it.
fn git(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "color.ui=false",
        ])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = git(cwd).args(args).output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// The absolute path a relative one names inside `cwd`, or why it is refused: absolute, a `..`,
/// `.git` anywhere, or anything that resolves outside `cwd`. The canonical path is what is then
/// opened, so the check and the read see one file.
pub fn within(cwd: &Path, relative: &str) -> Result<PathBuf> {
    let root = cwd
        .canonicalize()
        .with_context(|| format!("{} is not there", cwd.display()))?;
    let relative = relative.trim_matches('/');
    let relative = if relative == "." { "" } else { relative };
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(name) if name != ".git" => {}
            Component::CurDir => {}
            _ => bail!("{relative} is outside this thread's folder"),
        }
    }
    let full = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("{relative} is not there"))?;
    let inside = full
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("{relative} is outside this thread's folder"))?;
    ensure!(
        !inside.components().any(|c| c.as_os_str() == ".git"),
        "{relative} is outside this thread's folder"
    );
    Ok(full)
}

/// Reads the tree's git state. Outside a repository the state is empty and says so.
pub fn read_state(cwd: &Path) -> TreeState {
    let now = now_ms();
    let Some(prefix) = git_output(cwd, &["rev-parse", "--show-prefix"]) else {
        return TreeState {
            read_at: now,
            ..TreeState::default()
        };
    };
    let prefix = String::from_utf8_lossy(&prefix).trim().to_owned();
    let mut state = TreeState {
        repository: true,
        read_at: now,
        ..TreeState::default()
    };
    let Ok(mut child) = git(cwd)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--ignored=matching",
            "--untracked-files=all",
            "--",
            ".",
        ])
        .stdout(Stdio::piped())
        .spawn()
    else {
        return state;
    };
    let mut reader = std::io::BufReader::new(child.stdout.take().expect("piped"));
    let mut records = 0;
    let mut field = Vec::new();
    loop {
        field.clear();
        if reader.read_until(0, &mut field).unwrap_or(0) == 0 {
            break;
        }
        if field.last() == Some(&0) {
            field.pop();
        }
        if field.len() < 4 {
            continue;
        }
        records += 1;
        if records > STATUS_LIMIT {
            state.partial = true;
            break;
        }
        let (x, y) = (field[0], field[1]);
        // A rename or copy names its source in the next field.
        if matches!(x, b'R' | b'C') {
            let mut source = Vec::new();
            let _ = reader.read_until(0, &mut source);
        }
        let Some(status) = Status::parse(x, y) else {
            continue;
        };
        // Porcelain paths are relative to the repository, whatever directory git ran in.
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let Some(path) = path.strip_prefix(&prefix) else {
            continue;
        };
        let path = path.trim_end_matches('/').to_owned();
        if status == Status::Ignored {
            state.ignored.insert(path);
            continue;
        }
        let mut at = path.as_str();
        while let Some((parent, _)) = at.rsplit_once('/') {
            let entry = state.under.entry(parent.to_owned()).or_insert(status);
            if status.weight() > entry.weight() {
                *entry = status;
            }
            at = parent;
        }
        state.own.insert(path, status);
    }
    let _ = child.kill();
    let _ = child.wait();
    state
}

/// One directory level: directories first, then everything else, each case-insensitively.
pub fn list(cwd: &Path, dir: &str, state: &TreeState) -> Result<Vec<Entry>> {
    let full = within(cwd, dir)?;
    ensure!(full.is_dir(), "{dir} is not a folder");
    let dir = dir.trim_matches('/');
    let dir = if dir == "." { "" } else { dir };
    let join = |name: &str| {
        if dir.is_empty() {
            name.to_owned()
        } else {
            format!("{dir}/{name}")
        }
    };
    let mut entries = Vec::new();
    for item in std::fs::read_dir(&full)?.flatten().take(LIST_LIMIT) {
        let name = item.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(item.path()) else {
            continue;
        };
        let kind = if meta.file_type().is_symlink() {
            Kind::Symlink
        } else if meta.is_dir() {
            Kind::Dir
        } else {
            Kind::File
        };
        let path = join(&name);
        entries.push(Entry {
            status: state.status(&path),
            within: (kind == Kind::Dir).then(|| state.within(&path)).flatten(),
            size: (kind == Kind::File).then_some(meta.len()),
            name,
            path,
            kind,
            missing: false,
        });
    }
    for name in state.deleted_in(dir) {
        if entries.iter().any(|e| e.name == name) {
            continue;
        }
        entries.push(Entry {
            path: join(&name),
            name,
            kind: Kind::File,
            size: None,
            status: Some(Status::Deleted),
            within: None,
            missing: true,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != Kind::Dir)
            .cmp(&(b.kind != Kind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

const IMAGES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

/// A file's text, bounded; an image as a data URL; a binary file as what it is.
pub fn read(cwd: &Path, path: &str, state: &TreeState) -> Result<FileText> {
    let full = within(cwd, path)?;
    let meta = std::fs::metadata(&full)?;
    ensure!(meta.is_file(), "{path} is not a file");
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    let mut text = FileText {
        path: path.trim_matches('/').to_owned(),
        bytes: meta.len(),
        modified_at,
        text: String::new(),
        lines: 0,
        truncated: false,
        binary: false,
        image: None,
        status: state.status(path.trim_matches('/')),
    };
    let mime = full.extension().and_then(|e| e.to_str()).and_then(|e| {
        IMAGES
            .iter()
            .find(|(ext, _)| ext.eq_ignore_ascii_case(e))
            .map(|(_, mime)| *mime)
    });
    if let Some(mime) = mime
        && meta.len() <= IMAGE_LIMIT
    {
        let bytes = std::fs::read(&full)?;
        text.image = Some(format!(
            "data:{mime};base64,{}",
            crate::view::base64(&bytes)
        ));
        return Ok(text);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&full)?
        .take(READ_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.iter().take(8192).any(|b| *b == 0) {
        text.binary = true;
        return Ok(text);
    }
    if bytes.len() as u64 > READ_LIMIT {
        bytes.truncate(READ_LIMIT as usize);
        text.truncated = true;
    }
    text.text = String::from_utf8_lossy(&bytes).into_owned();
    // The cut may land inside a character; lossy decoding leaves a replacement there, not a
    // failure.
    text.lines = text.text.lines().count() as u64;
    Ok(text)
}

/// Every file a name search considers: git's own list, or a bounded walk outside a repository.
fn every_file(cwd: &Path) -> Vec<String> {
    if let Some(list) = git_output(
        cwd,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        let mut files: Vec<String> = String::from_utf8_lossy(&list)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        files.dedup();
        return files;
    }
    walkdir::WalkDir::new(cwd)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_str(),
                    Some(".git" | "node_modules" | "target")
                )
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .take(WALK_LIMIT)
        .filter_map(|e| {
            e.path()
                .strip_prefix(cwd)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

/// How well `path` answers `query`: a basename containing it, a path containing it, the
/// letters in order. `None` when it does not answer at all.
fn rank(path: &str, query: &str) -> Option<u8> {
    let lower = path.to_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name.contains(query) {
        return Some(0);
    }
    if lower.contains(query) {
        return Some(1);
    }
    let mut letters = lower.chars();
    query
        .chars()
        .all(|wanted| letters.any(|c| c == wanted))
        .then_some(2)
}

/// Files by name, or lines by content. `None`: content search needs a repository.
pub fn find(cwd: &Path, query: &str, by: By, limit: usize) -> Result<Option<Vec<Hit>>> {
    let limit = limit.clamp(1, 200);
    let query = query.trim();
    if query.chars().count() < 2 {
        return Ok(Some(vec![]));
    }
    let cwd = within(cwd, "")?;
    match by {
        By::Name => {
            let query = query.to_lowercase();
            let mut hits: Vec<(u8, String)> = every_file(&cwd)
                .into_iter()
                .filter_map(|path| rank(&path, &query).map(|r| (r, path)))
                .collect();
            hits.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(a.1.len().cmp(&b.1.len()))
                    .then(a.1.cmp(&b.1))
            });
            Ok(Some(
                hits.into_iter()
                    .take(limit)
                    .map(|(_, path)| Hit {
                        path,
                        line: None,
                        text: None,
                    })
                    .collect(),
            ))
        }
        By::Content => {
            if git_output(&cwd, &["rev-parse", "--is-inside-work-tree"]).is_none() {
                return Ok(None);
            }
            // Read only as far as the answer needs: a two-letter query in a large repository
            // matches more than anyone reads.
            let mut child = git(&cwd)
                .args([
                    "grep",
                    "-n",
                    "-I",
                    "-i",
                    "-z",
                    "-F",
                    "--untracked",
                    "--no-textconv",
                    "-e",
                    query,
                    "--",
                ])
                .stdout(Stdio::piped())
                .spawn()?;
            let reader = std::io::BufReader::new(child.stdout.take().expect("piped"));
            let mut hits = Vec::new();
            for line in reader.split(b'\n') {
                let Ok(line) = line else { break };
                let mut fields = line.splitn(3, |b| *b == 0);
                let (Some(path), Some(number), Some(text)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                hits.push(Hit {
                    path: String::from_utf8_lossy(path).into_owned(),
                    line: String::from_utf8_lossy(number).parse().ok(),
                    text: Some(orochi::context::bounded(
                        String::from_utf8_lossy(text).trim(),
                        300,
                    )),
                });
                if hits.len() >= limit {
                    break;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            Ok(Some(hits))
        }
    }
}

/// Opens a file in the program the system gives it, or shows it in its folder. A path only,
/// never a URL, and only one `within` allowed.
pub fn open(cwd: &Path, path: &str, reveal: bool) -> Result<()> {
    let full = within(cwd, path)?;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/open");
        if reveal {
            command.arg("-R");
        }
        command.arg(&full);
        command
    } else {
        let mut command = Command::new("xdg-open");
        let target = if reveal {
            full.parent().map(Path::to_path_buf).unwrap_or(full)
        } else {
            full
        };
        command.arg(target);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not open it")?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
