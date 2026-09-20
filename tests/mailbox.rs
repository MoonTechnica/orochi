#![cfg(unix)]
use orochi::{
    config::{AgentConfig, Config, MailboxConfig},
    mailbox::{Mailbox, channel},
    types::Provider,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
};

fn me() -> (i64, i64) {
    let id = orochi::process::identity(std::process::id() as i32).unwrap();
    (i64::from(id.pid), id.start as i64)
}
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn worktrees_of_one_repository_share_a_channel_and_other_repositories_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(
        &main,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ],
    );
    git(&main, &["worktree", "add", "-q", "../feature"]);
    let other = dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    git(&other, &["init", "-q"]);
    let feature = dir.path().join("feature");
    assert_eq!(channel(&main, "salt"), channel(&feature, "salt"));
    assert_ne!(channel(&main, "salt"), channel(&other, "salt"));
    assert_ne!(channel(&main, "salt"), channel(&main, "other-salt"));
}

#[test]
fn messages_reach_named_peers_and_broadcasts_but_never_other_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::open(dir.path(), &MailboxConfig::default()).unwrap();
    let root = dir.path();
    let alice = mailbox
        .register("repo", "alice", root, Some("main"), me())
        .unwrap();
    let bob = mailbox.register("repo", "bob", root, None, me()).unwrap();
    // Names are unique per repository; a duplicate gets a suffix.
    let twin = mailbox.register("repo", "Bob", root, None, me()).unwrap();
    assert_eq!(twin.name, "Bob-2");
    let outsider = mailbox
        .register("elsewhere", "bob", root, None, me())
        .unwrap();
    assert_eq!(outsider.name, "bob");

    mailbox.send(&alice.id, "BOB", "api changed").unwrap();
    mailbox
        .send(&alice.id, "all", "touching src/lib.rs")
        .unwrap();
    assert!(mailbox.send(&alice.id, "alice", "self").is_err());
    assert!(mailbox.send(&alice.id, "nobody", "lost").is_err());
    assert!(mailbox.send(&alice.id, "bob", "").is_err());

    let bodies = |id: &str| -> Vec<String> {
        mailbox
            .read(id, 20)
            .unwrap()
            .into_iter()
            .map(|m| m.body)
            .collect()
    };
    assert_eq!(bodies(&bob.id), ["api changed", "touching src/lib.rs"]);
    assert!(bodies(&bob.id).is_empty(), "read messages are not repeated");
    assert_eq!(bodies(&twin.id), ["touching src/lib.rs"]);
    assert!(
        bodies(&alice.id).is_empty(),
        "senders do not read their own messages"
    );
    assert!(bodies(&outsider.id).is_empty());
    // A peer that joins later hears nothing that was said before it existed: answering a
    // question from an earlier turn is worse than not answering.
    let late = mailbox.register("repo", "carol", root, None, me()).unwrap();
    assert!(bodies(&late.id).is_empty());
    mailbox.send(&alice.id, "all", "still here?").unwrap();
    assert_eq!(bodies(&late.id), ["still here?"]);

    mailbox.set_status(&bob.id, "editing the parser").unwrap();
    let peers = mailbox.peers("repo").unwrap();
    assert_eq!(peers.len(), 4);
    let listed = peers.iter().find(|p| p.name == "bob").unwrap();
    assert_eq!(listed.status, "editing the parser");
    mailbox.unregister(&bob.id).unwrap();
    assert!(mailbox.send(&alice.id, "bob", "gone").is_err());
}

