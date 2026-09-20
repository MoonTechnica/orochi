use orochi::{
    config::{AgentConfig, CheckCommand, Config},
    storage::Store,
    types::{Outcome, Provider},
};
use serde_json::json;
use std::{
    path::Path,
    process::{Command, Output},
    time::Duration,
};

fn fixture(agent: &str, behavior: &str, models: &str, log: &Path) -> AgentConfig {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut config = AgentConfig::preset(agent, Provider::Openai, "python3", &[]);
    config.args = vec![script.to_string_lossy().into_owned()];
    config.env.insert("MOCK_BEHAVIOR".into(), behavior.into());
    config.env.insert("MOCK_MODELS".into(), models.into());
    config
        .env
        .insert("MOCK_LOG".into(), log.to_string_lossy().into_owned());
    config
}
struct Workspace {
    dir: tempfile::TempDir,
    config: Config,
}
impl Workspace {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("repo")).unwrap();
        let mut config = Config::default();
        config.discovery.auto_add = false;
        config.evaluator.auto = false;
        config.classifier.enabled = false;
        config.scheduler.discovery_timeout_secs = 5;
        config.scheduler.prompt_timeout_secs = 5;
        config.agents = vec![fixture(
            "test",
            "success",
            "sol-test,astra-test",
            &dir.path().join("agent.jsonl"),
        )];
        Self { dir, config }
    }
    fn command(&self) -> Command {
        let config = self.dir.path().join("config.toml");
        std::fs::write(&config, toml::to_string(&self.config).unwrap()).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_orochi"));
        cmd.args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            self.dir.path().join("data").to_str().unwrap(),
            "--cwd",
            self.dir.path().join("repo").to_str().unwrap(),
        ]);
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn store(&self) -> Store {
        Store::open(&self.dir.path().join("data")).unwrap()
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("agent.jsonl")).unwrap_or_default()
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn dry_run_discovers_model_specific_reasoning_without_prompting() {
    let w = Workspace::new();
    let output = w.run(&["--dry-run", "--json", "Implement a small endpoint"]);
    success(&output);
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["candidates"][0]["model"], "sol-test");
    let frontier = plan["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["model"] == "astra-test")
        .unwrap();
    assert_eq!(frontier["reasoning_level"], "high");
    assert!(!w.log().contains("session/prompt"));
    assert!(!w.dir.path().join("repo/completed.txt").exists());
}

#[test]
fn completion_streams_and_stores_metadata_without_task_or_source() {
    let mut w = Workspace::new();
    w.config.evaluator.checks = vec![CheckCommand {name:"tests".into(), command:"python3".into(), args:vec!["-c".into(),"from pathlib import Path; assert Path('completed.txt').read_text() == 'mock success\\n'".into()]}];
    let output = w.run(&["Implement PRIVATE_CUSTOMER_TASK_SECRET"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Fixture completed."));
    let records = w.store().recent_runs(10).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, Outcome::Success);
    assert_eq!(records[0].usage.total(), Some(150));
    assert_eq!(records[0].usage.cached_tokens, Some(30));
    let serialized = serde_json::to_string(&records).unwrap();
    assert!(!serialized.contains("PRIVATE_CUSTOMER_TASK_SECRET"));
    assert!(!serialized.contains("mock success"));
    assert!(!serialized.contains(w.dir.path().to_str().unwrap()));
}

#[test]
fn rate_limit_hands_off_partial_files_and_persists_cooldown() {
    let mut w = Workspace::new();
    w.config.agents = vec![
        fixture("fast", "rate", "sol-test", &w.dir.path().join("fast.jsonl")),
        fixture(
            "backup",
            "requires_handoff",
            "astra-test",
            &w.dir.path().join("backup.jsonl"),
        ),
    ];
    let output = w.run(&["Implement an endpoint with validation"]);
    success(&output);
    assert!(w.dir.path().join("repo/partial.txt").exists());
    assert!(w.dir.path().join("repo/completed.txt").exists());
    let runs = w.store().recent_runs(10).unwrap();
    assert_eq!(runs.len(), 2);
    let runtime = w.store().runtime().unwrap();
    assert!(!runtime[&("fast".into(), "*".into())].available(orochi::types::now()));
    let prompts = std::fs::read_to_string(w.dir.path().join("backup.jsonl")).unwrap();
    assert!(prompts.contains("previous attempt"));
    let before = std::fs::read_to_string(w.dir.path().join("fast.jsonl")).unwrap();
    success(&w.run(&["--dry-run", "Another implementation task"]));
    assert_eq!(
        before,
        std::fs::read_to_string(w.dir.path().join("fast.jsonl")).unwrap()
    );
}

#[test]
fn model_specific_rate_limit_can_fall_back_within_same_agent() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "model_rate".into());
    let output = w.run(&["Implement a small endpoint"]);
    success(&output);
    let runs = w.store().recent_runs(10).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].candidate.model, "astra-test");
    let runtime = w.store().runtime().unwrap();
    assert!(!runtime[&("test".into(), "sol-test".into())].available(orochi::types::now()));
}

#[test]
fn single_model_can_retry_an_unfinished_turn_within_attempt_budget() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "retry_once".into());
    w.config.agents[0]
        .env
        .insert("MOCK_MODELS".into(), "sol-test".into());
    success(&w.run(&["Implement a small endpoint"]));
    assert_eq!(w.store().recent_runs(10).unwrap().len(), 2);
    assert!(w.log().contains("previous attempt"));
}

#[test]
fn explicit_unknown_model_never_reaches_prompt() {
    let w = Workspace::new();
    let output = w.run(&["--model", "invented-model", "Implement endpoint"]);
    assert!(!output.status.success());
    assert!(!w.log().contains("session/prompt"));
}

#[test]
fn permissions_select_allow_once_instead_of_first_option() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "permission".into());
    success(&w.run(&["--permission", "allow", "Implement endpoint"]));
    assert!(w.dir.path().join("repo/completed.txt").exists());
    let denied = Workspace::new();
    let mut denied = denied;
    denied.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "permission".into());
    let output = denied.run(&["--permission", "deny", "Implement endpoint"]);
    assert_eq!(output.status.code(), Some(130));
    assert!(!denied.dir.path().join("repo/completed.txt").exists());
    assert_eq!(denied.store().recent_runs(10).unwrap().len(), 1);
}

#[test]
fn legacy_model_discovery_and_agent_default_are_supported() {
    for behavior in ["legacy", "default_only"] {
        let mut w = Workspace::new();
        w.config.agents[0]
            .env
            .insert("MOCK_BEHAVIOR".into(), behavior.into());
        let output = w.run(&["Implement endpoint"]);
        success(&output);
        let runs = w.store().recent_runs(10).unwrap();
        assert_eq!(runs[0].candidate.reasoning_level, None);
        if behavior == "default_only" {
            assert_eq!(runs[0].candidate.model, "agent-default");
        }
    }
}

