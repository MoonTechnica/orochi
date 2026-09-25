#![cfg(unix)]
use orochi::{
    config::{AgentConfig, Config, DiscoveryConfig},
    discovery,
    types::Provider,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
fn native(dir: &Path, provider: Provider, name: &str, adapter: &str) -> AgentConfig {
    executable(&dir.join(name), "#!/bin/sh\nexit 0\n");
    let mut agent = AgentConfig::preset(name, provider, adapter, &[]);
    agent.env.insert("PATH".into(), dir.display().to_string());
    agent
}

#[test]
fn native_clis_are_installed_even_without_adapters_or_npm() {
    let dir = tempfile::tempdir().unwrap();
    for (provider, name, adapter) in [
        (Provider::OPENAI, "codex", "codex-acp"),
        (Provider::ANTHROPIC, "claude", "claude-agent-acp"),
    ] {
        let agent = native(dir.path(), provider, name, adapter);
        let state = discovery::inspect(&agent, &DiscoveryConfig::default(), dir.path());
        assert!(state.installed);
        assert_eq!(state.status, "setup_unavailable");
        assert_eq!(state.native_cli, Some(dir.path().join(name)));
        assert!(state.detail.unwrap().contains("npm"));
    }
}

#[test]
fn explicit_agent_settings_and_disables_survive_automatic_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[[agents]]\nid='claude'\nprovider='anthropic'\ncommand='/custom/bridge'\nenabled=false\n",
    )
    .unwrap();
    let config = Config::load(&path).unwrap();
    assert_eq!(config.agents.len(), 4);
    assert_eq!(config.agents.iter().filter(|a| a.id == "claude").count(), 1);
    assert!(!config.agents[0].enabled);
    assert_eq!(config.agents[0].command, "/custom/bridge");
    std::fs::write(&path, "[discovery]\nauto_add=false\n[[agents]]\nid='only'\nprovider='openai'\ncommand='/custom/bridge'\n").unwrap();
    assert_eq!(Config::load(&path).unwrap().agents.len(), 1);
}

#[test]
fn discovery_distinguishes_disabled_missing_native_and_direct_acp() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = native(
        dir.path(),
        Provider::ANTHROPIC,
        "claude",
        "claude-agent-acp",
    );
    let options = DiscoveryConfig {
        auto_install: false,
        ..DiscoveryConfig::default()
    };
    assert_eq!(
        discovery::inspect(&agent, &options, dir.path()).status,
        "setup_disabled"
    );
    agent.enabled = false;
    assert_eq!(
        discovery::inspect(&agent, &options, dir.path()).status,
        "disabled"
    );
    agent.enabled = true;
    agent.command = dir.path().join("custom-acp").display().to_string();
    assert!(!discovery::inspect(&agent, &options, dir.path()).installed);
    executable(Path::new(&agent.command), "#!/bin/sh\nexit 0\n");
    assert_eq!(
        discovery::inspect(&agent, &options, dir.path()).status,
        "ready"
    );
    agent.command = "gemini".into();
    agent.args = vec!["--acp".into()];
    agent.provider = Provider::GOOGLE;
    executable(&dir.path().join("gemini"), "#!/bin/sh\nexit 0\n");
    let state = discovery::inspect(&agent, &options, dir.path());
    assert_eq!(state.status, "ready");
    assert!(state.adapter_package.is_none());
}

