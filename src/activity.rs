//! The conversation: what the user asked, what the agents answered, what they did to the
//! repository and what they said to each other. It is the store a desktop client reads and
//! the memory a console session no longer has to keep in RAM.
//!
//! It is a **separate file** from `telemetry.sqlite3` on purpose. Telemetry holds labels and
//! numbers and must stay free of content (it is the file a user might hand over for
//! debugging); this one holds the content, `0600`, with `secure_delete` so a deleted thread
//! leaves nothing behind, a retention window, and `activity.enabled = false` to keep nothing
//! at all. The link between the two points one way: an attempt names a telemetry run, and
//! telemetry never names a thread.
//!
//! Every write names its columns, and additive changes do not move `user_version`, so an
//! older Orochi sharing the data directory keeps working (`storage::minor_migrations` makes
//! the same promise for telemetry).
pub mod recorder;

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

/// A change an older reader cannot survive moves this; additive ones do not (§5.3 of
/// `docs/desktop-app-design.md`).
const USER_VERSION: i64 = 1;
/// `PRAGMA application_id`: "OROA" as a big-endian i32, so the file identifies itself.
const APPLICATION_ID: i64 = 0x4f_52_4f_41;
/// What the app is promised. A client that does not know this number refuses to read rather
/// than rendering a view whose columns moved.
pub const VIEW_API: i64 = 1;

/// Item text past this keeps its tail and is marked truncated: a runaway tool output must not
/// be able to fill the disk, and nobody reads the middle of a megabyte of log.
const MAX_TEXT: usize = 256 * 1024;
/// `rawInput` / `rawOutput` are shown in a detail pane, not read as prose.
pub const MAX_RAW: usize = 16 * 1024;
/// A diff past this is summarized by its stats; the working tree is the authority anyway.
const MAX_PATCH: usize = 512 * 1024;
/// The last lines of a *failed* check (§10.3). Passing checks store nothing.
pub const CHECK_TAIL_LINES: usize = 200;

pub fn millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Keeps the tail: the end of a log is where the error is.
fn bounded_tail(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_owned(), false);
    }
    let start = text.len() - max;
    let start = (start..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    (text[start..].to_owned(), true)
}

/// The last `lines` lines, for a failed check's output.
pub fn tail_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// Lines of context kept either side of a change.
const CONTEXT: usize = 3;

/// ACP reports an edit as the whole old and new file text. Storing the diff instead keeps the
/// review pane's input while holding a fraction of the source.
///
/// This trims the common prefix and suffix and emits the region between them as **one** hunk.
/// It is a valid unified diff and linear in the file's size, but it is not a minimal one: two
/// edits far apart in a file come back as a single hunk spanning both. That is the right
/// trade here — the working tree is the authority for review (§6.5), and this record exists
/// for the turn that has already scrolled past.
fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let common = |a: &[&str], b: &[&str]| a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let head = common(&old_lines, &new_lines);
    let max_tail = old_lines.len().min(new_lines.len()) - head;
    let tail = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take(max_tail)
        .take_while(|(x, y)| x == y)
        .count();
    if head == old_lines.len() && old_lines.len() == new_lines.len() {
        return String::new();
    }
    let start = head.saturating_sub(CONTEXT);
    let old_end = (old_lines.len() - tail + CONTEXT).min(old_lines.len());
    let new_end = (new_lines.len() - tail + CONTEXT).min(new_lines.len());
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        start + 1,
        old_end - start,
        start + 1,
        new_end - start
    ));
    for line in &old_lines[start..head] {
        out.push_str(&format!(" {line}\n"));
    }
    for line in &old_lines[head..old_lines.len() - tail] {
        out.push_str(&format!("-{line}\n"));
    }
    for line in &new_lines[head..new_lines.len() - tail] {
        out.push_str(&format!("+{line}\n"));
    }
    for line in &old_lines[old_lines.len() - tail..old_end] {
        out.push_str(&format!(" {line}\n"));
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Console,
    Run,
    Collaborate,
    Serve,
    Desktop,
}
impl Origin {
    pub fn key(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Run => "run",
            Self::Collaborate => "collaborate",
            Self::Serve => "serve",
            Self::Desktop => "desktop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    Queued,
    Running,
    Completed,
    Failed,
    Interrupted,
    Withdrawn,
}
impl TurnState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Withdrawn => "withdrawn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatState {
    Choosing,
    Starting,
    Working,
    Asking,
    Checking,
    Done,
    Failed,
    Cancelled,
}
impl SeatState {
    pub fn key(self) -> &'static str {
        match self {
            Self::Choosing => "choosing",
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Asking => "asking",
            Self::Checking => "checking",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What a timeline row is. `Acp` is the escape hatch: an update kind this version does not
/// know is stored as received, so a thread recorded today still renders when the protocol
/// grows a kind tomorrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    UserMessage,
    AgentMessage,
    Thought,
    ToolCall,
    Plan,
    Route,
    Unavailable,
    Note,
    Checks,
    Mode,
    Context,
    Commands,
    Handover,
    Merge,
    Apply,
    Acp,
}
impl ItemKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::AgentMessage => "agent_message",
            Self::Thought => "thought",
            Self::ToolCall => "tool_call",
            Self::Plan => "plan",
            Self::Route => "route",
            Self::Unavailable => "unavailable",
            Self::Note => "note",
            Self::Checks => "checks",
            Self::Mode => "mode",
            Self::Context => "context",
            Self::Commands => "commands",
            Self::Handover => "handover",
            Self::Merge => "merge",
            Self::Apply => "apply",
            Self::Acp => "acp",
        }
    }
    /// Kinds that exist once per attempt and are replaced rather than appended: a plan, the
    /// context gauge, the agent's command list, the current mode.
    fn singleton(self) -> bool {
        matches!(
            self,
            Self::Plan | Self::Context | Self::Commands | Self::Mode
        )
    }
}

/// One row of `v_sidebar`, as a client renders it. `status` is derived at read time rather
/// than stored, because one of its values depends on whether a process is still alive and
/// SQLite cannot ask that.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadRow {
    pub id: String,
    pub project: String,
    pub project_id: String,
    pub project_root: String,
    pub title: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub origin: String,
    pub worktree: bool,
    pub pinned: bool,
    pub archived: bool,
    pub updated_at: i64,
    pub status: &'static str,
    pub seats: i64,
    pub asking: i64,
    pub unread: i64,
    /// The thread is being run by a terminal rather than a headless host.
    pub terminal: bool,
}

