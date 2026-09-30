//! What `orochi sandbox …` does: the host VM, the golden image, and each project's instance.
use super::{Focus, Incus, Mode, Project, State, ids, label, mounts};
use crate::config::{SandboxClient, SandboxConfig};
use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

pub const LIMA_TEMPLATE: &str = include_str!("lima.yaml");
pub const HOST_SCRIPT: &str = include_str!("host.sh");
pub const IMAGE_SCRIPT: &str = include_str!("image.sh");
pub const GATEWAY_SCRIPT: &str = include_str!("gateway.py");
pub const IDLE_SCRIPT: &str = include_str!("idle.py");

/// Where a sandbox's HTTP service opens on this machine, through the VM's gateway: a
/// `*.localhost` name, which browsers and curl resolve to the loopback address themselves, so
/// nothing here is configured and no root is needed. `None` shows the pattern.
pub fn url(config: &SandboxConfig, project: &str, port: Option<u16>) -> String {
    let port = port.map_or_else(|| "<port>".to_owned(), |p| p.to_string());
    format!("http://{port}-{project}.localhost:{}", config.gateway_port)
}

/// The image a tier's instances start from.
pub fn image(tier: Mode) -> &'static str {
    match tier {
        Mode::Vm => "sbx-golden-vm",
        _ => "sbx-golden",
    }
}
const POOL: &str = "sbx-pool";

fn limactl(config: &SandboxConfig) -> Result<PathBuf> {
    crate::discovery::locate(&config.limactl).with_context(|| {
        format!(
            "{} not found; install Lima (`brew install lima`) or set sandbox.limactl",
            config.limactl
        )
    })
}

/// The Lima instance's state: `Running`, `Stopped`, or `None` where it does not exist.
pub fn vm_status(config: &SandboxConfig) -> Result<Option<String>> {
    let output = std::process::Command::new(limactl(config)?)
        .args(["list", "--json"])
        .stderr(Stdio::null())
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|vm| vm["name"] == config.lima_instance.as_str())
        .and_then(|vm| vm["status"].as_str().map(str::to_owned)))
}