fn setup_installer(dir: &Path, fails: bool) -> (AgentConfig, PathBuf) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let mut agent = native(&bin, Provider::ANTHROPIC, "claude", "claude-agent-acp");
    executable(&bin.join("node"), "#!/bin/sh\nexit 0\n");
    let python = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = String::from_utf8(python.stdout).unwrap().trim().to_string();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let log = dir.join("install.jsonl");
    // A real installer subprocess creates a real ACP subprocess, without network.
    executable(
        &bin.join("npm"),
        &format!(
            r#"#!{python}
import json, pathlib, sys
with open({log}, 'a') as log: log.write(json.dumps(sys.argv[1:]) + '\n')
if {fails}: sys.stderr.write('fixture installation failed'); sys.exit(1)
prefix = pathlib.Path(sys.argv[sys.argv.index('--prefix') + 1])
target = prefix / 'node_modules/.bin/claude-agent-acp'
target.parent.mkdir(parents=True)
target.write_text('#!{python}\n' + pathlib.Path({fixture}).read_text())
target.chmod(0o755)
"#,
            log = serde_json::to_string(&log).unwrap(),
            fixture = serde_json::to_string(&fixture).unwrap(),
            fails = if fails { "True" } else { "False" }
        ),
    );
    agent
        .env
        .insert("MOCK_MODELS".into(), "claude-sonnet-test".into());
    (agent, log)
}

#[tokio::test]
async fn missing_adapter_is_prepared_once_and_binds_the_installed_cli() {
    let dir = tempfile::tempdir().unwrap();
    let (agent, log) = setup_installer(dir.path(), false);
    let options = DiscoveryConfig::default();
    assert_eq!(
        discovery::inspect(&agent, &options, dir.path()).status,
        "adapter_required"
    );
    let (a, b) = tokio::join!(
        discovery::prepare(&agent, &options, dir.path()),
        discovery::prepare(&agent, &options, dir.path())
    );
    let launch = a.unwrap();
    assert_eq!(launch.command, b.unwrap().command);
    assert_eq!(
        launch.env["CLAUDE_CODE_EXECUTABLE"],
        dir.path().join("bin/claude").display().to_string()
    );
    let installs = std::fs::read_to_string(log).unwrap();
    assert_eq!(installs.lines().count(), 1);
    assert!(installs.contains("@agentclientprotocol/claude-agent-acp@0.77.0"));
    assert!(installs.contains("--ignore-scripts"));
    assert!(installs.contains("--registry=https://registry.npmjs.org"));
    assert_eq!(
        discovery::inspect(&agent, &options, dir.path()).status,
        "ready"
    );
}

#[tokio::test]
async fn failed_setup_preserves_native_detection_and_is_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let (agent, _) = setup_installer(dir.path(), true);
    let error = discovery::prepare(&agent, &DiscoveryConfig::default(), dir.path())
        .await
        .unwrap_err();
    assert!(error.message.contains("fixture installation failed"));
    let state = discovery::inspect(&agent, &DiscoveryConfig::default(), dir.path());
    assert!(state.installed);
    assert_eq!(state.status, "adapter_required");
    let (agent, _) = setup_installer(dir.path(), false);
    assert!(
        discovery::prepare(&agent, &DiscoveryConfig::default(), dir.path())
            .await
            .is_ok()
    );
}

#[test]
fn native_only_claude_reaches_routing_and_executes_without_global_adapter() {
    let dir = tempfile::tempdir().unwrap();
    let (agent, log) = setup_installer(dir.path(), false);
    let mut config = Config::default();
    config.discovery.auto_add = false;
    config.agents = vec![agent];
    config.evaluator.auto = false;
    let path = dir.path().join("config.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_orochi"))
            .arg("--config")
            .arg(&path)
            .arg("--data-dir")
            .arg(dir.path().join("data"))
            .arg("--cwd")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap()
    };
    let inventory = run(&["agents", "--json"]);
    assert!(!log.exists(), "read-only inventory must not install");
    let inventory: serde_json::Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert_eq!(inventory[0]["installed"], true);
    assert_eq!(inventory[0]["availability"]["status"], "adapter_required");
    let plan = run(&["--dry-run", "--json", "Implement a small endpoint"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["candidates"][0]["agent"], "claude");
    assert!(!repo.join("completed.txt").exists());
    let execution = run(&["Implement a small endpoint"]);
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("completed.txt")).unwrap(),
        "mock success\n"
    );
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 1);
}
