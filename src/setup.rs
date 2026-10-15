//! First-run setup: what this machine has of what Orochi needs, and putting in what it lacks
//! without touching the system. Orochi drives the coding CLIs the user has, and every one of
//! them (the Claude and Codex ACP bridges, Gemini CLI, the CLIs themselves) runs on Node.js;
//! a window opened from Finder, with no shell `PATH`, found none of it. So two things live
//! here: the `PATH` everything Orochi starts is looked up on (`search_path`: its own
//! toolchain, the process's own, the login shell's where the process came from no shell, and
//! the places a user's tools go), and a toolchain of its own under `<data>/toolchain/`:
//! Node.js from nodejs.org, checked against the release's `SHASUMS256.txt`, and an agent's CLI
//! from the npm registry with install scripts disabled, reached through `<data>/toolchain/bin`.
//! The sandbox host is the other half: Lima from its GitHub release, checked against the
//! release's `SHA256SUMS`, under the same toolchain, and then `sandbox::setup::run` (what
//! `orochi init` does) for the VM, Incus and the image. Nothing is written outside the data
//! directory and nothing asks for root. `orochi setup` is the command; the desktop asks
//! `--check --json` when it opens and runs the same command in the background
//! (`sandbox::jobs::Request::Prepare`).
use crate::{
    config::{AgentConfig, Config},
    discovery,
    types::Provider,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};

/// The Node.js line put in: the current LTS. The adapters ask for 22 or later.
pub const NODE_MAJOR: u64 = 24;
pub const NODE_MINIMUM: u64 = 22;
pub const NODE_DIST: &str = "https://nodejs.org/dist";
/// The Lima put in for the sandbox VM, pinned as the adapters are; `SHA256SUMS` beside the
/// release is what the archive is checked against.
pub const LIMA_VERSION: &str = "2.2.1";
pub const LIMA_RELEASES: &str = "https://github.com/lima-vm/lima/releases/download";

/// The data directory this process works in, so a lookup that is given none (`locate`) still
/// finds the toolchain under it. Set once, by whoever resolved the paths.
static DATA: OnceLock<PathBuf> = OnceLock::new();
pub fn register_data(data: &Path) {
    let _ = DATA.set(data.to_path_buf());
}

/// What Orochi can put in for an agent: the package its vendor publishes the CLI as, and the
/// command that signs it in, which only a terminal can run.
#[derive(Clone, Copy)]
struct Package {
    agent: &'static str,
    native: &'static str,
    package: &'static str,
    docs: &'static str,
    login: &'static str,
}
const PACKAGES: [Package; 3] = [
    Package {
        agent: "claude",
        native: "claude",
        package: "@anthropic-ai/claude-code",
        docs: "https://code.claude.com/docs/en/setup",
        login: "claude",
    },
    Package {
        agent: "codex",
        native: "codex",
        package: "@openai/codex",
        docs: "https://developers.openai.com/codex/cli",
        login: "codex login",
    },
    Package {
        agent: "gemini",
        native: "gemini",
        package: "@google/gemini-cli",
        docs: "https://geminicli.com/docs/get-started/installation",
        login: "gemini",
    },
];
/// An agent Orochi knows the install of, by its preset id or the name of its command.
fn package(agent: &AgentConfig) -> Option<Package> {
    let command = Path::new(&agent.command)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    PACKAGES.iter().copied().find(|p| {
        p.agent == agent.id || command == p.native || command == format!("{}-acp", p.native)
    })
}
/// Where an agent Orochi cannot install is installed from.
fn docs(agent: &AgentConfig) -> Option<&'static str> {
    if agent.id == "antigravity" || agent.command.contains("agy") {
        return Some("https://antigravity.google/docs/cli/install");
    }
    package(agent).map(|p| p.docs)
}

// --- Where things are looked for ---------------------------------------------------------------

pub fn toolchain(data: &Path) -> PathBuf {
    data.join("toolchain")
}
/// The CLIs Orochi installed, by their own names.
pub fn bin(data: &Path) -> PathBuf {
    toolchain(data).join("bin")
}
/// The Node.js Orochi installed; `current` is a link to the version in use.
pub fn node_bin(data: &Path) -> PathBuf {
    toolchain(data).join("node/current/bin")
}