#[test]
fn dead_owners_expired_messages_and_rate_limits_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let config = MailboxConfig {
        max_messages_per_hour: 2,
        ..MailboxConfig::default()
    };
    let mailbox = Mailbox::open(dir.path(), &config).unwrap();
    let alice = mailbox
        .register("repo", "alice", dir.path(), None, me())
        .unwrap();
    // No process has this identity, like a killed Orochi run.
    let ghost = mailbox
        .register("repo", "ghost", dir.path(), None, (999_999, 1))
        .unwrap();
    assert!(
        mailbox
            .peers("repo")
            .unwrap()
            .iter()
            .all(|p| p.id != ghost.id)
    );
    let bob = mailbox
        .register("repo", "bob", dir.path(), None, me())
        .unwrap();
    mailbox.send(&alice.id, "bob", "one").unwrap();
    mailbox.send(&alice.id, "bob", "two").unwrap();
    let limited = mailbox.send(&alice.id, "bob", "three").unwrap_err();
    assert!(limited.to_string().contains("limit"));
    drop(mailbox);
    // Expire everything by reopening with a retention shorter than the messages' age.
    let connection = rusqlite::Connection::open(dir.path().join("mailbox.sqlite3")).unwrap();
    connection
        .execute("UPDATE messages SET sent_at = sent_at - 7200", [])
        .unwrap();
    drop(connection);
    let mailbox = Mailbox::open(
        dir.path(),
        &MailboxConfig {
            retention_secs: 3600,
            ..config
        },
    )
    .unwrap();
    assert!(mailbox.read(&bob.id, 20).unwrap().is_empty());
    assert!(mailbox.history("repo", 10).unwrap().is_empty());
}

struct Server {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next: u64,
}
impl Server {
    fn start(data: &Path, peer: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_orochi"))
            .args([
                orochi::mailbox::SERVE_FLAG,
                data.to_str().unwrap(),
                peer,
                "86400",
                "60",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            reader,
            next: 0,
        }
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        )
        .unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], self.next);
        reply
    }
    fn tool(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let reply = self.rpc("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (
            result["isError"].as_bool().unwrap(),
            serde_json::from_str(text).unwrap_or(json!(text)),
        )
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_server_exposes_the_mailbox_tools_to_one_peer() {
    let dir = tempfile::tempdir().unwrap();
    let mailbox = Mailbox::open(dir.path(), &MailboxConfig::default()).unwrap();
    let alice = mailbox
        .register("repo", "alice", dir.path(), None, me())
        .unwrap();
    let bob = mailbox
        .register("repo", "bob", dir.path(), None, me())
        .unwrap();
    let mut server = Server::start(dir.path(), &alice.id);
    let init = server.rpc(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "orochi-mailbox");
    let tools = server.rpc("tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        ["list_peers", "send_message", "read_messages", "set_status"]
    );
    let (error, peers) = server.tool("list_peers", json!({}));
    assert!(!error);
    let peers = peers["peers"].as_array().unwrap();
    assert_eq!(peers.len(), 2);
    assert!(
        peers
            .iter()
            .any(|p| p["name"] == "alice" && p["you"] == true)
    );
    assert!(peers.iter().all(|p| p.get("id").is_none()));
    let (error, _) = server.tool("send_message", json!({"to": "bob", "body": "ping"}));
    assert!(!error);
    assert_eq!(mailbox.read(&bob.id, 20).unwrap()[0].body, "ping");
    mailbox.send(&bob.id, "alice", "pong").unwrap();
    let (_, read) = server.tool("read_messages", json!({"wait_seconds": 1}));
    assert_eq!(read["messages"][0]["body"], "pong");
    assert_eq!(read["messages"][0]["from"], "bob");
    let (error, message) = server.tool("send_message", json!({"to": "nobody", "body": "x"}));
    assert!(error);
    assert!(message.as_str().unwrap().contains("no running peer"));
    let unknown = server.rpc("resources/list", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
}

fn fixture_config(dir: &Path, role: &str, peer: &str, shared: bool) -> std::path::PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut agent = AgentConfig::preset("test", Provider::Openai, "python3", &[]);
    agent.args = vec![script.display().to_string()];
    for (key, value) in [
        ("MOCK_BEHAVIOR", "mailbox_chat"),
        ("MOCK_MODELS", "sol-test"),
        ("MOCK_MAILBOX_ROLE", role),
        ("MOCK_MAILBOX_PEER", peer),
    ] {
        agent.env.insert(key.into(), value.into());
    }
    agent.env.insert(
        "MOCK_LOG".into(),
        dir.join(format!("{role}.jsonl")).display().to_string(),
    );
    let mut config = Config::default();
    config.discovery.auto_add = false;
    config.evaluator.auto = false;
    config.classifier.enabled = false;
    config.scheduler.discovery_timeout_secs = 10;
    config.scheduler.prompt_timeout_secs = 40;
    config.scheduler.shared_workspace = shared;
    config.agents = vec![agent];
    let path = dir.join(format!("{role}.toml"));
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    path
}
fn run(dir: &Path, config: &Path, name: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orochi"));
    command
        .arg("--config")
        .arg(config)
        .arg("--data-dir")
        .arg(dir.join("data"))
        .arg("-C")
        .arg(dir.join("repo"))
        .args(["--peer-name", name, "Implement the greeting"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

#[test]
fn concurrent_runs_in_one_directory_exchange_messages_through_the_mailbox() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo")).unwrap();
    let sender = fixture_config(dir.path(), "sender", "receiver", true);
    let receiver = fixture_config(dir.path(), "receiver", "sender", true);
    let first = run(dir.path(), &receiver, "receiver").spawn().unwrap();
    let second = run(dir.path(), &sender, "sender").spawn().unwrap();
    let outputs = [
        ("receiver", first.wait_with_output().unwrap()),
        ("sender", second.wait_with_output().unwrap()),
    ];
    for (name, output) in outputs {
        assert!(
            output.status.success(),
            "{name}: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            std::fs::read_to_string(dir.path().join(format!("{name}.jsonl.err")))
                .unwrap_or_default()
        );
    }
    let repo = dir.path().join("repo");
    assert_eq!(
        std::fs::read_to_string(repo.join("received.txt")).unwrap(),
        "hello from sender"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("reply.txt")).unwrap(),
        "ack: hello from sender"
    );
    // Both runs have ended: no peers remain, the messages stay until they expire.
    let config = sender.to_str().unwrap();
    let peers = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .args(["--config", config, "--data-dir"])
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(&repo)
        .args(["peers", "--messages", "--json"])
        .output()
        .unwrap();
    assert!(peers.status.success());
    let listed: Value = serde_json::from_slice(&peers.stdout).unwrap();
    assert!(listed["peers"].as_array().unwrap().is_empty());
    assert_eq!(listed["messages"].as_array().unwrap().len(), 2);
    // Message bodies never enter the telemetry database.
    let telemetry = std::fs::read(dir.path().join("data/telemetry.sqlite3")).unwrap();
    assert!(!telemetry.windows(17).any(|w| w == b"hello from sender"));
    // Execution sessions receive the mailbox server.
    let log = std::fs::read_to_string(dir.path().join("sender.jsonl")).unwrap();
    let new_session: Value = log
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["method"] == "session/new")
        .unwrap();
    let server = &new_session["params"]["mcpServers"][0];
    assert_eq!(server["name"], "orochi-mailbox");
    assert_eq!(server["args"][0], orochi::mailbox::SERVE_FLAG);
}

#[test]
fn exclusive_workspaces_still_reject_a_second_run() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo")).unwrap();
    let receiver = fixture_config(dir.path(), "receiver", "sender", false);
    let sender = fixture_config(dir.path(), "sender", "receiver", false);
    let mut first = run(dir.path(), &receiver, "receiver").spawn().unwrap();
    // Wait until the first run holds the lock (its agent has started).
    let log = dir.path().join("receiver.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline
        && !std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("session/prompt")
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let second = run(dir.path(), &sender, "sender").output().unwrap();
    let _ = first.kill();
    let _ = first.wait();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("shared_workspace"));
}

