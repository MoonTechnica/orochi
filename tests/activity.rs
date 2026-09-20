//! The conversation store and the telemetry changes that let an older Orochi keep sharing the
//! data directory with a newer one.
use orochi::{
    activity::{Activity, ItemKind, Origin, SeatState},
    storage::Store,
    types::*,
};
use rusqlite::params;

fn candidate(agent: &str, model: &str) -> ExecutionCandidate {
    ExecutionCandidate {
        id: format!("{agent}:{model}"),
        agent: agent.into(),
        provider: Provider::Anthropic,
        model: model.into(),
        reasoning_level: Some("high".into()),
        mode: None,
        session_strategy: "fresh".into(),
        context_strategy: "none".into(),
        success_probability: 0.8,
        expected_tokens: 1000.0,
        expected_cost: 1.0,
        confidence: 0.5,
        reasons: vec![],
        prediction: None,
    }
}

/// Everything a P0 screen needs, built the way a host builds it.
fn thread_with_a_turn(activity: &Activity) -> (String, String, String) {
    activity
        .project("proj", std::path::Path::new("/tmp/repo"))
        .unwrap();
    let thread = activity
        .create_thread(
            "proj",
            std::path::Path::new("/tmp/repo"),
            Some("main"),
            "repo-hash",
            Origin::Console,
            &Overrides::default(),
            "ask",
        )
        .unwrap();
    let turn = activity
        .queue_turn(&thread, "Fix the flaky test", &[], "auto", "terminal")
        .unwrap();
    (thread, turn, "proj".into())
}

#[test]
fn a_turn_reads_back_through_the_views_the_app_is_promised() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);

    let host = activity.register_host(Some(&thread), "terminal").unwrap();
    assert!(activity.claim_turn(&turn, &host).unwrap());
    // A second host cannot take a turn that is already running.
    assert!(!activity.claim_turn(&turn, "other").unwrap());

    let lead = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let side = activity
        .create_seat(&turn, 1, "reviewer", false, true, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&lead, &candidate("claude", "opus"), false, None)
        .unwrap();
    activity.seat_state(&lead, SeatState::Working).unwrap();

    let message = activity
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::AgentMessage,
            Some("streaming"),
            None,
            "",
            None,
        )
        .unwrap();
    activity.append(message, "The race ").unwrap();
    activity.append(message, "is in the sleep.").unwrap();
    activity.item_status(message, "completed").unwrap();

    let tool = activity
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::ToolCall,
            Some("in_progress"),
            Some("call-1"),
            "",
            Some(r#"{"title":"Edit","tool_kind":"edit"}"#),
        )
        .unwrap();
    assert_eq!(activity.tool_item(&attempt, "call-1").unwrap(), Some(tool));
    activity
        .patch(
            tool,
            &turn,
            "tests/mailbox.rs",
            Some("a\nb\nc\n"),
            "a\nB\nc\n",
        )
        .unwrap();
    activity
        .update_tool(tool, Some("completed"), Some("done"), None)
        .unwrap();

    let connection = Activity::open_read_only(dir.path()).unwrap();

    let (project, title, seats, running): (String, String, i64, i64) = connection
        .query_row(
            "SELECT project, title, seats, running FROM v_sidebar WHERE id=?1",
            [&thread],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(project, "repo");
    assert_eq!(
        title, "Fix the flaky test",
        "the first line titles the thread"
    );
    assert_eq!(seats, 2, "both seats are open");
    assert_eq!(running, 1);

    let timeline: Vec<(String, String, Option<i64>)> = connection
        .prepare("SELECT kind, text, lane FROM v_timeline WHERE thread_id=?1 ORDER BY seq")
        .unwrap()
        .query_map([&thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        timeline,
        vec![
            ("user_message".into(), "Fix the flaky test".into(), None),
            (
                "agent_message".into(),
                "The race is in the sleep.".into(),
                Some(0)
            ),
            ("tool_call".into(), "done".into(), Some(0)),
        ],
        "the timeline replays what happened, in order, with the seat that did it"
    );

    let (path, added, removed, change): (String, i64, i64, String) = connection
        .query_row(
            "SELECT path, added, removed, change FROM v_turn_files WHERE turn_id=?1",
            [&turn],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (path.as_str(), added, removed, change.as_str()),
        ("tests/mailbox.rs", 1, 1, "modify")
    );

    let roles: Vec<(String, i64, Option<String>)> = connection
        .prepare("SELECT role, read_only, model FROM v_roster WHERE thread_id=?1 ORDER BY ordinal")
        .unwrap()
        .query_map([&thread], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        roles,
        vec![
            ("implementer".into(), 0, Some("opus".into())),
            ("reviewer".into(), 1, None),
        ],
        "a seat appears in the roster before it has chosen an agent"
    );
    let _ = side;
}

#[test]
fn the_change_feed_names_what_moved_for_a_reader_in_another_process() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let reader = Activity::open_read_only(dir.path()).unwrap();
    let version = |c: &rusqlite::Connection| -> i64 {
        c.query_row("PRAGMA data_version", [], |r| r.get(0))
            .unwrap()
    };
    let before = version(&reader);
    let cursor: i64 = reader
        .query_row("SELECT COALESCE(max(id),0) FROM changes", [], |r| r.get(0))
        .unwrap();

    let (thread, turn, _) = thread_with_a_turn(&activity);

    assert_ne!(
        version(&reader),
        before,
        "another connection's commit moves data_version"
    );
    let moved: Vec<(String, Option<String>)> = reader
        .prepare("SELECT tbl, thread_id FROM changes WHERE id > ?1 ORDER BY id")
        .unwrap()
        .query_map([cursor], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(
        moved.contains(&("threads".into(), Some(thread.clone())))
            && moved.contains(&("turns".into(), Some(thread.clone())))
            && moved.contains(&("items".into(), Some(thread.clone()))),
        "the feed names the thread every change belongs to: {moved:?}"
    );
    let _ = turn;
}

#[test]
fn a_read_only_connection_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    thread_with_a_turn(&activity);
    let reader = Activity::open_read_only(dir.path()).unwrap();
    assert!(
        reader.execute("DELETE FROM threads", []).is_err(),
        "query_only is what makes a viewer a viewer"
    );
}

