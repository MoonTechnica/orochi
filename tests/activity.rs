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
        provider: Provider::ANTHROPIC,
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

#[test]
fn unregistering_a_host_interrupts_its_unfinished_turn() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (_, turn, _) = thread_with_a_turn(&activity);
    let host = activity.register_host(None, "headless").unwrap();
    assert!(activity.claim_turn(&turn, &host).unwrap());
    activity.unregister_host(&host).unwrap();
    let (state, ended): (String, Option<i64>) = activity
        .connection()
        .query_row(
            "SELECT state, ended_at FROM turns WHERE id=?1",
            [&turn],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "interrupted");
    assert!(ended.is_some());
}

#[test]
fn reaping_repairs_turns_left_running_after_their_host_was_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, orphan, _) = thread_with_a_turn(&activity);
    let host = activity.register_host(None, "headless").unwrap();
    assert!(activity.claim_turn(&orphan, &host).unwrap());
    let live_host = activity.register_host(None, "headless").unwrap();
    // Reproduce the state left by older versions, bypassing the repaired unregister.
    activity
        .connection()
        .execute("DELETE FROM hosts WHERE id=?1", [&host])
        .unwrap();
    let live = activity
        .queue_turn(&thread, "Next", &[], "auto", "terminal")
        .unwrap();
    assert!(activity.claim_turn(&live, &live_host).unwrap());
    activity.reap_hosts().unwrap();
    let state = |id: &str| {
        activity
            .connection()
            .query_row("SELECT state FROM turns WHERE id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
    };
    assert_eq!(state(&orphan), "interrupted");
    assert_eq!(state(&live), "running", "a live host keeps its turn");
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
            provider: Provider::ANTHROPIC,
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
        suited: None,
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

/// The window, the host and every run open this store, and each open recreates the views. A
/// view is dropped before it is created, so two opens at once must not meet between the two.
#[test]
fn several_openers_at_once_leave_the_views_intact() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let gate = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                gate.wait();
                Activity::open(&data, 30).expect("a concurrent open is not a broken store");
            });
        }
    });
    let activity = Activity::open(&data, 30).unwrap();
    let views: i64 = activity
        .connection()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='view'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(views >= 9, "{views} views");
}

/// `orochi threads` prints a short id so a person can read it; every command that takes one
/// must therefore accept it. It did not — `host --thread 275ae39b` answered "no thread
/// 275ae39b" about a thread the listing had just shown (2026-09-21).
#[test]
fn a_thread_answers_to_the_short_id_its_listing_shows() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(&dir.path().join("data"), 30).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let project = activity.project_for(&repo, "salt").unwrap();
    let start = || {
        activity
            .create_thread(
                &project,
                &repo,
                Some("main"),
                "repo-hash",
                Origin::Console,
                &Overrides::default(),
                "ask",
            )
            .unwrap()
    };
    let (one, two) = (start(), start());

    assert_eq!(
        activity.resolve_thread(&one).unwrap().as_deref(),
        Some(&one[..])
    );
    assert_eq!(
        activity.resolve_thread(&one[..8]).unwrap().as_deref(),
        Some(&one[..]),
        "the eight characters the listing prints"
    );
    assert_eq!(
        activity.resolve_thread("nothing-like-it").unwrap(),
        None,
        "and a prefix that names nothing still names nothing"
    );

    // A prefix that fits two threads is not a thread: answering with either would be a guess.
    let shared: String = one
        .chars()
        .zip(two.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect();
    if !shared.is_empty() {
        assert_eq!(activity.resolve_thread(&shared).unwrap(), None, "{shared}");
    }
}