#[test]
fn chat_shows_mailbox_traffic_between_processes_as_messages() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("repo")).unwrap();
    let sender = fixture_config(dir.path(), "sender", "receiver", true);
    let receiver = fixture_config(dir.path(), "receiver", "sender", true);
    let other = run(dir.path(), &receiver, "receiver").spawn().unwrap();
    let mut chat = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .arg("--config")
        .arg(&sender)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(dir.path().join("repo"))
        .args(["--peer-name", "sender", "chat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    chat.stdin
        .take()
        .unwrap()
        .write_all(b"Greet the receiver\n")
        .unwrap();
    let output = chat.wait_with_output().unwrap();
    let other = other.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(other.status.success());
    // A peer's header carries its route once that is known, and when it becomes known is a
    // race with the message itself; assert the parts of the line that do not depend on it.
    for expected in [
        &["✉ sender (this session)", "→ receiver"][..],
        &["▎ hello from sender"][..],
        &["✉ receiver", "→ sender (this session)"][..],
        &["▎ ack: hello from sender"][..],
    ] {
        assert!(
            stderr
                .lines()
                .any(|line| expected.iter().all(|part| line.contains(part))),
            "missing {expected:?} in {stderr}"
        );
    }
    // Our own session is never annotated with its own route: that would be the same message
    // rendered two ways depending on the race above.
    assert!(!stderr.contains("sender (this session) ("), "{stderr}");
    assert!(stderr.find("hello from sender") < stderr.find("ack: hello from sender"));
}

#[test]
fn agent_sessions_in_one_process_are_separate_peers_that_can_message_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let data = dir.path().join("data");
    let config = MailboxConfig::default();
    let _membership = orochi::mailbox::join(
        std::env::current_exe().unwrap(),
        &data,
        &config,
        &repo,
        "salt",
        Some("team"),
    )
    .unwrap();
    let mut designer = orochi::mailbox::register_session(Some("designer")).unwrap();
    let mut implementer = orochi::mailbox::register_session(Some("implementer")).unwrap();
    let mailbox = Mailbox::open(&data, &config).unwrap();
    let channel = channel(&repo, "salt");
    // A session becomes a peer when it starts working, not when it is opened.
    assert!(mailbox.peers(&channel).unwrap().is_empty());
    designer.activate("claude / opus");
    implementer.activate("codex / gpt");
    let names: Vec<_> = mailbox
        .peers(&channel)
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["designer", "implementer"]);
    assert_eq!(
        orochi::mailbox::our_names().into_iter().collect::<Vec<_>>(),
        ["designer", "implementer"]
    );
    // Each session gets its own MCP server arguments, so the agents are addressable.
    let (_, _, args) = orochi::mailbox::server_for(&designer).unwrap();
    assert!(args.contains(&designer.id));
    assert!(!args.contains(&implementer.id));
    mailbox
        .send(&designer.id, "implementer", "please implement this")
        .unwrap();
    let inbox = mailbox.read(&implementer.id, 10).unwrap();
    assert_eq!(inbox[0].from, "designer");
    assert_eq!(inbox[0].body, "please implement this");
    mailbox.send(&implementer.id, "designer", "done").unwrap();
    assert_eq!(mailbox.read(&designer.id, 10).unwrap()[0].body, "done");
    // A finished session stops being a peer; the rest keep working.
    drop(implementer);
    let left: Vec<_> = mailbox
        .peers(&channel)
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(left, ["designer"]);
}