/// A project section of the sidebar, with the threads under it.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub root: String,
    pub pinned: bool,
    pub collapsed: bool,
    pub threads: Vec<ThreadRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemRow {
    pub seq: i64,
    pub turn: String,
    pub turn_ordinal: i64,
    pub lane: Option<i64>,
    pub role: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub kind: String,
    pub status: Option<String>,
    pub text: String,
    pub data: Option<serde_json::Value>,
    pub truncated: bool,
    pub patches: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeatRow {
    pub seat_id: String,
    pub turn: String,
    pub ordinal: i64,
    pub role: String,
    pub lead: bool,
    pub read_only: bool,
    pub phase: Option<String>,
    pub state: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub outcome: Option<String>,
    pub peer: Option<String>,
    pub peer_status: Option<String>,
    pub doing: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub turn: String,
    pub path: String,
    pub change: String,
    pub added: i64,
    pub removed: i64,
    pub latest_patch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub thread: ThreadRow,
    pub items: Vec<ItemRow>,
    pub seats: Vec<SeatRow>,
    pub files: Vec<FileRow>,
}

/// One connection, shared by everything in this process that writes the store.
///
/// A connection per writer looked cheaper and was not: SQLite serializes writers to a
/// database anyway, so the connections contend; `busy_timeout` then *sleeps*, and on an async
/// task that stops every agent the task is driving. Closing one is worse — `sqlite3_close` of
/// a WAL connection takes the VFS's shared-memory mutex, so opening and closing them from
/// several tasks at once deadlocked the seats of one collaboration against each other.
///
/// One connection behind a mutex has none of that: writes are short and local, and nothing is
/// ever opened or closed while agents are running.
pub type Shared = std::sync::Arc<std::sync::Mutex<Activity>>;

pub fn share(activity: Activity) -> Shared {
    std::sync::Arc::new(std::sync::Mutex::new(activity))
}

/// A run that is one agent doing one thing: a thread, a turn already claimed, and the seat
/// that fills it. Free rather than a method because a `SeatRef` carries the store it writes
/// through, and a method would hand out a handle to itself.
#[allow(clippy::too_many_arguments)]
pub fn start_turn(
    store: &Shared,
    root: &Path,
    salt: &str,
    repository_id: &str,
    origin: Origin,
    task: &str,
    attachments: &[crate::types::Attachment],
    overrides: &crate::types::Overrides,
    permission: &str,
    role: &str,
) -> Result<(SeatRef, String)> {
    let activity = store.lock().expect("activity store");
    let (thread, turn, host) = activity.start_thread(
        root,
        salt,
        repository_id,
        origin,
        task,
        attachments,
        overrides,
        permission,
        "solo",
    )?;
    let seat = activity.create_seat(&turn, 0, role, true, false, None, None)?;
    Ok((
        SeatRef {
            thread,
            turn,
            seat,
            store: store.clone(),
        },
        host,
    ))
}

/// Where a run writes its rows, and what it writes through. The caller decides the shape of a
/// turn — one seat or several, phases, a collaboration — and hands the scheduler the one place
/// it is filling; the scheduler only ever adds attempts under it.
#[derive(Clone)]
pub struct SeatRef {
    pub thread: String,
    pub turn: String,
    pub seat: String,
    pub store: Shared,
}
impl std::fmt::Debug for SeatRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeatRef")
            .field("thread", &self.thread)
            .field("turn", &self.turn)
            .field("seat", &self.seat)
            .finish()
    }
}

pub struct Activity {
    connection: Connection,
    retention_days: i64,
}

impl Activity {
    /// Opens (and creates) the store. `secure_delete` is on for every connection, so a
    /// deleted thread's text is overwritten rather than left in free pages.
    pub fn open(data: &Path, retention_days: i64) -> Result<Self> {
        std::fs::create_dir_all(data)?;
        let path = data.join("activity.sqlite3");
        let connection = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        crate::storage::setup(
            &connection,
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             PRAGMA secure_delete=ON; PRAGMA foreign_keys=ON;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= USER_VERSION,
            "the activity database is newer than this Orochi version"
        );
        if version < USER_VERSION {
            crate::storage::setup(
                &connection,
                &format!(
                    "BEGIN IMMEDIATE; {SCHEMA} {TRIGGERS} {VIEWS}
                     PRAGMA application_id={APPLICATION_ID};
                     PRAGMA user_version={USER_VERSION}; COMMIT;"
                ),
            )?;
        }
        let activity = Self {
            connection,
            retention_days,
        };
        activity.prune()?;
        Ok(activity)
    }