#[test]
fn recorded_session_can_be_resumed_and_is_repository_scoped() {
    let w = Workspace::new();
    success(&w.run(&["Implement endpoint"]));
    let session = w.store().sessions(None).unwrap()[0].session_id.clone();
    success(&w.run(&["--resume", &session, "Add validation"]));
    assert!(w.log().contains("session/load"));
    let other = tempfile::tempdir().unwrap();
    let output = w
        .command()
        .args([
            "--cwd",
            other.path().to_str().unwrap(),
            "--resume",
            &session,
            "Add validation",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

fn chat(w: &Workspace, input: &str) -> Output {
    chat_with(w, &[], input)
}
fn chat_with(w: &Workspace, args: &[&str], input: &str) -> Output {
    let mut child = w
        .command()
        .args(args)
        .arg("chat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
/// (method, sessionId, prompt text) of the session requests the fixture received, in order.
fn session_requests(w: &Workspace) -> Vec<(String, String, String)> {
    w.log()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|r| {
            matches!(
                r["method"].as_str(),
                Some("session/load" | "session/prompt")
            )
        })
        .map(|r| {
            (
                r["method"].as_str().unwrap().to_owned(),
                r["params"]["sessionId"].as_str().unwrap().to_owned(),
                r["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn chat_continues_the_routed_session_until_reroute_or_new() {
    let w = Workspace::new();
    let output = chat(
        &w,
        "First PRIVATE_CHAT_QUESTION\nSecond\n/status\n/reroute\nThird\n/new\nFourth\n",
    );
    success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .matches("Fixture completed.\n")
            .count(),
        4
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Agent        test · sol-test"));
    let requests = session_requests(&w);
    let methods: Vec<_> = requests.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        methods,
        [
            "session/prompt",
            "session/load",
            "session/prompt",
            "session/prompt",
            "session/prompt"
        ]
    );
    let first = &requests[0].1;
    assert_eq!(&requests[1].1, first);
    assert_eq!(&requests[2].1, first);
    assert!(requests[2].2.ends_with("Second"));
    assert!(!requests[2].2.contains("Previous conversation"));
    assert_ne!(&requests[3].1, first);
    assert!(requests[3].2.contains("Previous conversation"));
    assert!(
        requests[3]
            .2
            .contains("User: First PRIVATE_CHAT_QUESTION\nAgent: Fixture completed.")
    );
    assert!(requests[3].2.ends_with("Current user request:\nThird"));
    assert!(!requests[4].2.contains("Previous conversation"));
    let runs = w.store().recent_runs(10).unwrap();
    assert_eq!(runs.len(), 4);
    let stored = serde_json::to_string(&(runs, w.store().sessions(None).unwrap())).unwrap();
    assert!(!stored.contains("PRIVATE_CHAT_QUESTION"));
    // A reloaded session keeps the agent-to-agent mailbox tools.
    for method in ["session/new", "session/load"] {
        let servers: Vec<_> = w
            .log()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|r| r["method"] == method)
            .map(|r| {
                r["params"]["mcpServers"][0]["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        assert!(!servers.is_empty());
        assert!(
            servers.iter().all(|name| name == "orochi-mailbox"),
            "{method}: {servers:?}"
        );
    }
}

#[test]
fn chat_passes_earlier_messages_as_context_when_the_agent_cannot_reload_sessions() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_NO_LOAD".into(), "1".into());
    let output = chat(&w, "Remember the word lantern\nWhich word?\n");
    success(&output);
    let requests = session_requests(&w);
    assert!(requests.iter().all(|r| r.0 == "session/prompt"));
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0].1, requests[1].1);
    assert!(requests[1].2.contains("User: Remember the word lantern"));
    assert!(
        requests[1]
            .2
            .ends_with("Current user request:\nWhich word?")
    );
    let sessions = w.store().sessions(None).unwrap();
    assert!(sessions.iter().all(|s| s.model == "sol-test"));
}

#[test]
fn chat_never_takes_a_piped_line_as_a_permission_answer() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "permission".into());
    let output = chat_with(&w, &["--permission", "ask"], "Write the file\ny\n");
    success(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Permission requested without a terminal")
    );
    let prompts: Vec<_> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/prompt")
        .map(|r| r.2)
        .collect();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].ends_with("\ny"));
    assert!(!w.dir.path().join("repo/completed.txt").exists());
}

fn appending_check(log: &Path, fails: bool) -> CheckCommand {
    CheckCommand {
        name: "tests".into(),
        command: "python3".into(),
        args: vec![
            "-c".into(),
            format!(
                "open({:?}, 'a').write('x'); raise SystemExit({})",
                log.to_str().unwrap(),
                u8::from(fails)
            ),
        ],
    }
}

#[test]
fn chat_sends_messages_as_typed_and_skips_checks_when_nothing_changed() {
    let mut w = Workspace::new();
    let checks = w.dir.path().join("checks.log");
    w.config.evaluator.checks = vec![appending_check(&checks, false)];
    w.config.agents[0]
        .env
        .insert("MOCK_READ_ONLY".into(), "1".into());
    success(&chat(&w, "test\nexplain the layout\n"));
    assert!(!checks.exists(), "checks ran although no file changed");
    let prompts: Vec<_> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/prompt")
        .map(|r| r.2)
        .collect();
    assert_eq!(prompts.len(), 2);
    assert!(prompts.iter().all(|p| !p.contains("coding task")));
    assert!(prompts[0].ends_with("\n\ntest") || prompts[0] == "test");
    let runs = w.store().recent_runs(10).unwrap();
    assert!(runs.iter().all(|r| r.outcome == Outcome::PartialSuccess));
    // A one-shot task keeps verifying even when nothing changed.
    success(&w.run(&["Check the layout"]));
    assert!(checks.exists());
}

#[test]
fn chat_shows_tools_plans_and_real_failures_without_routing_noise() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_TOOL".into(), "1".into());
    w.config.agents.push(AgentConfig::preset(
        "absent",
        Provider::Google,
        "orochi-test-missing-cli",
        &[],
    ));
    w.config.agents.push(fixture(
        "locked",
        "auth",
        "sol-test",
        &w.dir.path().join("locked.jsonl"),
    ));
    let output = chat(&w, "First\nSecond\n");
    success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in [
        "⎿ test · sol-test · medium",
        "⏺ List fixture files: $ ls ✓",
        "⎿ a.txt … +1 lines",
        "⏺ Run broken tool ✗",
        "⏺ Edit notes: notes.txt ✓",
        "⎿ Updated notes.txt with 1 addition and 1 removal",
        "- old line",
        "+ new line",
        "⏺ Plan",
        "☒ Inspect files",
        "☐ Report back",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in {stderr}"
        );
    }
    assert_eq!(stderr.matches("⎿ test · sol-test").count(), 1, "{stderr}");
    assert_eq!(stderr.matches("locked unavailable").count(), 1, "{stderr}");
    for noise in [
        "Profiling:",
        "Route:",
        "Unavailable:",
        "absent",
        "partial_success",
        "Mailbox peer",
    ] {
        assert!(!stderr.contains(noise), "unexpected {noise:?} in {stderr}");
    }
}