/// The `PATH` everything Orochi starts is looked up on. Its own toolchain first, so an agent
/// installed here is the one found; then what the process was given; then, where the process
/// came from no shell (Finder, a launch agent), what the login shell would have had, read once;
/// then the places a user's own tools go whichever way it was started. Relative entries are
/// dropped, as `discovery` already drops them. An agent's own `env.PATH` still replaces all
/// of this.
pub fn search_path(data: Option<&Path>) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(data) = data.or_else(|| DATA.get().map(PathBuf::as_path)) {
        dirs.push(bin(data));
        dirs.push(node_bin(data));
    }
    dirs.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    dirs.extend(login_shell_path().iter().cloned());
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".npm-global/bin"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs.push("/usr/bin".into());
    dirs.push("/bin".into());
    let mut seen = std::collections::HashSet::new();
    let dirs: Vec<PathBuf> = dirs
        .into_iter()
        .filter(|d| d.is_absolute() && seen.insert(d.clone()))
        .collect();
    std::env::join_paths(dirs).unwrap_or_default()
}

/// The login shell's `PATH`, asked of the shell the way an editor opened from the dock asks it:
/// interactive and login, so both `.zprofile` (where Homebrew writes) and `.zshrc` (where nvm
/// writes) have spoken. Only where this process came from no shell at all (`SHLVL` unset), and
/// only once per process; a hung shell is given five seconds and then left.
fn login_shell_path() -> &'static Vec<PathBuf> {
    static LOGIN: OnceLock<Vec<PathBuf>> = OnceLock::new();
    LOGIN.get_or_init(|| {
        if !cfg!(unix) || std::env::var_os("SHLVL").is_some() {
            return vec![];
        }
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let Ok(mut child) = std::process::Command::new(&shell)
            .args(["-ilc", "printf '__OROCHI__%s__OROCHI__' \"$PATH\""])
            .env("OROCHI_PROBE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return vec![];
        };
        let mut stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            use std::io::Read;
            let mut out = String::new();
            if let Some(stdout) = stdout.as_mut() {
                let _ = stdout.read_to_string(&mut out);
            }
            out
        });
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < Duration::from_secs(5) => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
        let out = reader.join().unwrap_or_default();
        let Some(start) = out.find("__OROCHI__") else {
            return vec![];
        };
        let rest = &out[start + "__OROCHI__".len()..];
        let Some(end) = rest.find("__OROCHI__") else {
            return vec![];
        };
        std::env::split_paths(&rest[..end]).collect()
    })
}

// --- What this machine has ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// Present and recent enough.
    pub ok: bool,
    /// The one Orochi installed, under its own data directory.
    pub managed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentState {
    pub id: String,
    pub provider: Provider,
    /// As `discovery::inspect` reports it: `ready`, `adapter_required`, `not_installed`, …
    pub status: String,
    pub native: Option<PathBuf>,
    /// Orochi knows how to install its CLI.
    pub installable: bool,
    pub package: Option<String>,
    /// Where to get it by hand.
    pub docs: Option<String>,
    /// What its own sign-in leaves behind is there; `None` where Orochi cannot tell.
    pub signed_in: Option<bool>,
    /// The command that signs it in, for a terminal.
    pub login: Option<String>,
    /// Installed and, as far as can be told, signed in: a route Orochi could take.
    pub usable: bool,
    pub detail: Option<String>,
}

/// The sandbox host: what a project's agents, tests and servers can be put in.
#[derive(Debug, Clone, Serialize)]
pub struct SandboxState {
    /// `lima` (a VM on this machine) or `incus` (a client for an Incus host).
    pub client: String,
    /// Orochi can set it up on this machine: Lima on macOS or Linux, local Incus on Linux.
    pub supported: bool,
    pub limactl: Option<PathBuf>,
    /// The Lima Orochi installed, under its own toolchain.
    pub managed: bool,
    /// The VM's state (`Running`, `Stopped`), `None` where it was never created.
    pub vm: Option<String>,
    /// The host exists: `orochi init` has been through once.
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub node: Tool,
    pub npm: Tool,
    pub git: Tool,
    pub config_file: bool,
    pub agents: Vec<AgentState>,
    pub sandbox: SandboxState,
    /// Some agent could be routed to, and what it runs on is there.
    pub ready: bool,
    pub toolchain: PathBuf,
}

