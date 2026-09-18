use orochi::{
    collaboration::{Participant, Plan, Role},
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
    }
}
#[test]
fn duplicate_agent_and_model_are_valid_but_duplicate_participant_is_not() {
    let config = Config {
        agents: vec![AgentConfig::preset(
            "same",
            Provider::Openai,
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
        let mut primary = AgentConfig::preset("primary", Provider::Anthropic, "python3", &[]);
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
        let mut backup = AgentConfig::preset("backup", Provider::Openai, "python3", &[]);
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
    for name in ["session-plan.json", "session-plan-parallel.json"] {
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
