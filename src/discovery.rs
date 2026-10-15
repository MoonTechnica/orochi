//! Resolve installed coding CLIs to ACP transports without requiring global adapters.
use crate::{
    agents::{AgentError, ErrorKind},
    config::{AgentConfig, DiscoveryConfig, LeanConfig},
    types::Provider,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Clone, Copy)]
struct Bridge {
    native: &'static str,
    binary: &'static str,
    package: &'static str,
    version: &'static str,
    native_env: &'static str,
}
const BRIDGES: [(Provider, Bridge); 2] = [
    (
        Provider::OPENAI,
        Bridge {
            native: "codex",
            binary: "codex-acp",
            package: "@agentclientprotocol/codex-acp",
            version: "1.10.0",
            native_env: "CODEX_PATH",
        },
    ),
    (
        Provider::ANTHROPIC,
        Bridge {
            native: "claude",
            binary: "claude-agent-acp",
            package: "@agentclientprotocol/claude-agent-acp",
            version: "0.77.0",
            native_env: "CLAUDE_CODE_EXECUTABLE",
        },
    ),
];
impl Bridge {
    fn spec(self) -> String {
        format!("{}@{}", self.package, self.version)
    }
    fn directory(self, data: &Path) -> PathBuf {
        data.join("adapters")
            .join(format!("{}-{}", self.binary, self.version))
    }
    fn binary_at(self, directory: &Path) -> PathBuf {
        directory.join("node_modules/.bin").join(self.binary)
    }
}

/// What an isolated session that only answers a question is given so it costs like one.
/// The coding agent's own system prompt, tools and settings are most of what such a session
/// costs — measured 2026-10-15: 43,854 tokens against 815 through `claude-agent-acp` 0.77.0,
/// and 20,587 against 14,645 on `codex` 0.160.1 with its base instructions replaced. ACP has
/// no field for any of it, so each agent is told in its own words (`LeanConfig`): `meta` goes
/// on `session/new` as `_meta`, with `{instruction}` filled in; `env` goes on the launch, with
/// `{instruction_file}` the path of a file holding the instruction, and a value that is a JSON
/// object merged into the JSON object already in that variable, so a session configuration
/// the user set keeps its other keys. An agent that takes neither is asked as it is, and
/// ignores a `_meta` it does not know, as ACP requires. The session's model is still chosen
/// afterwards, by the scorer, as for any adviser.
pub fn lean(
    launch: &mut AgentConfig,
    workspace: &Path,
    instruction: &str,
    lean: &LeanConfig,
) -> Option<Meta> {
    let mut file: Option<String> = None;
    for (key, template) in &lean.env {
        let value = if template.contains("{instruction_file}") {
            let path = match &file {
                Some(path) => path.clone(),
                None => {
                    let path = workspace.join("instructions.md");
                    // Without the file the variable would point at nothing: better not set.
                    if std::fs::write(&path, instruction).is_err() {
                        continue;
                    }
                    file.insert(path.to_string_lossy().into_owned()).clone()
                }
            };
            template.replace("{instruction_file}", &path)
        } else {
            template.clone()
        };
        let value = match (
            launch
                .env
                .get(key)
                .and_then(|text| serde_json::from_str::<Map<String, Value>>(text).ok()),
            serde_json::from_str::<Map<String, Value>>(&value).ok(),
        ) {
            (Some(mut existing), Some(added)) => {
                existing.extend(added);
                Value::Object(existing).to_string()
            }
            _ => value,
        };
        launch.env.insert(key.clone(), value);
    }
    match lean.meta.as_ref().map(|meta| fill(meta, instruction))? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}
fn fill(value: &Value, instruction: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace("{instruction}", instruction)),
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, instruction)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), fill(v, instruction)))
                .collect(),
        ),
        other => other.clone(),
    }
}
/// `_meta` on an ACP request: a JSON object.
pub type Meta = Map<String, Value>;

/// Whether this agent is a native CLI driven through one of the pinned ACP bridges.
pub fn bridged(agent: &AgentConfig) -> bool {
    bridge(agent).is_some()
}