#[test]
fn deleting_a_thread_leaves_none_of_its_text_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "the quick brown fox jumps over the lazy dog";
    {
        let activity = Activity::open(dir.path(), 30).unwrap();
        let (thread, ..) = thread_with_a_turn(&activity);
        activity
            .queue_turn(&thread, secret, &[], "auto", "terminal")
            .unwrap();
        let raw = std::fs::read(dir.path().join("activity.sqlite3")).unwrap();
        let wal = std::fs::read(dir.path().join("activity.sqlite3-wal")).unwrap_or_default();
        assert!(
            raw.windows(secret.len()).any(|w| w == secret.as_bytes())
                || wal.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "the text is there while the thread is"
        );
        assert!(activity.delete_thread(&thread).unwrap());
        activity
            .connection()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }
    for file in ["activity.sqlite3", "activity.sqlite3-wal"] {
        let raw = std::fs::read(dir.path().join(file)).unwrap_or_default();
        assert!(
            !raw.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "secure_delete overwrites a deleted thread's text in {file}"
        );
    }
}

#[test]
fn retention_removes_unpinned_threads_only() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (old, ..) = thread_with_a_turn(&activity);
    let kept = activity
        .create_thread(
            "proj",
            std::path::Path::new("/tmp/repo"),
            None,
            "repo-hash",
            Origin::Console,
            &Overrides::default(),
            "ask",
        )
        .unwrap();
    let stale = orochi::activity::millis() - 40 * 86_400_000;
    activity
        .connection()
        .execute("UPDATE threads SET updated_at=?1", [stale])
        .unwrap();
    activity
        .connection()
        .execute("UPDATE threads SET pinned=1 WHERE id=?1", [&kept])
        .unwrap();
    activity.prune().unwrap();
    let left: Vec<String> = activity
        .connection()
        .prepare("SELECT id FROM threads")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(left, vec![kept], "a pinned thread outlives the window");
    let _ = old;
}

#[test]
fn a_thread_is_interrupted_when_the_host_running_it_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (_, turn, _) = thread_with_a_turn(&activity);
    let host = activity.register_host(None, "headless").unwrap();
    assert!(activity.claim_turn(&turn, &host).unwrap());
    // A pid that cannot be running: the reaper judges by (pid, start), as the mailbox does.
    activity
        .connection()
        .execute(
            "UPDATE hosts SET pid=?2, start=1 WHERE id=?1",
            params![&host, i64::from(i32::MAX)],
        )
        .unwrap();
    activity.reap_hosts().unwrap();
    let state: String = activity
        .connection()
        .query_row("SELECT state FROM turns WHERE id=?1", [&turn], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "interrupted");
    let hosts: i64 = activity
        .connection()
        .query_row("SELECT count(*) FROM hosts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hosts, 0, "a dead host releases the thread it owned");
}

