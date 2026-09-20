//! What the window is allowed to ask for, and what it is allowed to say.
//!
//! These run against a fixture store rather than a window, because the app's whole claim is
//! that a screen is a query: if a view cannot answer one of these, the screen cannot be drawn,
//! and no amount of front-end work would fix it.
use orochi::activity::{Activity, ItemKind, Origin, share};
use orochi::types::*;
use orochi_desktop::view::{self, Client};

fn candidate() -> ExecutionCandidate {
    ExecutionCandidate {
        id: "claude:opus".into(),
        agent: "claude".into(),
        provider: Provider::Anthropic,
        model: "opus".into(),
        reasoning_level: Some("high".into()),
        mode: None,
        session_strategy: "fresh".into(),
        context_strategy: "none".into(),
        success_probability: 0.8,
        expected_tokens: 1000.0,
        expected_cost: 1.0,
        confidence: 0.5,
        reasons: vec!["normal implementation task".into()],
        prediction: None,
    }
}

/// A repository with one finished conversation and one question waiting.
fn fixture(dir: &std::path::Path) -> (String, String) {
    let store = share(Activity::open(dir, 30).unwrap());
    let db = store.lock().unwrap();
    db.project("proj", std::path::Path::new("/tmp/orochi")).unwrap();
    let thread = db
        .create_thread(
            "proj",
            std::path::Path::new("/tmp/orochi"),
            Some("main"),
            "repo-hash",
            Origin::Console,
            &Overrides::default(),
            "ask",
        )
        .unwrap();
    let turn = db
        .queue_turn(&thread, "Fix the flaky mailbox test", &[], "auto", "terminal")
        .unwrap();
    let host = db.register_host(Some(&thread), "terminal").unwrap();
    db.claim_turn(&turn, &host).unwrap();
    db.turn_shape(&turn, Some("seats"), None).unwrap();

    let lead = db
        .create_seat(&turn, 0, "implementer", true, false, None, None)
        .unwrap();
    let side = db
        .create_seat(&turn, 1, "reviewer", false, true, None, None)
        .unwrap();
    let attempt = db.create_attempt(&lead, &candidate(), false, None).unwrap();
    db.item(
        &thread,
        &turn,
        Some(&attempt),
        ItemKind::Route,
        None,
        None,
        "",
        Some(r#"{"agent":"claude","model":"opus","reasoning":"high"}"#),
    )
    .unwrap();
    let reply = db
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::AgentMessage,
            Some("completed"),
            None,
            "The race is in the sleep.",
            None,
        )
        .unwrap();
    let tool = db
        .item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::ToolCall,
            Some("completed"),
            Some("call-1"),
            "",
            Some(r#"{"title":"Edit","tool_kind":"edit"}"#),
        )
        .unwrap();
    db.patch(
        tool,
        &turn,
        "tests/mailbox.rs",
        Some("a\nsleep(1)\nc\n"),
        "a\nOrder::place()\nc\n",
    )
    .unwrap();
    let prompt = db
        .open_prompt(
            &thread,
            &turn,
            Some(&attempt),
            Some(tool),
            "permission",
            r#"{"toolCall":{"title":"Run tests"},"options":[{"optionId":"ok","kind":"allow_once","name":"Allow once"}]}"#,
        )
        .unwrap();
    let _ = (side, reply);
    drop(db);
    (thread, prompt)
}

#[test]
fn the_sidebar_groups_threads_under_their_project_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    let projects = client.sidebar(20, false).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "orochi", "named after the repository");
    assert_eq!(projects[0].threads.len(), 1);
    let row = &projects[0].threads[0];
    assert_eq!(row.id, thread);
    assert_eq!(row.title, "Fix the flaky mailbox test");
    assert_eq!(
        row.status, "needs_you",
        "an open question outranks everything else a thread could be"
    );
    assert_eq!(view::mark(row.status), "!");
}

#[test]
fn a_thread_renders_from_the_views_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    let thread = client.thread(&thread).unwrap().expect("the thread is there");
    let kinds: Vec<&str> = thread.items.iter().map(|i| i.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["user_message", "route", "agent_message", "tool_call"],
        "the timeline is what happened, in order"
    );
    assert_eq!(
        thread.items[1].data.as_ref().unwrap()["model"], "opus",
        "the route chip has what it needs without a second query"
    );
    assert_eq!(
        thread.seats.len(),
        2,
        "both seats of the turn are at the table"
    );
    assert!(thread.seats[1].read_only, "and one of them only reads");
    assert_eq!(thread.files.len(), 1);
    assert_eq!(thread.files[0].path, "tests/mailbox.rs");
    assert_eq!((thread.files[0].added, thread.files[0].removed), (1, 1));

    let patch = client.patch(thread.files[0].latest_patch).unwrap().unwrap();
    assert!(
        patch.contains("-sleep(1)") && patch.contains("+Order::place()"),
        "the review pane has a real diff: {patch}"
    );
}