/// Starts the VM if it stopped itself while unused, and waits until Incus answers. A no-op for
/// an Incus client, whose host is not this machine's to start. Runs arriving together start it
/// once: the start is serialized by a lock beside Lima's own files.
pub fn ensure_vm(config: &SandboxConfig, announce: bool) -> Result<()> {
    if config.client != SandboxClient::Lima {
        return Ok(());
    }
    if vm_status(config)?.as_deref() == Some("Running") {
        return Ok(());
    }
    let lock_path =
        std::env::temp_dir().join(format!("orochi-sandbox-{}.lock", config.lima_instance));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) };
    }
    match vm_status(config)?.as_deref() {
        Some("Running") => {}
        None => bail!(
            "the sandbox VM {} does not exist; `orochi sandbox up` creates it",
            config.lima_instance
        ),
        Some(_) => {
            if announce {
                eprintln!(
                    "Starting the sandbox VM {} (it stops itself when unused)…",
                    config.lima_instance
                );
            }
            let started = std::process::Command::new(limactl(config)?)
                .args(["start", "--tty=false", &config.lima_instance])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            ensure!(
                started.success(),
                "could not start the sandbox VM; `orochi sandbox up` says why"
            );
        }
    }
    let incus = Incus::new(config);
    for _ in 0..90 {
        if incus.run(&["project", "show", &config.project]).is_ok() {
            drop(lock);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    bail!("the sandbox VM started but Incus did not answer within 90 s")
}

/// The Lima template for this configuration.
pub fn lima_yaml(config: &SandboxConfig) -> String {
    let mounts = mounts(config)
        .iter()
        .map(|m| {
            format!(
                "- location: \"{}\"\n  mountPoint: \"{}\"\n  writable: true",
                m.display(),
                m.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    LIMA_TEMPLATE
        .replace("{{NESTED}}", "true")
        .replace("{{CPUS}}", &config.cpus.to_string())
        .replace("{{MEMORY}}", &config.memory_gib.to_string())
        .replace("{{DISK}}", &config.disk_gib.to_string())
        .replace("{{MOUNTS}}", &mounts)
}

fn host_env(config: &SandboxConfig) -> Vec<String> {
    let (uid, gid) = ids();
    vec![
        format!("SBX_PROJECT={}", config.project),
        format!("SBX_SUBNET={}", config.subnet),
        format!("SBX_POOL_GIB={}", config.pool_gib),
        format!("SBX_UID={uid}"),
        format!("SBX_GID={gid}"),
        format!("SBX_CPU={}", config.limits_cpu),
        format!("SBX_MEM_GIB={}", config.limits_memory_gib),
        format!("SBX_USER={}", std::env::var("USER").unwrap_or_default()),
        format!("SBX_GATEWAY_PORT={}", config.gateway_port),
        format!("SBX_VM_IDLE_MINUTES={}", config.vm_idle_minutes),
    ]
}

/// Creates or starts the host VM and (re)applies the Incus setup. With an `incus` client the
/// host is someone else's machine: only its reachability is checked.
pub fn up(config: &SandboxConfig, data: &Path) -> Result<()> {
    if config.client == SandboxClient::Incus {
        Incus::new(config).run(&["project", "show", &config.project])
            .context("the Incus host is not reachable or not prepared; run `orochi sandbox host-script | sudo bash` on it")?;
        println!("Incus host reachable; project {} ready", config.project);
        return Ok(());
    }
    let limactl = limactl(config)?;
    match vm_status(config)?.as_deref() {
        Some("Running") => println!("{} is running", config.lima_instance),
        Some(_) => {
            println!("Starting {}…", config.lima_instance);
            ensure!(
                std::process::Command::new(&limactl)
                    .args(["start", "--tty=false", &config.lima_instance])
                    .status()?
                    .success(),
                "limactl start failed"
            );
        }
        None => {
            let free = super::free_gib(&home());
            if let Some(free) = free {
                ensure!(
                    free >= config.min_free_gib + 5,
                    "only {free} GiB free; the sandbox VM needs about {} GiB to start. {}",
                    config.min_free_gib + 5,
                    reclaim_hint()
                );
            }
            let template = data
                .join("sandbox")
                .join(format!("{}.yaml", config.lima_instance));
            std::fs::create_dir_all(template.parent().expect("parent"))?;
            std::fs::write(&template, lima_yaml(config))?;
            println!(
                "Creating {} ({} CPU, {} GiB memory, {} GiB disk)…",
                config.lima_instance, config.cpus, config.memory_gib, config.disk_gib
            );
            ensure!(
                std::process::Command::new(&limactl)
                    .args(["start", "--tty=false", "--name", &config.lima_instance])
                    .arg(&template)
                    .status()?
                    .success(),
                "limactl start failed"
            );
        }
    }
    println!("Preparing Incus inside {}…", config.lima_instance);
    // The script goes in as a file and runs with stdin closed: `incus` reads a YAML
    // configuration from a stdin that is not a terminal, so a script piped into `bash -s`
    // was being swallowed by the first `incus network create` in it.
    let shell = |args: &[&str]| {
        let mut command = std::process::Command::new(&limactl);
        command.args(["shell", "--workdir", "/", &config.lima_instance]);
        command.args(args);
        command
    };
    for (path, content, what) in [
        (
            "/var/lib/orochi-sandbox-host.sh",
            HOST_SCRIPT,
            "setup script",
        ),
        (
            "/var/lib/orochi-sandbox-gateway.py",
            GATEWAY_SCRIPT,
            "gateway",
        ),
        ("/var/lib/orochi-sandbox-idle.py", IDLE_SCRIPT, "idle check"),
    ] {
        let mut copy = shell(&["sudo", "tee", path]);
        copy.stdin(Stdio::piped()).stdout(Stdio::null());
        let mut child = copy.spawn()?;
        {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("piped")
                .write_all(content.as_bytes())?;
        }
        ensure!(
            child.wait()?.success(),
            "could not copy the {what} into the VM"
        );
    }
    let env = host_env(config);
    let mut run: Vec<&str> = vec!["sudo", "env"];
    run.extend(env.iter().map(String::as_str));
    run.extend(["bash", "/var/lib/orochi-sandbox-host.sh"]);
    let mut child = shell(&run).stdin(Stdio::null()).spawn()?;
    ensure!(child.wait()?.success(), "the sandbox host setup failed");
    println!(
        "Sandbox host ready. Next: `orochi sandbox image build`. Services open at {} from this machine.",
        url(config, "<project>", None)
    );
    Ok(())
}

pub fn down(config: &SandboxConfig) -> Result<()> {
    ensure!(
        config.client == SandboxClient::Lima,
        "the Incus host is not this machine's to stop"
    );
    ensure!(
        std::process::Command::new(limactl(config)?)
            .args(["stop", &config.lima_instance])
            .status()?
            .success(),
        "limactl stop failed"
    );
    Ok(())
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn reclaim_hint() -> String {
    "Free space first; `limactl list` and `orb list` show VMs that may no longer be needed.".into()
}

/// Builds the golden image (and, for the container tier, the Docker volume every project's
/// Docker data is cloned from).
pub fn build_image(config: &SandboxConfig, tier: Mode) -> Result<()> {
    ensure_vm(config, true)?;
    ensure!(tier.sandboxed(), "an image is built for container or vm");
    let incus = Incus::new(config);
    let builder = format!("sbx-build-{}", tier.key());
    let (uid, gid) = ids();
    let _ = incus.run(&["delete", "--force", &incus.instance(&builder)]);
    let mut launch = vec![
        "init",
        "images:ubuntu/24.04",
        &builder,
        "--profile",
        "sbx-base",
    ];
    if tier == Mode::Vm {
        launch.push("--vm");
    } else {
        launch.extend(["--profile", "sbx-docker"]);
    }
    let instance = incus.instance(&builder);
    launch[2] = &instance;
    println!(
        "Building {} (this downloads Ubuntu, Docker, Node and Supabase's images)…",
        image(tier)
    );
    incus.run(&launch)?;
    if tier == Mode::Container {
        incus.run(&["config", "set", &instance, "raw.idmap", &idmap(uid, gid)])?;
    }
    incus.run(&["start", &instance])?;
    let bridges = crate::discovery::bridge_packages().join(" ");
    // In as a file, run with stdin closed: `apt`, `npm` and `incus` all read a stdin that is
    // not a terminal, and would eat a script piped into `bash -s`.
    incus.run_with(
        &[
            "exec",
            &instance,
            "-T",
            "--",
            "tee",
            "/root/orochi-image.sh",
        ],
        Some(IMAGE_SCRIPT.as_bytes()),
    )?;
    let script: Vec<String> = [
        "exec".to_owned(),
        instance.clone(),
        "-T".into(),
        "--env".into(),
        format!("SBX_UID={uid}"),
        "--env".into(),
        format!("SBX_GID={gid}"),
        "--env".into(),
        format!("SBX_BRIDGES={bridges}"),
        "--env".into(),
        "SBX_DOCKER=1".into(),
        "--".into(),
        "bash".into(),
        "/root/orochi-image.sh".into(),
    ]
    .into();
    let mut child = incus.command(&script)?.stdin(Stdio::null()).spawn()?;
    ensure!(child.wait()?.success(), "the image build failed");
    incus.run(&["stop", &instance])?;
    incus.run(&["publish", &instance, "--alias", image(tier), "--reuse"])?;
    incus.run(&["delete", &instance])?;
    // The first instance made from an image unpacks it into the pool (measured: 87 s, then
    // 4.2 s for the next); doing that here keeps it off the first project's creation.
    let warm = incus.instance(&format!("sbx-warm-{}", tier.key()));
    let mut init = vec!["init", image(tier), &warm, "--profile", "sbx-base"];
    if tier == Mode::Vm {
        init.push("--vm");
    }
    if incus.run(&init).is_ok() {
        let _ = incus.run(&["delete", &warm]);
    }
    println!(
        "Built {}. `orochi sandbox reset <path>` moves an existing sandbox onto it.",
        image(tier)
    );
    Ok(())
}

fn idmap(uid: u32, gid: u32) -> String {
    format!("uid {uid} {uid}\ngid {gid} {gid}")
}

fn volume_shadow(name: &str, index: usize) -> String {
    format!("shadow-{name}-{index}")
}

pub struct Create {
    pub mode: Mode,
    pub docker: bool,
    pub shadow: Vec<String>,
    pub ports: Vec<u16>,
}

/// Creates a project's sandbox and records it. The project's tree is mounted, never copied.
pub fn create(
    config: &SandboxConfig,
    data: &Path,
    root: &Path,
    request: Create,
) -> Result<Project> {
    ensure_vm(config, true)?;
    ensure!(
        request.mode.sandboxed(),
        "a sandbox is created as container or vm; `host` needs none"
    );
    let mut state = State::load(data)?;
    ensure!(
        state.exact(root).is_none(),
        "{} already has a sandbox; `orochi sandbox mode` or `reset` changes it",
        root.display()
    );
    if config.client == SandboxClient::Lima {
        let mounted = mounts(config);
        ensure!(
            mounted.iter().any(|m| root.starts_with(m)),
            "{} is outside what the sandbox VM mounts ({}); add its parent to sandbox.mounts and recreate the VM",
            root.display(),
            mounted
                .iter()
                .map(|m| m.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Some(free) = super::free_gib(&home()) {
            ensure!(
                free >= config.min_free_gib,
                "only {free} GiB free (sandbox.min_free_gib = {}). {}",
                config.min_free_gib,
                reclaim_hint()
            );
        }
    }
    let incus = Incus::new(config);
    incus
        .run(&["image", "show", image(request.mode)])
        .with_context(|| {
            format!(
                "no {} image; run `orochi sandbox up` and `orochi sandbox image build{}` first",
                image(request.mode),
                if request.mode == Mode::Vm {
                    " --vm"
                } else {
                    ""
                }
            )
        })?;
    let name = state.allocate(root);
    let project = Project {
        name,
        root: root.to_path_buf(),
        mode: request.mode,
        tier: request.mode,
        docker: request.docker,
        shadow: request.shadow,
        ports: request.ports,
        created_at: crate::types::now(),
        last_used: crate::types::now(),
    };
    provision(config, &project)?;
    state.projects.push(project.clone());
    state.save(data)?;
    Ok(project)
}

/// Makes the instance for a recorded project: from the tier's image, with its volumes.
fn provision(config: &SandboxConfig, project: &Project) -> Result<()> {
    let incus = Incus::new(config);
    let instance = incus.instance(&project.name);
    let root = project.root.to_string_lossy().into_owned();
    let (uid, gid) = ids();
    let container = project.tier == Mode::Container;
    let mut init = vec![
        "init",
        image(project.tier),
        &instance,
        "--profile",
        "sbx-base",
    ];
    if container && project.docker {
        init.extend(["--profile", "sbx-docker"]);
    }
    if !container {
        init.push("--vm");
    }
    println!("Creating sandbox {} for {root}…", project.name);
    incus.run(&init)?;
    let result = (|| -> Result<()> {
        if container {
            incus.run(&["config", "set", &instance, "raw.idmap", &idmap(uid, gid)])?;
        }
        incus.run(&[
            "config",
            "device",
            "add",
            &instance,
            "work",
            "disk",
            &format!("source={root}"),
            &format!("path={root}"),
        ])?;
        for (device, source, path) in [
            (
                "claude",
                "/var/lib/sbx/creds/claude",
                format!("{}/.claude", super::HOME),
            ),
            (
                "codex",
                "/var/lib/sbx/creds/codex",
                format!("{}/.codex", super::HOME),
            ),
        ] {
            incus.run(&[
                "config",
                "device",
                "add",
                &instance,
                device,
                "disk",
                &format!("source={source}"),
                &format!("path={path}"),
            ])?;
        }
        // Docker's data (images under /var/lib/containerd since Docker 29) stays on the root
        // disk, a ZFS clone of the golden image: the pre-pulled images are shared copy-on-write
        // and a snapshot of the instance covers them. The quota bounds that disk.
        incus.run(&[
            "config",
            "device",
            "override",
            &instance,
            "root",
            &format!("size={}GiB", config.quota_gib),
        ])?;
        if container {
            for (index, dir) in project.shadow.iter().enumerate() {
                // The mountpoint lives in the user's tree (the same path here); making it
                // here rather than letting Incus do it keeps it owned by the user.
                std::fs::create_dir_all(project.root.join(dir))?;
                let volume = volume_shadow(&project.name, index);
                incus.run(&["storage", "volume", "create", POOL, &volume])?;
                incus.run(&[
                    "config",
                    "device",
                    "add",
                    &instance,
                    &format!("shadow-{index}"),
                    "disk",
                    &format!("pool={POOL}"),
                    &format!("source={volume}"),
                    &format!("path={}", project.root.join(dir).display()),
                ])?;
            }
        }
        incus.run(&["start", &instance])?;
        let mut owned: Vec<String> = project
            .shadow
            .iter()
            .filter(|_| container)
            .map(|dir| project.root.join(dir).to_string_lossy().into_owned())
            .collect();
        if !owned.is_empty() {
            let mut args = vec![
                "exec".to_owned(),
                instance.clone(),
                "-T".into(),
                "--".into(),
                "chown".into(),
                format!("{uid}:{gid}"),
            ];
            args.append(&mut owned);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            incus.run(&args)?;
        }
        if !project.docker {
            let _ = incus.run(&[
                "exec",
                &instance,
                "-T",
                "--",
                "systemctl",
                "mask",
                "--now",
                "docker.service",
                "docker.socket",
            ]);
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = destroy(config, project);
        return Err(error);
    }
    Ok(())
}

/// Deletes the instance and its volumes; the project's tree is not touched.
fn destroy(config: &SandboxConfig, project: &Project) -> Result<()> {
    let incus = Incus::new(config);
    if incus.status(&project.name)?.is_some() {
        incus.run(&["delete", "--force", &incus.instance(&project.name)])?;
    }
    for volume in (0..project.shadow.len()).map(|i| volume_shadow(&project.name, i)) {
        if incus
            .run(&["storage", "volume", "show", POOL, &volume])
            .is_ok()
        {
            incus.run(&["storage", "volume", "delete", POOL, &volume])?;
        }
    }
    Ok(())
}

pub fn remove(config: &SandboxConfig, data: &Path, root: &Path) -> Result<Project> {
    ensure_vm(config, true)?;
    let mut state = State::load(data)?;
    let project = state
        .exact(root)
        .cloned()
        .with_context(|| format!("no sandbox for {}", root.display()))?;
    unfocus_if(config, &mut state, &project.name)?;
    destroy(config, &project)?;
    state.projects.retain(|p| p.root != root);
    state.save(data)?;
    Ok(project)
}

/// Recreates the instance from the current image, keeping what was chosen for it.
pub fn reset(
    config: &SandboxConfig,
    data: &Path,
    root: &Path,
    tier: Option<Mode>,
) -> Result<Project> {
    ensure_vm(config, true)?;
    let mut state = State::load(data)?;
    let mut project = state
        .exact(root)
        .cloned()
        .with_context(|| format!("no sandbox for {}", root.display()))?;
    unfocus_if(config, &mut state, &project.name)?;
    destroy(config, &project)?;
    if let Some(tier) = tier {
        project.tier = tier;
        project.mode = tier;
    }
    provision(config, &project)?;
    if let Some(slot) = state.exact_mut(root) {
        *slot = project.clone();
    }
    state.save(data)?;
    Ok(project)
}

/// Where the project's agents run from now on. Moving to a tier the instance is not recreates
/// it (the caller confirms); moving to `host` stops it and keeps it.
pub fn set_mode(config: &SandboxConfig, data: &Path, root: &Path, mode: Mode) -> Result<Project> {
    ensure_vm(config, true)?;
    let mut state = State::load(data)?;
    let project = state.exact(root).cloned().with_context(|| {
        format!(
            "no sandbox for {}; `orochi sandbox create` makes one",
            root.display()
        )
    })?;
    if mode.sandboxed() && mode != project.tier {
        return reset(config, data, root, Some(mode));
    }
    let incus = Incus::new(config);
    if !mode.sandboxed() {
        unfocus_if(config, &mut state, &project.name)?;
        if incus.status(&project.name)?.as_deref() == Some("RUNNING") {
            incus.run(&["stop", &incus.instance(&project.name)])?;
        }
    }
    let slot = state.exact_mut(root).expect("present");
    slot.mode = mode;
    let project = slot.clone();
    state.save(data)?;
    Ok(project)
}

fn default_snapshot() -> String {
    format!("s{}", crate::types::now())
}

pub fn snapshot(
    config: &SandboxConfig,
    data: &Path,
    root: &Path,
    name: Option<String>,
) -> Result<String> {
    ensure_vm(config, true)?;
    let project = recorded(data, root)?;
    let incus = Incus::new(config);
    let snap = name.unwrap_or_else(default_snapshot);
    ensure!(
        label(&snap).as_deref() == Some(snap.as_str()),
        "a snapshot name is a lowercase label"
    );
    incus.run(&["snapshot", "create", &incus.instance(&project.name), &snap])?;
    Ok(snap)
}

pub fn restore(config: &SandboxConfig, data: &Path, root: &Path, snap: &str) -> Result<()> {
    ensure_vm(config, true)?;
    let project = recorded(data, root)?;
    let incus = Incus::new(config);
    let instance = incus.instance(&project.name);
    let running = incus.status(&project.name)?.as_deref() == Some("RUNNING");
    if running {
        incus.run(&["stop", &instance])?;
    }
    incus.run(&["snapshot", "restore", &instance, snap])?;
    if running {
        incus.run(&["start", &instance])?;
    }
    Ok(())
}

fn recorded(data: &Path, root: &Path) -> Result<Project> {
    State::load(data)?
        .exact(root)
        .cloned()
        .with_context(|| format!("no sandbox for {}", root.display()))
}

/// TCP ports something inside listens on, from `ss`, above the privileged range.
pub fn listening(config: &SandboxConfig, name: &str) -> Result<Vec<u16>> {
    ensure_vm(config, true)?;
    let incus = Incus::new(config);
    let output = incus.run(&["exec", &incus.instance(name), "-T", "--", "ss", "-ltnH"])?;
    Ok(parse_listening(&output))
}

pub fn parse_listening(ss: &str) -> Vec<u16> {
    let mut ports: Vec<u16> = ss
        .lines()
        .filter_map(|line| line.split_whitespace().nth(3))
        .filter_map(|local| local.rsplit_once(':').and_then(|(_, p)| p.parse().ok()))
        .filter(|port| *port >= 1024)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

const PORT_DEVICE: &str = "sbx-port-";

/// The proxy devices focus added to an instance, as ports.
fn focused_devices(config: &SandboxConfig, name: &str) -> Result<Vec<u16>> {
    let incus = Incus::new(config);
    let devices = incus.run(&["config", "device", "list", &incus.instance(name)])?;
    Ok(devices
        .lines()
        .filter_map(|d| d.trim().strip_prefix(PORT_DEVICE))
        .filter_map(|p| p.parse().ok())
        .collect())
}

pub struct Focused {
    pub added: Vec<u16>,
    pub kept: Vec<u16>,
    /// Ports already taken on this machine, which are left alone.
    pub taken: Vec<u16>,
}

/// Puts this project's ports at this machine's `127.0.0.1`, unfocusing any other project.
pub fn focus(config: &SandboxConfig, data: &Path, root: &Path) -> Result<Focused> {
    ensure_vm(config, true)?;
    let mut state = State::load(data)?;
    let project = state
        .project_for(root)
        .cloned()
        .with_context(|| format!("no sandbox for {}", root.display()))?;
    ensure!(
        project.mode.sandboxed(),
        "{} runs on this machine (mode host); its ports are already here",
        project.root.display()
    );
    if let Some(other) = state.focus.clone().filter(|f| f.name != project.name) {
        unfocus_if(config, &mut state, &other.name)?;
    }
    let incus = Incus::new(config);
    let instance = incus.instance(&project.name);
    let mut wanted = listening(config, &project.name)?;
    wanted.extend(project.ports.iter().copied());
    wanted.sort_unstable();
    wanted.dedup();
    let present = focused_devices(config, &project.name)?;
    let mut focused = Focused {
        added: vec![],
        kept: vec![],
        taken: vec![],
    };
    for port in &present {
        if !wanted.contains(port) {
            incus.run(&[
                "config",
                "device",
                "remove",
                &instance,
                &format!("{PORT_DEVICE}{port}"),
            ])?;
        }
    }
    for port in wanted {
        if present.contains(&port) {
            focused.kept.push(port);
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
            focused.taken.push(port);
            continue;
        }
        incus.run(&[
            "config",
            "device",
            "add",
            &instance,
            &format!("{PORT_DEVICE}{port}"),
            "proxy",
            &format!("listen=tcp:127.0.0.1:{port}"),
            &format!("connect=tcp:127.0.0.1:{port}"),
            "bind=host",
        ])?;
        focused.added.push(port);
    }
    let mut ports: Vec<u16> = focused.added.iter().chain(&focused.kept).copied().collect();
    ports.sort_unstable();
    state.focus = Some(Focus {
        name: project.name.clone(),
        ports,
    });
    state.save(data)?;
    Ok(focused)
}

pub fn unfocus(config: &SandboxConfig, data: &Path) -> Result<Option<String>> {
    let mut state = State::load(data)?;
    let Some(current) = state.focus.clone() else {
        return Ok(None);
    };
    ensure_vm(config, true)?;
    unfocus_if(config, &mut state, &current.name)?;
    state.save(data)?;
    Ok(Some(current.name))
}

fn unfocus_if(config: &SandboxConfig, state: &mut State, name: &str) -> Result<()> {
    if state.focus.as_ref().is_none_or(|f| f.name != name) {
        return Ok(());
    }
    let incus = Incus::new(config);
    if incus.status(name)?.is_some() {
        for port in focused_devices(config, name)? {
            incus.run(&[
                "config",
                "device",
                "remove",
                &incus.instance(name),
                &format!("{PORT_DEVICE}{port}"),
            ])?;
        }
    }
    state.focus = None;
    Ok(())
}

/// Stops sandboxes nobody used for `idle_stop_minutes`, never the focused one.
pub fn gc(config: &SandboxConfig, data: &Path) -> Result<Vec<String>> {
    if config.client == SandboxClient::Lima && vm_status(config)?.as_deref() != Some("Running") {
        return Ok(vec![]);
    }
    let state = State::load(data)?;
    let incus = Incus::new(config);
    let cutoff = crate::types::now() - (config.idle_stop_minutes as i64) * 60;
    let mut stopped = vec![];
    for project in &state.projects {
        if state.focus.as_ref().is_some_and(|f| f.name == project.name)
            || project.last_used > cutoff
        {
            continue;
        }
        if incus.status(&project.name)?.as_deref() == Some("RUNNING") {
            incus.run(&["stop", &incus.instance(&project.name)])?;
            stopped.push(project.name.clone());
        }
    }
    Ok(stopped)
}

/// Every recorded project with its instance's state, for `status`.
pub fn statuses(
    config: &SandboxConfig,
    data: &Path,
) -> Result<Vec<(Project, String, Option<String>)>> {
    let state = State::load(data)?;
    let incus = Incus::new(config);
    // A VM that stopped itself while unused is not asked (asking would start nothing, and
    // looking should not start it): every sandbox in it is stopped.
    if config.client == SandboxClient::Lima && vm_status(config)?.as_deref() != Some("Running") {
        return Ok(state
            .projects
            .iter()
            .map(|p| (p.clone(), "stopped".to_owned(), None))
            .collect());
    }
    // A host that cannot be reached says so, rather than every sandbox reading as gone.
    let (listed, missing) = match incus.run(&["list", "--format", "json"]) {
        Ok(listed) => (listed, "missing"),
        Err(_) => ("[]".into(), "unreachable"),
    };
    let listed: Vec<serde_json::Value> = serde_json::from_str(&listed).unwrap_or_default();
    Ok(state
        .projects
        .iter()
        .map(|project| {
            let instance = listed.iter().find(|i| i["name"] == project.name.as_str());
            let status = instance
                .and_then(|i| i["status"].as_str())
                .unwrap_or(missing)
                .to_ascii_lowercase();
            let address = instance.and_then(|i| {
                i["state"]["network"]["eth0"]["addresses"]
                    .as_array()?
                    .iter()
                    .find(|a| a["family"] == "inet")
                    .and_then(|a| a["address"].as_str().map(str::to_owned))
            });
            (project.clone(), status, address)
        })
        .collect())
}