/// A store written before background seats existed gains their columns and table on open —
/// once, even with several processes opening it at the same moment — and keeps every row it
/// had. `USER_VERSION` does not move, so the older binary sharing the directory keeps working.
#[test]
fn a_store_from_before_background_seats_gains_their_columns_once_and_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let seat = {
        let activity = Activity::open(dir.path(), 30).unwrap();
        let (_, turn, _) = thread_with_a_turn(&activity);
        let seat = activity
            .create_seat(&turn, 0, "implementer", true, false, None, None)
            .unwrap();
        // Back to the shape an older Orochi wrote: its views, then its columns and tables.
        let connection = activity.connection();
        let views: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='view'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for view in views {
            connection
                .execute_batch(&format!("DROP VIEW {view};"))
                .unwrap();
        }
        connection
            .execute_batch(
                "DROP TABLE agent_requests; DROP TRIGGER search_user; DROP TRIGGER search_reply;
                 DROP TRIGGER search_gone; DROP TABLE search;
                 ALTER TABLE seats DROP COLUMN title; ALTER TABLE seats DROP COLUMN background;
                 ALTER TABLE seats DROP COLUMN origin; ALTER TABLE seats DROP COLUMN result;
                 ALTER TABLE turns DROP COLUMN origin;",
            )
            .unwrap();
        seat
    };
    let opened: Vec<bool> = (0..4)
        .map(|_| {
            let path = dir.path().to_owned();
            std::thread::spawn(move || Activity::open(&path, 30).is_ok())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(opened, vec![true; 4]);
    let activity = Activity::open(dir.path(), 30).unwrap();
    let version: i64 = activity
        .connection()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2, "an additive change does not move the version");
    activity
        .seat_details(&seat, Some("Map the request flow"), true, "lead")
        .unwrap();
    let (title, background): (String, i64) = activity
        .connection()
        .query_row(
            "SELECT title, background FROM v_roster WHERE seat_id=?1",
            [&seat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((title.as_str(), background), ("Map the request flow", 1));
    let requests: i64 = activity
        .connection()
        .query_row("SELECT count(*) FROM agent_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(requests, 0);
    // What was said before the search existed is found once it does.
    let found = activity.search("flaky", 10).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
}

/// A thread is found by its name and by what was said in it — the user's words, and an agent's
/// reply once it has finished rather than while it streams — and a deleted thread is gone
/// from the results with the rest of it.
#[test]
fn threads_are_found_by_name_and_by_what_was_said_until_they_are_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);
    let lead = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&lead, &candidate("claude", "opus"), false, None)
        .unwrap();
    let reply = activity
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
    activity.append(reply, "The placement race is in ").unwrap();
    assert!(
        activity.search("placement", 10).unwrap().is_empty(),
        "a reply is not searchable while it streams"
    );
    activity.append(reply, "Order::place.").unwrap();
    activity.item_status(reply, "completed").unwrap();

    let by_title = activity.search("flaky test", 10).unwrap();
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].thread_id, thread);
    assert_eq!(by_title[0].snippet, None, "found by its name");
    let by_content = activity.search("placem", 10).unwrap();
    assert_eq!(by_content.len(), 1, "a word is matched as a prefix");
    assert!(
        by_content[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("[placement]")
    );
    assert!(
        activity.search("\"unbalanced", 10).is_ok(),
        "any text is a query"
    );
    assert!(activity.search("nothing-like-this", 10).unwrap().is_empty());

    activity.delete_thread(&thread).unwrap();
    assert!(activity.search("placement", 10).unwrap().is_empty());
    let left: i64 = activity
        .connection()
        .query_row("SELECT count(*) FROM search", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0, "nothing of it stays in the index");
}

