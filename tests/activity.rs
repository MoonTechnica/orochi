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

/// A real agent names the file it changed with an absolute path (confirmed against Claude on
/// 2026-09-20). A review pane wants the path the repository uses, so it is made relative to
/// the thread's own directory — and left alone when it is somewhere else entirely.
#[test]
fn a_patch_is_filed_under_the_path_the_repository_uses() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    activity.project("proj", &repo).unwrap();
    let thread = activity
        .create_thread(
            "proj",
            &repo,
            None,
            "repo-hash",
            Origin::Run,
            &Overrides::default(),
            "allow",
        )
        .unwrap();
    let turn = activity
        .queue_turn(&thread, "write it", &[], "solo", "stdin")
        .unwrap();
    let seat = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&seat, &candidate("claude", "haiku"), false, None)
        .unwrap();
    let item = activity
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::ToolCall,
            Some("completed"),
            Some("call-1"),
            "",
            None,
        )
        .unwrap();

    let inside = repo.join("src/hello.txt");
    activity
        .patch(item, &turn, &inside.display().to_string(), None, "orochi\n")
        .unwrap();
    let outside = dir.path().join("elsewhere.txt");
    activity
        .patch(item, &turn, &outside.display().to_string(), None, "x\n")
        .unwrap();

    let paths: Vec<String> = activity
        .connection()
        .prepare("SELECT path FROM patches ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        paths[0], "src/hello.txt",
        "a file in the thread's own directory is named the way the repository names it"
    );
    assert_eq!(
        paths[1],
        outside.display().to_string(),
        "and one outside it keeps the only name it has"
    );
}

/// §2 and §9 asked for these to be measured rather than estimated. They are bounds, not
/// benchmarks: they fail if the store becomes slow enough to be felt, and the numbers they
/// print are what `docs/desktop-app-design.md` records.
#[test]
fn recording_a_busy_turn_stays_faster_than_a_reader_can_notice() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);
    // Six seats, as a discussion has, each streaming at once.
    let mut items = Vec::new();
    for ordinal in 0..6 {
        let seat = activity
            .create_seat(&turn, ordinal, "seat", ordinal == 0, false, None, None)
            .unwrap();
        let attempt = activity
            .create_attempt(&seat, &candidate("claude", "opus"), false, None)
            .unwrap();
        items.push(
            activity
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
                .unwrap(),
        );
    }

    // What one second of six agents streaming costs: each flushes at most every 80 ms.
    let flushes = 6 * (1000 / 80);
    let chunk = "the quick brown fox jumps over the lazy dog. ".repeat(4);
    let start = std::time::Instant::now();
    for n in 0..flushes {
        activity.append(items[n % items.len()], &chunk).unwrap();
    }
    let per_flush = start.elapsed() / flushes as u32;
    assert!(
        per_flush < std::time::Duration::from_millis(8),
        "a flush took {per_flush:?}; at this rate six streaming seats would be felt"
    );

    // What a reader pays for a quiet tick: the header read plus an empty feed query.
    let reader = Activity::open_read_only(dir.path()).unwrap();
    let cursor: i64 = reader
        .query_row("SELECT COALESCE(max(id),0) FROM changes", [], |r| r.get(0))
        .unwrap();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        let _: i64 = reader
            .query_row("PRAGMA data_version", [], |r| r.get(0))
            .unwrap();
        let mut statement = reader
            .prepare_cached("SELECT id, thread_id FROM changes WHERE id > ?1 ORDER BY id")
            .unwrap();
        let moved: Vec<i64> = statement
            .query_map([cursor + 1_000_000], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(moved.is_empty());
    }
    let per_poll = start.elapsed() / 100;
    assert!(
        per_poll < std::time::Duration::from_micros(500),
        "a quiet poll took {per_poll:?}; at 10 Hz that is not free"
    );

    println!("measured: flush {per_flush:?}, quiet poll {per_poll:?}");
}

