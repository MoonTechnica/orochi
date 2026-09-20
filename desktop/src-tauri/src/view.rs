//! Everything the window is allowed to ask for.
//!
//! The app's claim is that a screen is a query. So this layer holds no state of its own beyond
//! a read cursor: it opens the conversation store, reads the `v_*` views through the same
//! typed functions `orochi threads` uses, and writes only the four things a client may write —
//! a queued turn, an answer, a control, and its own view state.
//!
//! It never speaks to a host, and there is no second protocol: a message sent from here is a
//! row, and the host that owns the thread finds it on its next look.
use anyhow::{Context, Result, ensure};
use orochi::activity::{Activity, Origin, ProjectRow, Thread, VIEW_API};
use orochi::storage::Store;
use serde::Serialize;
use std::{collections::BTreeSet, path::Path};

/// The status marks of the sidebar, as `docs/desktop-app-design.md` §6.2 lists them.
pub fn mark(status: &str) -> &'static str {
    match status {
        "needs_you" => "!",
        "working" => "◉",
        "queued" => "⋯",
        "interrupted" => "⏸",
        "failed" => "×",
        "unread" => "●",
        _ => "○",
    }
}

/// A question waiting for an answer, with the options the agent itself offered. The window
/// never invents one: refusing is the absence of a choice, not a choice of its own.
#[derive(Debug, Clone, Serialize)]
pub struct Prompt {
    pub id: String,
    pub thread_id: String,
    pub thread_title: String,
    pub project: String,
    pub kind: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    pub options: Vec<(String, String)>,
    pub created_at: i64,
}

/// A folder a thread can be started in: one the store already knows about.
#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub name: String,
    pub root: String,
    pub threads: i64,
    pub updated_at: Option<i64>,
}

pub struct Client {
    activity: Activity,
    /// Telemetry, for the salt a project's identity is derived from — the same one the CLI
    /// uses, so a folder opened here and a run started in a terminal are one project.
    store: Store,
    /// Where this window has read the change feed up to.
    cursor: i64,
}