/// What this machine has, the keychain asked where the platform keeps a sign-in there.
pub fn status(config: &Config, config_path: &Path, data: &Path) -> Status {
    status_with(config, config_path, data, cfg!(target_os = "macos"))
}

/// `keychain` is whether Claude Code's macOS keychain item may be looked for (attributes only,
/// never the secret). Tests pass `false`: no test touches the keychain of the machine it runs
/// on.
pub fn status_with(config: &Config, config_path: &Path, data: &Path, keychain: bool) -> Status {
    let path = search_path(Some(data));
    let node = tool("node", &path, data, |v| {
        major(v).is_some_and(|m| m >= NODE_MINIMUM)
    });
    let npm = tool("npm", &path, data, |_| true);
    let git = tool("git", &path, data, |_| true);
    let agents: Vec<AgentState> = config
        .agents
        .iter()
        .filter(|a| a.enabled && !a.routing_only)
        .map(|agent| {
            let availability = discovery::inspect(agent, &config.discovery, data);
            let package = package(agent);
            let signed_in = signed_in(agent, keychain);
            let installed = matches!(availability.status, "ready" | "adapter_required");
            AgentState {
                id: agent.id.clone(),
                provider: agent.provider,
                status: availability.status.into(),
                native: availability.native_cli.or(availability.executable),
                installable: package.is_some(),
                package: package.map(|p| p.package.to_owned()),
                docs: docs(agent).map(str::to_owned),
                signed_in,
                login: package.map(|p| p.login.to_owned()),
                usable: installed && signed_in != Some(false),
                detail: availability.detail,
            }
        })
        .collect();
    let ready = node.ok && agents.iter().any(|a| a.usable);
    Status {
        node,
        npm,
        git,
        config_file: config_path.is_file(),
        agents,
        sandbox: sandbox_state(config, &path, data),
        ready,
        toolchain: toolchain(data),
    }
}

fn sandbox_state(config: &Config, path: &OsString, data: &Path) -> SandboxState {
    use crate::config::SandboxClient;
    let sandbox = &config.sandbox;
    let lima = sandbox.client == SandboxClient::Lima;
    let limactl = lima
        .then(|| discovery::find_in(&sandbox.limactl, path))
        .flatten();
    let vm = limactl
        .is_some()
        .then(|| crate::sandbox::ops::vm_status(sandbox).ok().flatten())
        .flatten();
    let local_incus = !lima && sandbox.remote.is_empty() && sandbox.incus == "incus";
    SandboxState {
        client: if lima { "lima" } else { "incus" }.into(),
        supported: (lima && cfg!(any(target_os = "macos", target_os = "linux")))
            || (local_incus && cfg!(target_os = "linux")),
        managed: limactl
            .as_ref()
            .is_some_and(|p| p.starts_with(toolchain(data))),
        limactl,
        ready: vm.is_some(),
        vm,
    }
}