    /// A second connection to a store that is already set up: the writers inside one run —
    /// a seat's recorder, a collaboration participant — each need their own, and a SQLite
    /// connection is cheap. `open` is not: its schema step takes a write transaction and
    /// retries a busy one by **sleeping**, which on an async task stops every agent that task
    /// is driving. Attach does neither, so the seats of one turn cannot stall each other.
    pub fn attach(data: &Path, retention_days: i64) -> Result<Self> {
        let path = data.join("activity.sqlite3");
        ensure!(path.exists(), "no activity database at {}", path.display());
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA secure_delete=ON; PRAGMA foreign_keys=ON;")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version == USER_VERSION,
            "the activity database is not the version this Orochi set up"
        );
        Ok(Self {
            connection,
            retention_days,
        })
    }

    /// A reader's connection: never writes, and never migrates. `query_only` is what makes
    /// that true of the *connection* rather than of the caller's good intentions. The file is
    /// opened read-write at the OS level on purpose — a `mode=ro` open of a WAL database
    /// fails when `-shm` / `-wal` are not there yet.
    pub fn open_read_only(data: &Path) -> Result<Connection> {
        let path = data.join("activity.sqlite3");
        ensure!(path.exists(), "no activity database at {}", path.display());
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let id: i64 = connection.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        ensure!(
            id == APPLICATION_ID,
            "{} is not an Orochi activity database",
            path.display()
        );
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= USER_VERSION,
            "the activity database is newer than this Orochi version"
        );
        connection.execute_batch("PRAGMA query_only=ON;")?;
        Ok(connection)
    }

    /// Drops threads past the retention window (pinned ones are exempt) and the change feed's
    /// old rows. `0` keeps everything until it is deleted by hand.
    pub fn prune(&self) -> Result<()> {
        self.connection.execute(
            "DELETE FROM changes WHERE at < ?1 AND id < (SELECT COALESCE(max(id),0)-10000 FROM changes)",
            [millis() - 3_600_000],
        )?;
        if self.retention_days <= 0 {
            return Ok(());
        }
        let cutoff = millis() - self.retention_days * 86_400_000;
        self.connection.execute(
            "DELETE FROM threads WHERE pinned=0 AND updated_at < ?1",
            [cutoff],
        )?;
        Ok(())
    }

    /// Everything about a thread, including its text. Used by `orochi threads delete` and by
    /// the app's delete; `secure_delete` makes it a real deletion.
    pub fn delete_thread(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM threads WHERE id=?1", [id])?
            == 1)
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// The sidebar: projects by name, pinned first, each with its threads newest first. The
    /// ordering is the one a client renders and is decided here rather than in the client, so
    /// `orochi threads` and the app agree about what the list looks like.
    pub fn sidebar(&self, limit: usize, archived: bool) -> Result<Vec<ProjectRow>> {
        self.reap_hosts()?;
        let mut statement = self.connection.prepare(
            "SELECT id, project, project_id, project_root, project_pinned, project_collapsed,
                    title, cwd, branch, origin, worktree, pinned, archived_at, updated_at,
                    running, queued, asking, seats, last_state, last_item, last_seen_item,
                    host_kind, host_pid, host_start
             FROM v_sidebar WHERE (?1 OR archived_at IS NULL)
             ORDER BY project_pinned DESC, project_sort, pinned DESC, updated_at DESC",
        )?;
        let mut projects: Vec<ProjectRow> = Vec::new();
        let rows = statement.query_map(params![archived], |r| {
            let running: i64 = r.get(14)?;
            let queued: i64 = r.get(15)?;
            let asking: i64 = r.get(16)?;
            let last_state: Option<String> = r.get(18)?;
            let last_item: Option<i64> = r.get(19)?;
            let last_seen: i64 = r.get(20)?;
            let host: Option<String> = r.get(21)?;
            let pid: Option<i64> = r.get(22)?;
            let start: Option<i64> = r.get(23)?;
            // The one judgement SQL cannot make: a turn still `running` under a process that
            // is gone was interrupted, not working.
            let alive = match (pid, start) {
                (Some(pid), Some(start)) => crate::process::owner_alive(pid, start),
                _ => false,
            };
            let status = if asking > 0 {
                "needs_you"
            } else if running > 0 && alive {
                "working"
            } else if queued > 0 {
                "queued"
            } else if running > 0 || last_state.as_deref() == Some("interrupted") {
                "interrupted"
            } else if last_state.as_deref() == Some("failed") {
                "failed"
            } else if last_item.unwrap_or(0) > last_seen {
                "unread"
            } else {
                "idle"
            };
            Ok((
                r.get::<_, String>(2)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? != 0,
                r.get::<_, i64>(5)? != 0,
                ThreadRow {
                    id: r.get(0)?,
                    project: r.get(1)?,
                    project_id: r.get(2)?,
                    project_root: r.get(3)?,
                    title: r.get(6)?,
                    cwd: r.get(7)?,
                    branch: r.get(8)?,
                    origin: r.get(9)?,
                    worktree: r.get::<_, i64>(10)? != 0,
                    pinned: r.get::<_, i64>(11)? != 0,
                    archived: r.get::<_, Option<i64>>(12)?.is_some(),
                    updated_at: r.get(13)?,
                    status,
                    seats: r.get(17)?,
                    asking,
                    unread: (last_item.unwrap_or(0) - last_seen).max(0),
                    terminal: host.as_deref() == Some("terminal") && alive,
                },
            ))
        })?;
        for row in rows {
            let (id, name, root, pinned, collapsed, thread) = row?;
            let project = match projects.iter_mut().find(|p| p.id == id) {
                Some(project) => project,
                None => {
                    projects.push(ProjectRow {
                        id,
                        name,
                        root,
                        pinned,
                        collapsed,
                        threads: vec![],
                    });
                    projects.last_mut().expect("just pushed")
                }
            };
            if project.threads.len() < limit {
                project.threads.push(thread);
            }
        }
        // A project with every thread archived or deleted keeps its place, so the list of
        // projects is stable between visits.
        let mut statement = self.connection.prepare(
            "SELECT id, name, root, pinned, collapsed FROM projects WHERE hidden_at IS NULL",
        )?;
        let empty = statement.query_map([], |r| {
            Ok(ProjectRow {
                id: r.get(0)?,
                name: r.get(1)?,
                root: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                collapsed: r.get::<_, i64>(4)? != 0,
                threads: vec![],
            })
        })?;
        for project in empty {
            let project = project?;
            if !projects.iter().any(|p| p.id == project.id) {
                projects.push(project);
            }
        }
        projects.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(projects)
    }

    /// Everything one thread's screens need, in one read.
    pub fn thread(&self, id: &str) -> Result<Option<Thread>> {
        let Some(thread) = self
            .sidebar(usize::MAX, true)?
            .into_iter()
            .flat_map(|p| p.threads)
            .find(|t| t.id == id)
        else {
            return Ok(None);
        };
        let items = self
            .connection
            .prepare(
                "SELECT seq, turn_id, turn_ordinal, lane, role, agent, model, kind, status,
                        text, data, truncated, patches
                 FROM v_timeline WHERE thread_id=?1 ORDER BY seq",
            )?
            .query_map([id], |r| {
                Ok(ItemRow {
                    seq: r.get(0)?,
                    turn: r.get(1)?,
                    turn_ordinal: r.get(2)?,
                    lane: r.get(3)?,
                    role: r.get(4)?,
                    agent: r.get(5)?,
                    model: r.get(6)?,
                    kind: r.get(7)?,
                    status: r.get(8)?,
                    text: r.get(9)?,
                    data: r
                        .get::<_, Option<String>>(10)?
                        .and_then(|d| serde_json::from_str(&d).ok()),
                    truncated: r.get::<_, i64>(11)? != 0,
                    patches: r.get(12)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let seats = self
            .connection
            .prepare(
                "SELECT seat_id, turn_id, ordinal, role, lead, read_only, phase, state,
                        agent, model, reasoning, outcome, peer, peer_status, doing
                 FROM v_roster WHERE thread_id=?1 ORDER BY turn_id, ordinal",
            )?
            .query_map([id], |r| {
                Ok(SeatRow {
                    seat_id: r.get(0)?,
                    turn: r.get(1)?,
                    ordinal: r.get(2)?,
                    role: r.get(3)?,
                    lead: r.get::<_, i64>(4)? != 0,
                    read_only: r.get::<_, i64>(5)? != 0,
                    phase: r.get(6)?,
                    state: r.get(7)?,
                    agent: r.get(8)?,
                    model: r.get(9)?,
                    reasoning: r.get(10)?,
                    outcome: r.get(11)?,
                    peer: r.get(12)?,
                    peer_status: r.get(13)?,
                    doing: r.get(14)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let files = self
            .connection
            .prepare(
                "SELECT turn_id, path, change, added, removed, latest_patch
                 FROM v_turn_files WHERE thread_id=?1 ORDER BY turn_id, path",
            )?
            .query_map([id], |r| {
                Ok(FileRow {
                    turn: r.get(0)?,
                    path: r.get(1)?,
                    change: r.get(2)?,
                    added: r.get(3)?,
                    removed: r.get(4)?,
                    latest_patch: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(Thread {
            thread,
            items,
            seats,
            files,
        }))
    }

    /// One stored patch, for a review pane.
    pub fn patch_text(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT patch FROM patches WHERE id=?1", [id], |r| r.get(0))
            .optional()?)
    }

    /// The repository a thread belongs to, by the identity the mailbox already uses for a
    /// room (the git common dir), so every worktree of one repository lands in one section of
    /// the sidebar and in one room. Created on first sight, including from a terminal run:
    /// the sidebar is a map of the work, not a list of what the app happened to start.
    pub fn project(&self, id: &str, root: &Path) -> Result<()> {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        self.connection.execute(
            "INSERT INTO projects (id, root, name, created_at) VALUES (?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET root=excluded.root",
            params![id, root.display().to_string(), name, millis()],
        )?;
        Ok(())
    }

    /// The project a working directory belongs to, created if it is new. Identified the way
    /// the mailbox identifies a room — the git repository all its worktrees share — so a run
    /// started from a terminal and a thread started in the app land in one section of the
    /// sidebar and one room, and `root` is the main worktree rather than whichever worktree
    /// happened to be first.
    pub fn project_for(&self, root: &Path, salt: &str) -> Result<String> {
        let id = crate::mailbox::channel(root, salt);
        let main = crate::context::git(
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .map(|out| std::path::PathBuf::from(out.trim()))
        .and_then(|git| git.parent().map(Path::to_path_buf))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| root.to_path_buf());
        self.project(&id, &main)?;
        Ok(id)
    }

    /// A run that is not a conversation — `orochi run`, a `serve` session, a collaboration —
    /// is still a thread of one turn, because that is what a client renders. The turn is
    /// opened already claimed by this process; its seats are the caller's to add.
    #[allow(clippy::too_many_arguments)]
    pub fn start_thread(
        &self,
        root: &Path,
        salt: &str,
        repository_id: &str,
        origin: Origin,
        task: &str,
        attachments: &[crate::types::Attachment],
        overrides: &crate::types::Overrides,
        permission: &str,
        shape: &str,
    ) -> Result<(String, String, String)> {
        let project = self.project_for(root, salt)?;
        let branch = crate::context::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty() && b != "HEAD");
        let thread = self.create_thread(
            &project,
            root,
            branch.as_deref(),
            repository_id,
            origin,
            overrides,
            permission,
        )?;
        let host = self.register_host(Some(&thread), "terminal")?;
        let turn = self.queue_turn(&thread, task, attachments, shape, "stdin")?;
        ensure!(self.claim_turn(&turn, &host)?, "the turn was already taken");
        self.turn_shape(&turn, Some(shape), None)?;
        Ok((thread, turn, host))
    }

    /// What a finished turn leaves behind for a reader: the seat closed, the turn in its final
    /// state, and the host released.
    pub fn end_turn(&self, seat: &SeatRef, host: &str, state: TurnState) -> Result<()> {
        self.seat_state(
            &seat.seat,
            match state {
                TurnState::Completed => SeatState::Done,
                TurnState::Interrupted => SeatState::Cancelled,
                _ => SeatState::Failed,
            },
        )?;
        self.turn_state(&seat.turn, state)?;
        self.unregister_host(host)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_thread(
        &self,
        project_id: &str,
        cwd: &Path,
        branch: Option<&str>,
        repository_id: &str,
        origin: Origin,
        overrides: &crate::types::Overrides,
        permission: &str,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let at = millis();
        self.connection.execute(
            "INSERT INTO threads (id, project_id, cwd, branch, repository_id, origin, overrides,
                permission, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            params![
                id,
                project_id,
                cwd.display().to_string(),
                branch,
                repository_id,
                origin.key(),
                serde_json::to_string(overrides)?,
                permission,
                at
            ],
        )?;
        Ok(id)
    }

    fn touch(&self, thread: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE threads SET updated_at=?2 WHERE id=?1",
            params![thread, millis()],
        )?;
        Ok(())
    }

    /// Names a thread. The user's own name for it wins and is never replaced; an agent may
    /// offer one through ACP `session_info_update`, which replaces only a name Orochi derived.
    /// No call of Orochi's own is ever made to write one (R6).
    pub fn title(&self, thread: &str, title: &str, from_user: bool) -> Result<()> {
        let title = crate::context::bounded(title.lines().next().unwrap_or("").trim(), 120);
        self.connection.execute(
            "UPDATE threads SET title=?2, titled=?3, updated_at=?4
             WHERE id=?1 AND (?3=1 OR titled=0)",
            params![thread, title, i64::from(from_user), millis()],
        )?;
        Ok(())
    }

    /// The opening message names the conversation. Later messages do not: a thread is called
    /// after what it was started to do.
    fn autotitle(&self, thread: &str, text: &str) -> Result<()> {
        let title = crate::context::bounded(text.lines().next().unwrap_or("").trim(), 120);
        self.connection.execute(
            "UPDATE threads SET title=?2 WHERE id=?1 AND title=''",
            params![thread, title],
        )?;
        Ok(())
    }

    pub fn set_continuation(&self, thread: &str, continuation: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE threads SET continuation=?2, updated_at=?3 WHERE id=?1",
            params![thread, continuation, millis()],
        )?;
        Ok(())
    }

    /// Sending a message is inserting a queued turn and its `user_message` in one
    /// transaction: a turn is never visible without the text that asked for it.
    pub fn queue_turn(
        &self,
        thread: &str,
        text: &str,
        attachments: &[crate::types::Attachment],
        steps: &str,
        source: &str,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let at = millis();
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let ordinal: i64 = tx.query_row(
            "SELECT COALESCE(max(ordinal),0)+1 FROM turns WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO turns (id, thread_id, ordinal, state, source, steps, created_at)
             VALUES (?1,?2,?3,'queued',?4,?5,?6)",
            params![id, thread, ordinal, source, steps, at],
        )?;
        let files: Vec<_> = attachments
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name, "path": a.path.display().to_string(),
                    "bytes": a.bytes, "image": a.image,
                })
            })
            .collect();
        let (text, truncated) = bounded_tail(text, MAX_TEXT);
        tx.execute(
            "INSERT INTO items (thread_id, turn_id, kind, text, data, truncated, created_at, updated_at)
             VALUES (?1,?2,'user_message',?3,?4,?5,?6,?6)",
            params![
                thread,
                id,
                text,
                (!files.is_empty())
                    .then(|| serde_json::json!({ "attachments": files }).to_string()),
                i64::from(truncated),
                at
            ],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at=?2 WHERE id=?1",
            params![thread, at],
        )?;
        tx.commit()?;
        self.autotitle(thread, text.as_str())?;
        Ok(id)
    }

    /// Takes a queued turn. The row count decides who won, so two hosts racing for one thread
    /// cannot both run it.
    pub fn claim_turn(&self, turn: &str, host: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE turns SET state='running', host_id=?2, started_at=?3
             WHERE id=?1 AND state='queued'",
            params![turn, host, millis()],
        )? == 1)
    }

    pub fn turn_state(&self, turn: &str, state: TurnState) -> Result<()> {
        let done = !matches!(state, TurnState::Queued | TurnState::Running);
        self.connection.execute(
            "UPDATE turns SET state=?2, ended_at=CASE WHEN ?3 THEN ?4 ELSE ended_at END WHERE id=?1",
            params![turn, state.key(), done, millis()],
        )?;
        Ok(())
    }

    /// What the turn became. Neither field is overwritten once set: a phase split is decided
    /// before the first phase runs, and each phase would otherwise report itself as the shape
    /// of the whole turn.
    pub fn turn_shape(
        &self,
        turn: &str,
        shape: Option<&str>,
        descriptor: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE turns SET shape=COALESCE(shape, ?2), descriptor=COALESCE(descriptor, ?3)
             WHERE id=?1",
            params![turn, shape, descriptor],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_seat(
        &self,
        turn: &str,
        ordinal: usize,
        role: &str,
        lead: bool,
        read_only: bool,
        phase: Option<&str>,
        workspace: Option<&Path>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.connection.execute(
            "INSERT INTO seats (id, turn_id, ordinal, role, lead, read_only, phase, workspace,
                state, started_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'choosing',?9)",
            params![
                id,
                turn,
                ordinal as i64,
                role,
                i64::from(lead),
                i64::from(read_only),
                phase,
                workspace.map(|w| w.display().to_string()),
                millis()
            ],
        )?;
        Ok(id)
    }

    /// Which step of a collaboration a seat belongs to, and which wave and part of the work
    /// graph it is running.
    pub fn seat_work(
        &self,
        seat: &str,
        stage: usize,
        wave: Option<usize>,
        part: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE seats SET stage=?2, wave=?3, part_id=?4 WHERE id=?1",
            params![seat, stage as i64, wave.map(|w| w as i64), part],
        )?;
        Ok(())
    }

    pub fn seat_state(&self, seat: &str, state: SeatState) -> Result<()> {
        let done = matches!(
            state,
            SeatState::Done | SeatState::Failed | SeatState::Cancelled
        );
        self.connection.execute(
            "UPDATE seats SET state=?2, ended_at=CASE WHEN ?3 THEN ?4 ELSE ended_at END WHERE id=?1",
            params![seat, state.key(), done, millis()],
        )?;
        Ok(())
    }

    /// One pass of the scheduler's retry loop. A failover is the next attempt in the same
    /// seat, which is what lets the timeline say "codex hit a rate limit, continuing on
    /// claude" without inventing a second turn.
    pub fn create_attempt(
        &self,
        seat: &str,
        candidate: &crate::types::ExecutionCandidate,
        resumed: bool,
        considered: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let n: i64 = tx.query_row(
            "SELECT COALESCE(max(n),0)+1 FROM attempts WHERE seat_id=?1",
            [seat],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO attempts (id, seat_id, n, agent, provider, model, reasoning, mode,
                resumed, considered, started_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                id,
                seat,
                n,
                candidate.agent,
                candidate.provider.to_string(),
                candidate.model,
                candidate.reasoning_level,
                candidate.mode,
                i64::from(resumed),
                considered,
                millis()
            ],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn attempt_session(&self, attempt: &str, session: &str, peer: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE attempts SET acp_session_id=?2, peer_id=COALESCE(?3, peer_id) WHERE id=?1",
            params![attempt, session, peer],
        )?;
        Ok(())
    }

    /// The labels and numbers a thread's UI needs, copied onto the attempt so that reading a
    /// conversation never needs the telemetry file. `run_id` is the one-way link to it.
    #[allow(clippy::too_many_arguments)]
    pub fn attempt_finished(
        &self,
        attempt: &str,
        run_id: Option<&str>,
        outcome: Option<&str>,
        error_kind: Option<&str>,
        error: Option<&str>,
        checks: Option<&str>,
        usage: Option<&crate::types::Usage>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE attempts SET run_id=COALESCE(?2, run_id), outcome=?3, error_kind=?4,
                error=?5, checks=?6, usage=?7, ended_at=?8 WHERE id=?1",
            params![
                attempt,
                run_id,
                outcome,
                error_kind,
                error,
                checks,
                usage.map(serde_json::to_string).transpose()?,
                millis()
            ],
        )?;
        Ok(())
    }

    /// Appends a new timeline row. `key` makes it upsertable (an ACP `toolCallId`); a
    /// singleton kind replaces the attempt's previous one instead of piling up.
    #[allow(clippy::too_many_arguments)]
    pub fn item(
        &self,
        thread: &str,
        turn: &str,
        attempt: Option<&str>,
        kind: ItemKind,
        status: Option<&str>,
        key: Option<&str>,
        text: &str,
        data: Option<&str>,
    ) -> Result<i64> {
        let (text, truncated) = bounded_tail(text, MAX_TEXT);
        let at = millis();
        if kind.singleton()
            && let Some(attempt) = attempt
            && let Some(id) = self
                .connection
                .query_row(
                    "SELECT id FROM items WHERE attempt_id=?1 AND kind=?2 ORDER BY id DESC LIMIT 1",
                    params![attempt, kind.key()],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
        {
            self.connection.execute(
                "UPDATE items SET text=?2, data=?3, status=?4, truncated=?5, updated_at=?6 WHERE id=?1",
                params![id, text, data, status, i64::from(truncated), at],
            )?;
            self.touch(thread)?;
            return Ok(id);
        }
        self.connection.execute(
            "INSERT INTO items (thread_id, turn_id, attempt_id, kind, status, key, text, data,
                truncated, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![
                thread,
                turn,
                attempt,
                kind.key(),
                status,
                key,
                text,
                data,
                i64::from(truncated),
                at
            ],
        )?;
        let id = self.connection.last_insert_rowid();
        self.touch(thread)?;
        Ok(id)
    }

    /// Grows a streaming row. The head is kept, because a reply is read from the top; a
    /// stream that will not stop is clipped rather than allowed to fill the disk.
    pub fn append(&self, item: i64, chunk: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE items SET text=substr(text || ?2, 1, ?3),
                truncated=CASE WHEN length(text)+length(?2) > ?3 THEN 1 ELSE truncated END,
                updated_at=?4 WHERE id=?1",
            params![item, chunk, MAX_TEXT as i64, millis()],
        )?;
        Ok(())
    }

    pub fn item_status(&self, item: i64, status: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE items SET status=?2, updated_at=?3 WHERE id=?1",
            params![item, status, millis()],
        )?;
        Ok(())
    }

    /// Finds an attempt's tool-call row by its ACP id, so an update merges into the call it
    /// belongs to instead of appending a second line for the same call.
    pub fn tool_item(&self, attempt: &str, key: &str) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM items WHERE attempt_id=?1 AND key=?2",
                params![attempt, key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Merges an ACP `tool_call_update` into the call it belongs to. The protocol says a field
    /// the update leaves out keeps its previous value, so `data` is merged rather than
    /// replaced — `json_patch` is RFC 7396, which is exactly that rule, and the caller passes
    /// only the fields the update carried.
    pub fn update_tool(
        &self,
        item: i64,
        status: Option<&str>,
        text: Option<&str>,
        data: Option<&str>,
    ) -> Result<()> {
        let bounded = text.map(|t| bounded_tail(t, MAX_TEXT));
        self.connection.execute(
            "UPDATE items SET status=COALESCE(?2, status), text=COALESCE(?3, text),
                data=CASE WHEN ?4 IS NULL THEN data
                     ELSE json_patch(COALESCE(data,'{}'), ?4) END,
                truncated=CASE WHEN ?5 THEN 1 ELSE truncated END, updated_at=?6 WHERE id=?1",
            params![
                item,
                status,
                bounded.as_ref().map(|(t, _)| t.as_str()),
                data,
                bounded.as_ref().is_some_and(|(_, t)| *t),
                millis()
            ],
        )?;
        Ok(())
    }

    /// What the agent said it changed. The working tree stays the authority for review; this
    /// is what remains once the tree has moved on, and the only record for an agent that
    /// edits through a shell.
    pub fn patch(
        &self,
        item: i64,
        turn: &str,
        path: &str,
        old: Option<&str>,
        new: &str,
    ) -> Result<()> {
        let change = match old {
            None => "add",
            Some(_) if new.is_empty() => "delete",
            Some(_) => "modify",
        };
        let diff = unified_diff(path, old.unwrap_or(""), new);
        let added = diff
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let removed = diff
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        let (patch, truncated) = bounded_tail(&diff, MAX_PATCH);
        self.connection.execute(
            "INSERT INTO patches (item_id, turn_id, path, change, added, removed, patch, truncated)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                item,
                turn,
                path,
                change,
                added as i64,
                removed as i64,
                patch,
                i64::from(truncated)
            ],
        )?;
        Ok(())
    }

    /// The next turn waiting to run in a thread, oldest first.
    pub fn next_queued(&self, thread: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT t.id, COALESCE((SELECT i.text FROM items i WHERE i.turn_id=t.id
                    AND i.kind='user_message' ORDER BY i.id LIMIT 1), '')
                 FROM turns t WHERE t.thread_id=?1 AND t.state='queued'
                 ORDER BY t.ordinal LIMIT 1",
                [thread],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// What a thread was left routed to, for a turn that continues it.
    pub fn continuation(&self, thread: &str) -> Result<Option<serde_json::Value>> {
        Ok(self
            .connection
            .query_row(
                "SELECT continuation FROM threads WHERE id=?1 AND continuation IS NOT NULL",
                [thread],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub fn thread_settings(&self, thread: &str) -> Result<Option<(String, String, String)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT cwd, overrides, permission FROM threads WHERE id=?1",
                [thread],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?)
    }

    /// A control a client asked for, and marking it taken. Controls are how the app presses
    /// the keys the terminal has: interrupt, change mode, start again.
    pub fn take_control(&self, thread: &str) -> Result<Option<(String, Option<String>)>> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let next: Option<(i64, String, Option<String>)> = tx
            .query_row(
                "SELECT id, kind, payload FROM controls
                 WHERE thread_id=?1 AND consumed_at IS NULL ORDER BY id LIMIT 1",
                [thread],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((id, ..)) = &next {
            tx.execute(
                "UPDATE controls SET consumed_at=?2 WHERE id=?1",
                params![id, millis()],
            )?;
        }
        tx.commit()?;
        Ok(next.map(|(_, kind, payload)| (kind, payload)))
    }

    pub fn control(&self, thread: &str, kind: &str, payload: Option<&str>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO controls (thread_id, kind, payload, created_at) VALUES (?1,?2,?3,?4)",
            params![thread, kind, payload, millis()],
        )?;
        Ok(())
    }

    /// Opens a question the user has to answer. `answer` is NULL while it is open, the empty
    /// string when it was refused, and otherwise the ACP option that was chosen.
    #[allow(clippy::too_many_arguments)]
    pub fn open_prompt(
        &self,
        thread: &str,
        turn: &str,
        attempt: Option<&str>,
        item: Option<i64>,
        kind: &str,
        request: &str,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.connection.execute(
            "INSERT INTO prompts (id, thread_id, turn_id, attempt_id, item_id, kind, request,
                created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, thread, turn, attempt, item, kind, request, millis()],
        )?;
        Ok(id)
    }

    /// Answers one. Whoever gets here first decides: the row count is the race, so a terminal
    /// dialog and a client's card cannot both answer the same question.
    pub fn answer_prompt(&self, id: &str, answer: Option<&str>, by: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE prompts SET answer=?2, answered_by=?3, answered_at=?4
             WHERE id=?1 AND answer IS NULL",
            params![id, answer.unwrap_or(""), by, millis()],
        )? == 1)
    }

    /// What a question was answered, if it has been.
    pub fn prompt_answer(&self, id: &str) -> Result<Option<Option<String>>> {
        Ok(self
            .connection
            .query_row(
                "SELECT answer FROM prompts WHERE id=?1 AND answer IS NOT NULL",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|answer| (!answer.is_empty()).then_some(answer)))
    }

    /// Registers this process as the thread's owner. The unique index is what stops two hosts
    /// from running one thread; a dead owner's row is cleared first, judged the way
    /// `mailbox::prune` judges a peer's.
    pub fn register_host(&self, thread: Option<&str>, kind: &str) -> Result<String> {
        self.reap_hosts()?;
        let id = uuid::Uuid::new_v4().to_string();
        let (pid, start) = crate::process::owner_identity()?;
        let at = millis();
        self.connection.execute(
            "INSERT INTO hosts (id, pid, start, kind, thread_id, version, started_at, heartbeat_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
            params![id, pid, start, kind, thread, env!("CARGO_PKG_VERSION"), at],
        )?;
        Ok(id)
    }

    pub fn heartbeat(&self, host: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE hosts SET heartbeat_at=?2 WHERE id=?1",
            params![host, millis()],
        )?;
        Ok(())
    }

    pub fn unregister_host(&self, host: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM hosts WHERE id=?1", [host])?;
        Ok(())
    }

    /// A host whose process is gone releases its thread, and a turn still `running` under it
    /// is what the reader shows as interrupted.
    pub fn reap_hosts(&self) -> Result<()> {
        let dead: Vec<String> = self
            .connection
            .prepare("SELECT id, pid, start FROM hosts")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter(|(_, pid, start)| !crate::process::owner_alive(*pid, *start))
            .map(|(id, ..)| id)
            .collect();
        for id in dead {
            self.connection.execute(
                "UPDATE turns SET state='interrupted', ended_at=?2
                 WHERE host_id=?1 AND state='running'",
                params![id, millis()],
            )?;
            self.connection
                .execute("DELETE FROM hosts WHERE id=?1", [id])?;
        }
        Ok(())
    }
}