impl Client {
    pub fn open(data: &Path) -> Result<Self> {
        // Read-only at the connection level would be wrong here — the window writes the four
        // things it is allowed to write — but the version gate is the same one a reader makes:
        // a store this build does not understand is refused rather than half-rendered.
        let activity = Activity::attach(data, 0)?;
        let api: i64 = activity
            .connection()
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            api == VIEW_API,
            "this window reads conversation store v{VIEW_API}, and found v{api}"
        );
        Ok(Self {
            activity,
            store: Store::open(data)?,
            cursor: 0,
        })
    }

    /// The folders this window offers: the projects already worked in, most recent first, so
    /// a repository used from a terminal is one click away.
    pub fn folders(&self) -> Result<Vec<Folder>> {
        let connection = self.activity.connection();
        let mut statement = connection.prepare(
            "SELECT name, root, threads, updated_at FROM v_projects
             WHERE hidden_at IS NULL ORDER BY pinned DESC, COALESCE(updated_at, 0) DESC, sort_key",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(Folder {
                name: r.get(0)?,
                root: r.get(1)?,
                threads: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })?;
        rows.map(|row| Ok(row?)).collect()
    }

    /// Starts a thread in a folder the user chose. The project is identified the way the
    /// mailbox identifies a room, so this thread and a terminal run in the same repository
    /// land in one section of the sidebar.
    pub fn new_thread(&self, root: &Path) -> Result<String> {
        ensure!(
            root.is_dir(),
            "{} is not a directory this window can work in",
            root.display()
        );
        let root = root.canonicalize()?;
        let project = self
            .activity
            .project_for(&root, &self.store.salt()?)
            .context("could not identify the folder's project")?;
        let branch = orochi::context::git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty() && b != "HEAD");
        self.activity.create_thread(
            &project,
            &root,
            branch.as_deref(),
            &self.store.repository_id(&root)?,
            Origin::Desktop,
            &orochi::types::Overrides::default(),
            "ask",
        )
    }

    pub fn sidebar(&self, limit: usize, archived: bool) -> Result<Vec<ProjectRow>> {
        self.activity.sidebar(limit, archived)
    }

    pub fn thread(&self, id: &str) -> Result<Option<Thread>> {
        self.activity.thread(id)
    }

    pub fn patch(&self, id: i64) -> Result<Option<String>> {
        self.activity.patch_text(id)
    }

    /// Every question waiting anywhere, newest last, so one window can supervise many threads.
    pub fn open_prompts(&self) -> Result<Vec<Prompt>> {
        let connection = self.activity.connection();
        let mut statement = connection.prepare(
            "SELECT id, thread_id, thread_title, project, kind, request, agent, model, role,
                    created_at
             FROM v_open_prompts ORDER BY created_at",
        )?;
        let rows = statement.query_map([], |r| {
            let request: String = r.get(5)?;
            let request: serde_json::Value =
                serde_json::from_str(&request).unwrap_or(serde_json::Value::Null);
            let call = &request["toolCall"];
            Ok(Prompt {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                thread_title: r.get(2)?,
                project: r.get(3)?,
                kind: r.get(4)?,
                agent: r.get(6)?,
                model: r.get(7)?,
                role: r.get(8)?,
                title: call["title"].as_str().unwrap_or("Permission").to_owned(),
                detail: call["rawInput"]["command"]
                    .as_str()
                    .or_else(|| call["rawInput"]["file_path"].as_str())
                    .map(str::to_owned),
                options: request["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some((
                            option["optionId"].as_str()?.to_owned(),
                            option["name"].as_str().unwrap_or("Allow").to_owned(),
                        ))
                    })
                    .collect(),
                created_at: r.get(9)?,
            })
        })?;
        rows.map(|row| Ok(row?)).collect()
    }

    /// Answers one. `None` refuses. The row count is the race with a terminal that may be
    /// showing the same question, so this reports whether the answer was the one that counted.
    pub fn answer(&self, prompt: &str, option: Option<&str>) -> Result<bool> {
        self.activity.answer_prompt(prompt, option, "desktop")
    }

    /// Sends a message: a queued turn, which the thread's host takes next.
    pub fn send(&self, thread: &str, text: &str) -> Result<String> {
        self.activity.queue_turn(thread, text, &[], "auto", "desktop")
    }

    /// Whether this thread has no living owner. A message to a thread a terminal is holding
    /// belongs to that terminal; one to a thread nobody owns needs a host started for it.
    pub fn needs_host(&self, thread: &str) -> Result<bool> {
        self.activity.reap_hosts()?;
        let owned: bool = self.activity.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM hosts WHERE thread_id=?1)",
            [thread],
            |r| r.get(0),
        )?;
        Ok(!owned)
    }

    pub fn interrupt(&self, thread: &str) -> Result<()> {
        self.activity.control(thread, "interrupt", None)
    }

    pub fn stop(&self, thread: &str) -> Result<()> {
        self.activity.control(thread, "stop", None)
    }

    /// The threads something has happened in since the last look.
    ///
    /// `PRAGMA data_version` says *whether* another connection committed; the feed says
    /// *what*. Reading the feed rather than the conversation is what lets the window poll at
    /// 100 ms without reading a megabyte to learn that nothing changed.
    pub fn changed(&mut self) -> Result<BTreeSet<String>> {
        let connection = self.activity.connection();
        let mut statement = connection
            .prepare("SELECT id, thread_id FROM changes WHERE id > ?1 ORDER BY id")?;
        let mut threads = BTreeSet::new();
        let mut last = self.cursor;
        for row in statement.query_map([self.cursor], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })? {
            let (id, thread) = row?;
            last = last.max(id);
            if let Some(thread) = thread {
                threads.insert(thread);
            }
        }
        self.cursor = last;
        Ok(threads)
    }

    /// Marks a thread read up to its last item, for the sidebar's unread mark.
    pub fn seen(&self, thread: &str) -> Result<()> {
        self.activity.connection().execute(
            "INSERT INTO ui_state (thread_id, last_seen_item)
             VALUES (?1, COALESCE((SELECT max(id) FROM items WHERE thread_id=?1), 0))
             ON CONFLICT(thread_id) DO UPDATE SET last_seen_item=excluded.last_seen_item",
            [thread],
        )?;
        Ok(())
    }
}