#[test]
fn chat_never_hands_a_finished_turn_with_failing_checks_to_another_agent() {
    let mut w = Workspace::new();
    let checks = w.dir.path().join("checks.log");
    w.config.evaluator.checks = vec![appending_check(&checks, true)];
    w.config.agents.push(fixture(
        "backup",
        "success",
        "sol-test",
        &w.dir.path().join("backup.jsonl"),
    ));
    let output = chat(&w, "Implement endpoint\n");
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("✗ Checks failed: tests"));
    let prompts = [
        w.log(),
        std::fs::read_to_string(w.dir.path().join("backup.jsonl")).unwrap(),
    ]
    .iter()
    .map(|log| log.matches("\"session/prompt\"").count())
    .sum::<usize>();
    assert_eq!(prompts, 1);
    let runs = w.store().recent_runs(10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].outcome, Outcome::Failure);
}

#[test]
fn yolo_allows_once_and_continue_resumes_the_latest_session() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "permission".into());
    let fresh = w.run(&["--continue", "Implement endpoint"]);
    assert_eq!(fresh.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&fresh.stderr).contains("no recorded session"));
    success(&w.run(&["--yolo", "Implement endpoint"]));
    assert!(w.dir.path().join("repo/completed.txt").exists());
    let session = w.store().sessions(None).unwrap()[0].session_id.clone();
    success(&w.run(&["-c", "--always-approve", "Add validation"]));
    let loads: Vec<_> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/load")
        .collect();
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].1, session);
    let both = w.run(&["--yolo", "--permission", "ask", "Implement endpoint"]);
    assert_eq!(both.status.code(), Some(2));
}

#[test]
fn team_runs_design_implement_and_review_as_separate_sessions() {
    let w = Workspace::new();
    let output = chat(&w, "/team add a health endpoint\n");
    success(&output);
    let requests = session_requests(&w);
    let prompts: Vec<_> = requests
        .iter()
        .filter(|r| r.0 == "session/prompt")
        .map(|r| r.2.as_str())
        .collect();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[0].contains("Plan the work first"));
    assert!(prompts[1].contains("following the plan above"));
    assert!(prompts[2].contains("Review"));
    // Each step hands its reply to the next one and works in its own session.
    assert!(prompts[1].contains("From the previous step:\nFixture completed."));
    assert!(prompts.iter().all(|p| p.contains("add a health endpoint")));
    // Every prompt Orochi writes is English, so each step is told to answer as the user wrote.
    assert!(
        prompts.iter().all(|p| p.contains("language the user used")),
        "{prompts:?}"
    );
    assert!(requests.iter().all(|r| r.0 != "session/load"));
    let sessions = w.store().sessions(None).unwrap();
    assert_eq!(sessions.len(), 3);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for step in [
        "⏵ design step 1/3",
        "⏵ implement step 2/3",
        "⏵ review step 3/3",
    ] {
        assert!(stderr.contains(step), "missing {step:?} in {stderr}");
    }
}

#[test]
fn chat_splits_only_the_work_that_needs_more_than_one_agent() {
    let w = Workspace::new();
    // A small request stays one turn; a whole-system rewrite is planned first.
    success(&chat(
        &w,
        "fix the typo in the header\nrewrite the entire architecture from scratch\n",
    ));
    // The implementation gets a read-only seat beside it; only the steps themselves count here.
    let prompts: Vec<_> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/prompt" && !r.2.contains("you cannot, every write tool"))
        .map(|r| r.2)
        .collect();
    assert_eq!(prompts.len(), 4);
    assert!(prompts[0].ends_with("fix the typo in the header"));
    assert!(prompts[1].contains("Plan the work first"));
    assert!(prompts[2].contains("following the plan above"));
    assert!(prompts[3].contains("Review"));
}

/// The design step may divide the work into parts for Orochi to order. That answer is for
/// Orochi: it never reaches the transcript or the next step's handover. Off a terminal, where
/// nobody can be asked before several agents start writing, the work stays one turn.
#[test]
fn a_divided_design_stays_one_turn_off_a_terminal_and_its_parts_stay_out_of_sight() {
    let mut w = Workspace::new();
    let parts = r#"{"parts":[{"id":"alpha","brief":"build alpha","paths":["alpha"]},{"id":"beta","brief":"build beta","paths":["beta"]}]}"#;
    w.config.agents[0].env.insert(
        "MOCK_DESIGN_REPLY".into(),
        format!("Build alpha and beta side by side.\n```json\n{parts}\n```\n"),
    );
    let output = chat(&w, "rewrite the entire architecture from scratch\n");
    success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Build alpha and beta side by side."),
        "{stdout}"
    );
    assert!(
        !stdout.contains("\"parts\"") && !stdout.contains("```"),
        "{stdout}"
    );
    // The implementation may get a read-only seat beside it; only the steps count here.
    let prompts: Vec<_> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/prompt" && !r.2.contains("you cannot, every write tool"))
        .map(|r| r.2)
        .collect();
    assert_eq!(prompts.len(), 3);
    assert!(prompts[0].contains("\"parts\""), "{}", prompts[0]);
    assert!(prompts[1].contains("From the previous step:\nBuild alpha and beta side by side."));
    assert!(!prompts[1].contains("\"parts\""), "{}", prompts[1]);
    assert!(!w.dir.path().join("data/collaborations").exists());

    // JSON that is not the division of the work is part of the answer, and stays in it.
    let mut plain = Workspace::new();
    plain.config.agents[0].env.insert(
        "MOCK_DESIGN_REPLY".into(),
        "Keep the config as:\n```json\n{\"partsList\": 1}\n```\n{\"particle\": true}\n".into(),
    );
    let output = chat(&plain, "rewrite the entire architecture from scratch\n");
    success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("```json\n{\"partsList\": 1}\n```\n{\"particle\": true}"),
        "{stdout}"
    );
}

/// Asking an agent what a request is costs tokens like any other call. What a run cost has to
/// include what it took to decide it, or Orochi looks cheaper than it is; the record carries
/// the classifier's route and usage, and — not being an execution — never teaches the router.
#[test]
fn what_the_classifier_spent_is_recorded_beside_the_run_it_decided() {
    let mut w = Workspace::new();
    w.config.classifier.enabled = true;
    w.config.agents[0].env.insert(
        "MOCK_CLASSIFICATION".into(),
        r#"{"task_type":"implementation","complexity":"normal","ambiguity":0.1}"#.into(),
    );
    success(&w.run(&["Implement the PRIVATE endpoint"]));
    let runs = w.store().recent_runs(10).unwrap();
    let classification: Vec<_> = runs
        .iter()
        .filter(|r| r.purpose == "classification")
        .collect();
    assert_eq!(classification.len(), 1, "{runs:?}");
    assert_eq!(classification[0].candidate.agent, "test");
    assert_eq!(classification[0].usage.total_tokens, Some(20));
    assert_eq!(runs.iter().filter(|r| r.purpose == "execution").count(), 1);
    let stored = serde_json::to_string(classification[0]).unwrap();
    assert!(!stored.contains("PRIVATE"), "{stored}");
}