/// The schema. Split from `open` so a reader (the desktop app) can be told exactly what it is
/// reading, and so the view definitions sit next to the tables they are a contract over.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  root TEXT NOT NULL,
  name TEXT NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0,
  collapsed INTEGER NOT NULL DEFAULT 0,
  hidden_at INTEGER,
  created_at INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS threads (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  cwd TEXT NOT NULL,
  branch TEXT,
  repository_id TEXT NOT NULL,
  origin TEXT NOT NULL CHECK (origin IN ('console','run','collaborate','serve','desktop')),
  title TEXT NOT NULL DEFAULT '',
  titled INTEGER NOT NULL DEFAULT 0,
  overrides TEXT NOT NULL DEFAULT '{}',
  permission TEXT NOT NULL DEFAULT 'ask',
  continuation TEXT,
  pinned INTEGER NOT NULL DEFAULT 0,
  archived_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS threads_sidebar ON threads(project_id, archived_at, updated_at DESC);

CREATE TABLE IF NOT EXISTS hosts (
  id TEXT PRIMARY KEY,
  pid INTEGER NOT NULL, start INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('terminal','headless','serve')),
  thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
  version TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  heartbeat_at INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS hosts_thread ON hosts(thread_id);

CREATE TABLE IF NOT EXISTS turns (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN
    ('queued','running','completed','failed','interrupted','withdrawn')),
  source TEXT NOT NULL CHECK (source IN ('terminal','desktop','stdin','acp')),
  steps TEXT NOT NULL DEFAULT 'auto',
  shape TEXT,
  descriptor TEXT,
  host_id TEXT,
  created_at INTEGER NOT NULL, started_at INTEGER, ended_at INTEGER,
  UNIQUE (thread_id, ordinal));