/// §9 asked how fast the store grows. A turn's worth of streamed text is the bulk of it.
#[test]
fn a_recorded_turn_costs_about_what_its_text_costs() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);
    let seat = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&seat, &candidate("claude", "opus"), false, None)
        .unwrap();

    // A heavy turn: 40 tool calls with their output, and 20 KiB of reply.
    for n in 0..40 {
        let item = activity
            .item(
                &thread,
                &turn,
                Some(&attempt),
                ItemKind::ToolCall,
                Some("completed"),
                Some(&format!("call-{n}")),
                &"output line\n".repeat(40),
                Some(r#"{"title":"Read","tool_kind":"read"}"#),
            )
            .unwrap();
        activity
            .patch(
                item,
                &turn,
                &format!("src/file{n}.rs"),
                Some(&"a\n".repeat(200)),
                &format!("{}changed\n", "a\n".repeat(200)),
            )
            .unwrap();
    }
    let reply = activity
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::AgentMessage,
            Some("completed"),
            None,
            &"x".repeat(20 * 1024),
            None,
        )
        .unwrap();
    let _ = reply;
    activity
        .connection()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();

    let bytes = std::fs::metadata(dir.path().join("activity.sqlite3"))
        .unwrap()
        .len();
    println!("measured: a heavy turn costs {} KiB", bytes / 1024);
    assert!(
        bytes < 1024 * 1024,
        "a heavy turn took {bytes} bytes; a day of work would not fit in the budget §9 assumes"
    );
}

/// The first message names the conversation, and nothing else does: no agent is asked for a
/// title (R6), and later messages do not rename a thread after what it drifted into.
#[test]
fn the_first_message_names_the_conversation_and_later_ones_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    activity
        .project("proj", std::path::Path::new("/tmp/repo"))
        .unwrap();
    let new_thread = || {
        activity
            .create_thread(
                "proj",
                std::path::Path::new("/tmp/repo"),
                None,
                "repo",
                Origin::Desktop,
                &Overrides::default(),
                "ask",
            )
            .unwrap()
    };
    let title = |thread: &str| -> String {
        activity
            .connection()
            .query_row("SELECT title FROM threads WHERE id=?1", [thread], |r| {
                r.get(0)
            })
            .unwrap()
    };

    let thread = new_thread();
    assert_eq!(
        title(&thread),
        "",
        "a thread nobody has written in has no name"
    );
    activity
        .queue_turn(
            &thread,
            "Fix the flaky mailbox test",
            &[],
            "auto",
            "desktop",
        )
        .unwrap();
    assert_eq!(title(&thread), "Fix the flaky mailbox test");
    activity
        .queue_turn(&thread, "and add a regression test", &[], "auto", "desktop")
        .unwrap();
    assert_eq!(
        title(&thread),
        "Fix the flaky mailbox test",
        "a conversation keeps the name it was opened with"
    );

    // What a pasted message looks like: a heading, wrapped lines, trailing blank lines.
    let pasted = new_thread();
    activity
        .queue_turn(
            &pasted,
            "## Fix   the\n  flaky mailbox test, which fails about one run in five on CI and\nnobody has looked at it\n\n",
            &[],
            "auto",
            "desktop",
        )
        .unwrap();
    let derived = title(&pasted);
    assert!(
        !derived.starts_with('#') && !derived.contains('\n') && !derived.contains("   "),
        "the name is a line of prose, not the markup it was pasted from: {derived:?}"
    );
    assert!(
        derived.starts_with("Fix the flaky mailbox test") && derived.ends_with('…'),
        "long messages are cut where a reader can still tell them apart: {derived:?}"
    );
    assert!(derived.chars().count() <= 61, "and cut short: {derived:?}");

    // A thread the user named themselves is never renamed.
    let named = new_thread();
    activity.title(&named, "Release checklist", true).unwrap();
    activity
        .queue_turn(&named, "start with the changelog", &[], "auto", "desktop")
        .unwrap();
    assert_eq!(title(&named), "Release checklist");
}