fn tool(name: &str, path: &OsString, data: &Path, recent: impl Fn(&str) -> bool) -> Tool {
    let found = discovery::find_in(name, path);
    let version = found.as_ref().and_then(|exe| {
        let out = std::process::Command::new(exe)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next()?.trim();
        (!line.is_empty()).then(|| line.to_owned())
    });
    Tool {
        ok: found.is_some() && version.as_deref().is_none_or(&recent),
        managed: found
            .as_ref()
            .is_some_and(|p| p.starts_with(toolchain(data))),
        path: found,
        version,
    }
}
fn major(version: &str) -> Option<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Whether what the agent's own sign-in leaves behind is there: the files and variables its
/// documentation names (`sandbox::auth::recipe`), and for Claude Code the credentials file or
/// the keychain item it keeps a login in. An agent whose sign-in leaves nothing Orochi knows
/// of is `None`, not `false`.
fn signed_in(agent: &AgentConfig, keychain: bool) -> Option<bool> {
    let recipe = crate::sandbox::auth::recipe(agent);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut files = recipe.files.clone();
    let claude = agent.id == "claude" || agent.command.contains("claude");
    if claude {
        files.push(".claude/.credentials.json".into());
    }
    if files.is_empty() && recipe.env.is_empty() {
        return None;
    }
    let file = home
        .as_ref()
        .is_some_and(|home| files.iter().any(|f| home.join(f).is_file()));
    let variable = recipe.env.iter().any(|k| {
        agent.env.get(k).is_some_and(|v| !v.is_empty())
            || std::env::var_os(k).is_some_and(|v| !v.is_empty())
    });
    let item = claude && keychain && keychain_item("Claude Code-credentials");
    Some(file || variable || item)
}
/// Whether a generic password item exists, read from its attributes alone: `-w` is what would
/// read the secret and bring up a prompt, and it is never passed.
fn keychain_item(service: &str) -> bool {
    std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", service])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The command that signs an agent in, for a terminal to run with Orochi's toolchain in front.
pub fn login_command(config: &Config, data: &Path, agent: &str) -> Result<String> {
    let agent = config
        .agents
        .iter()
        .find(|a| a.id == agent)
        .with_context(|| format!("no such agent: {agent}"))?;
    let package = package(agent).with_context(|| {
        format!(
            "{} signs in its own way; see {}",
            agent.id,
            docs(agent).unwrap_or("its documentation")
        )
    })?;
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
    Ok(format!(
        "PATH={}:{}:\"$PATH\" {}",
        quote(&bin(data)),
        quote(&node_bin(data)),
        package.login
    ))
}

// --- Putting in what is missing ----------------------------------------------------------------

/// What `orochi setup` was asked to do beyond what it always does.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Agents whose CLI to install, by id.
    pub install: Vec<String>,
    /// Set up the sandbox host too: Lima, the VM, Incus and the image (`orochi init`).
    pub sandbox: bool,
}

/// Puts in what is missing: the config file, Node.js where there is none or an old one, the
/// CLIs asked for, the ACP adapters of every CLI that is there, and with `plan.sandbox` the
/// sandbox host. Says what it does on stdout, one line each, since a background job's log is
/// all a window sees of it. `node_dist` is where Node.js releases are read from (`NODE_DIST`,
/// or a mirror), `lima_releases` where Lima's are.
pub async fn run(
    config: &Config,
    config_path: &Path,
    data: &Path,
    plan: &Plan,
    node_dist: &str,
    lima_releases: &str,
) -> Result<Status> {
    std::fs::create_dir_all(toolchain(data))?;
    if !config_path.is_file() {
        write_config(config_path)?;
        println!("Wrote {}", config_path.display());
    }
    let before = status_with(config, config_path, data, false);
    if before.node.ok {
        println!(
            "Node.js {} at {}",
            before.node.version.as_deref().unwrap_or("?"),
            before
                .node
                .path
                .as_deref()
                .unwrap_or(Path::new("?"))
                .display()
        );
    } else {
        let version = install_node(data, node_dist).await?;
        println!(
            "Node.js {version} installed under {}",
            toolchain(data).join("node").display()
        );
    }
    for id in &plan.install {
        let agent = config
            .agents
            .iter()
            .find(|a| &a.id == id)
            .with_context(|| format!("no such agent: {id}"))?;
        let package = package(agent).with_context(|| {
            format!(
                "Orochi cannot install {id}; see {}",
                docs(agent).unwrap_or("its documentation")
            )
        })?;
        println!("Installing {} ({})…", id, package.package);
        install_cli(data, package).await?;
        println!(
            "{id} installed; {} is on Orochi's toolchain",
            bin(data).join(package.native).display()
        );
    }
    for agent in config
        .agents
        .iter()
        .filter(|a| a.enabled && !a.routing_only)
    {
        let availability = discovery::inspect(agent, &config.discovery, data);
        if availability.status != "adapter_required" {
            continue;
        }
        println!("Preparing the ACP adapter for {}…", agent.id);
        discovery::prepare(agent, &config.discovery, data)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
    }
    if plan.sandbox {
        let state = sandbox_state(config, &search_path(Some(data)), data);
        if state.client == "lima" && state.limactl.is_none() {
            ensure!(
                state.supported,
                "Lima runs on macOS and Linux; on Windows use tools/windows/orochi.ps1 -Setup"
            );
            install_lima(data, lima_releases).await?;
            println!(
                "Lima {LIMA_VERSION} installed under {}",
                toolchain(data).join("lima").display()
            );
        } else if let Some(limactl) = &state.limactl {
            println!("Lima at {}", limactl.display());
        }
        println!("Setting up the sandbox host (the VM, Incus and the image)…");
        let (sandbox, data) = (config.sandbox.clone(), data.to_path_buf());
        tokio::task::spawn_blocking(move || crate::sandbox::setup::run(&sandbox, &data, false))
            .await??;
    }
    let after = status_with(config, config_path, data, cfg!(target_os = "macos"));
    for agent in &after.agents {
        println!(
            "{:<12} {:<18} {}",
            agent.id,
            agent.status,
            match agent.signed_in {
                Some(true) => "signed in",
                Some(false) => "not signed in",
                None => "sign-in unknown",
            }
        );
    }
    println!(
        "sandbox      {}",
        match &after.sandbox.vm {
            Some(state) => format!("VM {}", state.to_lowercase()),
            None => "not set up".to_owned(),
        }
    );
    println!(
        "{}",
        if after.ready {
            "Ready."
        } else {
            "Not ready yet: install or sign in to at least one agent."
        }
    );
    Ok(after)
}