CREATE INDEX IF NOT EXISTS turns_queue ON turns(thread_id, state, ordinal);

CREATE TABLE IF NOT EXISTS seats (
  id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  role TEXT NOT NULL,
  lead INTEGER NOT NULL DEFAULT 0,
  read_only INTEGER NOT NULL DEFAULT 0,
  phase TEXT,
  stage INTEGER, wave INTEGER, part_id TEXT,
  workspace TEXT,
  state TEXT NOT NULL CHECK (state IN
    ('choosing','starting','working','asking','checking','done','failed','cancelled')),
  started_at INTEGER NOT NULL, ended_at INTEGER);
CREATE INDEX IF NOT EXISTS seats_turn ON seats(turn_id, ordinal);

CREATE TABLE IF NOT EXISTS attempts (
  id TEXT PRIMARY KEY,
  seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
  n INTEGER NOT NULL,
  agent TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  reasoning TEXT, mode TEXT,
  resumed INTEGER NOT NULL DEFAULT 0,
  acp_session_id TEXT,
  peer_id TEXT,
  considered TEXT,
  run_id TEXT,
  outcome TEXT, error_kind TEXT, error TEXT,
  checks TEXT,
  usage TEXT,
  started_at INTEGER NOT NULL, ended_at INTEGER,
  UNIQUE (seat_id, n));

CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  status TEXT,
  key TEXT,
  text TEXT NOT NULL DEFAULT '',
  data TEXT,
  truncated INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS items_timeline ON items(thread_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS items_key ON items(attempt_id, key) WHERE key IS NOT NULL;

CREATE TABLE IF NOT EXISTS patches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  change TEXT NOT NULL CHECK (change IN ('add','modify','delete')),
  added INTEGER NOT NULL, removed INTEGER NOT NULL,
  patch TEXT NOT NULL,
  truncated INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS patches_turn ON patches(turn_id, path);

CREATE TABLE IF NOT EXISTS prompts (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE CASCADE,
  item_id INTEGER REFERENCES items(id) ON DELETE SET NULL,
  kind TEXT NOT NULL CHECK (kind IN ('permission','team','apply')),
  request TEXT NOT NULL,
  answer TEXT,
  answered_by TEXT,
  created_at INTEGER NOT NULL, answered_at INTEGER);
CREATE INDEX IF NOT EXISTS prompts_open ON prompts(thread_id) WHERE answer IS NULL;

CREATE TABLE IF NOT EXISTS controls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN
    ('interrupt','set_mode','set_permission','reroute','new','stop')),
  payload TEXT,
  created_at INTEGER NOT NULL, consumed_at INTEGER);
