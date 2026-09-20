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
