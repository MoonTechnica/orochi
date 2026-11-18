//! A store with something in every corner of the phone's screens, written through the same
//! `orochi::activity` calls a real run makes, so the real server can be driven (a simulator, a
//! screenshot) without any agent account. `seed` is the only entry; nothing here is reachable
//! from the window.
use anyhow::{Context, Result};
use orochi::activity::{Activity, ItemKind, Origin, SeatState, TurnState};
use orochi::media::{self, MediaRef, Source};
use orochi::types::*;
use std::path::{Path, PathBuf};

const PNG: &[u8] = include_bytes!("../assets/demo/demo.png");
const JPG: &[u8] = include_bytes!("../assets/demo/demo.jpg");
const MP4: &[u8] = include_bytes!("../assets/demo/demo.mp4");
const M4A: &[u8] = include_bytes!("../assets/demo/demo.m4a");

/// What `seed` made, for a script to read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Seeded {
    pub data: PathBuf,
    pub repo: PathBuf,
    pub project: String,
    pub threads: Vec<(String, String)>,
}

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
        reasons: vec!["demo".into()],
        prediction: None,
        spoken_for: false,
    }
}

fn repository(root: &Path) -> Result<()> {
    let files: &[(&str, &[u8])] = &[
        ("assets/chart.png", PNG),
        ("assets/photo.jpg", JPG),
        ("assets/clip.mp4", MP4),
        ("assets/voice.m4a", M4A),
        (
            "notes.txt",
            "Demo notes\n==========\nA plain text file in the thread's folder.\n".as_bytes(),
        ),
        (
            "changes.diff",
            b"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n }\n",
        ),
        ("README.md", b"# demo repo\n\nSeeded for the phone.\n"),
        ("src/lib.rs", b"fn main() {\n    println!(\"new\");\n}\n"),
    ];
    for (path, bytes) in files {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap())?;
        std::fs::write(at, bytes)?;
    }
    // Best effort: a repository lets the Git screens answer too.
    let git = |args: &[&str]| {
        let _ = std::process::Command::new("git")
            .args(["-c", "user.name=demo", "-c", "user.email=demo@example.com"])
            .args(args)
            .current_dir(root)
            .output();
    };
    if !root.join(".git").exists() {
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "demo"]);
    }
    Ok(())
}

struct Seeder<'a> {
    db: &'a Activity,
    project: String,
    root: PathBuf,
    data: PathBuf,
}

struct Turn {
    thread: String,
    turn: String,
    seat: String,
    attempt: String,
    host: String,
}