/// The name Lima publishes this machine's build under.
pub fn lima_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("Darwin-arm64"),
        ("macos", "x86_64") => Some("Darwin-x86_64"),
        ("linux", "x86_64") => Some("Linux-x86_64"),
        ("linux", "aarch64") => Some("Linux-aarch64"),
        _ => None,
    }
}

/// Lima into `<data>/toolchain/lima/<version>/` (its archive unpacks as `bin/` and `share/`,
/// which `limactl` finds beside itself), `current` pointing at it and `limactl` linked into
/// the toolchain's `bin`. The archive is checked against the release's SHA256SUMS before
/// anything is unpacked. The release binaries carry the virtualization entitlement, so no
/// Homebrew and no root.
pub async fn install_lima(data: &Path, releases: &str) -> Result<()> {
    let platform = lima_platform().context("no Lima build is published for this machine")?;
    let client = reqwest::Client::builder()
        .user_agent(format!("orochi/{}", env!("CARGO_PKG_VERSION")))
        .build()?;
    let archive = format!("lima-{LIMA_VERSION}-{platform}.tar.gz");
    let base = format!("{releases}/v{LIMA_VERSION}");
    println!("Downloading Lima {LIMA_VERSION} for {platform}…");
    let sums = client
        .get(format!("{base}/SHA256SUMS"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let (sum, file) = line.split_once("  ")?;
            (file.trim() == archive).then(|| sum.trim().to_owned())
        })
        .context("the release lists no checksum for this build; refusing to install it")?;
    let bytes = client
        .get(format!("{base}/{archive}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    {
        use sha2::Digest;
        let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
        ensure!(
            actual == expected,
            "the Lima download does not match its published checksum"
        );
    }
    let lima = toolchain(data).join("lima");
    std::fs::create_dir_all(&lima)?;
    let stage = tempfile::Builder::new()
        .prefix(".lima-")
        .tempdir_in(&lima)?;
    let file = stage.path().join(&archive);
    std::fs::write(&file, &bytes)?;
    let unpacked = stage.path().join("unpacked");
    std::fs::create_dir_all(&unpacked)?;
    let output = tokio::process::Command::new("tar")
        .arg("-xzf")
        .arg(&file)
        .arg("-C")
        .arg(&unpacked)
        .stdin(Stdio::null())
        .output()
        .await
        .context("unpacking needs `tar`")?;
    ensure!(
        output.status.success(),
        "unpacking Lima failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ensure!(
        unpacked.join("bin/limactl").is_file(),
        "the Lima archive did not contain bin/limactl"
    );
    let destination = lima.join(LIMA_VERSION);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&unpacked, &destination)?;
    point(&lima.join("current"), Path::new(LIMA_VERSION))?;
    std::fs::create_dir_all(bin(data))?;
    for command in ["limactl", "lima"] {
        if destination.join("bin").join(command).is_file() {
            point(
                &bin(data).join(command),
                &lima.join("current/bin").join(command),
            )?;
        }
    }
    Ok(())
}