/// Asking an agent what a request is costs a session of its own. Once Orochi has measured
/// what asking costs and what work like this costs, it stops paying for a question worth less
/// than its share of the work, and says so instead of going quiet.
#[test]
fn it_stops_paying_to_ask_what_a_request_is_once_that_costs_more_than_the_work() {
    let mut w = Workspace::new();
    w.config.classifier.enabled = true;
    // A share this small means: as soon as both costs are known, the question is not worth it.
    w.config.classifier.max_cost_share = 0.01;
    w.config.agents[0].env.insert(
        "MOCK_CLASSIFICATION".into(),
        r#"{"task_type":"implementation","complexity":"normal"}"#.into(),
    );
    let mut stderr = String::new();
    for number in 1..=4 {
        let output = w.run(&[&format!("Implement endpoint number {number}")]);
        success(&output);
        stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    }
    let runs = w.store().recent_runs(20).unwrap();
    // Three runs to measure both sides, and from the fourth the question is skipped.
    assert_eq!(
        runs.iter()
            .filter(|r| r.purpose == "classification")
            .count(),
        3,
        "{runs:?}"
    );
    assert_eq!(runs.iter().filter(|r| r.purpose == "execution").count(), 4);
    assert!(stderr.contains("would cost more than the work"), "{stderr}");
}

