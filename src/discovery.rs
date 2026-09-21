//! Resolve installed coding CLIs to ACP transports without requiring global adapters.
use crate::{
    agents::{AgentError, ErrorKind},
    config::{AgentConfig, DiscoveryConfig},
    types::Provider,
};
use serde::Serialize;
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
        Provider::Openai,
        Bridge {
            native: "codex",
            binary: "codex-acp",
            package: "@agentclientprotocol/codex-acp",
            version: "1.10.0",
            native_env: "CODEX_PATH",
        },
    ),
    (
        Provider::Anthropic,
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

fn effective_path(agent: &AgentConfig) -> OsString {
    agent
        .env
        .get("PATH")
        .map(OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default()
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

/// An MCP server's command, resolved the way an agent's is: an absolute path as given, a bare
/// name from an absolute `PATH` entry, nothing else. ACP asks for an absolute path.
pub fn locate(command: &str) -> Option<PathBuf> {
    find(command, &std::env::var_os("PATH").unwrap_or_default())
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
    let path = effective_path(agent);
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
    let npm =
        find("npm", &effective_path(agent)).ok_or_else(|| anyhow::anyhow!("npm not found"))?;
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
        .env("PATH", effective_path(agent))
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