/// The default configuration, as `orochi config init` writes it.
pub fn write_config(path: &Path) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(toml::to_string_pretty(&Config::default())?.as_bytes())?;
    Ok(())
}

/// The name nodejs.org publishes this machine's build under.
pub fn node_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        _ => None,
    }
}

/// The newest release of the line put in, from the index nodejs.org publishes; falling back to
/// the newest LTS the adapters accept where the line is not listed.
fn choose(index: &[serde_json::Value]) -> Option<String> {
    let version = |entry: &serde_json::Value| entry["version"].as_str().map(str::to_owned);
    index
        .iter()
        .find(|e| version(e).and_then(|v| major(&v)) == Some(NODE_MAJOR))
        .or_else(|| {
            index.iter().find(|e| {
                e["lts"].as_str().is_some()
                    && version(e)
                        .and_then(|v| major(&v))
                        .is_some_and(|m| m >= NODE_MINIMUM)
            })
        })
        .and_then(version)
}

/// Node.js into `<data>/toolchain/node/<version>/`, `current` pointing at it. The archive is
/// checked against the release's SHASUMS256.txt before anything is unpacked.
async fn install_node(data: &Path, dist: &str) -> Result<String> {
    let platform = node_platform().context(
        "no Node.js build is published for this machine; install Node.js 22 or later yourself",
    )?;
    let client = reqwest::Client::builder()
        .user_agent(format!("orochi/{}", env!("CARGO_PKG_VERSION")))
        .build()?;
    let index: Vec<serde_json::Value> = serde_json::from_slice(
        &client
            .get(format!("{dist}/index.json"))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?,
    )
    .context("reading the Node.js release index")?;
    let version = choose(&index).context("no Node.js release the adapters accept is listed")?;
    let name = format!("node-{version}-{platform}");
    let archive = format!("{name}.tar.gz");
    println!("Downloading Node.js {version} for {platform}…");
    let sums = client
        .get(format!("{dist}/{version}/SHASUMS256.txt"))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let (sum, file) = line.split_once("  ")?;
            (file.trim() == archive).then(|| sum.trim().to_owned())
        })
        .context("the release lists no checksum for this build; refusing to install it")?;
    let bytes = client
        .get(format!("{dist}/{version}/{archive}"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    {
        use sha2::Digest;
        let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
        ensure!(
            actual == expected,
            "the Node.js download does not match its published checksum"
        );
    }
    let node = toolchain(data).join("node");
    std::fs::create_dir_all(&node)?;
    let stage = tempfile::Builder::new()
        .prefix(".node-")
        .tempdir_in(&node)?;
    let file = stage.path().join(&archive);
    std::fs::write(&file, &bytes)?;
    let unpacked = tokio::process::Command::new("tar")
        .arg("-xzf")
        .arg(&file)
        .arg("-C")
        .arg(stage.path())
        .stdin(Stdio::null())
        .output()
        .await
        .context("unpacking needs `tar`")?;
    ensure!(
        unpacked.status.success(),
        "unpacking Node.js failed: {}",
        String::from_utf8_lossy(&unpacked.stderr).trim()
    );
    let destination = node.join(&version);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(stage.path().join(&name), &destination)?;
    ensure!(
        destination.join("bin/node").is_file(),
        "the Node.js archive did not contain bin/node"
    );
    point(&node.join("current"), Path::new(&version))?;
    Ok(version)
}

/// `link` → `target`, replaced atomically.
fn point(link: &Path, target: &Path) -> Result<()> {
    let fresh = link.with_extension("new");
    let _ = std::fs::remove_file(&fresh);
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &fresh)?;
    #[cfg(not(unix))]
    std::os::windows::fs::symlink_dir(target, &fresh)?;
    std::fs::rename(&fresh, link)?;
    Ok(())
}