/// The live comparison (`examples/compare_live.py`) is only worth its quota if its numbers are
/// right: a trial is scored by the hidden check alone, and an arm is charged every token its
/// runs spent — the classifier's and every retry's included. Driven here with the fixture.
#[test]
fn a_live_comparison_charges_every_token_and_scores_by_the_hidden_check() {
    let dir = tempfile::tempdir().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let agent = |id: &str| {
        json!({"id": id, "provider": "openai", "command": "python3",
               "args": [script.display().to_string()],
               "env": {"MOCK_BEHAVIOR": "success", "MOCK_MODELS": "sol-test",
                       "MOCK_CLASSIFICATION": r#"{"task_type":"implementation","complexity":"normal"}"#}})
    };
    let suite = json!({
        "schema_version": 2,
        "name": "fixture",
        "arms": [
            {"id": "alpha-alone", "agents": [agent("alpha")],
             "pin": {"agent": "alpha", "model": "sol-test"}, "attempts": 1, "classifier": false},
            {"id": "beta-alone", "agents": [agent("beta")],
             "pin": {"agent": "beta", "model": "sol-test"}, "attempts": 1, "classifier": false},
            {"id": "orochi", "agents": [agent("alpha"), agent("beta")],
             "pin": null, "attempts": 2, "classifier": true},
        ],
        "cases": [
            {"id": "done", "task": "Implement the endpoint", "files": {"app.py": "\n"},
             "visible": {"command": "python3", "args": ["-c", "import pathlib; assert pathlib.Path('completed.txt').exists()"]},
             "check": "import pathlib\nassert pathlib.Path('completed.txt').exists()\n",
             "reference": {"completed.txt": "done\n"}},
            {"id": "never", "task": "Implement the endpoint", "files": {"app.py": "\n"},
             "visible": {"command": "python3", "args": ["-c", "import pathlib; assert pathlib.Path('solved.txt').exists()"]},
             "check": "import pathlib\nassert pathlib.Path('solved.txt').exists()\n",
             "reference": {"solved.txt": "done\n"}},
        ],
    });
    let path = dir.path().join("suite.json");
    std::fs::write(&path, serde_json::to_vec(&suite).unwrap()).unwrap();
    let output = dir.path().join("out");
    let run = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compare_live.py"))
        .args([
            "--suite",
            path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .args([
            "--binary",
            env!("CARGO_BIN_EXE_orochi"),
            "--timeout",
            "30",
            "--execute",
        ])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let results: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("results.json")).unwrap()).unwrap();
    let trial = |case: &str, arm: &str| {
        results["trials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["case"] == case && t["arm"] == arm)
            .unwrap()
            .clone()
    };
    for arm in ["alpha-alone", "beta-alone", "orochi"] {
        assert_eq!(trial("done", arm)["status"], "measured", "{arm}");
        assert_eq!(trial("done", arm)["hidden_pass"], true, "{arm}");
        assert_eq!(trial("never", arm)["hidden_pass"], false, "{arm}");
    }
    // One execution each alone; the orochi arm also paid for asking what the task was.
    assert_eq!(trial("done", "alpha-alone")["tokens"]["total"], 150);
    assert_eq!(
        trial("done", "orochi")["tokens"]["by_purpose"]["classification"],
        20
    );
    assert_eq!(trial("done", "orochi")["tokens"]["total"], 170);
    // A failed visible check made it try the other agent, and that retry is charged too.
    let retried = trial("never", "orochi");
    assert_eq!(retried["attempts"], 2, "{retried}");
    assert_eq!(retried["tokens"]["total"], 320, "{retried}");
    let summary = std::fs::read_to_string(output.join("summary.md")).unwrap();
    for arm in ["alpha-alone", "beta-alone", "orochi"] {
        assert!(summary.contains(arm), "{summary}");
    }
    assert_eq!(results["arms"]["orochi"]["passed"], 1);
    assert_eq!(results["arms"]["orochi"]["tokens"], 490);

    // Without --execute it contacts nothing.
    let refused = Command::new("python3")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/compare_live.py"))
        .args(["--suite", path.to_str().unwrap(), "--output"])
        .arg(dir.path().join("refused"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!dir.path().join("refused").exists());
}

/// Every case of the comparison suite must fail as handed to the agents and pass with its
/// reference solution — otherwise a pass or a failure in a live run says nothing.
#[test]
fn every_comparison_case_fails_as_given_and_passes_with_its_reference() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = tempfile::tempdir().unwrap();
    let run = Command::new("python3")
        .arg(root.join("examples/compare_live.py"))
        .args(["--suite"])
        .arg(root.join("examples/compare-suite.json"))
        .args(["--output"])
        .arg(dir.path().join("validation"))
        .arg("--validate")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("validation/validation.json")).unwrap(),
    )
    .unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert!(cases.len() >= 10, "{report}");
    for case in cases {
        assert_eq!(case["starts_failing"], true, "{case}");
        assert_eq!(case["reference_passes"], true, "{case}");
        assert_eq!(case["visible_passes_with_reference"], true, "{case}");
    }
}

#[test]
fn a_message_needing_another_capability_goes_to_an_agent_that_has_it() {
    let mut w = Workspace::new();
    w.config.agents[0].image = false;
    w.config.agents.push(fixture(
        "artist",
        "success",
        "sol-test",
        &w.dir.path().join("artist.jsonl"),
    ));
    w.config.agents[1].image = true;
    let output = chat_with(
        &w,
        &["--agent", "test"],
        "explain this repository\n画像を生成して\n",
    );
    success(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("other capabilities · handing it over")
    );
    let artist: Vec<String> = std::fs::read_to_string(w.dir.path().join("artist.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|r| r["method"] == "session/prompt")
        .map(|r| {
            r["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(artist.len(), 1);
    assert!(artist[0].ends_with("画像を生成して"));
    // The conversation stays with the first agent, which is told what happened.
    let log = w.log();
    assert_eq!(log.matches("\"session/prompt\"").count(), 1);
    assert_eq!(w.store().sessions(None).unwrap().len(), 2);
}

#[test]
fn a_mode_from_an_earlier_turn_never_blocks_a_later_one() {
    let mut w = Workspace::new();
    w.config.scheduler.max_attempts = 1;
    // "plan" exists for sol-test in the fixture; the run must not stall when it does not.
    let output = w.run(&[
        "--mode",
        "plan",
        "--model",
        "sol-test",
        "Implement endpoint",
    ]);
    success(&output);
    let unavailable = w.run(&["--mode", "no-such-mode", "Implement endpoint"]);
    success(&unavailable);
    assert!(
        String::from_utf8_lossy(&unavailable.stderr).contains("continuing without it"),
        "{}",
        String::from_utf8_lossy(&unavailable.stderr)
    );
    assert!(w.dir.path().join("repo/completed.txt").exists());
}

#[test]
fn chat_attaches_referenced_files_and_images_to_the_prompt() {
    let w = Workspace::new();
    let repo = w.dir.path().join("repo");
    std::fs::write(repo.join("notes.md"), "# notes\nhello\n").unwrap();
    std::fs::write(repo.join("shot.png"), b"\x89PNG\r\n\x1a\nfake").unwrap();
    success(&chat(&w, "Look at @notes.md and @shot.png\n"));
    let log = w.log();
    let prompt = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|r| r["method"] == "session/prompt")
        .unwrap();
    let blocks = prompt["params"]["prompt"].as_array().unwrap();
    let text = blocks[0]["text"].as_str().unwrap();
    assert!(text.ends_with("Look at [File #1] and [Image #1]"), "{text}");
    let kinds: Vec<_> = blocks.iter().map(|b| b["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["text", "resource", "image"]);
    assert_eq!(blocks[1]["resource"]["text"], "# notes\nhello\n");
    assert_eq!(blocks[2]["mimeType"], "image/png");
    assert_eq!(blocks[2]["data"], "iVBORw0KGgpmYWtl");
}

#[test]
fn bare_orochi_without_a_terminal_prints_help_instead_of_waiting_for_input() {
    let w = Workspace::new();
    let output = w.run(&[]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(!w.log().contains("session/new"));
    let dry_run = w.run(&["--dry-run", "chat"]);
    assert_eq!(dry_run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&dry_run.stderr).contains("need a task"));
}

#[test]
fn failed_evaluation_does_not_count_as_success() {
    let mut w = Workspace::new();
    w.config.scheduler.max_attempts = 1;
    w.config.evaluator.checks = vec![CheckCommand {
        name: "tests".into(),
        command: "python3".into(),
        args: vec!["-c".into(), "raise SystemExit(1)".into()],
    }];
    let output = w.run(&["Implement endpoint"]);
    assert!(!output.status.success());
    assert_eq!(
        w.store().recent_runs(1).unwrap()[0].outcome,
        Outcome::Failure
    );
}

#[test]
fn prompt_timeout_terminates_agent_descendants() {
    let mut w = Workspace::new();
    w.config.scheduler.prompt_timeout_secs = 1;
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "hang".into());
    let output = w.run(&["Implement endpoint"]);
    assert!(!output.status.success());
    assert_eq!(
        w.store().recent_runs(1).unwrap()[0].error_kind.as_deref(),
        Some("timeout")
    );
    #[cfg(unix)]
    {
        let pid: i32 = std::fs::read_to_string(w.dir.path().join("repo/child.pid"))
            .unwrap()
            .parse()
            .unwrap();
        for _ in 0..20 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("agent child survived timeout");
    }
}

#[cfg(unix)]
#[test]
fn ctrl_c_records_cancellation_without_fallback() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_BEHAVIOR".into(), "hang".into());
    let stderr_path = w.dir.path().join("stderr.log");
    let mut child = w
        .command()
        .arg("Implement endpoint")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).unwrap())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if w.dir.path().join("repo/child.pid").exists() {
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !w.dir.path().join("repo/child.pid").exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!(
            "agent did not become ready: {}\nACP log: {}",
            std::fs::read_to_string(stderr_path).unwrap(),
            w.log()
        );
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    assert_eq!(child.wait().unwrap().code(), Some(130));
    let runs = w.store().recent_runs(10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].outcome, Outcome::Cancelled);
}

#[cfg(unix)]
fn running(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn wait_until(seconds: u64, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while std::time::Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    ready()
}

/// Starts a hanging agent with a tool process outside its process group and returns
/// (orochi, [agent, child, detached], lease).
#[cfg(unix)]
fn start_hanging_tree(w: &mut Workspace) -> (std::process::Child, [i32; 3], serde_json::Value) {
    w.config.scheduler.prompt_timeout_secs = 60;
    let env = &mut w.config.agents[0].env;
    env.insert("MOCK_BEHAVIOR".into(), "hang".into());
    env.insert("MOCK_DETACHED".into(), "1".into());
    let mut child = w
        .command()
        .arg("Implement endpoint")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let repo = w.dir.path().join("repo");
    let pid = |name: &str| -> Option<i32> {
        std::fs::read_to_string(repo.join(name))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    if !wait_until(15, || pid("child.pid").is_some()) {
        let _ = child.kill();
        let _ = child.wait();
        panic!("agent did not become ready: {}", w.log());
    }
    let pids = ["agent.pid", "child.pid", "detached.pid"].map(|n| pid(n).unwrap());
    let leases = w.dir.path().join("data/processes");
    let mut lease = serde_json::Value::Null;
    let recorded = wait_until(10, || {
        for entry in std::fs::read_dir(&leases).into_iter().flatten().flatten() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(entry.path()).unwrap_or_default(),
            ) && value["descendants"]
                .as_array()
                .is_some_and(|d| d.iter().any(|p| p["pid"] == pids[2]))
            {
                lease = value;
                return true;
            }
        }
        false
    });
    if !recorded {
        let _ = child.kill();
        let _ = child.wait();
        panic!("detached tool was not recorded in a lease");
    }
    (child, pids, lease)
}

#[cfg(unix)]
#[test]
fn forced_exit_stops_the_agent_tree_including_detached_tools() {
    let mut w = Workspace::new();
    let (mut orochi, pids, lease) = start_hanging_tree(&mut w);
    assert_eq!(lease["agent"]["pid"], pids[0]);
    // A concurrent invocation must not reap processes whose owner is alive.
    success(&w.run(&["runs"]));
    assert!(pids.iter().all(|pid| running(*pid)));
    orochi.kill().unwrap();
    orochi.wait().unwrap();
    assert!(
        wait_until(10, || pids.iter().all(|pid| !running(*pid))),
        "agent processes survived a forced exit: {pids:?}"
    );
    let leases = w.dir.path().join("data/processes");
    assert!(wait_until(5, || std::fs::read_dir(&leases)
        .unwrap()
        .next()
        .is_none()));
}

#[cfg(unix)]
#[test]
fn leases_left_by_killed_supervisors_are_reaped_on_the_next_start() {
    let mut w = Workspace::new();
    let (mut orochi, pids, lease) = start_hanging_tree(&mut w);
    let supervisor = lease["supervisor"]["pid"].as_i64().unwrap() as i32;
    // Freeze the owner so it cannot clean up after its supervisor disappears. Stopping the
    // supervisor instead would make the kernel SIGHUP its orphaned process group.
    unsafe {
        libc::kill(orochi.id() as i32, libc::SIGSTOP);
        libc::kill(supervisor, libc::SIGKILL);
    }
    std::thread::sleep(Duration::from_millis(200));
    orochi.kill().unwrap();
    orochi.wait().unwrap();
    assert!(wait_until(5, || !running(supervisor)));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        pids.iter().all(|pid| running(*pid)),
        "orphans are expected before reaping"
    );
    let output = w.run(&["runs"]);
    success(&output);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Stopped 3 agent process(es)"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(wait_until(5, || pids.iter().all(|pid| !running(*pid))));
    assert!(
        std::fs::read_dir(w.dir.path().join("data/processes"))
            .unwrap()
            .next()
            .is_none()
    );
    let again = w.run(&["runs"]);
    assert!(!String::from_utf8_lossy(&again.stderr).contains("Stopped"));
}

fn adviser(w: &mut Workspace, id: &str, votes: &str) -> orochi::config::RouterConfig {
    let mut agent = fixture(
        id,
        "adviser",
        "judge-model",
        &w.dir.path().join(format!("{id}.jsonl")),
    );
    agent.provider = Provider::Anthropic;
    agent.routing_only = true;
    agent.env.insert("MOCK_VOTES".into(), votes.into());
    w.config.agents.push(agent);
    orochi::config::RouterConfig {
        agent: id.into(),
        model: Some("judge-model".into()),
        reasoning: None,
        timeout_secs: 10,
        max_resource_fraction: 1.0,
        max_output_tokens: 64,
        session_overhead_tokens: Some(1000),
        fallbacks: vec![],
    }
}

#[test]
fn acp_judge_replaces_an_exhausted_adviser_and_only_reorders_eligible_candidates() {
    let mut w = Workspace::new();
    w.config.agents[0]
        .env
        .insert("MOCK_MODELS".into(), "sol-test,sol-two".into());
    let mut judge = adviser(&mut w, "exhausted", "CREDIT");
    judge.fallbacks = vec![adviser(&mut w, "judge", "LAST")];
    w.config.frontier = Some(judge);
    // Very short requests are ambiguous enough to consult the frontier judge.
    let output = w.run(&["--no-eval", "Fix QZX9Q"]);
    success(&output);
    let runs = w.store().recent_runs(10).unwrap();
    let advice: Vec<_> = runs
        .iter()
        .rev()
        .filter(|r| r.purpose == "frontier_routing")
        .collect();
    assert_eq!(advice.len(), 2);
    assert_eq!(advice[0].candidate.agent, "exhausted");
    assert_eq!(advice[0].candidate.provider, Provider::Anthropic);
    assert_eq!(advice[0].error_kind.as_deref(), Some("rate_limit"));
    assert_eq!(advice[1].candidate.agent, "judge");
    assert_eq!(advice[1].error_kind, None);
    assert_eq!(advice[1].usage.total(), Some(900));
    let execution = runs.iter().find(|r| r.purpose == "execution").unwrap();
    assert_eq!(execution.candidate.agent, "test");
    assert!(
        execution
            .candidate
            .reasons
            .iter()
            .any(|r| r == "frontier_routing recommendation passed scheduler constraints")
    );
    // Equal-cost candidates are ordered by ID; the adviser picked the last one.
    let log = std::fs::read_to_string(w.dir.path().join("judge.jsonl")).unwrap();
    let prompt = log
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|r| r["method"] == "session/prompt")
        .unwrap()["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!prompt.contains("QZX9Q"));
    assert!(!prompt.contains(w.dir.path().to_str().unwrap()));
    let chosen = prompt.rsplit("\"id\":\"").next().unwrap()[..16].to_owned();
    assert_eq!(execution.candidate.id, chosen);
    // Advisers get no coordination tools; the executing session does.
    let new_session = |log: &str| -> serde_json::Value {
        std::fs::read_to_string(w.dir.path().join(log))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .find(|r| r["method"] == "session/new")
            .unwrap()
    };
    assert!(
        new_session("judge.jsonl")["params"]
            .get("mcpServers")
            .is_none_or(|s| s.as_array().unwrap().is_empty())
    );
    assert_eq!(
        new_session("agent.jsonl")["params"]["mcpServers"][0]["name"],
        "orochi-mailbox"
    );
    // The exhausted adviser's account is now cooling down for execution as well.
    let runtime = w.store().runtime().unwrap();
    assert!(!runtime[&("exhausted".into(), "*".into())].available(orochi::types::now()));
}

#[test]
fn routing_only_agents_are_never_discovered_or_selected_for_execution() {
    let mut w = Workspace::new();
    let judge = adviser(&mut w, "judge", "LAST");
    w.config.router = Some(judge);
    let output = w.run(&["--dry-run", "--json", "Implement a small endpoint"]);
    success(&output);
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        plan["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["agent"] == "test")
    );
    assert!(!w.dir.path().join("judge.jsonl").exists());
    let rejected = w.run(&["--agent", "judge", "Implement a small endpoint"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("routing-only"));
    let status = w.run(&["status", "--json"]);
    success(&status);
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    let listed = status["configured_agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent"] == "judge")
        .unwrap();
    assert_eq!(listed["routing_only"], true);
}

#[test]
fn config_policy_status_and_sessions_commands_are_usable() {
    let w = Workspace::new();
    for args in [
        &["status", "--json"][..],
        &["agents", "--json"],
        &["sessions", "--json"],
        &["policy", "status", "--json"],
        &["config", "show", "--json"],
    ] {
        let output = w.run(args);
        success(&output);
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    }
    success(&w.run(&["policy", "update"]));
}

#[test]
fn status_always_lists_current_agents_even_with_stale_runtime_history() {
    let mut w = Workspace::new();
    let mut disabled = w.config.agents[0].clone();
    disabled.id = "disabled_agent".into();
    disabled.enabled = false;
    let mut missing = w.config.agents[0].clone();
    missing.id = "missing_agent".into();
    missing.command = w.dir.path().join("missing-binary").display().to_string();
    let mut blocked = w.config.agents[0].clone();
    blocked.id = "blocked_agent".into();
    w.config.agents.extend([disabled, missing, blocked]);
    let store = w.store();
    for id in ["disabled_agent", "missing_agent"] {
        store.update_runtime(id, "*", |_| {}).unwrap();
    }
    store
        .update_runtime("blocked_agent", "*", |state| {
            state.status = orochi::types::RuntimeStatus::Cooldown;
            state.cooldown_until = Some(orochi::types::now() + 3600);
        })
        .unwrap();
    let output = w.run(&["status"]);
    success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    for (id, status) in [
        ("test", "ready"),
        ("disabled_agent", "disabled"),
        ("missing_agent", "not_installed"),
        ("blocked_agent", "cooldown"),
    ] {
        let line = text.lines().find(|line| line.starts_with(id)).unwrap();
        assert!(line.contains(status), "{line}");
    }
    assert!(text.contains("status --discover"));
    let output = w.run(&["status", "--json"]);
    success(&output);
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["configured_agents"].as_array().unwrap().len(), 4);
    assert_eq!(status["configured_agents"][0]["status"], "ready");
    assert_eq!(status["configured_agents"][0]["discovery_checked"], false);
    let shortcut = w.run(&["--status", "--json"]);
    success(&shortcut);
    assert_eq!(
        status,
        serde_json::from_slice::<serde_json::Value>(&shortcut.stdout).unwrap()
    );
    assert!(!w.run(&["--status", "Implement a task"]).status.success());
    assert!(!w.run(&["--status", "agents"]).status.success());
    assert!(w.log().is_empty(), "local status must never start agents");
}

#[test]
fn status_discover_lists_verified_models_and_connection_failures_without_prompting() {
    let mut w = Workspace::new();
    w.config.agents.push(fixture(
        "needs_login",
        "auth",
        "sol-test",
        &w.dir.path().join("auth.jsonl"),
    ));
    let output = w.run(&["status", "--discover", "--json"]);
    success(&output);
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let agents = status["configured_agents"].as_array().unwrap();
    let connected = agents.iter().find(|a| a["agent"] == "test").unwrap();
    assert_eq!(connected["status"], "connected");
    assert_eq!(
        connected["capabilities"]["models"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let failed = agents.iter().find(|a| a["agent"] == "needs_login").unwrap();
    assert_ne!(failed["status"], "ready");
    assert!(
        failed["connection_error"]
            .as_str()
            .unwrap()
            .contains("Authentication")
    );
    assert_eq!(status["discovery_failures"].as_array().unwrap().len(), 1);
    assert!(w.log().contains("session/new"));
    assert!(!w.log().contains("session/prompt"));
    assert!(!w.dir.path().join("repo/completed.txt").exists());
}

#[test]
fn acp_gateway_streams_forwards_permissions_and_cancels_without_stdout_noise() {
    for mode in ["success", "allow", "deny", "cancel", "disconnect"] {
        let mut w = Workspace::new();
        let behavior = match mode {
            "allow" | "deny" => "permission",
            "cancel" | "disconnect" => "hang",
            _ => "success",
        };
        w.config.agents[0]
            .env
            .insert("MOCK_BEHAVIOR".into(), behavior.into());
        w.config.scheduler.prompt_timeout_secs = 30;
        let _ = w.command(); // Persist the config used by this client.
        let output = Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gateway_client.py"))
            .args([mode, env!("CARGO_BIN_EXE_orochi")])
            .arg(w.dir.path().join("config.toml"))
            .arg(w.dir.path().join("data"))
            .arg(w.dir.path().join("repo"))
            .output()
            .unwrap();
        success(&output);
        if matches!(mode, "deny" | "cancel" | "disconnect") {
            assert!(!w.dir.path().join("repo/completed.txt").exists());
        }
        if matches!(mode, "cancel" | "disconnect") {
            let pid: i32 = std::fs::read_to_string(w.dir.path().join("repo/child.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            #[cfg(unix)]
            {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
                assert_ne!(
                    unsafe { libc::kill(pid, 0) },
                    0,
                    "gateway left an agent descendant running"
                );
            }
        }
    }
}

#[test]
fn direct_quota_command_is_persisted_and_exhaustion_blocks_execution() {
    use orochi::config::{QuotaProbe, QuotaProbeKind};
    let mut w = Workspace::new();
    w.config.quota.probes.push(QuotaProbe { agent:"test".into(),kind:QuotaProbeKind::Command,command:"python3".into(),args:vec!["-c".into(),
        "import json,time; print(json.dumps({'schema_version':1,'windows':[{'bucket':'account','remaining':0,'reset_at':int(time.time())+3600,'model':None,'affects_routing':True}]}))".into()] });
    let output = w.run(&["quota", "--refresh"]);
    success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["probes"][0]["status"], "fresh");
    let output = w.run(&["Implement a small endpoint"]);
    assert!(!output.status.success());
    assert!(!w.log().contains("session/prompt"));
    let snapshot = w.store().quota_snapshots().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].windows[0].remaining, 0.0);
}

#[test]
fn failed_quota_probe_preserves_last_snapshot_without_leaking_output() {
    use orochi::config::{QuotaProbe, QuotaProbeKind};
    let mut w = Workspace::new();
    let saved = orochi::quota_sources::Snapshot {
        agent: "test".into(),
        source: "test".into(),
        observed_at: orochi::types::now(),
        valid_until: orochi::types::now() + 300,
        windows: vec![],
    };
    w.store().save_quota_snapshot(&saved).unwrap();
    w.config.quota.probes.push(QuotaProbe {
        agent: "test".into(),
        kind: QuotaProbeKind::Command,
        command: "python3".into(),
        args: vec!["-c".into(), "import sys;print('SECRET');sys.exit(1)".into()],
    });
    let output = w.run(&["quota", "--refresh"]);
    success(&output);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["probes"][0]["status"], "unavailable");
    assert_eq!(w.store().quota_snapshots().unwrap()[0].source, "test");
}

#[test]
fn live_e2e_harness_distinguishes_missing_cli_from_verified_fixture_run() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let base = || {
        let mut c = Command::new("python3");
        c.arg(manifest.join("examples/live_e2e.py"))
            .args([
                "--agent",
                "gemini",
                "--binary",
                env!("CARGO_BIN_EXE_orochi"),
                "--output",
            ])
            .arg(directory.path())
            .env("MOCK_BEHAVIOR", "live_e2e");
        c
    };
    let output = base()
        .args(["--command", "/nonexistent/orochi-fixture"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["executed"], false);
    let output = base()
        .args(["--command", "python3", "--arg"])
        .arg(manifest.join("tests/fixtures/mock_acp.py"))
        .arg("--execute")
        .output()
        .unwrap();
    success(&output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "passed");
    assert_eq!(value["executed"], true);
}

#[test]
fn codex_quota_rpc_uses_only_initialize_and_read_without_prompting() {
    use orochi::config::{QuotaProbe, QuotaProbeKind};
    let mut w = Workspace::new();
    w.config.quota.probes.push(QuotaProbe {
        agent: "test".into(),
        kind: QuotaProbeKind::CodexAppServer,
        command: "python3".into(),
        args: vec![
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/quota_rpc.py")
                .to_string_lossy()
                .into_owned(),
        ],
    });
    let output = w.run(&["quota", "--refresh"]);
    success(&output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["probes"][0]["status"], "fresh");
    assert_eq!(w.store().quota_snapshots().unwrap()[0].windows.len(), 2);
    assert!(w.log().contains("account/rateLimits/read"));
    assert!(!w.log().contains("session/prompt"));
}

/// A conversation keeps its model only while that model would still be chosen: work that
/// outgrows it is routed again instead of being answered by whatever answered last time.
#[test]
fn a_conversation_that_outgrows_its_model_is_routed_again() {
    let mut w = Workspace::new();
    w.config.agents = vec![fixture(
        "test",
        "success",
        "luna-test,astra-test",
        &w.dir.path().join("agent.jsonl"),
    )];
    let output = chat(
        &w,
        "rename the helper to `parse`\n/solo rewrite the entire architecture of this project from scratch\n",
    );
    success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("picking again"), "{stderr}");
    let models: Vec<String> = w
        .log()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|r| r["method"] == "session/prompt" || r["method"] == "session/set_config_option")
        .filter_map(|r| {
            (r["params"]["configId"] == "engine")
                .then(|| r["params"]["value"].as_str().unwrap_or_default().to_owned())
        })
        .collect();
    // The small model answered the small request; the large one was picked for the large one.
    assert!(models.contains(&"astra-test".to_owned()), "{models:?}");
    let sessions = w.store().sessions(None).unwrap();
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    assert!(
        sessions.iter().any(|s| s.model == "astra-test"),
        "{sessions:?}"
    );
    assert!(
        sessions.iter().any(|s| s.model == "luna-test"),
        "{sessions:?}"
    );
}