#[test]
fn answering_a_question_is_a_row_and_only_the_first_answer_counts() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, prompt) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    let open = client.open_prompts().unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, prompt);
    assert_eq!(open[0].thread_title, "Fix the flaky mailbox test");
    assert_eq!(
        open[0].options,
        vec![("ok".to_owned(), "Allow once".to_owned())],
        "the card offers the agent's own options, not invented ones"
    );

    assert!(client.answer(&prompt, Some("ok")).unwrap());
    assert!(
        !client.answer(&prompt, Some("ok")).unwrap(),
        "a question already answered cannot be answered again"
    );
    assert!(client.open_prompts().unwrap().is_empty());
    let _ = thread;
}

#[test]
fn sending_a_message_queues_a_turn_for_whoever_is_hosting_the_thread() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    client.send(&thread, "and add a test for it").unwrap();
    let queued = Activity::attach(dir.path(), 30)
        .unwrap()
        .next_queued(&thread)
        .unwrap();
    assert_eq!(
        queued.map(|(_, text)| text).as_deref(),
        Some("and add a test for it"),
        "the message is waiting where a host will find it"
    );

    client.interrupt(&thread).unwrap();
    let control = Activity::attach(dir.path(), 30)
        .unwrap()
        .take_control(&thread)
        .unwrap();
    assert_eq!(control.map(|(kind, _)| kind).as_deref(), Some("interrupt"));
}

/// The window polls; it must not read the whole conversation every 100 ms to notice nothing
/// happened.
#[test]
fn the_window_learns_what_changed_without_rereading_everything() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let mut client = Client::open(dir.path()).unwrap();
    assert!(
        !client.changed().unwrap().is_empty(),
        "the first look reports what is already there"
    );
    assert!(
        client.changed().unwrap().is_empty(),
        "and nothing at all when nothing has happened"
    );

    client.send(&thread, "another message").unwrap();
    let changed = client.changed().unwrap();
    assert!(
        changed.contains(&thread),
        "a change names the thread it belongs to: {changed:?}"
    );
}

#[test]
fn a_store_from_a_newer_orochi_is_refused_rather_than_misread() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    rusqlite::Connection::open(dir.path().join("activity.sqlite3"))
        .unwrap()
        .execute_batch("PRAGMA user_version=99;")
        .unwrap();
    let error = match Client::open(dir.path()) {
        Ok(_) => panic!("a store this window does not understand was opened anyway"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("newer"),
        "it says the store is newer than this window: {error}"
    );
}

/// A thread has to start somewhere, and the window is where you say where. Recent folders come
/// from the projects the store already knows, so a repository you have worked in from a
/// terminal is one click away.
#[test]
fn a_thread_can_be_started_in_a_folder_the_window_chooses() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let repo = dir.path().join("another-repo");
    std::fs::create_dir(&repo).unwrap();
    let client = Client::open(dir.path()).unwrap();

    let recent = client.folders().unwrap();
    assert_eq!(
        recent.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["orochi"],
        "the folders offered are the ones already worked in"
    );

    let thread = client.new_thread(&repo).unwrap();
    let projects = client.sidebar(20, false).unwrap();
    let section = projects
        .iter()
        .find(|p| p.name == "another-repo")
        .expect("the chosen folder is a project of its own");
    assert_eq!(section.threads[0].id, thread);
    assert_eq!(section.threads[0].origin, "desktop");
    assert!(
        section.threads[0].cwd.ends_with("another-repo"),
        "and the thread works in the folder that was chosen: {}",
        section.threads[0].cwd
    );

    assert_eq!(
        client.folders().unwrap().len(),
        2,
        "which then becomes one of the recent folders"
    );
}

#[test]
fn a_folder_that_is_not_a_directory_is_refused_rather_than_recorded() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();
    assert!(
        client.new_thread(&dir.path().join("nowhere")).is_err(),
        "a thread whose directory does not exist could never run"
    );
}