/// R8: a thread read back out of the store renders the same timeline the live one did. This
/// is the test that keeps "the app is a view over SQLite" true — if an event cannot be
/// recovered from rows, the app needs a second, live-only data path and the premise is false.
#[test]
fn a_recorded_turn_reads_back_as_the_events_that_made_it() {
    use orochi::acp::{ExecutionEvent, FileDiff, Progress, ToolUpdate};
    use orochi::activity::recorder::{Recorder, Seat};

    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);
    let seat = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&seat, &candidate("claude", "opus"), false, None)
        .unwrap();

    let events = vec![
        ExecutionEvent::Progress(Progress::Route {
            agent: "claude".into(),
            provider: Provider::Anthropic,
            model: "opus".into(),
            reasoning: Some("high".into()),
            resumed: false,
        }),
        ExecutionEvent::Progress(Progress::Thinking("Looking at the ".into())),
        ExecutionEvent::Progress(Progress::Thinking("placement test.".into())),
        ExecutionEvent::Progress(Progress::Tool(Box::new(ToolUpdate {
            id: "call-1".into(),
            title: Some("Edit".into()),
            status: Some("in_progress".into()),
            kind: Some("edit".into()),
            locations: vec![("tests/mailbox.rs".into(), Some(12))],
            raw_input: Some(serde_json::json!({"file_path": "tests/mailbox.rs"})),
            ..Default::default()
        }))),
        ExecutionEvent::Progress(Progress::Tool(Box::new(ToolUpdate {
            id: "call-1".into(),
            status: Some("completed".into()),
            diffs: vec![FileDiff {
                path: "tests/mailbox.rs".into(),
                old: Some("a\nsleep(1)\nc\n".into()),
                new: "a\nOrder::place()\nc\n".into(),
            }],
            ..Default::default()
        }))),
        ExecutionEvent::Text("The race ".into(), None),
        ExecutionEvent::Text("is in the sleep.".into(), None),
        ExecutionEvent::Progress(Progress::Context {
            used: 40_000,
            size: Some(200_000),
        }),
        // A kind this version does not know must survive, not be dropped.
        ExecutionEvent::Progress(Progress::Other(
            serde_json::json!({"sessionUpdate": "something_new", "payload": 1}),
        )),
        ExecutionEvent::Finished,
    ];

    {
        let mut recorder = Recorder::new(
            orochi::activity::share(Activity::open(dir.path(), 30).unwrap()),
            Seat {
                thread: thread.clone(),
                turn: turn.clone(),
                seat: seat.clone(),
                attempt: Some(attempt.clone()),
            },
            true,
        );
        for event in &events {
            recorder.record(event);
            // The flush window is 80 ms; a turn that streams faster than that must still end
            // up with every chunk, which is what closing the stream guarantees.
        }
    }

    let connection = Activity::open_read_only(dir.path()).unwrap();
    let rows: Vec<(String, String, Option<String>, Option<String>)> = connection
        .prepare(
            "SELECT kind, text, status, data FROM v_timeline
             WHERE thread_id=?1 AND kind<>'user_message' ORDER BY seq",
        )
        .unwrap()
        .query_map([&thread], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    let kinds: Vec<&str> = rows.iter().map(|(k, ..)| k.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "route",
            "thought",
            "tool_call",
            "agent_message",
            "context",
            "acp"
        ],
        "one row per thing that happened, in the order it happened"
    );

    assert_eq!(rows[1].1, "Looking at the placement test.", "chunks join");
    assert_eq!(rows[3].1, "The race is in the sleep.");
    assert_eq!(
        rows[3].2.as_deref(),
        Some("completed"),
        "a stream is closed when the turn ends"
    );

    let tool: serde_json::Value = serde_json::from_str(rows[2].3.as_deref().unwrap()).unwrap();
    assert_eq!(
        rows[2].2.as_deref(),
        Some("completed"),
        "an update merges into its call"
    );
    assert_eq!(tool["tool_kind"], "edit", "ACP's own kind is kept");
    assert_eq!(tool["locations"][0]["path"], "tests/mailbox.rs");
    assert_eq!(tool["locations"][0]["line"], 12);
    assert_eq!(tool["raw_input"]["file_path"], "tests/mailbox.rs");

    let unknown: serde_json::Value = serde_json::from_str(rows[5].3.as_deref().unwrap()).unwrap();
    assert_eq!(
        unknown["sessionUpdate"], "something_new",
        "an unknown update kind is kept verbatim rather than dropped"
    );

    let (patch, added, removed): (String, i64, i64) = connection
        .query_row(
            "SELECT patch, added, removed FROM patches WHERE turn_id=?1",
            [&turn],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(
        patch.contains("-sleep(1)") && patch.contains("+Order::place()"),
        "the diff is stored, not the two file texts: {patch}"
    );
    assert_eq!((added, removed), (1, 1));

    let state: String = connection
        .query_row("SELECT state FROM seats WHERE id=?1", [&seat], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "done", "Finished closes the seat");
}