impl Seeder<'_> {
    fn thread(&self, title: &str, user: &str, agent: &str, model: &str) -> Result<Turn> {
        let db = self.db;
        let thread = db.create_thread(
            &self.project,
            &self.root,
            Some("main"),
            "demo-repo-hash",
            Origin::Console,
            &Overrides::default(),
            "ask",
        )?;
        db.title(&thread, title, true)?;
        let turn = db.queue_turn(&thread, user, &[], "auto", "terminal")?;
        let host = db.register_host(Some(&thread), "terminal")?;
        db.claim_turn(&turn, &host)?;
        db.turn_shape(&turn, Some("seats"), None)?;
        let seat = db.create_seat(&turn, 0, "implementer", true, false, None, None)?;
        let attempt = db.create_attempt(&seat, &candidate(agent, model), false, None)?;
        db.item(
            &thread,
            &turn,
            Some(&attempt),
            ItemKind::Route,
            None,
            None,
            "",
            Some(&format!(
                r#"{{"agent":"{agent}","model":"{model}","reasoning":"high"}}"#
            )),
        )?;
        Ok(Turn {
            thread,
            turn,
            seat,
            attempt,
            host,
        })
    }

    fn say(&self, t: &Turn, text: &str) -> Result<()> {
        self.db.item(
            &t.thread,
            &t.turn,
            Some(&t.attempt),
            ItemKind::AgentMessage,
            Some("completed"),
            None,
            text,
            None,
        )?;
        Ok(())
    }

    fn media(&self, t: &Turn, name: &str, bytes: &[u8]) -> Result<()> {
        let kept = media::keep(
            &self.data,
            &t.thread,
            &MediaRef {
                name: name.into(),
                source: Source::Inline(bytes.to_vec()),
            },
        )?
        .with_context(|| format!("{name} is not media"))?;
        self.db.keep_media(&t.thread, &kept)?;
        self.db.item(
            &t.thread,
            &t.turn,
            Some(&t.attempt),
            ItemKind::Media,
            Some("completed"),
            Some(&format!("media:{}", kept.sha)),
            &kept.name,
            Some(&serde_json::json!(kept).to_string()),
        )?;
        Ok(())
    }

    /// The turn ends and its host goes: nothing in this thread is live.
    fn finish(&self, t: &Turn) -> Result<()> {
        self.db.seat_state(&t.seat, SeatState::Done)?;
        self.db.turn_state(&t.turn, TurnState::Completed)?;
        self.db.unregister_host(&t.host)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn message(
        &self,
        thread: &str,
        sender: &str,
        recipient: &str,
        via: &str,
        body: &str,
        member: Option<&str>,
        parent: Option<i64>,
        mentions: &[&str],
        at: i64,
    ) -> Result<i64> {
        self.db.connection().execute(
            "INSERT INTO messages (project_id, thread_id, sender, sender_name, recipient,
                recipient_name, via, body, sent_at, parent, member, mentions, broadcast)
             VALUES (?1,?2,?3,?3,?4,?4,?5,?6,?7,?8,?9,?10,0)",
            rusqlite::params![
                self.project,
                thread,
                sender,
                recipient,
                via,
                body,
                at,
                parent,
                member,
                (!mentions.is_empty()).then(|| serde_json::json!(mentions).to_string()),
            ],
        )?;
        Ok(self.db.connection().last_insert_rowid())
    }
}

