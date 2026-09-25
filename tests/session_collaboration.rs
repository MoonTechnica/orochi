use orochi::{
    collaboration::{Participant, Plan, Role, graph::Part},
    config::{AgentConfig, Config},
    types::Provider,
};
use serde_json::json;
use std::{fs, process::Command};

fn plan() -> Plan {
    Plan {
        participants: [
            ("author", Role::Implementer),
            ("reviewer", Role::Reviewer),
            ("integrator", Role::Integrator),
        ]
        .into_iter()
        .map(|(id, role)| Participant {
            id: id.into(),
            role,
            agent: "same".into(),
            model: Some("sol-test".into()),
            reasoning: None,
            mode: None,
            fallback: true,
            allowed_agents: vec![],
            paths: vec![],
        })
        .collect(),
        discussion: None,
        parts: vec![],
    }
}
fn part(id: &str, paths: &[&str], after: &[&str]) -> Part {
    Part {
        id: id.into(),
        brief: format!("build {id}"),
        paths: paths.iter().map(|p| (*p).into()).collect(),
        after: after.iter().map(|p| (*p).into()).collect(),
    }
}
#[test]
fn duplicate_agent_and_model_are_valid_but_duplicate_participant_is_not() {
    let config = Config {
        agents: vec![AgentConfig::preset(
            "same",
            Provider::OPENAI,
            "python3",
            &[],
        )],
        ..Config::default()
    };
    let mut plan = plan();
    plan.validate(&config).unwrap();
    plan.participants[1].id = "author".into();
    assert!(plan.validate(&config).is_err());
    plan.participants[1].id = "AUTHOR".into();
    assert!(plan.validate(&config).is_err());
    plan.participants[1].id = "BASELINE".into();
    assert!(plan.validate(&config).is_err());
}
#[test]
fn workspace_copy_excludes_environment_files_and_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir_all(source.join(".orochi")).unwrap();
    for name in [
        ".env",
        ".env.local",
        "nested/.env.production",
        ".orochi/report.json",
    ] {
        fs::write(source.join(name), "excluded").unwrap();
    }
    fs::write(source.join("nested/app.py"), "included").unwrap();
    let copy = temp.path().join("copy");
    orochi::collaboration::copy_workspace(&source, &copy).unwrap();
    assert_eq!(
        fs::read_to_string(copy.join("nested/app.py")).unwrap(),
        "included"
    );
    for name in [".env", ".env.local", "nested/.env.production", ".orochi"] {
        assert!(!copy.join(name).exists());
    }
}
#[test]
fn independent_sessions_review_and_integrate_without_leaking_review_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("seed.txt"), "original").unwrap();
    let config = temp.path().join("config.toml");
    let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
    let log = temp.path().join("requests.jsonl");
    fs::write(&config, format!(r#"
[discovery]
auto_add = false
[scheduler]
permission = "allow"
[evaluator]
auto = false
[[evaluator.checks]]
name = "result"
command = "python3"
args = ["-c", "from pathlib import Path; assert Path('completed.txt').read_text() in ('implementation', 'integrated')"]
[[agents]]
id = "same"
provider = "openai"
command = "python3"
args = [{}]
[agents.env]
MOCK_BEHAVIOR = "session_collaboration"
MOCK_LOG = {}
"#, json!(fixture), json!(log.to_str().unwrap()))).unwrap();
    let plan_path = temp.path().join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan()).unwrap()).unwrap();
    let output = temp.path().join("result");
    let result = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            temp.path().join("data").to_str().unwrap(),
            "-C",
            root.to_str().unwrap(),
            "collaborate",
            "Implement the task",
            "--plan",
            plan_path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let sessions = report["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 3);
    let ids: std::collections::BTreeSet<_> = sessions
        .iter()
        .map(|s| s["session_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(
        sessions
            .iter()
            .all(|s| s["candidate"]["agent"] == "same" && s["candidate"]["model"] == "sol-test")
    );
    assert_eq!(report["outcome"], "success");
    assert_eq!(
        fs::read_to_string(output.join("integrator/completed.txt")).unwrap(),
        "integrated"
    );
    assert_eq!(
        fs::read_to_string(output.join("reviewer/completed.txt")).unwrap(),
        "review edit must not leak"
    );
    assert!(!root.join("completed.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("seed.txt")).unwrap(),
        "original"
    );
}
#[test]
fn workspace_copy_rejects_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", source.join("external")).unwrap();
        assert!(orochi::collaboration::copy_workspace(&source, &temp.path().join("copy")).is_err());
    }
}

struct Scenario {
    temp: tempfile::TempDir,
    config: Config,
    plan: Plan,
}
impl Scenario {
    fn new(role: &str, failure: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("repo")).unwrap();
        let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
        let mut primary = AgentConfig::preset("primary", Provider::ANTHROPIC, "python3", &[]);
        primary.args = vec![fixture.clone()];
        primary
            .env
            .insert("MOCK_MODELS".into(), "haiku,sonnet".into());
        primary
            .env
            .insert("MOCK_BEHAVIOR".into(), "session_collaboration".into());
        primary.env.insert("MOCK_FAIL_ROLE".into(), role.into());
        primary
            .env
            .insert("MOCK_FAILURE_KIND".into(), failure.into());
        primary.env.insert(
            "MOCK_LOG".into(),
            temp.path().join("primary.jsonl").display().to_string(),
        );
        let mut backup = AgentConfig::preset("backup", Provider::OPENAI, "python3", &[]);
        backup.args = vec![fixture];
        backup.env.insert("MOCK_MODELS".into(), "sol-test".into());
        backup
            .env
            .insert("MOCK_BEHAVIOR".into(), "session_collaboration".into());
        backup.env.insert(
            "MOCK_LOG".into(),
            temp.path().join("backup.jsonl").display().to_string(),
        );
        let mut config = Config::default();
        config.discovery.auto_add = false;
        config.scheduler.permission = orochi::config::PermissionMode::Allow;
        config.scheduler.discovery_timeout_secs = 5;
        config.scheduler.prompt_timeout_secs = 5;
        config.evaluator.auto = false;
        config.evaluator.checks = vec![orochi::config::CheckCommand {
            name: "artifact".into(), command: "python3".into(), args: vec!["-c".into(), "from pathlib import Path; assert Path('completed.txt').read_text() in ('implementation', 'integrated')".into()]
        }];
        config.agents = vec![primary, backup];
        let mut plan = plan();
        for p in &mut plan.participants {
            p.agent = "primary".into();
            p.model = Some("haiku".into());
        }
        let mut coordinator = plan.participants[0].clone();
        coordinator.id = "manager".into();
        coordinator.role = Role::Coordinator;
        plan.participants.insert(0, coordinator);
        Self { temp, config, plan }
    }
    fn output(&self) -> std::path::PathBuf {
        self.temp.path().join("output")
    }
    fn command(&self, resume: bool) -> Command {
        let config = self.temp.path().join("config.toml");
        fs::write(&config, toml::to_string(&self.config).unwrap()).unwrap();
        let plan = self.temp.path().join("plan.json");
        fs::write(&plan, serde_json::to_vec(&self.plan).unwrap()).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_orochi"));
        cmd.arg("--config")
            .arg(config)
            .arg("--data-dir")
            .arg(self.temp.path().join("data"))
            .arg("-C")
            .arg(self.temp.path().join("repo"));
        if resume {
            cmd.arg("collaborate-resume");
        } else {
            cmd.arg("collaborate")
                .arg("Implement a small function")
                .arg("--plan")
                .arg(plan);
        }
        cmd.arg("--output").arg(self.output());
        cmd
    }
    fn invoke(&self, resume: bool) -> std::process::Output {
        self.command(resume).output().unwrap()
    }
    fn invoke_with(&self, resume: bool, extra: &[&str]) -> std::process::Output {
        self.command(resume).args(extra).output().unwrap()
    }
    fn env(&mut self, agent: usize, pairs: &[(&str, &str)]) {
        for (key, value) in pairs {
            self.config.agents[agent]
                .env
                .insert((*key).into(), (*value).into());
        }
    }
    fn repo(&self) -> std::path::PathBuf {
        self.temp.path().join("repo")
    }
    fn report(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.output().join("report.json")).unwrap()).unwrap()
    }
    fn prompts(&self, agent: &str) -> Vec<serde_json::Value> {
        fs::read_to_string(self.temp.path().join(format!("{agent}.jsonl")))
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
            .filter(|r| r["method"] == "session/prompt")
            .collect()
    }
}

