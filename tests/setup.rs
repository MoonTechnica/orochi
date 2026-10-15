//! First-run setup: Node.js and a CLI put in under the data directory, and the `PATH` Orochi
//! looks things up on. What depends on the environment is driven through the binary with the
//! environment set on the child; nothing here reaches nodejs.org, the npm registry or the
//! keychain.
#![cfg(unix)]
use orochi::{
    config::{AgentConfig, Config},
    sandbox::jobs::Request,
    setup,
    types::Provider,
};
use std::{
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

fn executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in for nodejs.org: the release index, one release's checksums, and its archive
/// for this machine, holding a `node` that says its version and an `npm` that installs by
/// writing the command a package would have put in `node_modules/.bin`.
fn node_dist(dir: &Path, tamper: bool) -> (PathBuf, PathBuf) {
    let platform = setup::node_platform().expect("a platform nodejs.org publishes for");
    let name = format!("node-v24.0.0-{platform}");
    let build = dir.join("build").join(&name).join("bin");
    std::fs::create_dir_all(&build).unwrap();
    executable(&build.join("node"), "#!/bin/sh\necho v24.0.0\n");
    let log = dir.join("npm.log");
    let python = Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    let python = String::from_utf8(python.stdout).unwrap().trim().to_string();
    executable(
        &build.join("npm"),
        &format!(
            r#"#!{python}
import json, pathlib, sys
with open({log}, 'a') as log: log.write(json.dumps(sys.argv[1:]) + '\n')
spec = sys.argv[-1]
package = spec.rsplit('@', 1)[0]
command = {{'@google/gemini-cli': 'gemini', '@anthropic-ai/claude-code': 'claude',
           '@openai/codex': 'codex'}}.get(package, package.rsplit('/', 1)[-1])
prefix = pathlib.Path(sys.argv[sys.argv.index('--prefix') + 1])
target = prefix / 'node_modules/.bin' / command
target.parent.mkdir(parents=True, exist_ok=True)
target.write_text('#!/bin/sh\necho 0.1.0\n')
target.chmod(0o755)
"#,
            log = serde_json::to_string(&log).unwrap(),
        ),
    );
    let dist = dir.join("dist");
    let release = dist.join("v24.0.0");
    std::fs::create_dir_all(&release).unwrap();
    let archive = release.join(format!("{name}.tar.gz"));
    assert!(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(dir.join("build"))
            .arg(&name)
            .status()
            .unwrap()
            .success()
    );
    let sum = {
        use sha2::Digest;
        format!(
            "{:x}",
            sha2::Sha256::digest(std::fs::read(&archive).unwrap())
        )
    };
    let sum = if tamper {
        sum.replace(|c: char| c.is_ascii_hexdigit(), "0")
    } else {
        sum
    };
    std::fs::write(
        release.join("SHASUMS256.txt"),
        format!("{sum}  {name}.tar.gz\n{sum}  {name}.tar.xz\n"),
    )
    .unwrap();
    std::fs::write(
        dist.join("index.json"),
        r#"[{"version":"v24.0.0","lts":"Krypton"},{"version":"v22.0.0","lts":"Jod"}]"#,
    )
    .unwrap();
    (dist, log)
}

/// `python3 -m http.server` on a loopback port of the system's choosing, stopped when dropped.
struct Served {
    child: Child,
    url: String,
}
impl Served {
    fn new(dir: &Path) -> Self {
        let mut child = Command::new("python3")
            .args([
                "-u",
                "-m",
                "http.server",
                "0",
                "--bind",
                "127.0.0.1",
                "--directory",
            ])
            .arg(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line
            .split("port ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .unwrap_or_else(|| panic!("no port in {line:?}"))
            .to_owned();
        Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
        }
    }
}
impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The binary, with nothing of this machine's environment but what the test gives it: a
/// `PATH` holding only `tar`, a home of its own, and no login shell to ask (`SHLVL` set).
struct Machine {
    dir: tempfile::TempDir,
}
impl Machine {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        let tar = Command::new("sh")
            .args(["-c", "command -v tar"])
            .output()
            .unwrap();
        let tar = String::from_utf8(tar.stdout).unwrap().trim().to_string();
        std::os::unix::fs::symlink(&tar, bin.join("tar")).unwrap();
        Self { dir }
    }
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }
    fn config(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }
    fn orochi(&self, dist: &str) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_orochi"));
        cmd.env_clear()
            .env("PATH", self.dir.path().join("bin"))
            .env("HOME", self.home())
            .env("SHLVL", "1")
            .env("OROCHI_NODE_DIST", dist)
            .args(["--config", self.config().to_str().unwrap()])
            .args(["--data-dir", self.data().to_str().unwrap()]);
        cmd
    }
    fn check(&self, dist: &str) -> serde_json::Value {
        let out = self
            .orochi(dist)
            .args(["--json", "setup", "--check"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

#[test]
fn setup_puts_node_and_a_cli_under_the_data_directory_from_checked_downloads_and_nothing_else() {
    let fixture = tempfile::tempdir().unwrap();
    let (dist, log) = node_dist(fixture.path(), false);
    let served = Served::new(&dist);
    let machine = Machine::new();

    // Before: no Node, nothing installed, and a window would have to ask.
    let before = machine.check(&served.url);
    assert_eq!(before["node"]["ok"], false);
    assert_eq!(before["ready"], false);
    assert_eq!(before["config_file"], false);
    let gemini = before["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "gemini")
        .unwrap();
    assert_eq!(gemini["status"], "not_installed");
    assert_eq!(gemini["installable"], true);
    assert_eq!(gemini["package"], "@google/gemini-cli");

    let out = machine
        .orochi(&served.url)
        .args(["setup", "--install", "gemini"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("Node.js v24.0.0 installed"), "{stdout}");
    assert!(stdout.contains("@google/gemini-cli at"), "{stdout}");
    assert!(stdout.contains(&format!("wrote {}", machine.config().display())));
    let toolchain = machine.data().join("toolchain");
    assert!(toolchain.join("node/current/bin/node").is_file());
    assert!(toolchain.join("node/v24.0.0/bin/node").is_file());
    let shim = toolchain.join("bin/gemini");
    assert!(shim.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(shim.is_file());
    assert_eq!(
        machine.config().metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    // The CLI came from the registry with install scripts disabled, into the toolchain.
    // `npm --version` is asked on every check; one install is what was run.
    let installs: String = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter(|l| l.contains("\"install\""))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(installs.lines().count(), 1, "{installs}");
    assert!(installs.contains("@google/gemini-cli@latest"));
    assert!(installs.contains("--ignore-scripts"));
    assert!(installs.contains("--registry=https://registry.npmjs.org"));
    assert!(installs.contains(&toolchain.join("cli").display().to_string()));
    // And nothing landed anywhere else.
    assert!(!machine.home().join(".npm").exists());
    assert_eq!(
        std::fs::read_dir(machine.home())
            .unwrap()
            .filter_map(|e| e.ok())
            .count(),
        0
    );

    // The run reported itself step by step, as a window follows it: decided up front, each
    // timed, the config written, Node put in, gemini installed, and no adapter step for an
    // agent that has no bridge.
    let progress = setup::Progress::read(&machine.data()).unwrap();
    assert!(progress.finished_at.is_some());
    let steps: Vec<(&str, &str)> = progress
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s.state.as_str()))
        .collect();
    assert_eq!(
        steps,
        [
            ("Writing the configuration file", "done"),
            ("Installing Node.js", "done"),
            ("Installing gemini", "done"),
        ]
    );
    for step in &progress.steps {
        assert!(step.started_at.is_some() && step.finished_at >= step.started_at);
    }
    assert!(
        progress.steps[1]
            .detail
            .as_deref()
            .unwrap()
            .contains("v24.0.0")
    );

    // After: the Node Orochi put in is the one found, and Gemini is installed but not yet
    // signed in; its own sign-in leaves a file, and that file is what makes it usable.
    let after = machine.check(&served.url);
    assert_eq!(after["node"]["ok"], true);
    assert_eq!(after["node"]["managed"], true);
    assert_eq!(after["node"]["version"], "v24.0.0");
    assert_eq!(after["config_file"], true);
    let gemini = |status: &serde_json::Value| {
        status["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "gemini")
            .unwrap()
            .clone()
    };
    assert_eq!(gemini(&after)["status"], "ready");
    assert_eq!(gemini(&after)["signed_in"], false);
    assert_eq!(gemini(&after)["login"], "gemini");
    assert_eq!(after["ready"], false);
    std::fs::create_dir_all(machine.home().join(".gemini")).unwrap();
    std::fs::write(machine.home().join(".gemini/oauth_creds.json"), "{}").unwrap();
    let signed = machine.check(&served.url);
    assert_eq!(gemini(&signed)["signed_in"], true);
    assert_eq!(gemini(&signed)["usable"], true);
    assert_eq!(signed["ready"], true);

    // Running it again changes nothing and downloads nothing.
    let again = machine.orochi(&served.url).arg("setup").output().unwrap();
    let stdout = String::from_utf8_lossy(&again.stdout);
    assert!(again.status.success());
    assert!(stdout.contains("Node.js v24.0.0 at"), "{stdout}");
    assert!(!stdout.contains("downloading"), "{stdout}");
    let again = setup::Progress::read(&machine.data()).unwrap();
    assert_eq!(again.steps.len(), 2);
    assert!(
        again.steps.iter().all(|s| s.state == "skipped"),
        "{again:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|l| l.contains("\"install\""))
            .count(),
        1
    );
}

#[test]
fn a_node_archive_that_does_not_match_its_published_checksum_is_never_unpacked() {
    let fixture = tempfile::tempdir().unwrap();
    let (dist, _) = node_dist(fixture.path(), true);
    let served = Served::new(&dist);
    let machine = Machine::new();
    let out = machine.orochi(&served.url).arg("setup").output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not match its published checksum"),
        "{stderr}"
    );
    // The step it failed on says so, and nothing after it was started.
    let progress = setup::Progress::read(&machine.data()).unwrap();
    assert_eq!(progress.steps[1].state, "failed");
    assert!(
        progress.steps[1]
            .detail
            .as_deref()
            .unwrap()
            .contains("checksum")
    );
    assert!(progress.finished_at.is_some());
    let node = machine.data().join("toolchain/node");
    assert!(!node.join("current").exists());
    assert!(!node.join("v24.0.0").exists());
    // The stage is gone with the failure; nothing half-unpacked is left to be found.
    let leftovers: Vec<_> = std::fs::read_dir(&node)
        .map(|d| d.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn the_search_path_puts_orochis_own_toolchain_first_and_keeps_only_absolute_entries() {
    let data = tempfile::tempdir().unwrap();
    let path = setup::search_path(Some(data.path()));
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    assert_eq!(dirs[0], data.path().join("toolchain/bin"));
    assert_eq!(dirs[1], data.path().join("toolchain/node/current/bin"));
    assert!(dirs.iter().all(|d| d.is_absolute()), "{dirs:?}");
    assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    assert!(dirs.contains(&PathBuf::from("/usr/bin")));
    let mut seen = std::collections::HashSet::new();
    assert!(
        dirs.iter().all(|d| seen.insert(d)),
        "no entry twice: {dirs:?}"
    );
    // Without a data directory the toolchain is not guessed at.
    let bare: Vec<PathBuf> = std::env::split_paths(&setup::search_path(None)).collect();
    assert!(!bare.iter().any(|d| d.ends_with("toolchain/bin")));
}

#[test]
fn a_setup_job_is_the_setup_command_a_person_would_type() {
    let request = Request::Prepare {
        install: vec!["claude".into(), "gemini".into()],
        sandbox: true,
    };
    assert_eq!(
        request.args(),
        [
            "setup",
            "--install",
            "claude",
            "--install",
            "gemini",
            "--sandbox"
        ]
    );
    assert_eq!(request.root(), None);
    assert_eq!(
        serde_json::from_str::<Request>(r#"{"op":"prepare","install":["claude"]}"#).unwrap(),
        Request::Prepare {
            install: vec!["claude".into()],
            sandbox: false,
        }
    );
    assert_eq!(
        serde_json::from_str::<Request>(r#"{"op":"prepare"}"#)
            .unwrap()
            .args(),
        ["setup"]
    );
}

#[test]
fn the_sign_in_command_is_the_agents_own_login_with_the_toolchain_in_front() {
    let data = tempfile::tempdir().unwrap();
    let config = Config::default();
    let codex = setup::login_command(&config, data.path(), "codex").unwrap();
    assert!(codex.ends_with(" codex login"), "{codex}");
    assert!(codex.starts_with(&format!(
        "PATH='{}':'{}':\"$PATH\" ",
        data.path().join("toolchain/bin").display(),
        data.path().join("toolchain/node/current/bin").display()
    )));
    assert!(
        setup::login_command(&config, data.path(), "claude")
            .unwrap()
            .ends_with(" claude")
    );
    // An agent Orochi cannot sign in says where its own instructions are.
    let error = setup::login_command(&config, data.path(), "antigravity").unwrap_err();
    assert!(error.to_string().contains("antigravity.google"), "{error}");
    assert!(
        setup::login_command(&config, data.path(), "nobody")
            .unwrap_err()
            .to_string()
            .contains("no such agent")
    );
}

#[test]
fn the_report_names_what_can_be_installed_and_where_the_rest_comes_from() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let mut config = Config::default();
    config.discovery.auto_add = false;
    let mut gemini = AgentConfig::preset("gemini", Provider::GOOGLE, "gemini", &["--acp"]);
    gemini
        .env
        .insert("PATH".into(), empty.display().to_string());
    let mut custom = AgentConfig::preset(
        "acme",
        Provider::new("acme").unwrap(),
        "/nowhere/acme-acp",
        &[],
    );
    custom
        .env
        .insert("PATH".into(), empty.display().to_string());
    let mut adviser = AgentConfig::preset("judge", Provider::OPENAI, "codex-acp", &[]);
    adviser.routing_only = true;
    config.agents = vec![gemini, custom, adviser];
    let status = setup::status_with(&config, &dir.path().join("config.toml"), dir.path(), false);
    assert!(!status.config_file);
    // An adviser is never offered: it runs no work.
    assert_eq!(status.agents.len(), 2);
    let gemini = &status.agents[0];
    assert_eq!(gemini.status, "not_installed");
    assert!(gemini.installable);
    assert_eq!(gemini.package.as_deref(), Some("@google/gemini-cli"));
    assert_eq!(gemini.login.as_deref(), Some("gemini"));
    assert!(!gemini.usable);
    let custom = &status.agents[1];
    assert!(!custom.installable);
    assert_eq!(custom.package, None);
    assert_eq!(custom.docs, None);
    // Nothing is known of how it signs in, so nothing is claimed.
    assert_eq!(custom.signed_in, None);
    assert!(!status.ready);
    let text = setup::describe(&status);
    assert!(text.contains("orochi setup --install gemini"), "{text}");
    assert!(text.contains("Not ready"), "{text}");
    // The sandbox host is reported beside them: nothing is created by looking.
    assert_eq!(status.sandbox.client, "lima");
    assert!(!status.sandbox.ready);
    assert!(!status.sandbox.managed);
    assert!(text.contains("sandbox"), "{text}");
}

/// A stand-in for Lima's GitHub release: `SHA256SUMS` and the archive for this machine,
/// which unpacks as `bin/` and `share/` with no directory above them, as the real one does.
fn lima_release(dir: &Path, tamper: bool) -> PathBuf {
    let platform = setup::lima_platform().expect("a platform Lima publishes for");
    let build = dir.join("build");
    std::fs::create_dir_all(build.join("bin")).unwrap();
    std::fs::create_dir_all(build.join("share/lima")).unwrap();
    executable(
        &build.join("bin/limactl"),
        "#!/bin/sh\ncase $1 in --version) echo limactl version fixture;; list) ;; esac\n",
    );
    std::fs::write(build.join("share/lima/README"), "templates").unwrap();
    let release = dir.join(format!("releases/v{}", setup::LIMA_VERSION));
    std::fs::create_dir_all(&release).unwrap();
    let name = format!("lima-{}-{platform}.tar.gz", setup::LIMA_VERSION);
    assert!(
        Command::new("tar")
            .arg("-czf")
            .arg(release.join(&name))
            .arg("-C")
            .arg(&build)
            .arg(".")
            .status()
            .unwrap()
            .success()
    );
    let sum = {
        use sha2::Digest;
        format!(
            "{:x}",
            sha2::Sha256::digest(std::fs::read(release.join(&name)).unwrap())
        )
    };
    let sum = if tamper {
        sum.replace(|c: char| c.is_ascii_hexdigit(), "0")
    } else {
        sum
    };
    std::fs::write(release.join("SHA256SUMS"), format!("{sum}  {name}\n")).unwrap();
    dir.join("releases")
}

#[tokio::test]
async fn lima_is_put_under_the_toolchain_from_a_checked_release_and_found_from_there() {
    let fixture = tempfile::tempdir().unwrap();
    let releases = lima_release(fixture.path(), false);
    let served = Served::new(&releases);
    let data = tempfile::tempdir().unwrap();
    setup::install_lima(data.path(), &served.url).await.unwrap();
    let toolchain = data.path().join("toolchain");
    assert!(
        toolchain
            .join(format!("lima/{}/bin/limactl", setup::LIMA_VERSION))
            .is_file()
    );
    assert!(toolchain.join("lima/current/share/lima/README").is_file());
    let shim = toolchain.join("bin/limactl");
    assert!(shim.symlink_metadata().unwrap().file_type().is_symlink());
    // The lookup everything Orochi starts uses finds it, so the sandbox code's bare
    // `limactl` resolves to this one and the report says it is Orochi's.
    let found = orochi::discovery::find_in("limactl", &setup::search_path(Some(data.path())));
    assert_eq!(found, Some(shim));
    let mut config = Config::default();
    config.discovery.auto_add = false;
    let status = setup::status_with(&config, &data.path().join("c.toml"), data.path(), false);
    assert_eq!(status.sandbox.limactl, Some(toolchain.join("bin/limactl")));
    assert!(status.sandbox.managed);
    assert_eq!(status.sandbox.vm, None, "the fixture lists no VM");
    assert!(!status.sandbox.ready);

    // A release whose checksum does not match is refused before anything is unpacked.
    let tampered = tempfile::tempdir().unwrap();
    let releases = lima_release(tampered.path(), true);
    let served = Served::new(&releases);
    let fresh = tempfile::tempdir().unwrap();
    let error = setup::install_lima(fresh.path(), &served.url)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not match"), "{error}");
    assert!(!fresh.path().join("toolchain/lima/current").exists());
}

#[test]
fn a_running_setup_job_can_be_stopped_and_is_written_down_as_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    // In place of the binary: something that would run for a long time, in a child of its own,
    // as a download or an npm install would be.
    let slow = dir.path().join("slow");
    executable(&slow, "#!/bin/sh\nsleep 300 &\nwait\n");
    let id = orochi::sandbox::jobs::start(
        &slow,
        &dir.path().join("config.toml"),
        &data,
        Request::Prepare {
            install: vec![],
            sandbox: false,
        },
    )
    .unwrap();
    let job = orochi::sandbox::jobs::list(&data, 1).unwrap().remove(0);
    assert_eq!(job.id, id);
    assert_eq!(job.exit, None);
    let pid = job.pid.expect("a job knows its process group");
    std::thread::sleep(std::time::Duration::from_millis(300));
    orochi::sandbox::jobs::cancel(&data, &id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let job = orochi::sandbox::jobs::list(&data, 1).unwrap().remove(0);
    assert_eq!(job.exit, Some(130));
    // The whole group is gone, the sleep with it.
    let alive = unsafe { libc::kill(-(pid as i32), 0) } == 0;
    assert!(!alive, "the job's process group is still there");
    // Stopping it again, or something that is not a job, changes nothing and says so.
    orochi::sandbox::jobs::cancel(&data, &id).unwrap();
    assert!(orochi::sandbox::jobs::cancel(&data, "../../etc").is_err());
    assert!(orochi::sandbox::jobs::cancel(&data, "nope").is_err());
}
