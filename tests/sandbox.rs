//! Per-project sandboxes, driven through the real binary against `fixtures/fake_incus.py`,
//! which runs what Orochi `exec`s "inside" as a process on this machine.
use orochi::{
    config::{AgentConfig, CheckCommand, Config, McpServerConfig, SandboxClient},
    sandbox::{Mode, Project, State, Subnet, label, ops::parse_listening, shadow_ok},
    storage::Store,
    types::{Outcome, Provider},
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn fixture(log: &Path) -> AgentConfig {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
    let mut config = AgentConfig::preset("test", Provider::OPENAI, "python3", &[]);
    config.args = vec![script.to_string_lossy().into_owned()];
    config.env.insert("MOCK_BEHAVIOR".into(), "success".into());
    config.env.insert("MOCK_MODELS".into(), "sol-test".into());
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
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_incus.py");
        let incus = dir.path().join("incus");
        std::fs::write(
            &incus,
            format!(
                "#!/bin/sh\nFAKE_INCUS_STATE='{}' FAKE_INCUS_LOG='{}' exec python3 '{}' \"$@\"\n",
                dir.path().join("incus.json").display(),
                dir.path().join("incus.jsonl").display(),
                fake.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&incus, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut config = Config::default();
        config.discovery.auto_add = false;
        config.evaluator.auto = false;
        config.classifier.enabled = false;
        config.mcp.keychain = false;
        config.scheduler.discovery_timeout_secs = 5;
        config.scheduler.prompt_timeout_secs = 5;
        config.sandbox.client = SandboxClient::Incus;
        config.sandbox.incus = incus.to_string_lossy().into_owned();
        config.sandbox.shadow = vec!["node_modules".into()];
        config.agents = vec![fixture(&dir.path().join("agent.jsonl"))];
        Self { dir, config }
    }
    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo").canonicalize().unwrap()
    }
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }
    fn command(&self) -> Command {
        let config = self.dir.path().join("config.toml");
        std::fs::write(&config, toml::to_string(&self.config).unwrap()).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_orochi"));
        cmd.args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            self.data().to_str().unwrap(),
            "--cwd",
            self.repo().to_str().unwrap(),
        ]);
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    /// Every call Orochi made to Incus, as argument lists without `--project sbx`.
    fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.dir.path().join("incus.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let mut argv: Vec<String> = serde_json::from_str(l).unwrap();
                if argv.first().map(String::as_str) == Some("--project") {
                    argv.drain(..2);
                }
                argv
            })
            .collect()
    }
    fn execs(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter(|c| c.first().map(String::as_str) == Some("exec"))
            .collect()
    }
    fn incus(&self) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.dir.path().join("incus.json")).unwrap()).unwrap()
    }
    fn agent_log(&self) -> String {
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
fn create(w: &Workspace) {
    let output = w.run(&["sandbox", "create", "--ports", "3000"]);
    success(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("http://<port>-repo.localhost:1355"),
        "creating says where its services open, with nothing to set up here"
    );
}

#[test]
fn a_project_sandbox_mounts_the_tree_at_its_own_path_with_docker_a_bounded_disk_and_the_users_ids()
{
    let w = Workspace::new();
    create(&w);
    let repo = w.repo();
    let state = State::load(&w.data()).unwrap();
    let project = state.exact(&repo).unwrap();
    assert_eq!(project.name, "repo");
    assert_eq!(
        (project.mode, project.tier),
        (Mode::Container, Mode::Container)
    );
    assert_eq!(project.ports, vec![3000]);
    let incus = w.incus();
    let instance = &incus["instances"]["repo"];
    assert_eq!(instance["status"], "Running");
    assert_eq!(
        instance["profiles"],
        serde_json::json!(["sbx-base", "sbx-docker"])
    );
    let (uid, gid) = orochi::sandbox::ids();
    assert_eq!(
        instance["config"]["raw.idmap"],
        format!("uid {uid} {uid}\ngid {gid} {gid}")
    );
    let devices = &instance["devices"];
    assert_eq!(devices["work"]["source"], repo.to_str().unwrap());
    assert_eq!(devices["work"]["path"], repo.to_str().unwrap());
    assert_eq!(
        devices["shadow-0"]["path"],
        repo.join("node_modules").to_str().unwrap()
    );
    assert_eq!(devices["claude"]["path"], "/home/dev/.claude");
    // Docker's data stays on the root disk (a clone of the golden image), which is bounded.
    assert_eq!(devices["root"]["size"], "30GiB");
    assert!(devices.get("docker").is_none());
}

#[test]
fn a_sandboxed_run_starts_its_agent_inside_in_the_same_directory_and_without_this_machines_servers()
{
    let mut w = Workspace::new();
    w.config.mcp.servers = vec![McpServerConfig {
        name: "local-tool".into(),
        command: "/bin/echo".into(),
        ..Default::default()
    }];
    create(&w);
    let output = w.run(&["Implement a small endpoint"]);
    success(&output);
    let repo = w.repo();
    let agent = w
        .execs()
        .into_iter()
        .find(|c| c.contains(&"sbx-run".to_owned()))
        .expect("the agent was started inside");
    let at = |flag: &str| agent[agent.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(at("--cwd"), repo.to_str().unwrap());
    assert_eq!(at("--user"), orochi::sandbox::ids().0.to_string());
    let run = agent.iter().position(|a| a == "sbx-run").unwrap();
    assert_eq!(agent[run + 1], "python3");
    assert!(agent.contains(&"MOCK_BEHAVIOR=success".to_owned()));
    assert!(agent.contains(&"HOME=/home/dev".to_owned()));
    assert!(!agent.iter().any(|a| a.starts_with("PATH=")));
    // The session's cwd is the path the tree has on this machine, unchanged.
    let log = w.agent_log();
    assert!(
        log.contains(&format!("\"cwd\": \"{}\"", repo.display()))
            || log.contains(&format!("\"cwd\":\"{}\"", repo.display())),
        "{log}"
    );
    // Neither the mailbox nor a stdio server naming this machine's executables goes inside.
    assert!(!log.contains("orochi-mailbox"), "{log}");
    assert!(!log.contains("local-tool"), "{log}");
    let records = Store::open(&w.data()).unwrap().recent_runs(10).unwrap();
    assert_eq!(records.len(), 1);
}

#[test]
fn checks_of_a_sandboxed_run_execute_inside_the_sandbox() {
    let mut w = Workspace::new();
    w.config.evaluator.checks = vec![CheckCommand {
        name: "tests".into(),
        command: "python3".into(),
        args: vec![
            "-c".into(),
            "import os; assert os.environ['CI'] == 'true' and os.environ['OROCHI_SANDBOX'] == 'repo'".into(),
        ],
    }];
    create(&w);
    success(&w.run(&["Implement a small endpoint"]));
    let check = w
        .execs()
        .into_iter()
        .find(|c| c.iter().any(|a| a == "-c"))
        .expect("the check ran through incus exec");
    assert!(check.contains(&"CI=true".to_owned()));
    let records = Store::open(&w.data()).unwrap().recent_runs(10).unwrap();
    assert_eq!(records[0].outcome, Outcome::Success);
}

#[test]
fn switching_a_project_to_host_runs_it_here_again_and_keeps_the_instance() {
    let w = Workspace::new();
    create(&w);
    success(&w.run(&["sandbox", "mode", "host"]));
    assert_eq!(w.incus()["instances"]["repo"]["status"], "Stopped");
    let before = w.execs().len();
    success(&w.run(&["Implement a small endpoint"]));
    assert_eq!(w.execs().len(), before, "nothing ran inside");
    assert!(w.agent_log().contains("session/prompt"));
    // And back, without recreating anything.
    success(&w.run(&["sandbox", "mode", "container"]));
    assert!(!w.calls().iter().any(|c| c[0] == "delete"));
    success(&w.run(&["Implement a small endpoint"]));
    assert!(w.execs().len() > before);
}

#[test]
fn the_sandbox_flag_runs_one_run_here_without_changing_the_project() {
    let w = Workspace::new();
    create(&w);
    let before = w.execs().len();
    success(&w.run(&["--sandbox=host", "Implement a small endpoint"]));
    assert_eq!(w.execs().len(), before);
    assert_eq!(
        State::load(&w.data())
            .unwrap()
            .exact(&w.repo())
            .unwrap()
            .mode,
        Mode::Container
    );
}

#[test]
fn naming_a_sandbox_for_a_directory_without_one_is_refused_rather_than_run_here() {
    let w = Workspace::new();
    let output = w.run(&["--sandbox=container", "Implement a small endpoint"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("orochi sandbox create"));
    assert!(!w.agent_log().contains("session/prompt"));
}

#[test]
fn a_subdirectory_runs_in_its_projects_sandbox() {
    let w = Workspace::new();
    create(&w);
    std::fs::create_dir(w.repo().join("web")).unwrap();
    let config = w.dir.path().join("config.toml");
    std::fs::write(&config, toml::to_string(&w.config).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_orochi"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            w.data().to_str().unwrap(),
        ])
        .args([
            "--cwd",
            w.repo().join("web").to_str().unwrap(),
            "Implement a small endpoint",
        ])
        .output()
        .unwrap();
    success(&output);
    assert!(w.execs().iter().any(|c| c.contains(&"sbx-run".to_owned())));
}

#[test]
fn creating_without_an_image_says_how_to_build_one_and_records_nothing() {
    let w = Workspace::new();
    std::fs::write(
        w.dir.path().join("incus.json"),
        r#"{"images": [], "instances": {}, "volumes": {}}"#,
    )
    .unwrap();
    let output = w.run(&["sandbox", "create"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("image build"));
    assert!(State::load(&w.data()).unwrap().projects.is_empty());
}

#[test]
fn a_failed_create_leaves_no_instance_behind() {
    let w = Workspace::new();
    // Creating a volume fails half way through, after the instance exists.
    let output = w
        .command()
        .args(["sandbox", "create"])
        .env("FAKE_INCUS_FAIL", "storage")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(w.incus()["instances"], serde_json::json!({}));
    assert!(State::load(&w.data()).unwrap().projects.is_empty());
}

#[test]
fn removing_a_sandbox_drops_its_volumes_and_keeps_the_projects_files() {
    let w = Workspace::new();
    std::fs::write(w.repo().join("keep.txt"), "mine").unwrap();
    create(&w);
    success(&w.run(&["sandbox", "rm", "--yes"]));
    let incus = w.incus();
    assert_eq!(incus["instances"], serde_json::json!({}));
    assert!(incus["volumes"].get("sbx-pool/shadow-repo-0").is_none());
    assert_eq!(
        std::fs::read_to_string(w.repo().join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(State::load(&w.data()).unwrap().projects.is_empty());
}

#[test]
fn removing_without_a_terminal_needs_yes() {
    let w = Workspace::new();
    create(&w);
    let output = w.run(&["sandbox", "rm"]);
    assert!(!output.status.success());
    assert!(w.incus()["instances"].get("repo").is_some());
}

#[test]
fn focus_puts_listening_ports_at_loopback_for_one_project_and_unfocus_takes_them_away() {
    let w = Workspace::new();
    create(&w);
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = taken.local_addr().unwrap().port();
    let output = w
        .command()
        .args(["sandbox", "focus"])
        .env("FAKE_SS", format!("{port},{busy},22"))
        .output()
        .unwrap();
    success(&output);
    let devices = &w.incus()["instances"]["repo"]["devices"];
    let device = &devices[format!("sbx-port-{port}")];
    assert_eq!(device["type"], "proxy");
    assert_eq!(device["listen"], format!("tcp:127.0.0.1:{port}"));
    assert_eq!(device["connect"], format!("tcp:127.0.0.1:{port}"));
    // The configured port is offered too; a port held here is left alone, a system one ignored.
    assert!(
        devices.get("sbx-port-3000").is_some()
            || String::from_utf8_lossy(&output.stderr).contains(":3000")
    );
    assert!(devices.get(format!("sbx-port-{busy}")).is_none());
    assert!(devices.get("sbx-port-22").is_none());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("127.0.0.1:{busy}")));
    success(&w.run(&["sandbox", "unfocus"]));
    let devices = &w.incus()["instances"]["repo"]["devices"];
    assert!(
        !devices
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.starts_with("sbx-port-"))
    );
    assert!(State::load(&w.data()).unwrap().focus.is_none());
    drop(taken);
}

#[test]
fn snapshots_cover_the_instance_which_holds_its_docker_data() {
    let w = Workspace::new();
    create(&w);
    success(&w.run(&["sandbox", "snapshot", "--name", "before"]));
    success(&w.run(&["sandbox", "restore", "before"]));
    let calls = w.calls();
    assert!(
        calls
            .iter()
            .any(|c| c[..2] == ["snapshot", "create"] && c.contains(&"before".to_owned()))
    );
    assert!(calls.iter().any(|c| c[..2] == ["snapshot", "restore"]));
    assert_eq!(w.incus()["instances"]["repo"]["status"], "Running");
}

#[test]
fn a_sandbox_without_docker_gets_no_nesting_and_its_daemon_is_masked() {
    let w = Workspace::new();
    success(&w.run(&["sandbox", "create", "--no-docker"]));
    let incus = w.incus();
    assert_eq!(
        incus["instances"]["repo"]["profiles"],
        serde_json::json!(["sbx-base"])
    );
    assert!(
        w.execs()
            .iter()
            .any(|c| c.contains(&"mask".to_owned()) && c.contains(&"docker.socket".to_owned()))
    );
}

#[test]
fn a_second_project_with_the_same_directory_name_gets_its_own_name() {
    let w = Workspace::new();
    create(&w);
    let other = w.dir.path().join("elsewhere/repo");
    std::fs::create_dir_all(&other).unwrap();
    success(&w.run(&["sandbox", "create", other.to_str().unwrap()]));
    let state = State::load(&w.data()).unwrap();
    assert_eq!(
        state.exact(&other.canonicalize().unwrap()).unwrap().name,
        "repo-2"
    );
}

#[test]
fn names_labels_subnets_and_shadows_are_what_incus_and_dns_accept() {
    assert_eq!(label("My App_v2").as_deref(), Some("my-app-v2"));
    assert_eq!(label("2048").as_deref(), Some("p-2048"));
    assert_eq!(label("---"), None);
    assert_eq!(label(&"a".repeat(80)).unwrap().len(), 63);
    let subnet = Subnet::parse("10.203.0.1/24").unwrap();
    assert_eq!(subnet.cidr(), "10.203.0.0/24");
    assert!(Subnet::parse("10.203.0.0/24").is_none());
    assert!(Subnet::parse("10.203.0.1").is_none());
    assert!(shadow_ok("node_modules") && shadow_ok("apps/web/.next"));
    assert!(!shadow_ok("../x") && !shadow_ok("/abs") && !shadow_ok(""));
    assert_eq!(
        parse_listening(
            "LISTEN 0 4096 0.0.0.0:54321 0.0.0.0:*\nLISTEN 0 4096 [::]:54321 [::]:*\nLISTEN 0 128 127.0.0.1:22 0.0.0.0:*\n"
        ),
        vec![54321]
    );
}

#[test]
fn a_directory_belongs_to_the_nearest_registered_project() {
    let mut state = State::default();
    for (name, root) in [("outer", "/w/app"), ("inner", "/w/app/packages/api")] {
        state.projects.push(Project {
            name: name.into(),
            root: root.into(),
            ..Project::default()
        });
    }
    assert_eq!(
        state
            .project_for(Path::new("/w/app/packages/api/src"))
            .unwrap()
            .name,
        "inner"
    );
    assert_eq!(
        state.project_for(Path::new("/w/app/web")).unwrap().name,
        "outer"
    );
    assert!(state.project_for(Path::new("/w/application")).is_none());
}

#[test]
fn an_invalid_sandbox_configuration_is_refused() {
    let mut config = Config::default();
    config.sandbox.subnet = "10.0.0.0/24".into();
    assert!(config.validate().is_err());
    let mut config = Config::default();
    config.sandbox.shadow = vec!["../escape".into()];
    assert!(config.validate().is_err());
    let mut config = Config::default();
    config.sandbox.project = "Not A Label".into();
    assert!(config.validate().is_err());
    assert!(Config::default().validate().is_ok());
}

#[test]
fn an_operation_started_without_a_terminal_runs_the_same_command_and_keeps_its_output() {
    use orochi::sandbox::jobs::{Request, list, start};
    let w = Workspace::new();
    create(&w);
    let config = w.dir.path().join("config.toml");
    let binary = Path::new(env!("CARGO_BIN_EXE_orochi"));
    let id = start(
        binary,
        &config,
        &w.data(),
        Request::Snapshot { root: w.repo() },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let job = loop {
        let job = list(&w.data(), 8)
            .unwrap()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        if job.exit.is_some() || std::time::Instant::now() > deadline {
            break job;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(job.exit, Some(0), "{}", job.tail);
    assert!(
        job.tail.starts_with('s'),
        "the snapshot's name is what it printed: {}",
        job.tail
    );
    assert!(w.calls().iter().any(|c| c[..2] == ["snapshot", "create"]));

    // A failure is kept with what it said.
    let other = tempfile::tempdir().unwrap();
    let id = start(
        binary,
        &config,
        &w.data(),
        Request::Remove {
            root: other.path().canonicalize().unwrap(),
        },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let job = loop {
        let job = list(&w.data(), 8)
            .unwrap()
            .into_iter()
            .find(|j| j.id == id)
            .unwrap();
        if job.exit.is_some() || std::time::Instant::now() > deadline {
            break job;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_ne!(job.exit, Some(0));
    assert!(job.tail.contains("no sandbox for"), "{}", job.tail);
}

#[test]
fn what_a_window_may_ask_for_becomes_fixed_arguments_and_nothing_else() {
    use orochi::sandbox::jobs::Request;
    let root = PathBuf::from("/work/web; rm -rf /");
    assert_eq!(
        Request::Create {
            root: root.clone(),
            mode: Mode::Container,
            docker: false
        }
        .args(),
        [
            "sandbox",
            "create",
            "/work/web; rm -rf /",
            "--mode",
            "container",
            "--no-docker"
        ]
    );
    assert_eq!(
        Request::Mode {
            root: root.clone(),
            mode: Mode::Host
        }
        .args(),
        ["sandbox", "mode", "host", "/work/web; rm -rf /", "--yes"]
    );
    assert_eq!(
        Request::Image { vm: true }.args(),
        ["sandbox", "image", "--vm"]
    );
    // Nothing a window can ask for needs an administrator: there is no network setup to run.
    assert!(serde_json::from_str::<Request>(r#"{"op":"network"}"#).is_err());
    let parsed: Request = serde_json::from_str(r#"{"op":"remove","root":"/work/web"}"#).unwrap();
    assert_eq!(parsed.args(), ["sandbox", "rm", "/work/web", "--yes"]);
    assert!(serde_json::from_str::<Request>(r#"{"op":"exec","root":"/"}"#).is_err());
    let relative = orochi::sandbox::jobs::start(
        Path::new("/bin/true"),
        Path::new("/c"),
        tempfile::tempdir().unwrap().path(),
        Request::Focus {
            root: "relative".into(),
        },
    );
    assert!(relative.is_err());
}

#[test]
fn the_gateway_routes_by_host_and_passes_upgraded_connections_through() {
    let status = Command::new("python3")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "tests",
            "-p",
            "test_sandbox_gateway.py",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn the_vm_powers_itself_off_only_when_nothing_inside_or_from_here_uses_it() {
    let status = Command::new("python3")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "tests",
            "-p",
            "test_sandbox_idle.py",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn ports_lists_what_listens_inside_with_the_address_each_opens_at_here() {
    let w = Workspace::new();
    create(&w);
    let output = w
        .command()
        .args(["sandbox", "ports"])
        .env("FAKE_SS", "54323,5173")
        .output()
        .unwrap();
    success(&output);
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("http://54323-repo.localhost:1355"), "{said}");
    assert!(said.contains("http://5173-repo.localhost:1355"), "{said}");
    assert!(
        !w.run(&["sandbox", "network"]).status.success(),
        "the root-only setup is gone"
    );
}

impl Workspace {
    /// Reaches Incus through a fake `limactl` whose one VM starts stopped, as the real VM is
    /// after it powered itself off while unused.
    fn behind_a_vm(mut self, running: bool) -> Self {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_limactl.py");
        let limactl = self.dir.path().join("limactl");
        std::fs::write(
            &limactl,
            format!(
                "#!/bin/sh\nFAKE_LIMA_STATE='{}' FAKE_LIMA_LOG='{}' FAKE_INCUS='{}' exec python3 '{}' \"$@\"\n",
                self.dir.path().join("vm.state").display(),
                self.dir.path().join("limactl.jsonl").display(),
                self.dir.path().join("incus").display(),
                fake.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&limactl, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(
            self.dir.path().join("vm.state"),
            if running { "Running" } else { "Stopped" },
        )
        .unwrap();
        self.config.sandbox.client = SandboxClient::Lima;
        self.config.sandbox.limactl = limactl.to_string_lossy().into_owned();
        self.config.sandbox.mounts = vec![self.dir.path().canonicalize().unwrap()];
        self.config.sandbox.min_free_gib = 0;
        self
    }
    fn vm(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("vm.state")).unwrap()
    }
    fn set_vm(&self, state: &str) {
        std::fs::write(self.dir.path().join("vm.state"), state).unwrap();
    }
    fn starts(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("limactl.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("[\"start\""))
            .count()
    }
}

#[test]
fn a_run_in_a_sandboxed_project_starts_the_vm_that_stopped_itself_and_then_works() {
    let w = Workspace::new().behind_a_vm(true);
    create(&w);
    // The VM powered itself off while nothing used it.
    w.set_vm("Stopped");
    success(&w.run(&["Implement a small endpoint"]));
    assert_eq!(w.vm(), "Running");
    assert_eq!(w.starts(), 1, "started once, for the run that needed it");
    assert!(w.execs().iter().any(|c| c.contains(&"sbx-run".to_owned())));
    assert!(w.agent_log().contains("session/prompt"));
}

#[test]
fn looking_at_sandboxes_never_starts_the_vm() {
    let w = Workspace::new().behind_a_vm(true);
    create(&w);
    w.set_vm("Stopped");
    let output = w.run(&["sandbox", "status"]);
    success(&output);
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("stopped (starts when needed)"), "{said}");
    assert!(said.contains("repo") && said.contains("stopped"), "{said}");
    success(&w.run(&["sandbox", "gc"]));
    assert_eq!(w.vm(), "Stopped");
    assert_eq!(w.starts(), 0);
}

#[test]
fn an_operation_that_needs_the_vm_starts_it_and_says_so() {
    let w = Workspace::new().behind_a_vm(false);
    let output = w.run(&["sandbox", "create"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Starting the sandbox VM"));
    assert_eq!(w.vm(), "Running");
    assert_eq!(w.starts(), 1);
}

#[test]
fn a_vm_that_was_never_created_is_named_rather_than_started() {
    let w = Workspace::new().behind_a_vm(false);
    std::fs::write(w.dir.path().join("vm.state"), "").unwrap();
    // The fake lists no VM when its state is empty: mimic `limactl list` knowing none.
    let fake = w.dir.path().join("limactl");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    let output = w.run(&["sandbox", "create"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("orochi sandbox up"));
}