fn bridge(agent: &AgentConfig) -> Option<Bridge> {
    // Existing custom ACP commands/arguments remain authoritative. Native commands
    // may also be configured by absolute path or under a custom agent ID.
    BRIDGES.iter().find_map(|(provider, bridge)| {
        (agent.provider == *provider
            && agent.args.is_empty()
            && (agent.command == bridge.binary
                || Path::new(&agent.command)
                    .file_name()
                    .is_some_and(|n| n == bridge.native)))
        .then_some(*bridge)
    })
}

/// Where this agent's executables are looked for: a `PATH` it was configured with, else
/// `setup::search_path` — Orochi's own toolchain, the process's `PATH`, and what a window
/// opened from Finder was never given.
fn effective_path(agent: &AgentConfig, data: &Path) -> OsString {
    agent
        .env
        .get("PATH")
        .map(OsString::from)
        .unwrap_or_else(|| crate::setup::search_path(Some(data)))
}
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
fn find(command: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    let command = Path::new(command);
    if command.is_absolute() {
        return executable(command).then(|| command.to_path_buf());
    }
    if command.components().count() != 1 {
        return None;
    }
    // Never auto-run a relative PATH entry from an untrusted target repository.
    std::env::split_paths(path)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(command))
        .find(|p| executable(p))
}

/// What to run for this agent inside a project's sandbox, where the golden image installed the
/// pinned ACP bridges on its `PATH`: a native CLI becomes its bridge, and a path on this machine
/// becomes the bare name it has there.
pub fn inside(agent: &AgentConfig) -> (String, Vec<String>) {
    if let Some(bridge) = bridge(agent) {
        return (bridge.binary.to_owned(), vec![]);
    }
    let command = Path::new(&agent.command)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| agent.command.clone());
    (command, agent.args.clone())
}

/// The pinned bridges, as `package@version`, for the image that runs them.
pub fn bridge_packages() -> Vec<String> {
    BRIDGES.iter().map(|(_, b)| b.spec()).collect()
}

/// An MCP server's command, resolved the way an agent's is: an absolute path as given, a bare
/// name from an absolute `PATH` entry, nothing else. ACP asks for an absolute path.
pub fn locate(command: &str) -> Option<PathBuf> {
    find(command, &crate::setup::search_path(None))
}
/// The same lookup on a given `PATH`.
pub fn find_in(command: &str, path: &std::ffi::OsStr) -> Option<PathBuf> {
    find(command, path)
}

#[derive(Debug, Serialize)]
pub struct Availability {
    pub installed: bool,
    pub status: &'static str,
    pub native_cli: Option<PathBuf>,
    pub executable: Option<PathBuf>,
    pub adapter_package: Option<String>,
    pub detail: Option<String>,
    #[serde(skip)]
    launch: Option<AgentConfig>,
}

pub fn inspect(agent: &AgentConfig, config: &DiscoveryConfig, data: &Path) -> Availability {
    let path = effective_path(agent, data);
    let bridge = bridge(agent);
    let native_cli = bridge.and_then(|b| {
        let command = agent
            .env
            .get(b.native_env)
            .map(String::as_str)
            .unwrap_or_else(|| {
                if Path::new(&agent.command)
                    .file_name()
                    .is_some_and(|n| n == b.native)
                {
                    &agent.command
                } else {
                    b.native
                }
            });
        find(command, &path)
    });
    let resolved = if let Some(b) = bridge {
        find(b.binary, &path).or_else(|| {
            let cached = b.binary_at(&b.directory(data));
            executable(&cached).then_some(cached)
        })
    } else {
        find(&agent.command, &path)
    };
    let mut status = if resolved.is_some() {
        "ready"
    } else if native_cli.is_some() {
        "adapter_required"
    } else {
        "not_installed"
    };
    let mut detail = None;
    if status == "not_installed" {
        detail = Some(match bridge {
            Some(b) => format!(
                "neither {} nor {} was found; configure an absolute path if the CLI is outside PATH",
                b.native, b.binary
            ),
            None => format!("executable not found: {}", agent.command),
        });
    } else if status == "adapter_required" {
        if !config.auto_install {
            status = "setup_disabled";
            detail = Some(
                "native CLI found, but adapter setup is disabled (discovery.auto_install = false)"
                    .into(),
            );
        } else if find("npm", &path).is_none() || find("node", &path).is_none() {
            status = "setup_unavailable";
            detail = Some(
                "native CLI found; Node.js 22+ and npm are required to prepare its ACP adapter"
                    .into(),
            );
        }
    }
    let launch = resolved.as_ref().map(|executable| {
        let mut launch = agent.clone();
        launch.command = executable.to_string_lossy().into_owned();
        // The bridges are `#!/usr/bin/env node` scripts: the process that runs one has to find
        // node where this lookup found it, which a window opened from Finder cannot otherwise.
        launch
            .env
            .entry("PATH".into())
            .or_insert_with(|| path.to_string_lossy().into_owned());
        if let (Some(b), Some(native)) = (bridge, &native_cli) {
            launch
                .env
                .entry(b.native_env.into())
                .or_insert_with(|| native.to_string_lossy().into_owned());
        }
        launch
    });
    Availability {
        installed: resolved.is_some() || native_cli.is_some(),
        status: if agent.enabled { status } else { "disabled" },
        native_cli,
        executable: resolved,
        adapter_package: bridge.map(Bridge::spec),
        detail,
        launch,
    }
}

