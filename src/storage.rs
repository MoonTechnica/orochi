use crate::types::*;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::BTreeMap, fs::File, path::Path, time::Duration};

/// A classification is a property of the wording, not of the repository, but a stale one
/// would outlive the model that produced it.
const CLASSIFICATION_TTL: i64 = 30 * 24 * 3600;

/// Additive changes an older binary survives: virtual generated columns over `record` and the
/// indexes on them. They take no space, `INSERT INTO runs VALUES (…)` without a column list
/// does not count them, and the JSON stays the source of truth — so `user_version` must *not*
/// move for these, or a newer Orochi touching the file would lock the user's older CLI out of
/// it. `metadata.schema_minor` records them instead.
const SCHEMA_MINOR: i64 = 1;

/// What a client reads telemetry through, so the numbers on its screens are the ones this
/// crate computes rather than a second calculation that can drift from them. Recreated on
/// every open, because a view is a definition and not data.
///
/// The distinction `calibrate` makes is kept here: only a verified outcome is evidence, and a
/// weak signal (what the user did next) is counted beside it, never inside it.
const VIEWS: &str = r#"
DROP VIEW IF EXISTS v_runs;
CREATE VIEW v_runs AS
SELECT id, task_id, repository_id, task_type, agent, model, started_at,
       purpose, language, framework, complexity, provider, reasoning_level, mode,
       outcome, error_kind, feedback_signal,
       json_extract(record,'$.duration_ms') AS duration_ms,
       json_extract(record,'$.usage.total_tokens') AS total_tokens,
       json_extract(record,'$.attempt') AS attempt,
       json_extract(record,'$.prediction.success') AS predicted_success,
       json_extract(record,'$.prediction.tokens') AS predicted_tokens
FROM runs;

DROP VIEW IF EXISTS v_route_stats;
CREATE VIEW v_route_stats AS
SELECT agent, model, task_type, provider, reasoning_level, mode,
       count(*) FILTER (WHERE outcome IN ('success','failure')) AS verified,
       count(*) FILTER (WHERE outcome='failure') AS failures,
       count(*) FILTER (WHERE outcome NOT IN ('success','failure')
                          AND feedback_signal IS NOT NULL) AS weak,
       COALESCE(avg(CASE WHEN outcome IN ('success','failure')
                         THEN CASE WHEN outcome='success' THEN 1.0 ELSE 0.0 END END), 0.0)
         AS success_rate,
       COALESCE(avg(CASE WHEN outcome IN ('success','failure')
                         THEN json_extract(record,'$.usage.total_tokens') END), 0.0)
         AS mean_tokens,
       COALESCE(avg(CASE WHEN outcome IN ('success','failure')
                         THEN json_extract(record,'$.duration_ms') END), 0.0)
         AS mean_duration_ms,
       max(started_at) AS last_at
FROM runs WHERE purpose='execution'
GROUP BY agent, model, task_type, provider, reasoning_level, mode;

DROP VIEW IF EXISTS v_runtime;
CREATE VIEW v_runtime AS
SELECT agent, model,
       json_extract(record,'$.status') AS status,
       json_extract(record,'$.quota_estimate') AS quota_estimate,
       json_extract(record,'$.reset_at') AS reset_at,
       json_extract(record,'$.cooldown_until') AS cooldown_until,
       json_extract(record,'$.consecutive_failures') AS consecutive_failures,
       json_extract(record,'$.rate_limit_count') AS rate_limit_count,
       json_extract(record,'$.last_success_at') AS last_success_at,
       -- Seconds of cooldown left, so a screen does not have to know the clock's units.
       max(0, COALESCE(json_extract(record,'$.cooldown_until'), 0) - unixepoch()) AS cooling
FROM runtime;

DROP VIEW IF EXISTS v_quota;
CREATE VIEW v_quota AS
SELECT q.agent,
       json_extract(q.record,'$.source') AS source,
       json_extract(q.record,'$.observed_at') AS observed_at,
       json_extract(q.record,'$.valid_until') AS valid_until,
       json_extract(w.value,'$.bucket') AS bucket,
       json_extract(w.value,'$.remaining') AS remaining,
       json_extract(w.value,'$.reset_at') AS reset_at,
       json_extract(w.value,'$.model') AS model,
       json_extract(w.value,'$.affects_routing') AS affects_routing
FROM quota_snapshots q, json_each(q.record,'$.windows') w;
"#;

