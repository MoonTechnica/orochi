//! Everything the window is allowed to ask for.
//!
//! The app's claim is that a screen is a query. So this layer holds no state of its own beyond
//! a read cursor: it opens the conversation store, reads the `v_*` views through the same
//! typed functions `orochi threads` uses, and writes only the four things a client may write —
//! a queued turn, an answer, a control, and its own view state.
//!
//! It never speaks to a host, and there is no second protocol: a message sent from here is a
//! row, and the host that owns the thread finds it on its next look.

/// Starts `orochi host` for a thread and makes sure it took. Spawning succeeds whatever the
/// binary then does, and one too old to know the subcommand rejects it and exits at once — so
/// the window would report a host while the turn sat queued for ever with nothing said. A host
/// that is still running once it has had a moment is a host; one that has already stopped is
/// reported with whatever it said on its way out.
pub fn start_host(binary: &std::path::Path, thread: &str) -> Result<()> {
    use std::io::Read;
    let mut child = std::process::Command::new(binary)
        .args(["host", "--thread", thread])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start {}", binary.display()))?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    let Some(status) = child.try_wait()? else {
        // It is running, and it goes on writing as it works. The pipe has to keep being read
        // for as long as it does: the first line written after this end went away would kill
        // it, and a full buffer would stop it just as dead.
        if let Some(mut errors) = child.stderr.take() {
            std::thread::Builder::new()
                .name("orochi-host-stderr".into())
                .spawn(move || {
                    let mut sink = [0u8; 4096];
                    while matches!(errors.read(&mut sink), Ok(read) if read > 0) {}
                })
                .ok();
        }
        return Ok(());
    };
    let mut said = String::new();
    if let Some(mut errors) = child.stderr.take() {
        let _ = errors.read_to_string(&mut said);
    }
    let said = said
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    bail!(
        "{} could not host this thread ({status}){}{said}",
        binary.display(),
        if said.is_empty() { "" } else { ": " }
    )
}
use anyhow::{Context, Result, bail, ensure};
use orochi::activity::{Activity, FileRow, Origin, ProjectRow, Thread, VIEW_API};
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

/// One line in the room: an agent's message, a person's note, or someone arriving or leaving.
#[derive(Debug, Clone, Serialize)]
pub struct Said {
    pub seq: i64,
    pub kind: String,
    pub who: String,
    pub whom: Option<String>,
    pub via: String,
    pub text: String,
    pub at: i64,
    pub role: Option<String>,
    pub model: Option<String>,
}

/// What one look at a thread found: whether the agent running it has gone, and whether there
/// is work nobody has picked up.
#[derive(Debug, Clone, Serialize)]
pub struct Watch {
    /// A turn was left unfinished by a host that is no longer there. Said once: the turn is
    /// written down as interrupted, so the next look has nothing new to report.
    pub lost: bool,
    /// Work is queued and no host owns the thread, so one has to be started for it to run.
    pub queued: bool,
}

/// An agent's readiness, as `orochi status` reports it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRow {
    pub agent: String,
    pub model: String,
    pub status: String,
    pub cooling: i64,
    pub reset_at: Option<i64>,
    pub quota_estimate: Option<f64>,
    pub failures: i64,
    pub windows: Vec<(String, f64, Option<i64>)>,
}

/// What a route has actually done. Verified evidence and weak signals are separate columns
/// here for the same reason `calibrate` keeps them apart: one is measured, the other is a
/// guess about what the user meant by carrying on.
#[derive(Debug, Clone, Serialize)]
pub struct RouteStats {
    pub agent: String,
    pub model: String,
    pub task_type: String,
    pub reasoning: Option<String>,
    pub verified: i64,
    pub failures: i64,
    pub weak: i64,
    pub success_rate: f64,
    pub mean_tokens: f64,
    pub mean_duration_ms: f64,
    pub last_at: Option<i64>,
}