fn seats_config(dir: &Path) -> std::path::PathBuf {
    seats_config_for(dir, 1)
}
fn seats_config_for(dir: &Path, seats: usize) -> std::path::PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut agent = AgentConfig::preset("test", Provider::Openai, "python3", &[]);
    agent.args = vec![script.display().to_string()];
    for (key, value) in [
        ("MOCK_BEHAVIOR", "seats"),
        ("MOCK_MODELS", "sol-test,astra-test"),
        ("MOCK_SEATS", &seats.to_string()),
    ] {
        agent.env.insert(key.into(), value.into());
    }
    agent.env.insert(
        "MOCK_LOG".into(),
        dir.join("agent.jsonl").display().to_string(),
    );
    let mut config = Config::default();
    config.discovery.auto_add = false;
    config.evaluator.auto = false;
    config.classifier.enabled = false;
    config.scheduler.discovery_timeout_secs = 10;
    config.scheduler.prompt_timeout_secs = 60;
    config.agents = vec![agent];
    let path = dir.join("config.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    path
}

/// Work big enough to be worth it seats a second agent beside the one doing it: same working
/// tree, read-only, talking to the first over the mailbox, all inside one Orochi process.
#[test]
fn a_heavy_turn_seats_a_read_only_agent_beside_the_one_doing_the_work() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let config = seats_config(dir.path());
    let mut chat = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .arg("--config")
        .arg(&config)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(&repo)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    chat.stdin
        .take()
        .unwrap()
        .write_all(b"redesign the architecture of the storage layer so every caller goes through one interface\n")
        .unwrap();
    let output = chat.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let fixture_error =
        std::fs::read_to_string(dir.path().join("agent.jsonl.err")).unwrap_or_default();
    assert!(output.status.success(), "{stderr}{fixture_error}");
    assert!(fixture_error.is_empty(), "{fixture_error}");
    let read = |name: &str| std::fs::read_to_string(repo.join(name)).unwrap_or_default();
    assert!(
        read("advice.txt").contains("watch the error path"),
        "the working agent never heard back: {stderr}"
    );
    assert!(!read("asked.txt").is_empty(), "the second seat never ran");
    // Its write tools are refused inside Orochi, so the user is never asked about them.
    assert!(
        read("refused.txt").contains("cancelled"),
        "a read-only seat was allowed to edit: {}",
        read("refused.txt")
    );
    assert!(!stderr.contains("Patch the parser"), "{stderr}");
    // Both seats are told to answer as the user wrote, or they drift into different languages.
    let prompts: Vec<String> = std::fs::read_to_string(dir.path().join("agent.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|r| r["method"] == "session/prompt")
        .map(|r| {
            r["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert!(
        prompts
            .iter()
            .filter(|p| p.contains("language the user used"))
            .count()
            >= 2,
        "{prompts:?}"
    );
    for expected in [
        "beside implement: reviewer",
        "✉ implement",
        "▎ watch the error path, it is unhandled",
        "⎿ reviewer",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }
    // A seat that only reads takes the agent's own read-only mode, so it never gets as far
    // as asking to write.
    let log = std::fs::read_to_string(dir.path().join("agent.jsonl")).unwrap_or_default();
    assert!(
        log.lines()
            .any(|line| line.contains("\"configId\": \"mode\"")
                && line.contains("\"value\": \"plan\"")),
        "{log}"
    );
    // Both seats are this process's own, so neither is singled out as "this session".
    assert!(!stderr.contains("implement (this session)"), "{stderr}");
}

/// Asking for a table of agents seats a table of agents: one Orochi process, one working tree,
/// several ACP sessions at once, talking to each other.
#[test]
fn a_request_for_several_agents_seats_all_of_them_in_one_process() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let config = seats_config_for(dir.path(), 4);
    let mut chat = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .arg("--config")
        .arg(&config)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(&repo)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    chat.stdin
        .take()
        .unwrap()
        .write_all(
            "5人くらいのエージェントで適当なディスカッションをしてみて\nもう少し続けて\n"
                .as_bytes(),
        )
        .unwrap();
    let output = chat.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let fixture_error =
        std::fs::read_to_string(dir.path().join("agent.jsonl.err")).unwrap_or_default();
    assert!(output.status.success(), "{stderr}{fixture_error}");
    assert!(fixture_error.is_empty(), "{fixture_error}");
    // Four panellists answered the one running the discussion, each from its own session.
    let advice = std::fs::read_to_string(repo.join("advice.txt")).unwrap_or_default();
    assert_eq!(advice.lines().count(), 4, "{advice}\n{stderr}");
    for seat in ["skeptic", "architect", "simplifier", "operator"] {
        assert!(advice.contains(seat), "{seat} never answered: {advice}");
        assert!(stderr.contains(seat), "missing {seat:?} in {stderr}");
    }
    let log = std::fs::read_to_string(dir.path().join("agent.jsonl")).unwrap_or_default();
    // The seats end with the turn, so the follow-up seats a whole table again rather than
    // talking into an empty room.
    assert_eq!(
        log.lines()
            .filter(|line| line.contains("session/prompt"))
            .count(),
        10,
        "{log}"
    );
    // And they are not all the same model: a table of one model is worth nothing extra.
    let models: std::collections::BTreeSet<&str> = log
        .lines()
        .filter(|line| line.contains("\"configId\": \"engine\""))
        .filter_map(|line| line.rsplit_once("\"value\": \"").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(models.len() > 1, "every seat ran {models:?}");
}

/// The console path has its own profile and its own seat decision, so the classifier being
/// wired into `scheduler` proves nothing about it. A task the local heuristic reads as an
/// ordinary one-agent change must actually get the heavier treatment when the classifier
/// says so — and the phase split and the seats must agree, having asked only once.
#[test]
fn a_chat_turn_is_seated_by_the_classifier_not_by_the_keyword_profile() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut agent = AgentConfig::preset("test", Provider::Openai, "python3", &[]);
    agent.args = vec![script.display().to_string()];
    for (key, value) in [
        ("MOCK_BEHAVIOR", "seats"),
        ("MOCK_MODELS", "sol-test,astra-test"),
        ("MOCK_SEATS", "1"),
        (
            "MOCK_CLASSIFICATION",
            r#"{"task_type":"review","complexity":"complex","ambiguity":0.1,
                "remember":[{"text":"diffs stay small","scope":"repo"}]}"#,
        ),
    ] {
        agent.env.insert(key.into(), value.into());
    }
    agent.env.insert(
        "MOCK_LOG".into(),
        dir.path().join("agent.jsonl").display().to_string(),
    );
    let mut config = Config::default();
    config.discovery.auto_add = false;
    config.evaluator.auto = false;
    config.scheduler.discovery_timeout_secs = 10;
    config.scheduler.prompt_timeout_secs = 60;
    config.agents = vec![agent];
    config.classifier = orochi::config::ClassifierConfig {
        agent: Some("test".into()),
        model: Some("sol-test".into()),
        timeout_secs: 30,
        ..Default::default()
    };
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();

    // Left to the keyword profile this is "implementation / normal": one agent, no second
    // seat. tests/core.rs pins that reading.
    let mut chat = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .arg("--config")
        .arg(&path)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(&repo)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    chat.stdin
        .take()
        .unwrap()
        .write_all("CIが赤いので直してください\n".as_bytes())
        .unwrap();
    let output = chat.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let fixture_error =
        std::fs::read_to_string(dir.path().join("agent.jsonl.err")).unwrap_or_default();
    assert!(output.status.success(), "{stderr}{fixture_error}");
    assert!(fixture_error.is_empty(), "{fixture_error}");
    assert!(
        stderr.contains("classified as review / complex"),
        "the classifier never reached the console: {stderr}"
    );
    assert!(
        !std::fs::read_to_string(repo.join("asked.txt"))
            .unwrap_or_default()
            .is_empty(),
        "a complex turn was left with one seat: {stderr}"
    );
    // One request, and one line about it. A second announcement used to arrive while the turn
    // was already drawing its route and landed on top of that half-written row.
    let log = std::fs::read_to_string(dir.path().join("agent.jsonl")).unwrap_or_default();
    assert_eq!(
        log.lines()
            .filter(|l| l.contains("You are a task classifier"))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        stderr.matches("classified as").count(),
        1,
        "the turn announced its classification more than once: {stderr}"
    );

    // The same reply said what was worth keeping; nothing extra was asked for it. It is shown
    // when it is kept, lives outside telemetry, and reaches the next fresh session.
    assert!(stderr.contains("remembered: diffs stay small"), "{stderr}");
    let notes = std::fs::read_dir(dir.path().join("data/memory/repos"))
        .unwrap()
        .map(|e| e.unwrap().path().join("MEMORY.md"))
        .collect::<Vec<_>>();
    assert_eq!(notes.len(), 1);
    assert!(
        std::fs::read_to_string(&notes[0])
            .unwrap()
            .contains("diffs stay small")
    );
    let telemetry = std::fs::read(dir.path().join("data/telemetry.sqlite3")).unwrap();
    assert!(!String::from_utf8_lossy(&telemetry).contains("diffs stay small"));

    let mut again = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .arg("--config")
        .arg(&path)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .arg("-C")
        .arg(&repo)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    again
        .stdin
        .take()
        .unwrap()
        .write_all("typo in the readme\n".as_bytes())
        .unwrap();
    let output = again.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = std::fs::read_to_string(dir.path().join("agent.jsonl")).unwrap_or_default();
    let prompts: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("session/prompt") && !l.contains("You are a task classifier"))
        .collect();
    assert!(
        prompts
            .last()
            .is_some_and(|p| p.contains("What Orochi remembers") && p.contains("diffs stay small")),
        "{log}"
    );
}