/// Columns the learning queries filter on, so the planner has an index to use instead of
/// evaluating `json_extract` per row.
const GENERATED: &[(&str, &str)] = &[
    ("purpose", "$.purpose"),
    ("language", "$.language"),
    ("framework", "$.framework"),
    ("complexity", "$.complexity"),
    ("provider", "$.candidate.provider"),
    ("reasoning_level", "$.candidate.reasoning_level"),
    ("mode", "$.candidate.mode"),
    ("outcome", "$.outcome"),
    ("error_kind", "$.error_kind"),
    ("feedback_signal", "$.feedback.signal"),
];

/// Schema setup and the WAL switch can report BUSY without waiting when several Orochi
/// processes open a fresh database at once; retry them briefly.
pub(crate) fn setup(connection: &Connection, sql: &str) -> Result<()> {
    let mut attempts = 0;
    loop {
        match connection.execute_batch(sql) {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) && attempts < 100 =>
            {
                if !connection.is_autocommit() {
                    let _ = connection.execute_batch("ROLLBACK");
                }
                attempts += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return Ok(result?),
        }
    }
}

/// Applies the additive changes of `SCHEMA_MINOR`. An older binary that never ran this still
/// reads and writes the file; a newer one that finds them already applied does nothing.
fn minor_migrations(connection: &Connection) -> Result<()> {
    let applied: i64 = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='schema_minor'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if applied >= SCHEMA_MINOR {
        return setup(connection, VIEWS);
    }
    // Another process may have applied these between the read and the write, and a column
    // already there is not an error worth failing an `open` over.
    let existing: std::collections::BTreeSet<String> = connection
        .prepare("SELECT name FROM pragma_table_info('runs')")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut sql = String::from("BEGIN IMMEDIATE;");
    for (column, path) in GENERATED {
        if !existing.contains(*column) {
            sql.push_str(&format!(
                "ALTER TABLE runs ADD COLUMN {column} TEXT GENERATED ALWAYS AS (json_extract(record,'{path}')) VIRTUAL;"
            ));
        }
    }
    sql.push_str(
        "CREATE INDEX IF NOT EXISTS runs_learning ON runs(agent, model, task_type, purpose, started_at);
         CREATE INDEX IF NOT EXISTS runs_tier ON runs(provider, task_type, complexity, started_at);
         CREATE INDEX IF NOT EXISTS runs_purpose ON runs(purpose, started_at);
         INSERT INTO metadata VALUES ('schema_minor', '1') ON CONFLICT(key) DO UPDATE SET value=excluded.value;
         COMMIT;",
    );
    setup(connection, &sql)?;
    // A view is a definition, not data: recreating it costs nothing and keeps an older
    // binary's copy from outliving the columns it was written against.
    setup(connection, VIEWS)
}