/// A message is only a row until something runs it. The window starts the thread's host when
/// there is none, and leaves the one that is already there alone — including a terminal's.
#[test]
fn sending_starts_a_host_only_when_the_thread_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    // `fixture` registered a live terminal host for this thread, so the window must not start
    // a second one: the thread already has an owner.
    assert!(
        !client.needs_host(&thread).unwrap(),
        "a thread a terminal is holding is not the window's to run"
    );

    let idle = client.new_thread(dir.path()).unwrap();
    assert!(
        client.needs_host(&idle).unwrap(),
        "a thread nobody owns needs one before its messages can run"
    );
}

/// P4: a person can leave a note in the room, and the room is what the Team pane shows.
#[test]
fn the_room_carries_what_agents_and_the_person_said() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    assert!(client.room(&thread).unwrap().is_empty(), "nobody has spoken yet");
    client.say(&thread, None, "prefer the simpler shape").unwrap();

    let room = client.room(&thread).unwrap();
    assert_eq!(room.len(), 1);
    assert_eq!(room[0].who, "user");
    assert_eq!(room[0].via, "user");
    assert_eq!(room[0].text, "prefer the simpler shape");
    assert_eq!(room[0].whom.as_deref(), Some("all"));
}

/// P3: the agents screen, and the numbers behind Insights. Both are telemetry's own views, so
/// what a window shows is what `orochi status` and `orochi calibrate` show.
#[test]
fn the_agents_and_insights_screens_read_telemetry_through_its_views() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let store = orochi::storage::Store::open(dir.path()).unwrap();
    store
        .save_runtime(&orochi::types::RuntimeState {
            status: orochi::types::RuntimeStatus::Cooldown,
            cooldown_until: Some(orochi::types::now() + 300),
            ..orochi::types::RuntimeState::new("codex", "*")
        })
        .unwrap();
    let client = Client::open(dir.path()).unwrap();

    let agents = client.agents().unwrap();
    let codex = agents.iter().find(|a| a.agent == "codex").unwrap();
    assert_eq!(codex.status, "cooldown");
    assert!(codex.cooling > 0, "with the time left on it");

    assert!(
        client.insights().unwrap().is_empty(),
        "no runs yet means no numbers to show, rather than zeroes to misread"
    );
}

/// §6.10: the window can read and change the settings, through the same validation the CLI
/// uses — an invalid one is refused rather than written and discovered at the next run.
#[test]
fn settings_are_readable_and_only_valid_ones_are_written() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        r#"
[activity]
retention_days = 30
[[agents]]
id = "claude"
provider = "anthropic"
command = "claude-agent-acp"
[agents.env]
ANTHROPIC_API_KEY = "sk-secret-value"
"#,
    )
    .unwrap();
    let client = Client::open(dir.path()).unwrap().with_config(&config);

    let shown = client.settings().unwrap();
    assert_eq!(shown["activity"]["retention_days"], 30);
    assert_eq!(
        shown["agents"][0]["env"]["ANTHROPIC_API_KEY"], "[redacted]",
        "a window never shows a secret it had no reason to read"
    );

    let mut changed = shown.clone();
    changed["activity"]["retention_days"] = serde_json::json!(7);
    client.save_settings(&changed).unwrap();
    assert_eq!(client.settings().unwrap()["activity"]["retention_days"], 7);
    assert!(
        std::fs::read_to_string(&config).unwrap().contains("sk-secret-value"),
        "and writing settings back does not overwrite the secret with its redaction",
    );

    let mut invalid = shown.clone();
    invalid["activity"]["retention_days"] = serde_json::json!(-1);
    let error = client.save_settings(&invalid).unwrap_err().to_string();
    assert!(
        error.contains("retention_days"),
        "an invalid setting is refused, and says which one: {error}"
    );
    assert_eq!(
        client.settings().unwrap()["activity"]["retention_days"],
        7,
        "and the file it would have broken is untouched"
    );
}

/// Memory is the user's own text, so the window edits it as text and nothing else writes it.
#[test]
fn memory_is_readable_and_editable_as_the_text_it_is() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    assert_eq!(client.memory().unwrap().user, "", "nothing is remembered yet");
    client.save_memory("Prefers small commits.\n").unwrap();
    assert_eq!(client.memory().unwrap().user, "Prefers small commits.\n");
    assert!(
        dir.path().join("memory/USER.md").exists(),
        "written where the core reads it, not somewhere of the window's own"
    );
}