/// R5: the desktop app bundles an `orochi`, and the user may have an older one on `PATH`.
/// Neither may lock the other out of the data directory.
#[test]
fn an_older_binarys_writes_still_work_after_the_telemetry_migration() {
    let dir = tempfile::tempdir().unwrap();
    // What an older Orochi wrote: the v2 schema, and an INSERT with no column list.
    {
        let connection = rusqlite::Connection::open(dir.path().join("telemetry.sqlite3")).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE runs (
                    id TEXT PRIMARY KEY, task_id TEXT NOT NULL, repository_id TEXT NOT NULL,
                    task_type TEXT NOT NULL, agent TEXT NOT NULL, model TEXT NOT NULL,
                    started_at INTEGER NOT NULL, record TEXT NOT NULL);
                 CREATE TABLE runtime (agent TEXT NOT NULL, model TEXT NOT NULL, record TEXT NOT NULL, PRIMARY KEY(agent, model));
                 CREATE TABLE sessions (session_id TEXT NOT NULL, agent TEXT NOT NULL, repository_id TEXT NOT NULL,
                    updated_at INTEGER NOT NULL, record TEXT NOT NULL, PRIMARY KEY(agent, session_id));
                 CREATE TABLE quota_snapshots (agent TEXT PRIMARY KEY, record TEXT NOT NULL);
                 CREATE TABLE classifications (task_hash TEXT PRIMARY KEY, record TEXT NOT NULL, created_at INTEGER NOT NULL);
                 PRAGMA user_version=2;",
            )
            .unwrap();
    }
    let record = |id: &str| {
        serde_json::json!({
            "id": id, "task_id": "t", "repository_id": "r", "task_type": "implementation",
            "language": "rust", "framework": null, "scope": 1, "context_size": 1,
            "candidate": {"id": "c", "agent": "claude", "provider": "anthropic", "model": "opus",
                "reasoning_level": "high", "mode": null, "session_strategy": "fresh",
                "context_strategy": "none", "success_probability": 0.8, "expected_tokens": 1.0,
                "expected_cost": 1.0, "confidence": 0.5, "reasons": []},
            "usage": {"total_tokens": 10}, "duration_ms": 1, "attempt": 1, "outcome": "success",
            "checks": [], "error_kind": null, "started_at": 100, "purpose": "execution",
            "complexity": "normal"
        })
        .to_string()
    };

    let store = Store::open(dir.path()).unwrap();
    let version: i64 = rusqlite::Connection::open(dir.path().join("telemetry.sqlite3"))
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2, "an additive change must not move user_version");

    // The older binary, still on PATH, writing the way it always has.
    {
        let old = rusqlite::Connection::open(dir.path().join("telemetry.sqlite3")).unwrap();
        old.execute(
            "INSERT INTO runs VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                "old",
                "t",
                "r",
                "implementation",
                "claude",
                "opus",
                100,
                record("old")
            ],
        )
        .expect("a column-less INSERT still matches the table after the generated columns");
    }

    let task = TaskDescriptor {
        task_type: "implementation".into(),
        language: "rust".into(),
        framework: None,
        repo_size: 1,
        candidate_files: vec![],
        estimated_scope: 1,
        estimated_context: 1,
        complexity: Complexity::Normal,
        requires_architecture_change: false,
        requires_browser: false,
        requires_web: false,
        requires_image: false,
        collaborative: false,
        seats: None,
        tests_available: false,
        ambiguity: 0.0,
        long_horizon: false,
        preferred: None,
    };
    let runs = store
        .learning_runs(&candidate("claude", "opus"), &task)
        .unwrap();
    assert_eq!(
        runs.len(),
        1,
        "the generated columns read the JSON an older binary wrote"
    );
    assert_eq!(runs[0].id, "old");
}