#[test]
fn every_role_replaces_an_exhausted_account_and_preserves_partial_state() {
    for role in ["coordinator", "implementer", "reviewer", "integrator"] {
        let mut s = Scenario::new(
            role,
            if role == "coordinator" {
                "credit"
            } else {
                "rate"
            },
        );
        if role != "coordinator" {
            // Exercise coordinator failover after an earlier successful planning turn.
            s.config.agents[1]
                .env
                .insert("MOCK_EXPECT_HANDOFF".into(), role.into());
        }
        let output = s.invoke(false);
        assert!(
            output.status.success(),
            "{role}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report = s.report();
        assert_eq!(report["status"], "completed");
        assert_eq!(report["next_stage"], 7);
        assert_eq!(report["outcome"], "success");
        assert!(report["management"]["plan"].is_array());
        let sessions = report["sessions"].as_array().unwrap();
        let failed = sessions.iter().find(|r| r["status"] == "failed").unwrap();
        assert_eq!(failed["candidate"]["agent"], "primary");
        assert_eq!(failed["error_kind"], "rate_limit");
        assert!(
            failed["response"]
                .as_str()
                .unwrap()
                .contains("CHECKPOINT_NOTE")
        );
        let replacement = sessions
            .iter()
            .find(|r| r["stage"] == failed["stage"] && r["status"] == "completed")
            .unwrap();
        assert_eq!(replacement["candidate"]["agent"], "backup");
        assert_eq!(replacement["candidate"]["model"], "sol-test");
        assert_ne!(failed["worker_id"], replacement["worker_id"]);
        assert_ne!(failed["session_id"], replacement["session_id"]);
        let after_failure = sessions
            .iter()
            .skip_while(|r| r["status"] != "failed")
            .skip(1);
        assert!(
            after_failure
                .into_iter()
                .all(|r| r["candidate"]["agent"] == "backup")
        );
        assert_eq!(
            fs::read_to_string(s.output().join("integrator/completed.txt")).unwrap(),
            "integrated"
        );
        assert!(!s.temp.path().join("repo/completed.txt").exists());
    }
}

#[test]
fn coordinator_keeps_prior_decisions_when_replaced_after_implementation() {
    let mut s = Scenario::new("coordinator", "rate");
    s.config.agents[0]
        .env
        .insert("MOCK_FAIL_STAGE".into(), "2".into());
    let result = s.invoke(false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let prompts = s.prompts("backup");
    let prompt = prompts[0]["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(prompt.contains("CHECKPOINT_NOTE"));
    assert!(prompt.contains("Check edge cases"));
    assert!(prompt.contains("Implementation ready"));
    assert!(prompt.contains("Verify result"));
    assert!(prompt.contains("stage 2."));
}

#[test]
fn exhausted_pool_resumes_the_same_stage_without_repeating_completed_work() {
    let mut s = Scenario::new("integrator", "rate");
    s.config.agents[1].enabled = false;
    assert!(!s.invoke(false).status.success());
    let blocked = s.report();
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["next_stage"], 5);
    assert!(s.output().join("integrator/partial.txt").is_file());
    let previous_prompts = s.prompts("primary").len();
    s.config.agents[1].enabled = true;
    s.config.agents[1]
        .env
        .insert("MOCK_EXPECT_HANDOFF".into(), "integrator".into());
    let result = s.invoke(true);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let resumed = s.report();
    assert_eq!(resumed["collaboration_id"], blocked["collaboration_id"]);
    assert_eq!(resumed["status"], "completed");
    assert_eq!(s.prompts("primary").len(), previous_prompts);
    let prompts = s.prompts("backup");
    assert_eq!(prompts.len(), 2); // integrator, then final coordinator assessment
    assert!(
        prompts[0]["params"]["prompt"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Role: Integrator")
    );
    let finished = s.invoke(true);
    assert!(finished.status.success());
    assert_eq!(s.prompts("backup").len(), 2);
}

#[test]
fn cancellation_and_explicit_pinning_never_launch_a_replacement() {
    for cancellation in [true, false] {
        let mut s = Scenario::new("coordinator", if cancellation { "cancel" } else { "rate" });
        if !cancellation {
            s.plan.participants[0].fallback = false;
        }
        let result = s.invoke(false);
        assert_eq!(
            result.status.code(),
            Some(if cancellation { 130 } else { 1 })
        );
        assert!(s.prompts("backup").is_empty());
        let report = s.report();
        assert_eq!(report["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            report["status"],
            if cancellation { "cancelled" } else { "blocked" }
        );
    }
}

#[test]
fn missing_preferred_agent_can_use_an_authenticated_replacement() {
    let mut s = Scenario::new("", "rate");
    s.config.agents[0].command = s.temp.path().join("not-installed").display().to_string();
    let result = s.invoke(false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = s.report();
    assert!(
        report["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["candidate"]["agent"] == "backup")
    );
    assert!(!report["discovery_failures"].as_array().unwrap().is_empty());
}

#[test]
fn model_scoped_limits_allow_a_different_model_in_the_same_agent() {
    let mut s = Scenario::new("coordinator", "model_rate");
    s.config.agents[0]
        .env
        .insert("MOCK_FAIL_MODEL".into(), "haiku".into());
    for p in &mut s.plan.participants {
        p.allowed_agents = vec!["primary".into()];
    }
    let result = s.invoke(false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = s.report();
    let sessions = report["sessions"].as_array().unwrap();
    assert_eq!(sessions[0]["candidate"]["model"], "haiku");
    assert_eq!(sessions[0]["status"], "failed");
    assert_eq!(sessions[1]["candidate"]["model"], "sonnet");
    assert_eq!(sessions[1]["candidate"]["agent"], "primary");
    assert!(s.prompts("backup").is_empty());
}

#[test]
fn replacement_respects_allowed_agents_and_attempt_budget() {
    for restrict_agents in [true, false] {
        let mut s = Scenario::new("coordinator", "rate");
        if restrict_agents {
            s.plan.participants[0].allowed_agents = vec!["primary".into()];
        } else {
            s.config.scheduler.max_attempts = 1;
        }
        let result = s.invoke(false);
        assert!(!result.status.success());
        assert_eq!(s.report()["status"], "blocked");
        assert!(s.prompts("backup").is_empty());
    }
}

#[test]
fn coordinator_accepts_a_final_state_after_cli_notices() {
    let mut s = Scenario::new("", "rate");
    s.config.agents[0]
        .env
        .insert("MOCK_COORDINATOR_PREFIX".into(), "1".into());
    let result = s.invoke(false);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = s.report();
    assert_eq!(report["management"]["decisions"][0], "Check edge cases");
    assert!(
        report["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "completed")
    );
    assert!(s.prompts("backup").is_empty());
}

#[test]
fn disconnect_and_timeout_can_replace_the_current_coordinator() {
    for kind in ["disconnect", "hang"] {
        let mut s = Scenario::new("coordinator", kind);
        s.config.scheduler.prompt_timeout_secs = 1;
        // There is only one primary model, so transport failures cannot just retry
        // the same broken adapter under a different model name.
        s.config.agents[0]
            .env
            .insert("MOCK_MODELS".into(), "haiku".into());
        let result = s.invoke(false);
        assert!(
            result.status.success(),
            "{kind}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report = s.report();
        assert!(
            report["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["status"] == "failed")
        );
        assert!(
            report["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["candidate"]["agent"] == "backup")
        );
    }
}

#[test]
fn resume_uses_an_exclusive_lock() {
    let mut s = Scenario::new("coordinator", "rate");
    s.config.agents[1].enabled = false;
    assert!(!s.invoke(false).status.success());
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(s.output().join(".run.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    s.config.agents[1].enabled = true;
    let result = s.invoke(true);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("another process"));
    assert!(s.prompts("backup").is_empty());
}

#[cfg(unix)]
#[test]
fn streaming_state_survives_ctrl_c_and_resume() {
    use std::time::{Duration, Instant};
    let mut s = Scenario::new("coordinator", "hang");
    s.config.scheduler.prompt_timeout_secs = 30;
    s.config.agents[0]
        .env
        .insert("MOCK_FAIL_STAGE".into(), "2".into());
    let mut child = s
        .command(false)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let ready = s.output().join("control/2/ready.pid");
    // Only how long to wait for the fixture to start, not a claim about speed: a loaded
    // machine can take far longer to launch a Python process than an idle one.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !ready.exists() {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !ready.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("fixture did not become ready");
    }
    // Wait for the streamed chunk to reach the durable checkpoint before signalling.
    while Instant::now() < deadline
        && !s.report()["sessions"].as_array().unwrap().last().unwrap()["response"]
            .as_str()
            .unwrap()
            .contains("CHECKPOINT_NOTE")
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cancel did not finish");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(130));
    let report = s.report();
    assert_eq!(report["status"], "cancelled");
    assert_eq!(report["next_stage"], 2);
    assert_eq!(report["sessions"][2]["status"], "interrupted");
    assert!(
        report["sessions"][2]["response"]
            .as_str()
            .unwrap()
            .contains("CHECKPOINT_NOTE")
    );
    assert_eq!(report["management"]["decisions"][0], "Check edge cases");
    assert!(s.prompts("backup").is_empty());
    s.config.agents[0].enabled = false;
    let resumed = s.invoke(true);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let prompts = s.prompts("backup");
    let first = prompts[0]["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(first.contains("stage 2."));
    assert!(first.contains("CHECKPOINT_NOTE"));
    assert!(first.contains("Check edge cases"));
}

#[cfg(unix)]
#[test]
fn forced_exit_stops_the_role_holder_and_resume_continues_the_stage() {
    use std::time::{Duration, Instant};
    let mut s = Scenario::new("coordinator", "hang");
    s.config.scheduler.prompt_timeout_secs = 60;
    s.config.agents[0]
        .env
        .insert("MOCK_FAIL_STAGE".into(), "2".into());
    let mut child = s
        .command(false)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let ready = s.output().join("control/2/ready.pid");
    // Only how long to wait for the fixture to start, not a claim about speed: a loaded
    // machine can take far longer to launch a Python process than an idle one.
    let deadline = Instant::now() + Duration::from_secs(60);
    // The fixture creates the file before it writes the pid into it: wait for a pid, not a file.
    let read_pid = || {
        fs::read_to_string(&ready)
            .ok()
            .and_then(|pid| pid.trim().parse::<i32>().ok())
    };
    while Instant::now() < deadline && read_pid().is_none() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let Some(agent) = read_pid() else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("fixture did not become ready");
    };
    while Instant::now() < deadline
        && !s.report()["sessions"].as_array().unwrap().last().unwrap()["response"]
            .as_str()
            .unwrap()
            .contains("CHECKPOINT_NOTE")
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && unsafe { libc::kill(agent, 0) } == 0 {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(
        unsafe { libc::kill(agent, 0) },
        0,
        "role holder survived a forced exit"
    );
    // The killed process could not checkpoint the in-flight attempt.
    let report = s.report();
    assert_eq!(report["status"], "running");
    assert_eq!(report["sessions"][2]["status"], "running");
    s.config.agents[0].enabled = false;
    let resumed = s.invoke(true);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["sessions"][2]["status"], "interrupted");
    assert_eq!(report["sessions"][3]["stage"], 2);
    assert_eq!(report["sessions"][3]["candidate"]["agent"], "backup");
    let first = s.prompts("backup")[0]["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.contains("stage 2."));
    assert!(first.contains("CHECKPOINT_NOTE"));
}

fn succeeded(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parallel_implementers_are_merged_and_the_integrator_must_clear_conflicts() {
    let mut s = Scenario::new("", "rate");
    fs::write(s.repo().join("shared.txt"), "base\n").unwrap();
    let meeting = s.temp.path().join("rendezvous");
    fs::create_dir(&meeting).unwrap();
    let meeting = meeting.display().to_string();
    let mut second = s.plan.participants[1].clone();
    s.plan.participants[1].id = "alice".into();
    s.plan.participants[1].paths = vec!["alice.txt".into()];
    second.id = "bob".into();
    second.paths = vec!["bob.txt".into()];
    s.plan.participants.insert(2, second);
    s.plan.participants[3].id = "checker".into();
    s.plan.participants[4].id = "merger".into();
    let shared = [
        ("MOCK_PARALLEL", "1"),
        ("MOCK_RENDEZVOUS", meeting.as_str()),
        ("MOCK_RENDEZVOUS_COUNT", "2"),
    ];
    s.env(0, &shared);
    s.env(0, &[("MOCK_KEEP_MARKERS", "1")]);
    s.env(1, &shared);
    let result = s.invoke(false);
    succeeded(&result);
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["outcome"], "success");
    assert_eq!(report["next_stage"], 8);
    let conflicts = report["merges"][0]["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["path"], "shared.txt");
    assert_eq!(conflicts[0]["markers"], true);
    let sessions = report["sessions"].as_array().unwrap();
    let authors: Vec<_> = sessions
        .iter()
        .filter(|r| r["role"] == "implementer")
        .collect();
    assert_eq!(authors.len(), 2);
    assert!(
        authors
            .iter()
            .all(|r| r["stage"] == 1 && r["status"] == "completed")
    );
    let merges: Vec<_> = sessions
        .iter()
        .filter(|r| r["participant"] == "merger")
        .collect();
    let rejected = merges.iter().find(|r| r["status"] == "failed").unwrap();
    assert!(
        rejected["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "git_conflict_markers" && c["passed"] == false)
    );
    let accepted = merges.iter().find(|r| r["status"] == "completed").unwrap();
    assert_eq!(accepted["candidate"]["agent"], "backup");
    let integrated = s.output().join("merger");
    assert_eq!(
        fs::read_to_string(integrated.join("shared.txt")).unwrap(),
        "alice+bob\n"
    );
    assert!(integrated.join("alice.txt").is_file() && integrated.join("bob.txt").is_file());
    assert!(!s.repo().join("alice.txt").exists());
    assert_eq!(
        fs::read_to_string(s.repo().join("shared.txt")).unwrap(),
        "base\n"
    );
}

#[test]
fn participants_exchange_addressed_messages_over_multiple_rounds() {
    let mut s = Scenario::new("", "rate");
    s.plan.participants.remove(0);
    s.plan.discussion = Some(orochi::collaboration::Discussion { max_rounds: 3 });
    s.env(0, &[("MOCK_DISCUSSION", "1")]);
    let result = s.invoke(false);
    succeeded(&result);
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["next_stage"], 6);
    let turns: Vec<_> = report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["participant"].as_str().unwrap().to_owned(),
                r["stage"].as_u64().unwrap(),
                r["turn"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let expected = [
        ("author", 0, "scheduled"),
        ("reviewer", 1, "scheduled"),
        ("author", 2, "discussion"),
        ("reviewer", 3, "discussion"),
        ("integrator", 5, "scheduled"),
    ]
    .map(|(p, stage, kind)| (p.to_owned(), stage, kind.to_owned()));
    assert_eq!(turns, expected);
    let messages = report["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["to"], "author");
    assert!(messages[0]["rejected"].is_null());
    assert_eq!(messages[1]["rejected"], "unknown recipient");
    assert_eq!(messages[2]["from"], "author");
    assert_eq!(messages[2]["to"], "reviewer");
    assert!(s.output().join("integrator/validation.txt").is_file());
    assert!(!s.repo().join("validation.txt").exists());
}

#[test]
fn verified_results_are_applied_and_conflicts_with_user_edits_are_resolved() {
    let mut s = Scenario::new("", "rate");
    s.plan.participants.remove(0);
    let repo = s.repo();
    fs::write(repo.join("seed.txt"), "line1\nline2\nline3\n").unwrap();
    fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    fs::write(repo.join("user.txt"), "untouched\n").unwrap();
    let edit = repo.join("seed.txt").display().to_string();
    s.env(
        0,
        &[
            ("MOCK_COLLAB_EDIT", "line1\ncollab2\nline3\n"),
            ("MOCK_USER_EDIT", edit.as_str()),
            ("MOCK_USER_EDIT_CONTENT", "line1\nuser2\nline3\n"),
            ("MOCK_RESOLVED", "line1\nuser2 collab2\nline3\n"),
        ],
    );
    let result = s.invoke_with(false, &["--apply"]);
    succeeded(&result);
    let report = s.report();
    assert_eq!(report["application"]["status"], "applied");
    assert_eq!(report["application"]["conflicts"][0]["path"], "seed.txt");
    let files: Vec<_> = report["application"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert_eq!(files, ["completed.txt", "seed.txt"]);
    let resolution = report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["turn"] == "resolution")
        .unwrap();
    assert_eq!(resolution["stage"], 3);
    assert_eq!(resolution["status"], "completed");
    assert_eq!(
        fs::read_to_string(repo.join("seed.txt")).unwrap(),
        "line1\nuser2 collab2\nline3\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("completed.txt")).unwrap(),
        "integrated"
    );
    assert_eq!(fs::read_to_string(repo.join(".env")).unwrap(), "SECRET=1\n");
    assert_eq!(
        fs::read_to_string(repo.join("user.txt")).unwrap(),
        "untouched\n"
    );
}

#[test]
fn only_verified_results_reach_the_working_tree_and_can_be_applied_later() {
    let mut unverified = Scenario::new("", "rate");
    unverified.plan.participants.remove(0);
    unverified.config.evaluator.checks.clear();
    let result = unverified.invoke_with(false, &["--apply"]);
    assert_eq!(result.status.code(), Some(1));
    let report = unverified.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["outcome"], "partial_success");
    assert_eq!(report["application"]["status"], "skipped");
    assert!(!unverified.repo().join("completed.txt").exists());

    let mut later = Scenario::new("", "rate");
    later.plan.participants.remove(0);
    succeeded(&later.invoke(false));
    assert!(!later.repo().join("completed.txt").exists());
    let sessions = later.report()["sessions"].as_array().unwrap().len();
    succeeded(&later.invoke_with(true, &["--apply"]));
    let report = later.report();
    assert_eq!(report["application"]["status"], "applied");
    assert_eq!(report["sessions"].as_array().unwrap().len(), sessions);
    assert_eq!(
        fs::read_to_string(later.repo().join("completed.txt")).unwrap(),
        "integrated"
    );
}

#[test]
fn working_tree_edits_during_resolution_are_never_overwritten() {
    let mut s = Scenario::new("", "rate");
    s.plan.participants.remove(0);
    let repo = s.repo();
    fs::write(repo.join("seed.txt"), "line1\nline2\nline3\n").unwrap();
    let edit = repo.join("seed.txt").display().to_string();
    s.env(
        0,
        &[
            ("MOCK_COLLAB_EDIT", "line1\ncollab2\nline3\n"),
            ("MOCK_USER_EDIT", edit.as_str()),
            ("MOCK_USER_EDIT_CONTENT", "line1\nuser2\nline3\n"),
            ("MOCK_RESOLVED", "line1\nuser2 collab2\nline3\n"),
            ("MOCK_EDIT_DURING_RESOLUTION", edit.as_str()),
        ],
    );
    let blocked = s.invoke_with(false, &["--apply"]);
    assert_eq!(blocked.status.code(), Some(1));
    let report = s.report();
    assert_eq!(report["status"], "blocked");
    assert!(
        report["error"]
            .as_str()
            .unwrap()
            .contains("working tree changed")
    );
    assert_eq!(report["application"]["status"], "pending");
    assert_eq!(
        fs::read_to_string(repo.join("seed.txt")).unwrap(),
        "line1\nuser3\nline3\n"
    );
    assert!(!repo.join("completed.txt").exists());
    succeeded(&s.invoke(true));
    let report = s.report();
    assert_eq!(report["application"]["status"], "applied");
    assert_eq!(report["application"]["rounds"], 2);
    let resolutions = report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["turn"] == "resolution" && r["status"] == "completed")
        .count();
    assert_eq!(resolutions, 2);
    assert_eq!(
        fs::read_to_string(repo.join("completed.txt")).unwrap(),
        "integrated"
    );
}

#[test]
fn bundled_example_plans_validate_against_the_default_agents() {
    let config = Config::default();
    for name in [
        "session-plan.json",
        "session-plan-parallel.json",
        "session-plan-parts.json",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(name);
        let plan: Plan = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        plan.validate(&config)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// Every team the planner derives has to satisfy `Plan::validate`, for any task: it is the
/// only thing standing between a derived plan and a run that cannot start.
#[test]
fn a_derived_team_is_always_a_valid_plan() {
    use orochi::{collaboration::plan, router::profiler, types::Complexity};
    let config = Config::default();
    let dir = tempfile::tempdir().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for text in [
        "typo",
        "READMEの起動手順を更新してください",
        "Implement a health endpoint",
        "アーキテクチャを全面的に刷新する",
        "Split the entire monolith into services from scratch",
        "10人のエージェントで議論してから実装して",
        "2人で分担して",
        "エージェント同士で相談しながら大規模な移行を進めて",
    ] {
        for complexity in [
            Complexity::Simple,
            Complexity::Normal,
            Complexity::Complex,
            Complexity::Extreme,
        ] {
            let mut task = profiler::profile(text, dir.path());
            task.complexity = complexity;
            for files in [
                vec![],
                vec!["src/a.rs".into()],
                vec![
                    "src/a.rs".into(),
                    "tests/b.rs".into(),
                    "docs/c.md".into(),
                    "web/app/d.ts".into(),
                ],
            ] {
                task.candidate_files = files;
                let derived = plan::derive(&task, dir.path());
                derived.validate(&config).unwrap_or_else(|e| {
                    panic!("{text} / {complexity:?}: {e:#}");
                });
                // Nobody is asked to own a path unless the work was actually split.
                let implementers = derived
                    .participants
                    .iter()
                    .filter(|p| p.role == Role::Implementer)
                    .count();
                for participant in &derived.participants {
                    assert!(participant.paths.is_empty() || implementers > 1);
                }
                seen.insert(derived.summary());
            }
        }
    }
    // The planner must actually vary with the task, not emit one team for everything.
    assert!(seen.len() > 4, "{seen:?}");
}

/// Splitting the tree is only worth it when each implementer gets somewhere of its own.
#[test]
fn parallel_implementers_own_disjoint_areas_or_none_at_all() {
    use orochi::{collaboration::plan, router::profiler, types::Complexity};
    let dir = tempfile::tempdir().unwrap();
    let mut task = profiler::profile("rewrite the entire system from scratch", dir.path());
    task.complexity = Complexity::Extreme;
    task.candidate_files = vec![
        "src/one.rs".into(),
        "src/two.rs".into(),
        "web/app.ts".into(),
        "docs/guide.md".into(),
    ];
    let derived = plan::derive(&task, dir.path());
    let owned: Vec<&String> = derived
        .participants
        .iter()
        .flat_map(|p| p.paths.iter())
        .collect();
    let unique: std::collections::BTreeSet<_> = owned.iter().collect();
    assert_eq!(owned.len(), unique.len(), "{owned:?}");
    assert!(owned.contains(&&"src".to_string()), "{owned:?}");

    // One area cannot be split, however big the task is: a second implementer there would
    // only race the first one to the same files.
    task.candidate_files = vec!["src/one.rs".into(), "src/two.rs".into()];
    let narrow = plan::derive(&task, dir.path());
    assert_eq!(
        narrow
            .participants
            .iter()
            .filter(|p| p.role == Role::Implementer)
            .count(),
        1
    );
}

mod graph {
    use super::part;
    use orochi::collaboration::graph::{Part, waves};
    /// Each wave as the parts of each unit, `+`-joined when a chain was fused.
    fn shape(parts: &[Part], width: usize) -> Vec<Vec<String>> {
        waves(parts, width)
            .unwrap()
            .iter()
            .map(|wave| wave.iter().map(|unit| unit.parts.join("+")).collect())
            .collect()
    }

    /// A model proposes the graph; a graph that cannot be ordered is not trimmed into one
    /// that can — it is refused whole, so the plan falls back to the shape it had.
    #[test]
    fn a_work_graph_that_cannot_be_ordered_is_refused_whole() {
        let refused = [
            vec![part("a", &["x"], &["b"]), part("b", &["y"], &["a"])],
            vec![part("a", &["x"], &["a"])],
            vec![part("a", &["x"], &["nowhere"])],
            vec![part("a", &["x"], &[]), part("a", &["y"], &[])],
            vec![part("Upper", &["x"], &[])],
            vec![part("", &["x"], &[])],
            vec![part(&"a".repeat(33), &["x"], &[])],
            vec![part("a", &["../outside"], &[])],
            vec![part("a", &["/etc"], &[])],
            vec![],
            (0..13).map(|i| part(&format!("p{i}"), &[], &[])).collect(),
        ];
        for parts in refused {
            assert!(waves(&parts, 4).is_err(), "{parts:?}");
        }
        let mut long = part("a", &["x"], &[]);
        long.brief = "x".repeat(2049);
        assert!(waves(&[long.clone()], 4).is_err());
        long.brief = " ".into();
        assert!(waves(&[long], 4).is_err());
    }

    #[test]
    fn parts_with_separate_areas_and_no_order_share_a_wave() {
        let parts = [part("api", &["api"], &[]), part("web", &["web"], &[])];
        assert_eq!(shape(&parts, 4), [["api", "web"]]);
    }

    /// Serial unless shown independent: two parts that may write the same place — the same
    /// path, one inside the other, or a part that never said where it writes — run in the
    /// order they were listed, however they were declared.
    #[test]
    fn parts_that_may_write_the_same_place_never_share_a_wave() {
        let nested = [
            part("store", &["src"], &[]),
            part("schema", &["src/db"], &[]),
            part("web", &["web"], &[]),
            part("docs", &["docs/"], &[]),
            part("guide", &["docs"], &[]),
        ];
        for wave in waves(&nested, 4).unwrap() {
            for (i, a) in wave.iter().enumerate() {
                for b in &wave[i + 1..] {
                    let overlap = a.paths.iter().any(|x| {
                        b.paths.iter().any(|y| {
                            std::path::Path::new(x).starts_with(y)
                                || std::path::Path::new(y).starts_with(x)
                        })
                    });
                    assert!(!overlap, "{a:?} and {b:?} share a wave");
                }
            }
        }
        // Nobody knows what a part without paths writes, so it runs with nothing beside it.
        let anywhere = [
            part("api", &["api"], &[]),
            part("web", &["web"], &[]),
            part("everything", &[], &[]),
        ];
        assert_eq!(
            shape(&anywhere, 4),
            vec![vec!["api", "web"], vec!["everything"]]
        );
        // The same list always gives the same order.
        assert_eq!(shape(&nested, 4), shape(&nested, 4));
    }

    /// A straight line of parts gains nothing from separate sessions: one agent holding the
    /// whole context does a sequence better than three handing notes along.
    #[test]
    fn a_straight_chain_of_parts_runs_as_one_session() {
        let parts = [
            part("schema", &["db"], &[]),
            part("api", &["api"], &["schema"]),
            part("web", &["web"], &[]),
        ];
        let graph = waves(&parts, 4).unwrap();
        assert_eq!(graph.len(), 1);
        let fused = &graph[0][0];
        assert_eq!(fused.id, "schema");
        assert_eq!(fused.parts, ["schema", "api"]);
        assert_eq!(fused.paths, ["db", "api"]);
        let (schema, api) = (
            fused.brief.find("build schema").unwrap(),
            fused.brief.find("build api").unwrap(),
        );
        assert!(schema < api, "{}", fused.brief);
        assert_eq!(graph[0][1].parts, ["web"]);

        let line = [
            part("a", &["a"], &[]),
            part("b", &["b"], &["a"]),
            part("c", &["c"], &["b", "a"]),
        ];
        assert_eq!(shape(&line, 4), [["a+b+c"]]);
    }

    #[test]
    fn a_join_waits_for_every_branch_it_builds_on() {
        let diamond = [
            part("base", &["core"], &[]),
            part("left", &["left"], &["base"]),
            part("right", &["right"], &["base"]),
            part("join", &["app"], &["left", "right"]),
        ];
        assert_eq!(
            shape(&diamond, 4),
            vec![vec!["base"], vec!["left", "right"], vec!["join"]]
        );
    }

    #[test]
    fn a_wave_never_runs_wider_than_the_seats_that_hold_it() {
        let parts = [
            part("a", &["a"], &[]),
            part("b", &["b"], &[]),
            part("c", &["c"], &[]),
        ];
        assert_eq!(shape(&parts, 2), vec![vec!["a", "b"], vec!["c"]]);
        // One seat means one thing at a time, and that is one session, not three.
        assert_eq!(shape(&parts, 1), [["a+b+c"]]);
    }
}

/// The shape the work-graph scenarios share: two implementer seats, a reviewer and an
/// integrator, and three parts of which the third needs the other two.
fn graph_scenario() -> Scenario {
    let mut s = Scenario::new("", "rate");
    s.plan.participants.remove(0);
    let mut second = s.plan.participants[0].clone();
    second.id = "second".into();
    s.plan.participants.insert(1, second);
    s.plan.parts = vec![
        part("alpha", &["alpha"], &[]),
        part("beta", &["beta"], &[]),
        part("gamma", &["gamma"], &["alpha", "beta"]),
    ];
    let shared = [
        ("MOCK_PARTS", "1"),
        ("MOCK_PART_NEEDS", "gamma:alpha/beta"),
        ("MOCK_EXPECT_PARTS", "alpha,beta,gamma"),
    ];
    s.env(0, &shared);
    s.env(1, &shared);
    s
}
fn by_participant<'a>(report: &'a serde_json::Value, id: &str) -> Vec<&'a serde_json::Value> {
    report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["participant"] == id)
        .collect()
}

/// Parts with nothing between them run at the same time, each in its own copy; a part that
/// needs them starts only from what they merged into, and only the integrated result is
/// verified.
#[test]
fn a_later_wave_starts_from_what_the_earlier_wave_merged() {
    let mut s = graph_scenario();
    let meeting = s.temp.path().join("rendezvous");
    fs::create_dir(&meeting).unwrap();
    let meeting = meeting.display().to_string();
    let together = [
        ("MOCK_RENDEZVOUS", meeting.as_str()),
        ("MOCK_RENDEZVOUS_PARTS", "alpha,beta"),
    ];
    s.env(0, &together);
    s.env(1, &together);
    succeeded(&s.invoke(false));
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["outcome"], "success");
    // wave 0, its merge, wave 1, review, integration
    assert_eq!(report["next_stage"], 5);
    for (id, stage) in [("alpha", 0), ("beta", 0), ("gamma", 2)] {
        let sessions = by_participant(&report, id);
        assert_eq!(sessions.len(), 1, "{id}");
        assert_eq!(sessions[0]["stage"], stage, "{id}");
        assert_eq!(sessions[0]["role"], "implementer");
        assert_eq!(sessions[0]["status"], "completed");
    }
    // Nobody works as a seat: the seats only say who may hold a part.
    assert!(by_participant(&report, "author").is_empty());
    let merge = &report["merges"][0];
    assert_eq!(merge["stage"], 1);
    assert_eq!(merge["sources"], json!(["alpha", "beta"]));
    assert!(merge["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(merge["strays"], json!({"alpha": 0, "beta": 0}));
    assert!(report["elapsed_ms"].as_u64().is_some());
    let integrated = s.output().join("integrator");
    for id in ["alpha", "beta", "gamma"] {
        assert_eq!(
            fs::read_to_string(integrated.join(id).join("done.txt")).unwrap(),
            id
        );
        assert!(!s.repo().join(id).exists());
    }
    let prompts: Vec<String> = s
        .prompts("primary")
        .iter()
        .map(|r| {
            r["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let gamma = prompts
        .iter()
        .find(|p| p.contains("Your part: gamma"))
        .unwrap();
    assert!(gamma.contains("build gamma") && gamma.contains("build alpha"));
    assert!(gamma.contains("Paths you own: [\"gamma\"]"));
    // Only the integrated tree is expected to pass the project's checks.
    for id in ["alpha", "beta", "gamma"] {
        assert!(
            by_participant(&report, id)[0]["checks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn an_interrupted_wave_resumes_only_the_part_that_did_not_finish() {
    let mut s = graph_scenario();
    s.env(
        0,
        &[
            ("MOCK_FAIL_ROLE", "implementer"),
            ("MOCK_FAIL_PART", "beta"),
        ],
    );
    s.config.agents[1].enabled = false;
    assert!(!s.invoke(false).status.success());
    let blocked = s.report();
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["next_stage"], 0);
    assert_eq!(by_participant(&blocked, "alpha")[0]["status"], "completed");
    s.config.agents[1].enabled = true;
    succeeded(&s.invoke(true));
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["outcome"], "success");
    assert_eq!(by_participant(&report, "alpha").len(), 1);
    let beta = by_participant(&report, "beta");
    assert_eq!(beta.last().unwrap()["status"], "completed");
    assert_eq!(beta.last().unwrap()["candidate"]["agent"], "backup");
    assert!(s.prompts("backup").iter().all(|r| {
        !r["params"]["prompt"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Your part: alpha")
    }));
}

/// Declared write sets are guidance, not a fence. When two parts of one wave touch the same
/// file anyway, the conflict is resolved before anything builds on it — and a merge that
/// still carries markers stops the run rather than handing broken files to the next wave.
#[test]
fn a_conflict_between_parts_is_resolved_before_the_next_wave_builds_on_it() {
    let mut s = graph_scenario();
    fs::write(s.repo().join("shared.txt"), "base\n").unwrap();
    let meeting = s.temp.path().join("rendezvous");
    fs::create_dir(&meeting).unwrap();
    let meeting = meeting.display().to_string();
    let straying = [
        ("MOCK_RENDEZVOUS", meeting.as_str()),
        ("MOCK_RENDEZVOUS_PARTS", "alpha,beta"),
        ("MOCK_PART_SHARED", "1"),
    ];
    s.env(0, &straying);
    s.env(1, &straying);
    succeeded(&s.invoke(false));
    let report = s.report();
    assert_eq!(report["status"], "completed");
    let merge = &report["merges"][0];
    assert_eq!(merge["conflicts"][0]["path"], "shared.txt");
    assert_eq!(merge["strays"], json!({"alpha": 1, "beta": 1}));
    let resolution: Vec<_> = report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["turn"] == "resolution")
        .collect();
    assert_eq!(resolution.len(), 1);
    assert_eq!(resolution[0]["stage"], 1);
    assert_eq!(resolution[0]["participant"], "integrator");
    assert_eq!(by_participant(&report, "gamma")[0]["status"], "completed");
    assert_eq!(
        fs::read_to_string(s.output().join("integrator/shared.txt")).unwrap(),
        "alpha+beta\n"
    );

    let mut stuck = graph_scenario();
    fs::write(stuck.repo().join("shared.txt"), "base\n").unwrap();
    let meeting = stuck.temp.path().join("rendezvous");
    fs::create_dir(&meeting).unwrap();
    let meeting = meeting.display().to_string();
    for agent in 0..2 {
        stuck.env(
            agent,
            &[
                ("MOCK_RENDEZVOUS", meeting.as_str()),
                ("MOCK_RENDEZVOUS_PARTS", "alpha,beta"),
                ("MOCK_PART_SHARED", "1"),
                ("MOCK_KEEP_MARKERS", "1"),
            ],
        );
    }
    assert!(!stuck.invoke(false).status.success());
    let report = stuck.report();
    assert_eq!(report["status"], "blocked");
    assert_eq!(report["next_stage"], 1);
    assert!(by_participant(&report, "gamma").is_empty());
    let attempts: Vec<_> = report["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["turn"] == "resolution")
        .collect();
    assert!(!attempts.is_empty());
    for attempt in attempts {
        assert_eq!(attempt["status"], "failed");
        assert!(
            attempt["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["name"] == "git_conflict_markers" && c["passed"] == false),
            "{attempt}"
        );
    }
}

#[test]
fn a_plan_file_with_parts_is_checked_and_its_order_printed_on_a_dry_run() {
    let s = graph_scenario();
    let output = s.invoke_with(false, &["--dry-run"]);
    succeeded(&output);
    let printed: Plan = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed.parts, s.plan.parts);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Order: alpha ‖ beta → gamma"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut cyclic = graph_scenario();
    cyclic.plan.parts[0].after = vec!["gamma".into()];
    let output = cyclic.invoke_with(false, &["--dry-run"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cycle"));
    // A part may not take a name a participant already answers to.
    let mut clash = graph_scenario();
    clash.plan.parts[0].id = "reviewer".into();
    clash.plan.parts[2].after = vec!["reviewer".into(), "beta".into()];
    assert_eq!(
        clash.invoke_with(false, &["--dry-run"]).status.code(),
        Some(1)
    );
}

/// A coordinator and two implementer seats, the coordinator proposing `proposal` as parts on
/// its first turn and `later` on every turn after that.
fn proposing(proposal: serde_json::Value, later: serde_json::Value) -> Scenario {
    let mut s = Scenario::new("", "rate");
    let mut second = s.plan.participants[1].clone();
    second.id = "second".into();
    s.plan.participants.insert(2, second);
    let (proposal, later) = (proposal.to_string(), later.to_string());
    for agent in 0..2 {
        s.env(
            agent,
            &[
                ("MOCK_PARTS", "1"),
                ("MOCK_PART_NEEDS", "gamma:alpha/beta"),
                ("MOCK_COORDINATOR_PARTS", proposal.as_str()),
                ("MOCK_COORDINATOR_LATER_PARTS", later.as_str()),
            ],
        );
    }
    s
}
fn coordinator_prompts(s: &Scenario) -> Vec<String> {
    s.prompts("primary")
        .iter()
        .map(|r| {
            r["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .filter(|p| p.contains("Role: Coordinator."))
        .collect()
}

/// The first coordinator turn has read the repository, so it — not a count of directories —
/// says how the work divides. Its answer is taken once; nothing said later reshapes the graph.
#[test]
fn the_coordinator_divides_the_work_once_and_later_proposals_change_nothing() {
    let mut s = proposing(
        json!([
            {"id": "alpha", "brief": "build alpha", "paths": ["alpha"]},
            {"id": "beta", "brief": "build beta", "paths": ["beta"]},
            {"id": "gamma", "brief": "build gamma", "paths": ["gamma"], "after": ["alpha", "beta"]},
        ]),
        json!([{"id": "late", "brief": "start over", "paths": ["late"]}]),
    );
    for agent in 0..2 {
        s.env(agent, &[("MOCK_EXPECT_PARTS", "alpha,beta,gamma")]);
    }
    succeeded(&s.invoke(false));
    let report = s.report();
    assert_eq!(report["status"], "completed");
    assert_eq!(report["outcome"], "success");
    let parts: Vec<&str> = report["plan"]["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(parts, ["alpha", "beta", "gamma"]);
    // coordinator, wave 1, merge, coordinator, wave 2, coordinator, review, coordinator,
    // integration, coordinator
    assert_eq!(report["next_stage"], 10);
    assert_eq!(by_participant(&report, "gamma")[0]["stage"], 4);
    assert!(by_participant(&report, "late").is_empty());
    assert!(report["management"].get("parts").is_none());
    let prompts = coordinator_prompts(&s);
    assert!(prompts[0].contains("\"parts\"") && prompts[0].contains("Up to 2 parts"));
    assert!(prompts[1..].iter().all(|p| !p.contains("Up to 2 parts")));
}

/// A division Orochi cannot order, or one where nothing runs side by side, changes nothing:
/// the team keeps the shape it had. A single implementer seat is never asked for one.
#[test]
fn a_division_that_cannot_run_side_by_side_leaves_the_team_as_it_was() {
    for proposal in [
        json!([
            {"id": "alpha", "brief": "build alpha", "paths": ["alpha"], "after": ["beta"]},
            {"id": "beta", "brief": "build beta", "paths": ["beta"], "after": ["alpha"]},
        ]),
        json!([
            {"id": "alpha", "brief": "build alpha", "paths": ["alpha"]},
            {"id": "beta", "brief": "build beta", "paths": ["beta"], "after": ["alpha"]},
        ]),
        json!({"id": "not a list"}),
    ] {
        let s = proposing(proposal.clone(), json!([]));
        succeeded(&s.invoke(false));
        let report = s.report();
        assert_eq!(report["status"], "completed", "{proposal}");
        assert!(report["plan"].get("parts").is_none(), "{proposal}");
        assert_eq!(report["next_stage"], 8, "{proposal}");
        for seat in ["author", "second"] {
            assert_eq!(by_participant(&report, seat)[0]["stage"], 1, "{proposal}");
        }
    }
    let alone = Scenario::new("", "rate");
    succeeded(&alone.invoke(false));
    assert!(
        coordinator_prompts(&alone)
            .iter()
            .all(|p| !p.contains("\"parts\""))
    );
}

/// A design that divided the work needs no coordinator — the design was that turn — and one
/// implementer seat for each part that runs at the same time.
#[test]
fn a_divided_design_seats_one_implementer_for_each_part_that_runs_at_once() {
    use orochi::{collaboration::plan, router::profiler};
    let dir = tempfile::tempdir().unwrap();
    let task = profiler::profile("rewrite the entire architecture from scratch", dir.path());
    let parts = vec![
        part("alpha", &["alpha"], &[]),
        part("beta", &["beta"], &[]),
        part("gamma", &["gamma"], &["alpha", "beta"]),
    ];
    let team = plan::with_parts(&task, parts.clone()).unwrap();
    team.validate(&Config::default()).unwrap();
    let roles: Vec<Role> = team.participants.iter().map(|p| p.role).collect();
    assert_eq!(roles.iter().filter(|r| **r == Role::Implementer).count(), 2);
    assert!(!roles.contains(&Role::Coordinator));
    assert_eq!(roles.last(), Some(&Role::Integrator));
    assert!(team.discussion.is_none());
    assert_eq!(team.parts, parts);

    let wide: Vec<Part> = ["p0", "p1", "p2", "p3", "p4"]
        .iter()
        .map(|id| part(id, &[id], &[]))
        .collect();
    let team = plan::with_parts(&task, wide).unwrap();
    team.validate(&Config::default()).unwrap();
    assert_eq!(
        team.participants
            .iter()
            .filter(|p| p.role == Role::Implementer)
            .count(),
        4
    );
    // Nothing to run side by side, or nothing Orochi can order: no team.
    let chain = vec![part("a", &["a"], &[]), part("b", &["b"], &["a"])];
    assert!(plan::with_parts(&task, chain).is_none());
    let cycle = vec![part("a", &["a"], &["b"]), part("b", &["b"], &["a"])];
    assert!(plan::with_parts(&task, cycle).is_none());
}

/// The parts of one wave choose their routes one after another, and each sees what the ones
/// before it took: of two accounts that are otherwise equal, each gets one part.
#[test]
fn the_parts_of_one_wave_spread_over_accounts_that_are_otherwise_equal() {
    let mut s = graph_scenario();
    s.plan.parts.truncate(2);
    for participant in &mut s.plan.participants {
        participant.agent = String::new();
        participant.model = None;
    }
    let meeting = s.temp.path().join("rendezvous");
    fs::create_dir(&meeting).unwrap();
    let meeting = meeting.display().to_string();
    let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
    s.config.agents = ["twin-a", "twin-b"]
        .iter()
        .map(|id| {
            let mut twin = AgentConfig::preset(id, Provider::OPENAI, "python3", &[]);
            twin.args = vec![fixture.clone()];
            for (key, value) in [
                ("MOCK_BEHAVIOR", "session_collaboration"),
                ("MOCK_MODELS", "sol-test"),
                ("MOCK_PARTS", "1"),
                ("MOCK_EXPECT_PARTS", "alpha,beta"),
                ("MOCK_RENDEZVOUS", meeting.as_str()),
                ("MOCK_RENDEZVOUS_PARTS", "alpha,beta"),
            ] {
                twin.env.insert(key.into(), value.into());
            }
            twin
        })
        .collect();
    succeeded(&s.invoke(false));
    let report = s.report();
    let agents: std::collections::BTreeSet<&str> = ["alpha", "beta"]
        .iter()
        .map(|id| {
            by_participant(&report, id)[0]["candidate"]["agent"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(agents.len(), 2, "both parts ran on {agents:?}");
}

/// A collaboration is a thread whose seats are its participants, each on its own lane, so the
/// work of a team reads as one conversation rather than as a report file nobody opens.
#[test]
fn a_collaboration_records_its_participants_as_the_seats_of_one_turn() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("seed.txt"), "original").unwrap();
    let config = temp.path().join("config.toml");
    let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
    fs::write(
        &config,
        format!(
            r#"
[discovery]
auto_add = false
[scheduler]
permission = "allow"
[evaluator]
auto = false
[classifier]
enabled = false
[[agents]]
id = "same"
provider = "openai"
command = "python3"
args = [{}]
[agents.env]
MOCK_BEHAVIOR = "session_collaboration"
"#,
            json!(fixture)
        ),
    )
    .unwrap();
    let plan_path = temp.path().join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan()).unwrap()).unwrap();
    let output = temp.path().join("result");
    let result = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            temp.path().join("data").to_str().unwrap(),
            "-C",
            root.to_str().unwrap(),
            "collaborate",
            "Implement the task",
            "--plan",
            plan_path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let activity = orochi::activity::Activity::open(&temp.path().join("data"), 30).unwrap();
    let listed = activity.sidebar(20, true).unwrap();
    let row = listed
        .iter()
        .flat_map(|p| &p.threads)
        .find(|t| t.origin == "collaborate")
        .expect("a collaboration leaves a thread");
    let thread = activity.thread(&row.id).unwrap().unwrap();

    let roles: Vec<&str> = thread.seats.iter().map(|s| s.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["author", "reviewer", "integrator"],
        "each participant is a seat, named as the plan names it"
    );
    assert!(
        thread.seats.iter().all(|s| s.model.is_some()),
        "each seat records the route it took: {:?}",
        thread.seats
    );
    let lanes: Vec<Option<i64>> = thread
        .items
        .iter()
        .filter(|i| i.kind == "agent_message")
        .map(|i| i.lane)
        .collect();
    assert!(
        lanes.len() >= 3 && lanes.iter().all(Option::is_some),
        "every participant's reply is on its own lane: {lanes:?}"
    );
}