/// The pinned input and the transcript share one screen: only a terminal model can tell whether
/// a turn flowed on or was drawn over the last one. Driven in a pty by `tests/test_chat_terminal.py`.
#[test]
fn chat_turns_flow_on_screen_without_drawing_over_each_other() {
    let status = std::process::Command::new("python3")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "tests",
            "-p",
            "test_chat_terminal.py",
        ])
        .env("OROCHI_BIN", env!("CARGO_BIN_EXE_orochi"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
}

/// A fresh session hears what is remembered; a resumed one already heard it when it began, and
/// is not told again. `/new` starts fresh, so it hears it once more.
#[test]
fn a_remembered_note_opens_each_fresh_session_once() {
    let w = Workspace::new();
    let store = w.store();
    let root = w.dir.path().join("repo").canonicalize().unwrap();
    let repository = store.repository_id(&root).unwrap();
    let note = w
        .dir
        .path()
        .join(format!("data/memory/repos/{repository}/MEMORY.md"));
    std::fs::create_dir_all(note.parent().unwrap()).unwrap();
    std::fs::write(&note, "- keep every diff small\n").unwrap();
    drop(store);

    let output = chat(&w, "First\nSecond\n/new\nThird\n");
    success(&output);
    let prompts: Vec<(String, String, String)> = session_requests(&w)
        .into_iter()
        .filter(|r| r.0 == "session/prompt")
        .collect();
    assert_eq!(prompts.len(), 3);
    let heard = |text: &str| text.contains("keep every diff small");
    assert!(heard(&prompts[0].2), "{}", prompts[0].2);
    assert!(!heard(&prompts[1].2), "a resumed session was told again");
    assert_eq!(prompts[1].1, prompts[0].1);
    assert!(heard(&prompts[2].2), "a new conversation starts without it");
    // Standing notes come before the task and never pretend to be part of it.
    assert!(prompts[0].2.find("What Orochi remembers") < prompts[0].2.find("First"));
}

/// A turn nothing verified is labeled by what the user does next, and only then: carrying on
/// counts for its route, `/reroute` against it, `/new` and quitting say nothing. Each message
/// is typed after the previous answer is recorded, as a person would; one queued while the
/// agent was still working was written before the answer was seen and judges nothing.
#[test]
fn what_the_user_does_after_an_answer_is_recorded_as_a_weak_label() {
    let w = Workspace::new();
    let mut child = w
        .command()
        .arg("chat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    use std::io::Write;
    let store = w.store();
    let after = |runs: usize| {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline && store.recent_runs(10).unwrap().len() < runs {
            std::thread::sleep(Duration::from_millis(50));
        }
        // The turn loop ends right after its record; let it finish before the next line.
        std::thread::sleep(Duration::from_millis(400));
    };
    for (line, runs) in [
        ("First", 1),
        ("Second", 2),
        ("/reroute", 2),
        ("Third", 3),
        ("/new", 3),
        ("Fourth", 4),
    ] {
        stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
        after(runs);
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let mut runs = store.recent_runs(10).unwrap();
    runs.reverse();
    let signals: Vec<Option<String>> = runs
        .iter()
        .map(|r| r.feedback.as_ref().map(|f| f.signal.clone()))
        .collect();
    assert_eq!(
        signals,
        [
            Some("continued".into()),
            Some("rerouted".into()),
            None,
            None
        ],
        "{runs:#?}"
    );
    // The labels reach learning; the prediction each run froze is untouched.
    assert!(orochi::learning::labeled(&runs[0]) && orochi::learning::labeled(&runs[1]));
    assert!(runs.iter().all(|r| r.prediction.is_some()));
}

/// Stopping a turn is no verdict by itself: the user may only have forgotten to say something.
/// What they do next decides. Carrying on with the same agent labels nothing; asking for
/// another one labels the stopped turn against its route.
#[test]
fn an_interruption_is_judged_by_what_the_user_does_next() {
    use std::io::Write;
    let interrupted_then = |next: &str| {
        let mut w = Workspace::new();
        w.config.agents[0]
            .env
            .insert("MOCK_BEHAVIOR".into(), "hang".into());
        let mut child = w
            .command()
            .arg("chat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let store = w.store();
        let wait = |ready: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            while std::time::Instant::now() < deadline && !ready() {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(ready(), "{}", w.log());
        };
        let pid = w.dir.path().join("repo/child.pid");
        stdin.write_all(b"Implement endpoint\n").unwrap();
        wait(&|| pid.exists());
        std::fs::remove_file(&pid).unwrap();
        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        wait(&|| store.recent_runs(1).unwrap().len() == 1);
        // The turn loop ends right after its record; let it finish before the next line.
        std::thread::sleep(Duration::from_millis(400));
        stdin.write_all(format!("{next}\n").as_bytes()).unwrap();
        if !next.starts_with('/') {
            // A message starts a new turn, which also hangs; stop it too.
            wait(&|| pid.exists());
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
            wait(&|| store.recent_runs(2).unwrap().len() == 2);
        } else {
            std::thread::sleep(Duration::from_millis(400));
        }
        drop(stdin);
        let _ = child.wait();
        let mut runs = store.recent_runs(10).unwrap();
        runs.reverse();
        assert_eq!(runs[0].outcome, Outcome::Cancelled);
        runs
    };

    let carried_on = interrupted_then("Also add a test for it");
    assert_eq!(carried_on[0].feedback, None, "{carried_on:#?}");
    assert_eq!(orochi::learning::label(&carried_on[0]), None);

    let rerouted = interrupted_then("/reroute");
    assert_eq!(
        rerouted[0].feedback.as_ref().map(|f| f.signal.as_str()),
        Some("rerouted")
    );
    assert_eq!(orochi::learning::label(&rerouted[0]), Some((false, 0.3)));
}