/// Writes the demo repository and a store into `data`, and returns what is where. Threads that
/// are *live* (a working seat, a turn in progress) are attached to **this process** as their
/// host, so they read as live exactly as long as this process runs: seed and serve from the
/// same process (`orochi-remote --demo-data`) to see them.
pub fn seed(data: &Path) -> Result<Seeded> {
    std::fs::create_dir_all(data)?;
    let data = std::fs::canonicalize(data)?;
    let root = data.join("demo-repo");
    repository(&root)?;
    let db = Activity::open(&data, 0)?;
    let project = "demo-project".to_owned();
    db.project(&project, &root)?;
    let s = Seeder {
        db: &db,
        project: project.clone(),
        root: root.clone(),
        data: data.clone(),
    };
    let mut threads = vec![];
    let mut named = |title: &str, id: &str| threads.push((title.to_owned(), id.to_owned()));
    let now = orochi::activity::millis() / 1000;

    // 1. A chat with pictures, a recording and a clip.
    let t = s.thread(
        "Show me what you made",
        "Make a chart, a photo, a short clip and a voice note",
        "claude",
        "opus",
    )?;
    s.say(&t, "Here is the chart and the photo.")?;
    s.media(&t, "chart.png", PNG)?;
    s.media(&t, "photo.jpg", JPG)?;
    s.say(&t, "And a short clip with a voice note.")?;
    s.media(&t, "clip.mp4", MP4)?;
    s.media(&t, "voice.m4a", M4A)?;
    s.finish(&t)?;
    named("media", &t.thread);

    // 2. Markdown whose links point into the thread's working folder.
    let t = s.thread(
        "Write up with files",
        "Summarise the change and link what you touched",
        "codex",
        "gpt-5",
    )?;
    s.say(
        &t,
        "# Summary\n\nThe **change** is small.\n\n![the chart](assets/chart.png)\n\n\
         - clip: [clip.mp4](assets/clip.mp4)\n- voice: [voice.m4a](assets/voice.m4a)\n\
         - notes: [notes.txt](notes.txt)\n- diff: [changes.diff](changes.diff)\n\n\
         ```rust\nfn main() {\n    println!(\"new\");\n}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
    )?;
    let tool = s.db.item(
        &t.thread,
        &t.turn,
        Some(&t.attempt),
        ItemKind::ToolCall,
        Some("completed"),
        Some("call-1"),
        "",
        Some(r#"{"title":"Edit","tool_kind":"edit"}"#),
    )?;
    s.db.patch(
        tool,
        &t.turn,
        "src/lib.rs",
        Some("fn main() {\n    println!(\"old\");\n}\n"),
        "fn main() {\n    println!(\"new\");\n}\n",
    )?;
    s.finish(&t)?;
    named("markdown", &t.thread);

    // 3. A team channel: the lead hands work to two members, who answer in their threads.
    let t = s.thread(
        "Split the work",
        "Have alice build it and bob review it",
        "claude",
        "opus",
    )?;
    s.say(&t, "I asked alice to implement and bob to review.")?;
    for (ordinal, name, role, read_only, agent, model) in [
        (1, "alice", "implementer", false, "codex", "gpt-5"),
        (2, "bob", "reviewer", true, "claude", "sonnet"),
    ] {
        let seat = db.create_seat(&t.turn, ordinal, role, false, read_only, None, None)?;
        db.seat_member(&seat, name, None)?;
        let attempt = db.create_attempt(&seat, &candidate(agent, model), false, None)?;
        db.item(
            &t.thread,
            &t.turn,
            Some(&attempt),
            ItemKind::AgentMessage,
            Some("completed"),
            None,
            &format!("{name} here."),
            None,
        )?;
        db.seat_state(
            &seat,
            if name == "alice" {
                SeatState::Working
            } else {
                SeatState::Done
            },
        )?;
    }
    let a = s.message(
        &t.thread,
        "lead",
        "alice",
        "handoff",
        "@alice implement the change in src/lib.rs",
        Some("alice"),
        None,
        &[],
        now - 120,
    )?;
    s.message(
        &t.thread,
        "alice",
        "lead",
        "mailbox",
        "On it. Started with the print statement.",
        Some("alice"),
        Some(a),
        &[],
        now - 90,
    )?;
    let b = s.message(
        &t.thread,
        "lead",
        "bob",
        "handoff",
        "@bob review alice's change when it lands",
        Some("bob"),
        None,
        &[],
        now - 80,
    )?;
    s.message(
        &t.thread,
        "bob",
        "you",
        "mailbox",
        "@you the change looks right, but should the message stay lowercase?",
        Some("bob"),
        Some(b),
        &["you"],
        now - 30,
    )?;
    s.message(
        &t.thread,
        "alice",
        "channel",
        "mailbox",
        "@channel the implementation is done, over to bob",
        None,
        None,
        &["channel"],
        now - 10,
    )?;
    // The lead's host is this process, so the seats read as live while it runs.
    db.seat_state(&t.seat, SeatState::Working)?;
    named("team", &t.thread);

    // 4. A question waiting for the user.
    let t = s.thread(
        "Run the tests",
        "Run the whole test suite",
        "claude",
        "opus",
    )?;
    let call = s.db.item(
        &t.thread,
        &t.turn,
        Some(&t.attempt),
        ItemKind::ToolCall,
        Some("pending"),
        Some("call-2"),
        "",
        Some(r#"{"title":"Run tests","tool_kind":"execute"}"#),
    )?;
    db.open_prompt(
        &t.thread,
        &t.turn,
        Some(&t.attempt),
        Some(call),
        "permission",
        r#"{"toolCall":{"title":"Run tests"},"options":[{"optionId":"ok","kind":"allow_once","name":"Allow once"},{"optionId":"no","kind":"reject_once","name":"Reject"}]}"#,
    )?;
    db.seat_state(&t.seat, SeatState::Asking)?;
    named("approval", &t.thread);

    // 5. Work in progress.
    let t = s.thread(
        "Refactor the parser",
        "Refactor the parser module",
        "codex",
        "gpt-5",
    )?;
    s.say(&t, "Reading the parser first.")?;
    s.db.item(
        &t.thread,
        &t.turn,
        Some(&t.attempt),
        ItemKind::ToolCall,
        Some("in_progress"),
        Some("call-3"),
        "Reading src/parser.rs",
        Some(r#"{"title":"Read src/parser.rs","tool_kind":"read"}"#),
    )?;
    db.seat_state(&t.seat, SeatState::Working)?;
    named("live", &t.thread);

    Ok(Seeded {
        data,
        repo: root,
        project,
        threads,
    })
}