/// R4: delete means gone, and the window can do it for everything at once.
#[test]
fn every_conversation_can_be_deleted_from_the_window() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();
    assert_eq!(client.sidebar(20, true).unwrap()[0].threads.len(), 1);

    let removed = client.forget_all().unwrap();
    assert_eq!(removed, 1);
    assert!(
        client.sidebar(20, true).unwrap()[0].threads.is_empty(),
        "the conversations are gone; the project keeps its place"
    );
}

/// §6.5: the working tree is the authority for review, so the Changes pane reads `git diff`
/// as well as the patches a turn recorded. The stored ones are what remains once the tree has
/// moved on; the live ones are what is actually there now.
#[test]
fn the_changes_pane_reads_the_working_tree_as_well_as_what_was_recorded() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let repo = dir.path().join("worktree");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("kept.txt"), "one\ntwo\n").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.email=t@e", "-c", "user.name=t", "commit", "-q", "-m", "base"]);
    std::fs::write(repo.join("kept.txt"), "one\nchanged\n").unwrap();

    let client = Client::open(dir.path()).unwrap();
    let thread = client.new_thread(&repo).unwrap();

    let unstaged = client.tree_files(&thread, "unstaged").unwrap();
    assert_eq!(unstaged.len(), 1);
    assert_eq!(unstaged[0].path, "kept.txt");
    assert_eq!((unstaged[0].added, unstaged[0].removed), (1, 1));

    let diff = client.tree_patch(&thread, "unstaged", "kept.txt").unwrap();
    assert!(
        diff.contains("-two") && diff.contains("+changed"),
        "the pane shows what is actually in the tree: {diff}"
    );

    git(&["add", "."]);
    assert!(
        client.tree_files(&thread, "unstaged").unwrap().is_empty(),
        "staging moves a file between the scopes, as git says it does"
    );
    assert_eq!(client.tree_files(&thread, "staged").unwrap().len(), 1);
    assert_eq!(
        client.tree_files(&thread, "branch").unwrap().len(),
        1,
        "and the branch scope is everything since the merge base"
    );
}

/// Comments on a diff become the next message, rather than a review mechanism of their own.
#[test]
fn review_comments_become_the_next_message() {
    let dir = tempfile::tempdir().unwrap();
    let (thread, _) = fixture(dir.path());
    let client = Client::open(dir.path()).unwrap();

    let turn = client
        .comment(
            &thread,
            &[
                ("src/lib.rs".into(), 12, "this allocates in a loop".into()),
                ("src/lib.rs".into(), 40, "and this can be `?`".into()),
            ],
        )
        .unwrap();
    let text: String = Activity::attach(dir.path(), 30)
        .unwrap()
        .connection()
        .query_row(
            "SELECT text FROM items WHERE turn_id=?1 AND kind='user_message'",
            [&turn],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        text,
        "src/lib.rs:12 — this allocates in a loop\nsrc/lib.rs:40 — and this can be `?`",
        "one message carrying every comment, in the order they were left"
    );
    assert!(
        client.comment(&thread, &[]).is_err(),
        "an empty review is not a message"
    );
}

/// Someone who installs the window before ever running the CLI has no store yet, and someone
/// whose store was left half-made by an older build has a file that is not one. Neither is a
/// reason to refuse to open a window.
#[test]
fn the_window_opens_on_a_data_directory_that_has_no_store_yet() {
    let dir = tempfile::tempdir().unwrap();
    let client = Client::open(dir.path()).expect("a first run has nothing to read, not an error");
    assert!(
        client.sidebar(20, true).unwrap().is_empty(),
        "and it shows an empty window rather than inventing something"
    );
    assert!(dir.path().join("activity.sqlite3").exists());

    // A file that is not a store — what an interrupted first run leaves behind.
    let empty = tempfile::tempdir().unwrap();
    std::fs::write(empty.path().join("activity.sqlite3"), b"").unwrap();
    let client = Client::open(empty.path()).expect("an empty file is set up, not refused");
    assert!(client.sidebar(20, true).unwrap().is_empty());
}

/// The window opens filling the screen. A conversation, its sidebar and the team beside it are
/// three columns; a small default window puts the third one off the edge.
#[test]
fn the_window_opens_filling_the_screen() {
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let window = &config["app"]["windows"][0];
    assert_eq!(window["maximized"], true);
    assert!(
        window["width"].as_u64().unwrap() >= 1100,
        "and unmaximizes to a size the three columns still fit in"
    );
}