pub struct Store {
    connection: Connection,
    data: std::path::PathBuf,
}
impl Store {
    pub fn open(data: &Path) -> Result<Self> {
        std::fs::create_dir_all(data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(data, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = data.join("telemetry.sqlite3");
        let connection = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        connection.busy_timeout(Duration::from_secs(5))?;
        setup(
            &connection,
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 2,
            "telemetry database is newer than this Orochi version"
        );
        setup(&connection, "BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL, repository_id TEXT NOT NULL,
                task_type TEXT NOT NULL, agent TEXT NOT NULL, model TEXT NOT NULL,
                started_at INTEGER NOT NULL, record TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS runs_history ON runs(agent, model, task_type, started_at);
            CREATE TABLE IF NOT EXISTS runtime (agent TEXT NOT NULL, model TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(agent, model));
            CREATE TABLE IF NOT EXISTS sessions (session_id TEXT NOT NULL, agent TEXT NOT NULL, repository_id TEXT NOT NULL,
                updated_at INTEGER NOT NULL, record TEXT NOT NULL, PRIMARY KEY(agent, session_id));
            CREATE TABLE IF NOT EXISTS quota_snapshots (agent TEXT PRIMARY KEY, record TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS classifications (task_hash TEXT PRIMARY KEY, record TEXT NOT NULL, created_at INTEGER NOT NULL);
            PRAGMA user_version=2; COMMIT;")?;
        connection.execute(
            "INSERT OR IGNORE INTO metadata VALUES ('repository_salt', ?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        minor_migrations(&connection)?;
        Ok(Self {
            connection,
            data: std::fs::canonicalize(data)?,
        })
    }
    pub fn data_dir(&self) -> &Path {
        &self.data
    }
    /// For reading the `v_*` views. Telemetry is written only through the methods below, so
    /// this hands out a reader, not a way in.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn salt(&self) -> Result<String> {
        Ok(self.connection.query_row(
            "SELECT value FROM metadata WHERE key='repository_salt'",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn repository_id(&self, root: &Path) -> Result<String> {
        Ok(crate::context::hash(&[
            self.salt()?.as_bytes(),
            root.as_os_str().as_encoded_bytes(),
        ]))
    }
    pub fn record(&self, run: &RunRecord) -> Result<()> {
        self.connection.execute(
            "INSERT INTO runs (id, task_id, repository_id, task_type, agent, model, started_at, record)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                run.id,
                run.task_id,
                run.repository_id,
                run.task_type,
                run.candidate.agent,
                run.candidate.model,
                run.started_at,
                serde_json::to_string(run)?
            ],
        )?;
        Ok(())
    }
    /// What work like this has cost here: the median measured tokens of recent executions
    /// carrying the same labels, or of recent executions at large when that stratum is too
    /// thin to mean anything. `None` until there is anything to go by.
    pub fn typical_tokens(&self, task_type: &str, complexity: Complexity) -> Result<Option<f64>> {
        const SAMPLES: usize = 3;
        let mut statement = self.connection.prepare(
            "SELECT record FROM runs WHERE json_extract(record, '$.purpose')='execution'
             ORDER BY started_at DESC, rowid DESC LIMIT 128",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let (mut stratum, mut every) = (vec![], vec![]);
        for row in rows {
            let Ok(run) = serde_json::from_str::<RunRecord>(&row?) else {
                continue;
            };
            let Some(total) = run.usage.total_tokens else {
                continue;
            };
            every.push(total as f64);
            if run.task_type == task_type && run.complexity == Some(complexity) {
                stratum.push(total as f64);
            }
        }
        let mut measured = if stratum.len() >= SAMPLES {
            stratum
        } else {
            every
        };
        if measured.len() < SAMPLES {
            return Ok(None);
        }
        measured.sort_by(f64::total_cmp);
        Ok(Some(measured[measured.len() / 2]))
    }

    /// What asking each agent for advice has cost lately: the mean total tokens of its most
    /// recent records of that purpose. Advice is not execution and never teaches the router;
    /// this is only about what the question itself costs.
    pub fn advice_cost(&self, purpose: &str, limit: usize) -> Result<BTreeMap<String, f64>> {
        let mut statement = self.connection.prepare(
            "SELECT record FROM runs WHERE purpose=?1
             ORDER BY started_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![purpose, limit as i64], |row| {
            row.get::<_, String>(0)
        })?;
        let mut spent: BTreeMap<String, (f64, usize)> = BTreeMap::new();
        for row in rows {
            let Ok(run) = serde_json::from_str::<RunRecord>(&row?) else {
                continue;
            };
            let Some(total) = run.usage.total_tokens else {
                continue;
            };
            let entry = spent.entry(run.candidate.agent).or_default();
            entry.0 += total as f64;
            entry.1 += 1;
        }
        Ok(spent
            .into_iter()
            .map(|(agent, (total, count))| (agent, total / count as f64))
            .collect())
    }

    pub fn recent_runs(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let mut stmt = self
            .connection
            .prepare("SELECT record FROM runs ORDER BY started_at DESC, rowid DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |row| row.get::<_, String>(0))?;
        rows.map(|s| Ok(serde_json::from_str(&s?)?)).collect()
    }
    pub fn history(
        &self,
        candidate: &ExecutionCandidate,
        descriptor: &TaskDescriptor,
    ) -> Result<History> {
        let mut stmt = self.connection.prepare("SELECT record FROM runs WHERE agent=?1 AND model=?2 AND task_type=?3 ORDER BY started_at DESC LIMIT 100")?;
        let rows = stmt.query_map(
            params![candidate.agent, candidate.model, descriptor.task_type],
            |r| r.get::<_, String>(0),
        )?;
        let mut history = History::default();
        for row in rows {
            let run: RunRecord = serde_json::from_str(&row?)?;
            if run.purpose != "execution"
                || run.candidate.reasoning_level != candidate.reasoning_level
                || run.language != descriptor.language
                || run.outcome == Outcome::Cancelled
            {
                continue;
            }
            if matches!(
                run.error_kind.as_deref(),
                Some("rate_limit" | "authentication" | "unavailable")
            ) {
                continue;
            }
            // Unverified completions are not positive labels for the success model.
            if run.outcome == Outcome::PartialSuccess {
                continue;
            }
            history.samples += 1;
            history.successes += usize::from(run.outcome == Outcome::Success);
            if let Some(tokens) = run.usage.total() {
                history.total_tokens += tokens;
                history.token_samples += 1;
            }
            history.duration_ms += run.duration_ms;
        }
        Ok(history)
    }
    /// Match the arm and context before applying LIMIT; unrelated runs cannot crowd out samples.
    pub fn learning_runs(
        &self,
        candidate: &ExecutionCandidate,
        task: &TaskDescriptor,
    ) -> Result<Vec<RunRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT record FROM runs WHERE agent=?1 AND model=?2 AND task_type=?3
            AND purpose='execution'
            AND language=?4
            AND framework IS ?5
            AND reasoning_level IS ?6
            AND mode IS ?7
            AND complexity=?8
            AND provider=?9
            ORDER BY started_at DESC, rowid DESC LIMIT 500",
        )?;
        let rows = stmt.query_map(
            params![
                candidate.agent,
                candidate.model,
                task.task_type,
                task.language,
                task.framework,
                candidate.reasoning_level,
                candidate.mode,
                task.complexity.key(),
                candidate.provider.to_string()
            ],
            |r| r.get::<_, String>(0),
        )?;
        let mut runs: Vec<RunRecord> = rows
            .map(|s| Ok(serde_json::from_str(&s?)?))
            .collect::<Result<_>>()?;
        runs.reverse();
        Ok(runs)
    }
    /// Other models of this provider on the same kind of work, oldest first, for tier pooling.
    /// Tier is read from the policy by the caller (`same_tier`), not stored, so every run
    /// recorded before tiers were pooled still counts.
    pub fn tier_runs(
        &self,
        candidate: &ExecutionCandidate,
        task: &TaskDescriptor,
        same_tier: impl Fn(&str) -> bool,
    ) -> Result<Vec<RunRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT record FROM runs WHERE model!=?1 AND task_type=?2
            AND purpose='execution'
            AND provider=?3
            AND complexity=?4
            AND reasoning_level IS ?5
            AND mode IS ?6
            ORDER BY started_at DESC, rowid DESC LIMIT 500",
        )?;
        let rows = stmt.query_map(
            params![
                candidate.model,
                task.task_type,
                candidate.provider.to_string(),
                task.complexity.key(),
                candidate.reasoning_level,
                candidate.mode
            ],
            |r| r.get::<_, String>(0),
        )?;
        let mut runs = Vec::new();
        for row in rows {
            let run: RunRecord = serde_json::from_str(&row?)?;
            if same_tier(&run.candidate.model) {
                runs.push(run);
            }
        }
        runs.reverse();
        Ok(runs)
    }
    /// The same arm in every other task context, oldest first.
    pub fn pooled_runs(
        &self,
        candidate: &ExecutionCandidate,
        task: &TaskDescriptor,
    ) -> Result<Vec<RunRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT record FROM runs WHERE agent=?1 AND model=?2
            AND purpose='execution'
            AND reasoning_level IS ?3
            AND mode IS ?4
            AND provider=?5
            AND NOT (task_type=?6 AND language=?7
                AND framework IS ?8
                AND complexity IS ?9)
            ORDER BY started_at DESC, rowid DESC LIMIT 500",
        )?;
        let rows = stmt.query_map(
            params![
                candidate.agent,
                candidate.model,
                candidate.reasoning_level,
                candidate.mode,
                candidate.provider.to_string(),
                task.task_type,
                task.language,
                task.framework,
                task.complexity.key()
            ],
            |r| r.get::<_, String>(0),
        )?;
        let mut runs: Vec<RunRecord> = rows
            .map(|s| Ok(serde_json::from_str(&s?)?))
            .collect::<Result<_>>()?;
        runs.reverse();
        Ok(runs)
    }
    pub fn runtime(&self) -> Result<RuntimeMap> {
        let mut stmt = self.connection.prepare("SELECT record FROM runtime")?;
        let mut map = RuntimeMap::new();
        for row in stmt.query_map([], |row| row.get::<_, String>(0))? {
            let state: RuntimeState = serde_json::from_str(&row?)?;
            map.insert((state.agent.clone(), state.model.clone()), state);
        }
        for snapshot in self.quota_snapshots()? {
            crate::quota_sources::apply(&mut map, &snapshot, now());
        }
        Ok(map)
    }
    /// Adds what the user did after a run. The first signal is the one that counts, and nothing
    /// else in the record changes — in particular not its frozen prediction.
    pub fn feedback(&self, run_id: &str, feedback: &Feedback) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE runs SET record=json_set(record, '$.feedback', json(?2))
            WHERE id=?1 AND json_extract(record, '$.feedback') IS NULL",
            params![run_id, serde_json::to_string(feedback)?],
        )? == 1)
    }
    /// Keyed by the salted hash of the task, so a classification can be reused without the
    /// task text being stored anywhere.
    pub fn classification_key(&self, task: &str) -> Result<String> {
        Ok(crate::context::hash(&[
            self.salt()?.as_bytes(),
            b"classification",
            task.as_bytes(),
        ]))
    }
    pub fn classification(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT record FROM classifications WHERE task_hash=?1 AND created_at>?2",
                params![key, crate::types::now() - CLASSIFICATION_TTL],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_classification(&self, key: &str, record: &str) -> Result<()> {
        let now = crate::types::now();
        self.connection.execute(
            "INSERT INTO classifications VALUES (?1,?2,?3) ON CONFLICT(task_hash) DO UPDATE SET record=excluded.record, created_at=excluded.created_at",
            params![key, record, now],
        )?;
        self.connection.execute(
            "DELETE FROM classifications WHERE created_at<=?1",
            [now - CLASSIFICATION_TTL],
        )?;
        Ok(())
    }

    pub fn save_quota_snapshot(&self, snapshot: &crate::quota_sources::Snapshot) -> Result<()> {
        self.connection.execute("INSERT INTO quota_snapshots VALUES (?1,?2) ON CONFLICT(agent) DO UPDATE SET record=excluded.record",
            params![snapshot.agent, serde_json::to_string(snapshot)?])?;
        Ok(())
    }
    pub fn quota_snapshots(&self) -> Result<Vec<crate::quota_sources::Snapshot>> {
        let mut stmt = self
            .connection
            .prepare("SELECT record FROM quota_snapshots ORDER BY agent")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|s| Ok(serde_json::from_str(&s?)?))
            .collect()
    }
    pub fn save_runtime(&self, state: &RuntimeState) -> Result<()> {
        self.connection.execute("INSERT INTO runtime VALUES (?1,?2,?3) ON CONFLICT(agent,model) DO UPDATE SET record=excluded.record", params![state.agent, state.model, serde_json::to_string(state)?])?;
        Ok(())
    }
    /// Serialize read-modify-write across runs in different repositories sharing an account.
    pub fn update_runtime(
        &self,
        agent: &str,
        model: &str,
        update: impl FnOnce(&mut RuntimeState),
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let record: Option<String> = tx
            .query_row(
                "SELECT record FROM runtime WHERE agent=?1 AND model=?2",
                params![agent, model],
                |r| r.get(0),
            )
            .optional()?;
        let mut state = match record {
            Some(record) => serde_json::from_str(&record)?,
            None => RuntimeState::new(agent, model),
        };
        update(&mut state);
        tx.execute("INSERT INTO runtime VALUES (?1,?2,?3) ON CONFLICT(agent,model) DO UPDATE SET record=excluded.record", params![agent, model, serde_json::to_string(&state)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn session(&self, session: &SessionRecord) -> Result<()> {
        self.connection.execute("INSERT INTO sessions VALUES (?1,?2,?3,?4,?5) ON CONFLICT(agent,session_id) DO UPDATE SET updated_at=excluded.updated_at, record=excluded.record", params![session.session_id, session.agent, session.repository_id, session.updated_at, serde_json::to_string(session)?])?;
        Ok(())
    }
    pub fn sessions(&self, repo: Option<&str>) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.connection.prepare("SELECT record FROM sessions WHERE (?1 IS NULL OR repository_id=?1) ORDER BY updated_at DESC LIMIT 100")?;
        stmt.query_map([repo], |r| r.get::<_, String>(0))?
            .map(|s| Ok(serde_json::from_str(&s?)?))
            .collect()
    }
    pub fn get_session(&self, agent: &str, id: &str, repo: &str) -> Result<Option<SessionRecord>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT record FROM sessions WHERE agent=?1 AND session_id=?2 AND repository_id=?3",
                params![agent, id, repo],
                |r| r.get(0),
            )
            .optional()?;
        value.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
}

pub struct WorkspaceLock(File);
impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Exclusive by default. Shared locks let concurrent runs use one directory, and still
/// exclude an exclusive holder (for example applying a collaboration result).
pub fn workspace_lock(data: &Path, repo_id: &str, shared: bool) -> Result<WorkspaceLock> {
    let dir = data.join("locks");
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(format!("{repo_id}.lock")))?;
    let locked = if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    };
    locked.map_err(|_| {
        anyhow::anyhow!(
            "another Orochi run is using this repository (set scheduler.shared_workspace = true to run several in one directory)"
        )
    })?;
    Ok(WorkspaceLock(file))
}