CREATE INDEX IF NOT EXISTS controls_open ON controls(thread_id, id) WHERE consumed_at IS NULL;

CREATE TABLE IF NOT EXISTS parts (
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  id TEXT NOT NULL,
  brief TEXT NOT NULL,
  paths TEXT NOT NULL DEFAULT '[]', after TEXT NOT NULL DEFAULT '[]',
  wave INTEGER,
  unit TEXT,
  state TEXT NOT NULL DEFAULT 'waiting',
  strays INTEGER,
  PRIMARY KEY (turn_id, id));

CREATE TABLE IF NOT EXISTS ui_state (
  thread_id TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
  last_seen_item INTEGER NOT NULL DEFAULT 0,
  draft TEXT);

CREATE TABLE IF NOT EXISTS changes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  tbl TEXT NOT NULL, row TEXT NOT NULL, thread_id TEXT, at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS changes_cursor ON changes(id);

-- The mailbox (§4.7). Created here so the room views resolve; `mailbox.rs` starts writing
-- these instead of its own file in P1, at which point a peer is no longer deleted when its
-- session ends but keeps its row with `left_at` set — a room needs who *was* there.
CREATE TABLE IF NOT EXISTS peers (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  name TEXT NOT NULL,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE SET NULL,
  worktree TEXT NOT NULL, branch TEXT, route TEXT,
  status TEXT NOT NULL DEFAULT '',
  owner_pid INTEGER NOT NULL, owner_start INTEGER NOT NULL,
  last_read INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  left_at INTEGER);
CREATE UNIQUE INDEX IF NOT EXISTS peers_live_name
  ON peers(project_id, lower(name)) WHERE left_at IS NULL;

CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT NOT NULL,
  thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
  sender TEXT NOT NULL, sender_name TEXT NOT NULL,
  recipient TEXT NOT NULL, recipient_name TEXT NOT NULL,
  via TEXT NOT NULL DEFAULT 'mailbox' CHECK (via IN ('mailbox','handoff','user')),
  body TEXT NOT NULL,
  sent_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS messages_room ON messages(project_id, id);

CREATE TABLE IF NOT EXISTS peer_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  peer_id TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('joined','left','status','route')),
  text TEXT NOT NULL DEFAULT '',
  at INTEGER NOT NULL);
"#;

/// `sqlite3_update_hook` only ever sees its own connection, so a reader in another process
/// learns what moved from these rows: `PRAGMA data_version` says *something* committed, the
/// feed says what. Triggers rather than writer discipline, because there are several writers
/// (hosts, the mailbox MCP subprocess, the app) and a forgotten notification is a stale UI.
const TRIGGERS: &str = r#"
CREATE TRIGGER IF NOT EXISTS ch_threads_i AFTER INSERT ON threads BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('threads',new.id,new.id,new.updated_at); END;
CREATE TRIGGER IF NOT EXISTS ch_threads_u AFTER UPDATE ON threads BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('threads',new.id,new.id,new.updated_at); END;
CREATE TRIGGER IF NOT EXISTS ch_turns_i AFTER INSERT ON turns BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('turns',new.id,new.thread_id,new.created_at); END;
CREATE TRIGGER IF NOT EXISTS ch_turns_u AFTER UPDATE ON turns BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('turns',new.id,new.thread_id,new.created_at); END;
CREATE TRIGGER IF NOT EXISTS ch_seats_i AFTER INSERT ON seats BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    SELECT 'seats',new.id,thread_id,new.started_at FROM turns WHERE id=new.turn_id; END;
CREATE TRIGGER IF NOT EXISTS ch_seats_u AFTER UPDATE ON seats BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    SELECT 'seats',new.id,thread_id,new.started_at FROM turns WHERE id=new.turn_id; END;
CREATE TRIGGER IF NOT EXISTS ch_attempts_i AFTER INSERT ON attempts BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    SELECT 'attempts',new.id,t.thread_id,new.started_at FROM seats s JOIN turns t ON t.id=s.turn_id
    WHERE s.id=new.seat_id; END;
CREATE TRIGGER IF NOT EXISTS ch_attempts_u AFTER UPDATE ON attempts BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    SELECT 'attempts',new.id,t.thread_id,new.started_at FROM seats s JOIN turns t ON t.id=s.turn_id
    WHERE s.id=new.seat_id; END;
CREATE TRIGGER IF NOT EXISTS ch_items_i AFTER INSERT ON items BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('items',new.id,new.thread_id,new.updated_at); END;
CREATE TRIGGER IF NOT EXISTS ch_items_u AFTER UPDATE ON items BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('items',new.id,new.thread_id,new.updated_at); END;
CREATE TRIGGER IF NOT EXISTS ch_prompts_i AFTER INSERT ON prompts BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('prompts',new.id,new.thread_id,new.created_at); END;
CREATE TRIGGER IF NOT EXISTS ch_prompts_u AFTER UPDATE ON prompts BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('prompts',new.id,new.thread_id,new.created_at); END;
CREATE TRIGGER IF NOT EXISTS ch_controls_i AFTER INSERT ON controls BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('controls',new.id,new.thread_id,new.created_at); END;
CREATE TRIGGER IF NOT EXISTS ch_patches_i AFTER INSERT ON patches BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    SELECT 'patches',new.id,thread_id,0 FROM turns WHERE id=new.turn_id; END;
CREATE TRIGGER IF NOT EXISTS ch_messages_i AFTER INSERT ON messages BEGIN
  INSERT INTO changes (tbl,row,thread_id,at)
    VALUES ('messages',new.id,new.thread_id,new.sent_at); END;
CREATE TRIGGER IF NOT EXISTS ch_peers_i AFTER INSERT ON peers BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('peers',new.id,NULL,new.started_at); END;
CREATE TRIGGER IF NOT EXISTS ch_peers_u AFTER UPDATE ON peers BEGIN
  INSERT INTO changes (tbl,row,thread_id,at) VALUES ('peers',new.id,NULL,new.started_at); END;
"#;

/// What the desktop client is allowed to read. Base tables are free to change underneath
/// these; `VIEW_API` is the number that says whether they did. `orochi threads` reads the
/// same views from a terminal, which is what keeps the contract honest — a screen that
/// cannot be produced from a view means the view is missing something.
///
/// Liveness is deliberately *not* decided here: SQLite cannot ask whether a pid is alive, so
/// the views expose `host_pid` / `host_start` and the reader applies `process::alive`, the
/// same judgement `mailbox::prune` makes.
const VIEWS: &str = r#"
DROP VIEW IF EXISTS v_projects;
CREATE VIEW v_projects AS
SELECT p.id, p.root, p.name, p.pinned, p.collapsed, p.hidden_at,
       (SELECT count(*) FROM threads t WHERE t.project_id=p.id AND t.archived_at IS NULL) AS threads,
       (SELECT count(*) FROM threads t JOIN turns tn ON tn.thread_id=t.id
         WHERE t.project_id=p.id AND tn.state IN ('queued','running')) AS active,
       (SELECT count(*) FROM threads t JOIN prompts pr ON pr.thread_id=t.id
         WHERE t.project_id=p.id AND pr.answer IS NULL) AS asking,
       (SELECT max(t.updated_at) FROM threads t WHERE t.project_id=p.id) AS updated_at,
       lower(p.name) AS sort_key