/// A thread whose lead has answered while its helpers still work is neither idle nor working:
/// it reads as working in the background, lists each helper with its title, and says what the
/// lead is doing while a turn runs.
#[test]
fn a_thread_whose_helpers_outlive_its_turn_reads_as_working_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, turn, _) = thread_with_a_turn(&activity);
    let host = activity.register_host(Some(&thread), "terminal").unwrap();
    assert!(activity.claim_turn(&turn, &host).unwrap());
    let lead = activity
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let attempt = activity
        .create_attempt(&lead, &candidate("claude", "opus"), false, None)
        .unwrap();
    activity
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::ToolCall,
            Some("in_progress"),
            Some("call-1"),
            "$ curl -sL https://example.com/llms.txt",
            None,
        )
        .unwrap();
    let helper = activity
        .create_seat(&turn, 1, "spec", false, true, None, None)
        .unwrap();
    activity
        .seat_details(
            &helper,
            Some("Research Agents API model support"),
            true,
            "lead",
        )
        .unwrap();
    activity
        .create_attempt(&helper, &candidate("codex", "gpt"), false, None)
        .unwrap();
    activity.seat_state(&helper, SeatState::Working).unwrap();

    let row = |activity: &Activity| {
        activity
            .sidebar(10, false)
            .unwrap()
            .into_iter()
            .flat_map(|p| p.threads)
            .find(|t| t.id == thread)
            .unwrap()
    };
    let working = row(&activity);
    assert_eq!(working.status, "working");
    assert_eq!(
        working.doing.as_deref(),
        Some("$ curl -sL https://example.com/llms.txt")
    );
    assert!(working.running_since.is_some());

    activity.seat_state(&lead, SeatState::Done).unwrap();
    activity
        .turn_state(&turn, orochi::activity::TurnState::Completed)
        .unwrap();
    let waiting = row(&activity);
    assert_eq!(waiting.status, "background");
    assert_eq!(waiting.background, 1);
    assert_eq!(
        waiting.doing, None,
        "nothing runs, so the lead is doing nothing"
    );
    // The conversation is its members: the lead that answered, and the helper still at it.
    let members: Vec<(String, String, i64, bool)> = waiting
        .agents
        .iter()
        .map(|m| (m.role.clone(), m.agent.clone(), m.live, m.lead))
        .collect();
    assert_eq!(
        members,
        vec![
            ("implementer".to_owned(), "claude".to_owned(), 0, true),
            ("spec".to_owned(), "codex".to_owned(), 1, false),
        ]
    );

    let live = activity.live_seats().unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(
        live[0].title.as_deref(),
        Some("Research Agents API model support")
    );
    assert!(live[0].background && live[0].read_only);
    assert_eq!(live[0].agent.as_deref(), Some("codex"));

    activity
        .seat_result(&helper, "Found three endpoints.")
        .unwrap();
    activity.seat_state(&helper, SeatState::Done).unwrap();
    let seats = activity.thread(&thread).unwrap().unwrap().seats;
    let reported = seats.iter().find(|s| s.seat_id == helper).unwrap();
    assert!(reported.reported && reported.background);
    assert_ne!(row(&activity).status, "background");
    assert!(activity.live_seats().unwrap().is_empty());
}

/// An agent names its session after the prompt it was sent, and Orochi opens that prompt with
/// its own instructions: that title names the preamble, so it never replaces the one taken from
/// the person's message — and a thread already named that way is named again on open.
#[test]
fn an_agent_title_that_is_orochis_own_preamble_never_names_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let thread = {
        let activity = Activity::open(dir.path(), 30).unwrap();
        let (thread, _, _) = thread_with_a_turn(&activity);
        activity
            .title(
                &thread,
                "Coordination: you are peer \"orochi-6de1\". Other agents…",
                false,
            )
            .unwrap();
        let title: String = activity
            .connection()
            .query_row("SELECT title FROM threads WHERE id=?1", [&thread], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Fix the flaky test");
        activity
            .title(&thread, "Flaky mailbox test", false)
            .unwrap();
        // As an older Orochi left it.
        activity
            .connection()
            .execute(
                "UPDATE threads SET title='Coordination: you are peer \"x\"' WHERE id=?1",
                [&thread],
            )
            .unwrap();
        thread
    };
    let activity = Activity::open(dir.path(), 30).unwrap();
    let title: String = activity
        .connection()
        .query_row("SELECT title FROM threads WHERE id=?1", [&thread], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        title, "Fix the flaky test",
        "named again from the message that opened it"
    );
}

/// The sidebar's context menus write rows, and the rows mean what they say: a hidden project
/// keeps its conversations and comes back when something works in it; a deleted project
/// takes every conversation with it, and `all_threads` is the one list that sees everything.
#[test]
fn a_hidden_project_keeps_its_threads_and_a_deleted_one_takes_them() {
    let dir = tempfile::tempdir().unwrap();
    let activity = Activity::open(dir.path(), 30).unwrap();
    let (thread, ..) = thread_with_a_turn(&activity);
    activity
        .project("other", std::path::Path::new("/tmp/other"))
        .unwrap();

    assert!(activity.hide_project("proj", true).unwrap());
    let names: Vec<String> = activity
        .sidebar(20, true)
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(
        names,
        vec!["other"],
        "hidden means out of the list, archived included"
    );
    assert_eq!(
        activity.all_threads().unwrap(),
        vec![thread.clone()],
        "and still there for whoever deletes everything"
    );
    activity
        .project("proj", std::path::Path::new("/tmp/repo"))
        .unwrap();
    assert_eq!(
        activity.sidebar(20, false).unwrap().len(),
        2,
        "working in it brings it back"
    );

    assert!(activity.delete_project("proj").unwrap());
    assert!(
        activity.thread(&thread).unwrap().is_none(),
        "its threads go with it"
    );
    assert!(activity.all_threads().unwrap().is_empty());
    assert!(!activity.delete_project("proj").unwrap());
    assert!(!activity.hide_project("proj", false).unwrap());
}