/// A preference often only shows across several messages. Once, at the end of a session that
/// had more than one, everything the user said is looked back over; a single-message session
/// was already read when its message was sent and costs nothing more.
#[test]
fn a_session_is_looked_back_over_once_at_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut agent = AgentConfig::preset("test", Provider::Openai, "python3", &[]);
    agent.args = vec![script.display().to_string()];
    for (key, value) in [
        ("MOCK_MODELS", "sol-test"),
        (
            "MOCK_CLASSIFICATION",
            r#"{"task_type":"small_edit","complexity":"simple"}"#,
        ),
        (
            "MOCK_DISTILLED",
            r#"{"remember":[{"text":"no keyword heuristics","scope":"repo"}]}"#,
        ),
    ] {
        agent.env.insert(key.into(), value.into());
    }
    agent.env.insert(
        "MOCK_LOG".into(),
        dir.path().join("agent.jsonl").display().to_string(),
    );
    let mut config = Config::default();
    config.discovery.auto_add = false;
    config.evaluator.auto = false;
    config.scheduler.discovery_timeout_secs = 10;
    config.scheduler.prompt_timeout_secs = 60;
    config.agents = vec![agent];
    config.classifier = orochi::config::ClassifierConfig {
        agent: Some("test".into()),
        model: Some("sol-test".into()),
        timeout_secs: 30,
        ..Default::default()
    };
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let session = |input: &str| {
        let mut chat = Command::new(env!("CARGO_BIN_EXE_orochi"))
            .arg("--config")
            .arg(&path)
            .arg("--data-dir")
            .arg(dir.path().join("data"))
            .arg("-C")
            .arg(&repo)
            .arg("chat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        chat.stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = chat.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{stderr}");
        stderr
    };
    let looked_back = || {
        std::fs::read_to_string(dir.path().join("agent.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("You are reviewing a coding session"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    let single = session("fix the typo in README\n");
    assert!(looked_back().is_empty(), "{single}");

    let stderr = session("fix the typo in README\n/new\nPRIVATE please stop using keywords\n");
    let requests = looked_back();
    assert_eq!(requests.len(), 1, "{stderr}");
    // What the user said, across `/new`, and not what the agent answered.
    assert!(requests[0].contains("fix the typo in README"));
    assert!(requests[0].contains("PRIVATE please stop using keywords"));
    assert!(!requests[0].contains("Fixture completed."));
    assert!(
        stderr.contains("remembered: no keyword heuristics"),
        "{stderr}"
    );
    let notes: Vec<_> = std::fs::read_dir(dir.path().join("data/memory/repos"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path().join("MEMORY.md")).unwrap())
        .collect();
    assert!(notes.concat().contains("no keyword heuristics"));
    let telemetry = std::fs::read(dir.path().join("data/telemetry.sqlite3")).unwrap();
    assert!(!String::from_utf8_lossy(&telemetry).contains("PRIVATE please stop"));
}

/// A seat claims its route the moment it chooses, so the next seat of the same process sees it
/// without waiting for the first to register with the mailbox — or for a clock to run out.
#[test]
fn a_route_is_busy_from_the_moment_a_seat_chooses_it() {
    let route = ("claimed-agent".to_owned(), "claimed-model".to_owned());
    let claim = orochi::mailbox::claim(Some("claimer"), &route.0, &route.1);
    assert!(orochi::mailbox::busy_routes(Some("another-seat")).contains(&route));
    assert!(orochi::mailbox::busy_agents(None).contains(&route.0));
    // A seat is never in its own way.
    assert!(!orochi::mailbox::busy_routes(Some("claimer")).contains(&route));
    drop(claim);
    assert!(!orochi::mailbox::busy_routes(Some("another-seat")).contains(&route));
}

/// The seats of one turn take their places in order — the one doing the work first — however
/// long each one's discovery takes, so the lead is never left with what a seat did not want and
/// no seat starts before the one it answers to is there to hear it.
#[tokio::test]
async fn the_seats_of_one_turn_choose_their_routes_in_order() {
    use std::time::Duration;
    use tokio::time::timeout;
    let order = orochi::mailbox::Order::new(2);
    let (lead, seat) = (order.place(0), order.place(1));
    assert!(
        timeout(Duration::from_millis(100), seat.turn())
            .await
            .is_err(),
        "a seat chose before the lead"
    );
    timeout(Duration::from_secs(1), lead.turn()).await.unwrap();
    lead.seated();
    timeout(Duration::from_secs(1), seat.turn()).await.unwrap();

    // A seat that gives up without choosing never holds up the ones after it.
    let order = orochi::mailbox::Order::new(3);
    let (first, second, third) = (order.place(0), order.place(1), order.place(2));
    drop(first);
    assert!(
        timeout(Duration::from_millis(100), third.turn())
            .await
            .is_err()
    );
    second.seated();
    timeout(Duration::from_secs(1), third.turn()).await.unwrap();
}
