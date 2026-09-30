//! Per-project Linux sandboxes: one Incus instance per project, with a Docker daemon of its
//! own, in which the project's agents and checks run (`docs/sandbox-design.md`).
//!
//! The project's tree stays where it is and is mounted into its sandbox **at the same absolute
//! path**, so nothing Orochi sends an agent (the session's `cwd`, attachment paths, file names
//! in a reply) needs translating, and switching a project between the sandbox and this machine
//! moves no file. Orochi itself stays on this machine: only the agent process and the
//! evaluator's checks are started inside, through `incus exec`, whose stdio carries ACP.
use crate::config::{AgentConfig, SandboxClient, SandboxConfig};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

pub mod jobs;
pub mod ops;

/// Where a project's agents run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// On this machine, as without a sandbox.
    #[default]
    Host,
    /// In an Incus system container (Docker inside by nesting).
    Container,
    /// In an Incus virtual machine.
    Vm,
}
impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Mode::Host => "host",
            Mode::Container => "container",
            Mode::Vm => "vm",
        }
    }
    pub fn sandboxed(self) -> bool {
        self != Mode::Host
    }
}

/// The user a sandbox runs agents as: this machine's own IDs, mapped one to one so what an
/// agent writes into the mounted tree belongs to the user here.
pub const HOME: &str = "/home/dev";

/// One project's sandbox, as remembered in `<data>/sandbox/projects.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Project {
    /// The Incus instance, a DNS label: also the project's name on the bridge (`<name>.sbx`).
    pub name: String,
    pub root: PathBuf,
    pub mode: Mode,
    /// The tier the instance was created as (`container` or `vm`); `mode` may be `host` while
    /// the instance is kept.
    pub tier: Mode,
    pub docker: bool,
    pub shadow: Vec<String>,
    /// Ports focus always offers, beside the ones found listening.
    pub ports: Vec<u16>,
    pub created_at: i64,
    pub last_used: i64,
}
impl Default for Project {
    fn default() -> Self {
        Self {
            name: String::new(),
            root: PathBuf::new(),
            mode: Mode::Host,
            tier: Mode::Container,
            docker: true,
            shadow: vec![],
            ports: vec![],
            created_at: 0,
            last_used: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub version: u32,
    pub projects: Vec<Project>,
    /// The one project whose ports appear at this machine's `127.0.0.1`, and those ports.
    pub focus: Option<Focus>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Focus {
    pub name: String,
    pub ports: Vec<u16>,
}

impl State {
    pub fn path(data: &Path) -> PathBuf {
        data.join("sandbox/projects.json")
    }
    pub fn load(data: &Path) -> Result<Self> {
        match std::fs::read(Self::path(data)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("unreadable {}", Self::path(data).display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                version: 1,
                ..Self::default()
            }),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, data: &Path) -> Result<()> {
        let path = Self::path(data);
        let dir = path.parent().expect("state has a parent");
        std::fs::create_dir_all(dir)?;
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(self)?)?;
        file.persist(&path)?;
        Ok(())
    }
    /// The project `root` lies in: the registered directory that is the longest ancestor of it,
    /// so a subdirectory runs in its project's sandbox.
    pub fn project_for(&self, root: &Path) -> Option<&Project> {
        self.projects
            .iter()
            .filter(|p| root.starts_with(&p.root))
            .max_by_key(|p| p.root.components().count())
    }
    pub fn exact(&self, root: &Path) -> Option<&Project> {
        self.projects.iter().find(|p| p.root == root)
    }
    pub fn exact_mut(&mut self, root: &Path) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.root == root)
    }
    pub fn named(&self, name: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.name == name)
    }
    /// A name for a new project's instance from its directory, `-2`… where it is taken.
    pub fn allocate(&self, root: &Path) -> String {
        let base = root
            .file_name()
            .and_then(|n| label(&n.to_string_lossy()))
            .unwrap_or_else(|| "project".into());
        let base: String = base.chars().take(56).collect();
        let base = base.trim_end_matches('-').to_owned();
        (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|candidate| self.named(candidate).is_none())
            .expect("an unused name")
    }
}

/// `s` as an Incus instance name and DNS label: lowercase letters, digits and `-`, starting
/// with a letter, at most 63 characters. `None` if nothing usable is left.
pub fn label(s: &str) -> Option<String> {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let out = if out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p-{out}")
    } else {
        out.to_owned()
    };
    let out: String = out.chars().take(63).collect();
    let out = out.trim_end_matches('-').to_owned();
    (!out.is_empty()).then_some(out)
}

/// A shadowed directory: a plain relative path inside the project.
pub fn shadow_ok(s: &str) -> bool {
    let path = Path::new(s);
    !s.is_empty()
        && path.is_relative()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// The bridge's gateway address and prefix, as `sandbox.subnet` writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    pub gateway: std::net::Ipv4Addr,
    pub prefix: u8,
}
impl Subnet {
    pub fn parse(s: &str) -> Option<Self> {
        let (address, prefix) = s.split_once('/')?;
        let gateway: std::net::Ipv4Addr = address.parse().ok()?;
        let prefix: u8 = prefix.parse().ok()?;
        let subnet = Self { gateway, prefix };
        ((8..=30).contains(&prefix) && subnet.network() != gateway).then_some(subnet)
    }
    pub fn network(self) -> std::net::Ipv4Addr {
        let mask = u32::MAX << (32 - self.prefix);
        std::net::Ipv4Addr::from(u32::from(self.gateway) & mask)
    }
    pub fn cidr(self) -> String {
        format!("{}/{}", self.network(), self.prefix)
    }
}