/// An agent's CLI from the npm registry into `<data>/toolchain/cli/<agent>/`, with install
/// scripts disabled as the adapters are, and its command linked into `<data>/toolchain/bin`.
async fn install_cli(data: &Path, package: Package) -> Result<()> {
    let path = search_path(Some(data));
    let npm = discovery::find_in("npm", &path).context("npm was not found")?;
    let cli = toolchain(data).join("cli");
    std::fs::create_dir_all(&cli)?;
    std::fs::create_dir_all(bin(data))?;
    let stage = tempfile::Builder::new()
        .prefix(".setup-")
        .tempdir_in(&cli)?;
    let output = tokio::process::Command::new(npm)
        .args(["install", "--prefix"])
        .arg(stage.path())
        .arg("--cache")
        .arg(toolchain(data).join("npm-cache"))
        .args([
            "--registry=https://registry.npmjs.org",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ])
        .arg(format!("{}@latest", package.package))
        .current_dir(stage.path())
        .env("PATH", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await?;
    ensure!(
        output.status.success(),
        "npm install of {} failed: {}",
        package.package,
        crate::context::bounded(&String::from_utf8_lossy(&output.stderr), 2000)
    );
    let installed = stage.path().join("node_modules/.bin").join(package.native);
    ensure!(
        installed.exists(),
        "{} installed no `{}` command",
        package.package,
        package.native
    );
    let destination = cli.join(package.agent);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(stage.keep(), &destination)?;
    point(
        &bin(data).join(package.native),
        &destination.join("node_modules/.bin").join(package.native),
    )?;
    Ok(())
}

/// What `orochi setup` prints without `--json`.
pub fn describe(status: &Status) -> String {
    let mut out = String::new();
    let line = |name: &str, tool: &Tool| {
        format!(
            "{:<12} {:<18} {}\n",
            name,
            match (&tool.path, tool.ok) {
                (Some(_), true) if tool.managed => "ready (orochi)",
                (Some(_), true) => "ready",
                (Some(_), false) => "too old",
                (None, _) => "not found",
            },
            tool.version
                .as_deref()
                .map(|v| v.to_owned())
                .or_else(|| tool.path.as_ref().map(|p| p.display().to_string()))
                .unwrap_or_default()
        )
    };
    out.push_str(&line("node", &status.node));
    out.push_str(&line("npm", &status.npm));
    out.push_str(&line("git", &status.git));
    for agent in &status.agents {
        out.push_str(&format!(
            "{:<12} {:<18} {}\n",
            agent.id,
            agent.status,
            match agent.signed_in {
                Some(true) => "signed in".to_owned(),
                Some(false) => format!(
                    "not signed in{}",
                    agent
                        .login
                        .as_deref()
                        .map(|l| format!(" (run `{l}`)"))
                        .unwrap_or_default()
                ),
                None => String::new(),
            }
        ));
        if !matches!(agent.status.as_str(), "ready" | "adapter_required") {
            if let Some(package) = &agent.package {
                out.push_str(&format!(
                    "  install with `orochi setup --install {}` ({package})\n",
                    agent.id
                ));
            } else if let Some(docs) = &agent.docs {
                out.push_str(&format!("  install by hand: {docs}\n"));
            }
        }
    }
    out.push_str(&format!(
        "{:<12} {:<18} {}\n",
        "sandbox",
        match (&status.sandbox.vm, status.sandbox.supported) {
            (Some(state), _) => format!("VM {}", state.to_lowercase()),
            (None, true) => "not set up".to_owned(),
            (None, false) => "unsupported here".to_owned(),
        },
        if status.sandbox.vm.is_none() && status.sandbox.supported {
            "set up with `orochi setup --sandbox`"
        } else {
            ""
        }
    ));
    out.push_str(if status.ready {
        "Ready.\n"
    } else {
        "Not ready: install or sign in to at least one agent.\n"
    });
    out
}