pub struct Client {
    activity: Activity,
    /// Telemetry, for the salt a project's identity is derived from — the same one the CLI
    /// uses, so a folder opened here and a run started in a terminal are one project.
    store: Store,
    /// Where this window has read the change feed up to.
    cursor: i64,
    /// The room's limits, as configured.
    mailbox: orochi::config::MailboxConfig,
    /// The configuration file this window reads and writes, and the data directory it was
    /// opened on (memory lives beside the databases, as plain Markdown).
    config: std::path::PathBuf,
    data: std::path::PathBuf,
}

/// What Orochi remembers, as the text it is. The window edits it as text because that is what
/// it is — the user's own words, which only Orochi writes and no agent ever does.
#[derive(Debug, Clone, Serialize)]
pub struct Remembered {
    pub user: String,
    pub path: String,
}

impl Client {
    pub fn open(data: &Path) -> Result<Self> {
        // The window is a front door as much as the CLI is: someone may install it before
        // ever running `orochi`, and an interrupted first run leaves a file that is not yet a
        // store. `open` sets either up. The version gate below is still the one a reader
        // makes — a store this build does not understand is refused, not half-rendered.
        let activity = Activity::open(data, 0)?;
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
            mailbox: orochi::config::MailboxConfig::default(),
            config: orochi::config::Paths::resolve(None, Some(data.to_path_buf()))
                .map(|paths| paths.config)
                .unwrap_or_else(|_| data.join("config.toml")),
            data: data.to_path_buf(),
        })
    }

    /// Points this window at a particular configuration file, as `--config` does.
    pub fn with_config(mut self, config: &Path) -> Self {
        self.config = config.to_path_buf();
        self
    }

    /// The settings, with `agent.env` values redacted: a window has no reason to read a
    /// secret, so it is never given one to display.
    pub fn settings(&self) -> Result<serde_json::Value> {
        let mut config = orochi::config::Config::load(&self.config)?;
        for agent in &mut config.agents {
            for value in agent.env.values_mut() {
                *value = "[redacted]".into();
            }
        }
        Ok(serde_json::to_value(config)?)
    }

    /// Writes settings back, through the same validation the CLI uses, so an invalid one is
    /// refused here rather than discovered at the next run. Redacted secrets are put back
    /// from the file, never written as the word `[redacted]`.
    pub fn save_settings(&self, settings: &serde_json::Value) -> Result<()> {
        let mut config: orochi::config::Config = serde_json::from_value(settings.clone())?;
        let current = orochi::config::Config::load(&self.config).unwrap_or_default();
        for agent in &mut config.agents {
            let Some(existing) = current.agents.iter().find(|a| a.id == agent.id) else {
                continue;
            };
            for (key, value) in &mut agent.env {
                if value == "[redacted]"
                    && let Some(kept) = existing.env.get(key)
                {
                    *value = kept.clone();
                }
            }
        }
        config.validate()?;
        if let Some(parent) = self.config.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.config, toml::to_string_pretty(&config)?)?;
        Ok(())
    }

    pub fn memory(&self) -> Result<Remembered> {
        let path = self.data.join("memory/USER.md");
        Ok(Remembered {
            user: std::fs::read_to_string(&path).unwrap_or_default(),
            path: path.display().to_string(),
        })
    }

    pub fn save_memory(&self, text: &str) -> Result<()> {
        let path = self.data.join("memory/USER.md");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text)?;
        Ok(())
    }

    /// Deletes every conversation. R4: delete means gone, and `secure_delete` means the text
    /// is overwritten rather than left in free pages.
    pub fn forget_all(&self) -> Result<usize> {
        let mut removed = 0;
        for project in self.activity.sidebar(usize::MAX, true)? {
            for thread in project.threads {
                if self.activity.delete_thread(&thread.id)? {
                    removed += 1;
                }
            }
        }
        Ok(removed)
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

    /// What has been said in this thread's room, oldest first.
    pub fn room(&self, thread: &str) -> Result<Vec<Said>> {
        let connection = self.activity.connection();
        let mut statement = connection.prepare(
            "SELECT seq, kind, who, whom, via, text, at, role, model FROM v_room
             WHERE project_id = (SELECT project_id FROM threads WHERE id=?1)
               AND (thread_id IS NULL OR thread_id=?1)
             ORDER BY at, seq",
        )?;
        let rows = statement.query_map([thread], |r| {
            Ok(Said {
                seq: r.get(0)?,
                kind: r.get(1)?,
                who: r.get(2)?,
                whom: r.get(3)?,
                via: r.get(4)?,
                text: r.get(5)?,
                // The mailbox keeps seconds and the store keeps milliseconds. A client reads
                // them as one conversation, so they leave here on one clock — taken as-is, a
                // message sorted before every item ever written.
                at: r.get::<_, i64>(6)? * 1000,
                role: r.get(7)?,
                model: r.get(8)?,
            })
        })?;
        rows.map(|row| Ok(row?)).collect()
    }

    /// Leaves a note in the room, for one seat or for everyone. It reaches them when they next
    /// read their messages — a note on the table, not an interruption.
    pub fn say(&self, thread: &str, to: Option<&str>, text: &str) -> Result<()> {
        let project: String = self.activity.connection().query_row(
            "SELECT project_id FROM threads WHERE id=?1",
            [thread],
            |r| r.get(0),
        )?;
        let mailbox = orochi::mailbox::Mailbox::open(self.store.data_dir(), &self.mailbox)?;
        mailbox.speak(&project, to, text, Some(thread))?;
        Ok(())
    }

    /// Each agent's readiness and what is left of its quota.
    pub fn agents(&self) -> Result<Vec<AgentRow>> {
        let connection = self.store.connection();
        let mut windows: std::collections::BTreeMap<String, Vec<(String, f64, Option<i64>)>> =
            Default::default();
        let mut statement =
            connection.prepare("SELECT agent, bucket, remaining, reset_at FROM v_quota")?;
        for row in statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })? {
            let (agent, bucket, remaining, reset) = row?;
            windows
                .entry(agent)
                .or_default()
                .push((bucket, remaining, reset));
        }
        let mut statement = connection.prepare(
            "SELECT agent, model, status, cooling, reset_at, quota_estimate,
                    consecutive_failures
             FROM v_runtime ORDER BY agent, model",
        )?;
        let rows = statement.query_map([], |r| {
            let agent: String = r.get(0)?;
            Ok(AgentRow {
                windows: windows.get(&agent).cloned().unwrap_or_default(),
                agent,
                model: r.get(1)?,
                status: r.get(2)?,
                cooling: r.get(3)?,
                reset_at: r.get(4)?,
                quota_estimate: r.get(5)?,
                failures: r.get(6)?,
            })
        })?;
        rows.map(|row| Ok(row?)).collect()
    }

    /// What each route has actually done, best evidence first.
    pub fn insights(&self) -> Result<Vec<RouteStats>> {
        let connection = self.store.connection();
        let mut statement = connection.prepare(
            "SELECT agent, model, task_type, reasoning_level, verified, failures, weak,
                    success_rate, mean_tokens, mean_duration_ms, last_at
             FROM v_route_stats ORDER BY verified DESC, agent, model",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(RouteStats {
                agent: r.get(0)?,
                model: r.get(1)?,
                task_type: r.get(2)?,
                reasoning: r.get(3)?,
                verified: r.get(4)?,
                failures: r.get(5)?,
                weak: r.get(6)?,
                success_rate: r.get(7)?,
                mean_tokens: r.get(8)?,
                mean_duration_ms: r.get(9)?,
                last_at: r.get(10)?,
            })
        })?;
        rows.map(|row| Ok(row?)).collect()
    }

    /// The scopes §6.5 offers over the working tree. `last turn` is the stored patches; these
    /// three are `git diff`, which is the authority for review — what a turn recorded is what
    /// the agent *said* it changed, and the tree may have moved on since.
    fn scope_args(scope: &str) -> Result<Vec<&'static str>> {
        Ok(match scope {
            "unstaged" => vec!["diff"],
            "staged" => vec!["diff", "--cached"],
            // Everything since this branch left the one it came from.
            "branch" => vec!["diff", "--merge-base", "HEAD@{upstream}"],
            other => anyhow::bail!("no diff scope called {other}"),
        })
    }

    fn cwd(&self, thread: &str) -> Result<std::path::PathBuf> {
        let cwd: String = self.activity.connection().query_row(
            "SELECT cwd FROM threads WHERE id=?1",
            [thread],
            |r| r.get(0),
        )?;
        Ok(std::path::PathBuf::from(cwd))
    }

    /// What the working tree has in this scope, with its stats.
    pub fn tree_files(&self, thread: &str, scope: &str) -> Result<Vec<FileRow>> {
        let cwd = self.cwd(thread)?;
        let mut args = Self::scope_args(scope)?;
        args.push("--numstat");
        let out = orochi::context::git(&cwd, &args)
            // A branch with no upstream has nothing to compare against; that is not an error
            // worth a dialog, it is an empty list.
            .or_else(|| {
                (scope == "branch").then(|| {
                    orochi::context::git(&cwd, &["diff", "--numstat", "HEAD"]).unwrap_or_default()
                })
            })
            .unwrap_or_default();
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let added = fields.next()?;
                let removed = fields.next()?;
                let path = fields.next()?;
                Some(FileRow {
                    turn: String::new(),
                    path: path.to_owned(),
                    // `-` is git's way of saying binary.
                    change: "modify".into(),
                    added: added.parse().unwrap_or(0),
                    removed: removed.parse().unwrap_or(0),
                    latest_patch: 0,
                })
            })
            .collect())
    }

    /// One file's diff in that scope.
    pub fn tree_patch(&self, thread: &str, scope: &str, path: &str) -> Result<String> {
        let cwd = self.cwd(thread)?;
        let mut args = Self::scope_args(scope)?;
        args.push("--");
        args.push(path);
        Ok(orochi::context::git(&cwd, &args).unwrap_or_default())
    }

    /// Turns comments left on a diff into the next message. There is no review mechanism of
    /// its own here: it is a message, routed like any other, continuing the same thread.
    pub fn comment(&self, thread: &str, comments: &[(String, u64, String)]) -> Result<String> {
        ensure!(!comments.is_empty(), "there is nothing to say");
        let text = comments
            .iter()
            .map(|(path, line, note)| format!("{path}:{line} — {note}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.send(thread, &text)
    }

    /// The work graph of a turn, when it has one.
    pub fn board(&self, thread: &str) -> Result<Vec<serde_json::Value>> {
        let connection = self.activity.connection();
        let mut statement = connection.prepare(
            "SELECT id, brief, paths, after, wave, state, strays, seat_id FROM v_board
             WHERE thread_id=?1 ORDER BY COALESCE(wave, 0), id",
        )?;
        let rows = statement.query_map([thread], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, String>(0)?,
                "brief": r.get::<_, String>(1)?,
                "paths": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(2)?)
                    .unwrap_or_default(),
                "after": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(3)?)
                    .unwrap_or_default(),
                "wave": r.get::<_, Option<i64>>(4)?,
                "state": r.get::<_, String>(5)?,
                "strays": r.get::<_, Option<i64>>(6)?,
                "seat": r.get::<_, Option<String>>(7)?,
            }))
        })?;
        rows.map(|row| Ok(row?)).collect()
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
    /// Looked at on every poll, because a host can stop while the window stays open — its
    /// machine sleeps, it is killed, it crashes — and nothing else here would ever notice.
    pub fn watch(&self, thread: &str) -> Result<Watch> {
        let running: bool = self.activity.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE thread_id=?1 AND state='running')",
            [thread],
            |r| r.get(0),
        )?;
        self.activity.reap_hosts()?;
        let (still, queued): (bool, bool) = self.activity.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE thread_id=?1 AND state='running'),
                    EXISTS(SELECT 1 FROM turns WHERE thread_id=?1 AND state='queued')",
            [thread],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(Watch {
            lost: running && !still,
            queued,
        })
    }

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