/// How Orochi reaches Incus.
pub struct Incus<'a> {
    pub config: &'a SandboxConfig,
}
impl<'a> Incus<'a> {
    pub fn new(config: &'a SandboxConfig) -> Self {
        Self { config }
    }
    /// The program and the arguments before `incus`'s own. A bare name is resolved the way an
    /// agent's is, never from a relative `PATH` entry.
    fn base(&self) -> Result<(PathBuf, Vec<String>)> {
        let (program, prefix) = match self.config.client {
            SandboxClient::Lima => (
                &self.config.limactl,
                vec![
                    "shell".into(),
                    "--workdir".into(),
                    "/".into(),
                    self.config.lima_instance.clone(),
                    // Joining `incus-admin` takes effect only at a new login, and Lima keeps
                    // its SSH connection open; the VM's user has password-less sudo.
                    "sudo".into(),
                    "incus".into(),
                ],
            ),
            SandboxClient::Incus => (&self.config.incus, vec![]),
        };
        let resolved =
            crate::discovery::locate(program).with_context(|| match self.config.client {
                SandboxClient::Lima => format!(
                    "{program} not found; install Lima (`brew install lima`) or set sandbox.limactl"
                ),
                SandboxClient::Incus => format!("{program} not found; set sandbox.incus"),
            })?;
        Ok((resolved, prefix))
    }
    /// An instance as Incus names it, with the remote where one is configured.
    pub fn instance(&self, name: &str) -> String {
        if self.config.remote.is_empty() || self.config.client == SandboxClient::Lima {
            name.to_owned()
        } else {
            format!("{}:{name}", self.config.remote)
        }
    }
    /// `incus --project <project> <args…>`, as a program and its arguments.
    pub fn argv(&self, args: &[String]) -> Result<(PathBuf, Vec<String>)> {
        let (program, mut argv) = self.base()?;
        argv.push("--project".into());
        argv.push(self.config.project.clone());
        argv.extend(args.iter().cloned());
        Ok((program, argv))
    }
    pub fn command(&self, args: &[String]) -> Result<std::process::Command> {
        let (program, argv) = self.argv(args)?;
        let mut command = std::process::Command::new(program);
        command.args(argv);
        Ok(command)
    }
    /// Runs and returns stdout; a failure carries what Incus said.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_with(args, None)
    }
    pub fn run_with(&self, args: &[&str], input: Option<&[u8]>) -> Result<String> {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let mut command = self.command(&args)?;
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("could not run incus {}", args.join(" ")))?;
        if let Some(input) = input {
            use std::io::Write;
            let mut stdin = child.stdin.take().expect("piped stdin");
            stdin.write_all(input)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            bail!(
                "incus {} failed: {}",
                args.first().map(String::as_str).unwrap_or(""),
                said.trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    /// Runs with this terminal attached (a shell, an image build's progress).
    pub fn attached(&self, args: &[String]) -> Result<bool> {
        Ok(self.command(args)?.status()?.success())
    }
    /// `RUNNING`, `STOPPED`…, or `None` for an instance that does not exist.
    pub fn status(&self, name: &str) -> Result<Option<String>> {
        let listed = self.run(&["list", &self.instance(name), "--format", "json"])?;
        let listed: Vec<serde_json::Value> = serde_json::from_str(&listed).unwrap_or_default();
        Ok(listed
            .iter()
            .find(|i| i["name"] == name)
            .and_then(|i| i["status"].as_str())
            .map(|s| s.to_ascii_uppercase()))
    }
}

/// Where one run's agents and checks execute: a project's running sandbox.
#[derive(Debug, Clone)]
pub struct Placement {
    pub name: String,
    pub root: PathBuf,
    pub mode: Mode,
    pub uid: u32,
    pub gid: u32,
}

/// This machine's user, mapped into every sandbox.
pub fn ids() -> (u32, u32) {
    #[cfg(unix)]
    unsafe {
        (libc::getuid(), libc::getgid())
    }
    #[cfg(not(unix))]
    (1000, 1000)
}

/// The placement for a run in `root`: its project's mode, or `force` where the run named one.
/// `None` runs on this machine. A mode named for a directory no sandbox was created for is an
/// error rather than a silent run here.
pub fn placement(data: &Path, root: &Path, force: Option<Mode>) -> Result<Option<Placement>> {
    let state = State::load(data)?;
    let project = state.project_for(root);
    let mode = match (force, project) {
        (Some(mode), _) if !mode.sandboxed() => return Ok(None),
        (Some(mode), Some(project)) => {
            ensure!(
                mode == project.tier,
                "{} has a {} sandbox; `orochi sandbox mode {} {}` recreates it as a {}",
                project.root.display(),
                project.tier.key(),
                project.root.display(),
                mode.key(),
                mode.key()
            );
            mode
        }
        (Some(mode), None) => bail!(
            "no sandbox for {}; create one with `orochi sandbox create --mode {}`",
            root.display(),
            mode.key()
        ),
        (None, Some(project)) => project.mode,
        (None, None) => return Ok(None),
    };
    let project = project.expect("a sandboxed mode has a project");
    if !mode.sandboxed() {
        return Ok(None);
    }
    let (uid, gid) = ids();
    Ok(Some(Placement {
        name: project.name.clone(),
        root: project.root.clone(),
        mode,
        uid,
        gid,
    }))
}

/// Variables that describe this machine and would mislead a process inside.
const LOCAL_ONLY: [&str; 6] = ["PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR"];

impl Placement {
    /// Starts the instance if it is stopped and marks the project used now.
    pub fn ready(&self, config: &SandboxConfig, data: &Path) -> Result<()> {
        let incus = Incus::new(config);
        match incus.status(&self.name)?.as_deref() {
            Some("RUNNING") => {}
            Some(_) => {
                incus.run(&["start", &incus.instance(&self.name)])?;
            }
            None => bail!(
                "the sandbox {} no longer exists; `orochi sandbox reset {}` recreates it",
                self.name,
                self.root.display()
            ),
        }
        let mut state = State::load(data)?;
        if let Some(project) = state.projects.iter_mut().find(|p| p.name == self.name) {
            project.last_used = crate::types::now();
            state.save(data)?;
        }
        Ok(())
    }

    /// `incus exec` arguments that run `command args…` as the mapped user in `cwd`.
    pub fn exec_args(
        &self,
        config: &SandboxConfig,
        cwd: &Path,
        env: &[(String, String)],
        command: &str,
        args: &[String],
        terminal: bool,
    ) -> Vec<String> {
        let incus = Incus::new(config);
        let mut argv = vec![
            "exec".to_owned(),
            incus.instance(&self.name),
            if terminal { "-t" } else { "-T" }.to_owned(),
            "--cwd".into(),
            cwd.to_string_lossy().into_owned(),
            "--user".into(),
            self.uid.to_string(),
            "--group".into(),
            self.gid.to_string(),
        ];
        let fixed = [
            ("HOME", HOME.to_owned()),
            ("USER", "dev".to_owned()),
            ("CLAUDE_CONFIG_DIR", format!("{HOME}/.claude")),
            ("CODEX_HOME", format!("{HOME}/.codex")),
            ("OROCHI_SANDBOX", self.name.clone()),
        ];
        for (key, value) in fixed
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .chain(
                env.iter()
                    .filter(|(k, _)| !LOCAL_ONLY.contains(&k.as_str()))
                    .cloned(),
            )
        {
            argv.push("--env".into());
            argv.push(format!("{key}={value}"));
        }
        argv.push("--".into());
        argv.push("sbx-run".into());
        argv.push(command.to_owned());
        argv.extend(args.iter().cloned());
        argv
    }

    /// The agent's launch, wrapped to start inside: the command becomes `incus exec` on this
    /// machine, and what the agent itself is resolved as comes from the image, not from here.
    pub fn launch(&self, config: &SandboxConfig, agent: &AgentConfig) -> Result<AgentConfig> {
        let (command, args) = crate::discovery::inside(agent);
        let env: Vec<(String, String)> = agent
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let inner = self.exec_args(config, &self.root, &env, &command, &args, false);
        let (program, argv) = Incus::new(config).argv(&inner)?;
        let mut launch = agent.clone();
        launch.command = program.to_string_lossy().into_owned();
        launch.args = argv;
        launch.env = Default::default();
        launch.sandboxed = true;
        Ok(launch)
    }

    /// A check or any other command, run inside instead of here.
    pub fn command(
        &self,
        config: &SandboxConfig,
        cwd: &Path,
        env: &[(String, String)],
        command: &str,
        args: &[String],
    ) -> Result<tokio::process::Command> {
        // The command does not read stdin; `sbx-run` watches the stdin the caller holds open
        // instead, so killing the caller ends it inside too.
        let mut env = env.to_vec();
        env.push(("SBX_HOLD".into(), "1".into()));
        let inner = self.exec_args(config, cwd, &env, command, args, false);
        let (program, argv) = Incus::new(config).argv(&inner)?;
        let mut command = tokio::process::Command::new(program);
        command.args(argv);
        Ok(command)
    }
}

/// Free space on the disk holding `path`, in GiB.
pub fn free_gib(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut probe = path.to_path_buf();
        while !probe.exists() {
            probe = probe.parent()?.to_path_buf();
        }
        let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64) >> 30)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// The directories the VM mounts: the configured ones, else the home directory.
pub fn mounts(config: &SandboxConfig) -> Vec<PathBuf> {
    if !config.mounts.is_empty() {
        return config.mounts.clone();
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .into_iter()
        .collect()
}