pub async fn prepare(
    agent: &AgentConfig,
    config: &DiscoveryConfig,
    data: &Path,
) -> Result<AgentConfig, AgentError> {
    let availability = inspect(agent, config, data);
    if availability.status == "ready" {
        return Ok(availability.launch.expect("ready launch"));
    }
    if availability.status != "adapter_required" {
        return Err(AgentError::new(
            ErrorKind::Unavailable,
            availability
                .detail
                .unwrap_or_else(|| availability.status.into()),
        ));
    }
    let bridge = bridge(agent).expect("adapter required only for a native bridge");
    eprintln!(
        "Preparing {}: {} (native CLI detected)",
        agent.id,
        bridge.spec()
    );
    tokio::time::timeout(
        Duration::from_secs(config.setup_timeout_secs),
        install(bridge, agent, data),
    )
    .await
    .map_err(|_| {
        AgentError::new(
            ErrorKind::Timeout,
            format!(
                "{} adapter setup timed out; native CLI remains detected",
                agent.id
            ),
        )
    })?
    .map_err(|error| {
        AgentError::new(
            ErrorKind::Configuration,
            format!("{} adapter setup failed: {error:#}", agent.id),
        )
    })?;
    inspect(agent, config, data).launch.ok_or_else(|| {
        AgentError::new(
            ErrorKind::Configuration,
            "adapter installation did not produce an executable",
        )
    })
}

#[cfg_attr(not(unix), allow(dead_code))]
struct InstallProcess(Option<u32>);
impl Drop for InstallProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

async fn install(bridge: Bridge, agent: &AgentConfig, data: &Path) -> anyhow::Result<()> {
    let adapters = data.join("adapters");
    std::fs::create_dir_all(&adapters)?;
    // Serialize setup across repositories, reusing an atomic completed install.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(adapters.join(format!("{}-{}.lock", bridge.binary, bridge.version)))?;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    let destination = bridge.directory(data);
    if executable(&bridge.binary_at(&destination)) {
        return Ok(());
    }
    let stage = tempfile::Builder::new()
        .prefix(".setup-")
        .tempdir_in(&adapters)?;
    let npm = find("npm", &effective_path(agent, data))
        .ok_or_else(|| anyhow::anyhow!("npm not found"))?;
    let mut command = tokio::process::Command::new(npm);
    command
        .args(["install", "--prefix"])
        .arg(stage.path())
        .args(["--cache"])
        .arg(adapters.join("npm-cache"))
        .args([
            "--registry=https://registry.npmjs.org",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--save-exact",
            "--engine-strict",
        ])
        .arg(bridge.spec())
        .current_dir(stage.path())
        .env("PATH", effective_path(agent, data))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    let _process = InstallProcess(child.id());
    let output = child.wait_with_output().await?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        crate::context::bounded(&String::from_utf8_lossy(&output.stderr), 2000)
    );
    anyhow::ensure!(
        executable(&bridge.binary_at(stage.path())),
        "package did not provide {}",
        bridge.binary
    );
    // Only our own pinned cache location is replaced, never a global installation.
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(stage.path(), &destination)?;
    Ok(())
}