FROM projects p;

DROP VIEW IF EXISTS v_sidebar;
CREATE VIEW v_sidebar AS
SELECT t.id, t.project_id, p.name AS project, p.root AS project_root, lower(p.name) AS project_sort,
       p.pinned AS project_pinned, p.collapsed AS project_collapsed,
       t.title, t.cwd, t.branch, t.origin, t.pinned, t.archived_at, t.updated_at,
       t.cwd <> p.root AS worktree,
       h.kind AS host_kind, h.pid AS host_pid, h.start AS host_start,
       (SELECT count(*) FROM turns tn WHERE tn.thread_id=t.id AND tn.state='running') AS running,
       (SELECT count(*) FROM turns tn WHERE tn.thread_id=t.id AND tn.state='queued') AS queued,
       (SELECT count(*) FROM prompts pr WHERE pr.thread_id=t.id AND pr.answer IS NULL) AS asking,
       (SELECT count(*) FROM seats s JOIN turns tn ON tn.id=s.turn_id
         WHERE tn.thread_id=t.id AND s.ended_at IS NULL) AS seats,
       (SELECT tn.state FROM turns tn WHERE tn.thread_id=t.id
         ORDER BY tn.ordinal DESC LIMIT 1) AS last_state,
       (SELECT max(i.id) FROM items i WHERE i.thread_id=t.id) AS last_item,
       COALESCE((SELECT u.last_seen_item FROM ui_state u WHERE u.thread_id=t.id),0) AS last_seen_item
FROM threads t JOIN projects p ON p.id=t.project_id
LEFT JOIN hosts h ON h.thread_id=t.id;

DROP VIEW IF EXISTS v_timeline;
CREATE VIEW v_timeline AS
SELECT i.thread_id, i.id AS seq, i.turn_id, tn.ordinal AS turn_ordinal,
       i.attempt_id, s.id AS seat_id, s.ordinal AS lane, s.role, s.lead, s.read_only, s.phase,
       a.agent, a.provider, a.model, a.reasoning,
       i.kind, i.status, i.key, i.text, i.data, i.truncated,
       i.created_at, i.updated_at,
       (SELECT count(*) FROM patches pa WHERE pa.item_id=i.id) AS patches
FROM items i
JOIN turns tn ON tn.id=i.turn_id
LEFT JOIN attempts a ON a.id=i.attempt_id
LEFT JOIN seats s ON s.id=a.seat_id;

DROP VIEW IF EXISTS v_room;
CREATE VIEW v_room AS
SELECT m.project_id, m.thread_id, m.id AS seq, 'message' AS kind,
       m.sender_name AS who, m.recipient_name AS whom, m.via, m.body AS text, m.sent_at AS at,
       a.agent, a.model, s.role
FROM messages m
LEFT JOIN peers pe ON pe.id=m.sender
LEFT JOIN attempts a ON a.id=pe.attempt_id
LEFT JOIN seats s ON s.id=a.seat_id
UNION ALL
SELECT pe.project_id, NULL AS thread_id, e.id AS seq, e.kind,
       pe.name AS who, NULL AS whom, 'mailbox' AS via, e.text, e.at,
       a.agent, a.model, s.role
FROM peer_events e
JOIN peers pe ON pe.id=e.peer_id
LEFT JOIN attempts a ON a.id=pe.attempt_id
LEFT JOIN seats s ON s.id=a.seat_id;

DROP VIEW IF EXISTS v_roster;
CREATE VIEW v_roster AS
SELECT s.id AS seat_id, tn.thread_id, s.turn_id, s.ordinal, s.role, s.lead, s.read_only,
       s.phase, s.stage, s.wave, s.part_id, s.workspace, s.state, s.started_at, s.ended_at,
       a.id AS attempt_id, a.n, a.agent, a.provider, a.model, a.reasoning, a.mode, a.resumed,
       a.outcome, a.error_kind, a.usage, a.run_id,
       pe.name AS peer, pe.status AS peer_status, pe.left_at AS peer_left_at,
       (SELECT i.text FROM items i WHERE i.attempt_id=a.id AND i.kind='tool_call'
         AND i.status='in_progress' ORDER BY i.id DESC LIMIT 1) AS doing,
       (SELECT i.data FROM items i WHERE i.attempt_id=a.id AND i.kind='context'
         ORDER BY i.id DESC LIMIT 1) AS context
FROM seats s
JOIN turns tn ON tn.id=s.turn_id
LEFT JOIN attempts a ON a.seat_id=s.id
  AND a.n=(SELECT max(a2.n) FROM attempts a2 WHERE a2.seat_id=s.id)
LEFT JOIN peers pe ON pe.attempt_id=a.id;

DROP VIEW IF EXISTS v_turn_files;
CREATE VIEW v_turn_files AS
SELECT pa.turn_id, tn.thread_id, pa.path,
       sum(pa.added) AS added, sum(pa.removed) AS removed,
       count(*) AS revisions, max(pa.id) AS latest_patch,
       (SELECT change FROM patches p2 WHERE p2.turn_id=pa.turn_id AND p2.path=pa.path
         ORDER BY p2.id DESC LIMIT 1) AS change
FROM patches pa JOIN turns tn ON tn.id=pa.turn_id
GROUP BY pa.turn_id, pa.path;

DROP VIEW IF EXISTS v_open_prompts;
CREATE VIEW v_open_prompts AS
SELECT pr.id, pr.thread_id, t.title AS thread_title, p.name AS project,
       pr.turn_id, pr.attempt_id, pr.item_id, pr.kind, pr.request, pr.created_at,
       a.agent, a.model, s.role, i.text AS tool_text, i.data AS tool_data
FROM prompts pr
JOIN threads t ON t.id=pr.thread_id
JOIN projects p ON p.id=t.project_id
LEFT JOIN attempts a ON a.id=pr.attempt_id
LEFT JOIN seats s ON s.id=a.seat_id
LEFT JOIN items i ON i.id=pr.item_id
WHERE pr.answer IS NULL;

DROP VIEW IF EXISTS v_board;
CREATE VIEW v_board AS
SELECT pt.turn_id, tn.thread_id, pt.id, pt.brief, pt.paths, pt.after, pt.wave, pt.unit,
       pt.state, pt.strays,
       (SELECT s.id FROM seats s WHERE s.turn_id=pt.turn_id AND s.part_id=pt.id LIMIT 1) AS seat_id
FROM parts pt JOIN turns tn ON tn.id=pt.turn_id;

DROP VIEW IF EXISTS v_route;
CREATE VIEW v_route AS
SELECT a.id AS attempt_id, tn.thread_id, s.turn_id, s.id AS seat_id, s.role,
       a.n, a.agent, a.provider, a.model, a.reasoning, a.mode, a.resumed,
       a.considered, a.run_id, a.outcome, a.error_kind, a.error, a.checks, a.usage,
       a.started_at, a.ended_at
FROM attempts a JOIN seats s ON s.id=a.seat_id JOIN turns tn ON tn.id=s.turn_id;
"#;
