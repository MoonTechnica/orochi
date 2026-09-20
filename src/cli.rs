use crate::{
    config::{Config, Paths, PermissionMode},
    policy::Registry,
    scheduler::{self, RunOptions},
    storage::{Store, workspace_lock},
    types::{Overrides, now},
};
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{IsTerminal, Write},
    path::PathBuf,
    time::Duration,
};

#[derive(Debug, Parser)]
#[command(
    name = "orochi",
    version,
    about = "Adaptive agent and model scheduler for ACP coding agents",
    subcommand_precedence_over_arg = true
)]
pub struct Cli {
    /// Task to execute. Quote the entire task as one argument.
    pub task: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
    /// User configuration file (never read from the target repository automatically).
    #[arg(long, global = true, env = "OROCHI_CONFIG")]
    pub config: Option<PathBuf>,
    #[arg(long, global = true, env = "OROCHI_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
    #[arg(short = 'C', long, global = true, default_value = ".")]
    pub cwd: PathBuf,
    /// JSON output for inspection commands or --dry-run.
    #[arg(long, global = true)]
    pub json: bool,
    /// Shortcut for `orochi status` (local agent inventory and saved runtime state).
    #[arg(long, conflicts_with_all = ["task", "agent", "model", "reasoning", "mode", "dry_run", "resume", "permission", "no_eval"])]
    pub status: bool,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub reasoning: Option<String>,
    #[arg(long)]
    pub mode: Option<String>,
    /// Discover and score candidates without sending a task prompt.
    #[arg(long)]
    pub dry_run: bool,
    /// Resume a locally recorded ACP session belonging to this repository (without a task: in a conversation).
    #[arg(long)]
    pub resume: Option<String>,
    /// Continue the most recent recorded session in this directory.
    #[arg(short = 'c', long = "continue", conflicts_with = "resume")]
    pub continue_last: bool,
    /// How to answer ACP permission requests; defaults to ask (denies without a TTY).
    #[arg(long, value_enum)]
    pub permission: Option<PermissionMode>,
    /// Allow every permission request once without asking (same as --permission allow).
    #[arg(long, visible_alias = "yolo", conflicts_with = "permission")]
    pub always_approve: bool,
    #[arg(long)]
    pub no_eval: bool,
    /// Ask configured council members to deliberate about routing in two rounds.
    #[arg(long)]
    pub council: bool,
    /// Name other agents see for this run in the repository mailbox.
    #[arg(long, global = true, env = "OROCHI_PEER_NAME")]
    pub peer_name: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Converse interactively; the default when `orochi` starts on a terminal without a task.
    Chat,
    /// Expose Orochi as an ACP v1 agent over stdin/stdout.
    Serve,
    /// Implement, review and integrate through independent ACP sessions.
    Collaborate {
        task: String,
        /// Team to run. Omitted: derived from the task, as `--dry-run` prints it.
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long, required_unless_present = "dry_run")]
        output: Option<PathBuf>,
        /// Merge a verified result into the working tree, resolving conflicts in a session.
        #[arg(long)]
        apply: bool,
        /// Print the team that would run, as a plan file, and stop.
        #[arg(long)]
        dry_run: bool,
    },
    /// Continue a saved collaboration without repeating completed stages.
    CollaborateResume {
        #[arg(long)]
        output: PathBuf,
        /// Also apply a verified result to the working tree (may follow a finished run).
        #[arg(long)]
        apply: bool,
    },
    /// List all agents, readiness, saved quota, and the last execution route.
    Status {
        /// Verify ACP connections and list current models (may prepare adapters).
        #[arg(long)]
        discover: bool,
    },
    /// Show configured adapters; --discover starts their ACP processes.
    Agents {
        #[arg(long)]
        discover: bool,
    },
    /// List agents Orochi is running in this repository (all worktrees) and their messages.
    Peers {
        #[arg(long)]
        messages: bool,
    },
    /// Read the conversation store: the same views a desktop client renders.
    Threads {
        #[command(subcommand)]
        command: Option<ThreadCommand>,
        /// Include archived threads.
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
    /// List sessions recorded for this repository.
    Sessions {
        #[arg(long)]
        all: bool,
    },
    /// Inspect local execution telemetry.
    Runs {
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
    /// Ingest the official Claude Code statusline JSON from stdin (only quota fields are saved).
    QuotaIngest {
        #[arg(long, default_value = "claude")]
        agent: String,
    },
    /// Fetch quota through configured read-only CLI probes.
    Quota {
        #[arg(long)]
        refresh: bool,
    },
    /// Show what Orochi remembers about you and this repository.
    Memory {
        /// Remove one item by its id (`u1`, `r3`) as the listing shows it.
        #[arg(long)]
        forget: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compare frozen predictions with local observed outcomes.
    Calibrate {
        #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u32).range(1..=100000))]
        limit: u32,
    },
    /// Replay paired measured outcomes without contacting agents or changing learning history.
    Benchmark {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value_t = 100000.0)]
        failure_penalty: f64,
    },
    /// Tune learning coefficients on a training prefix and report a held-out suffix.
    BenchmarkTune {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        training_cases: usize,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value_t = 100000.0)]
        failure_penalty: f64,
    },
    #[command(subcommand)]
    Config(ConfigCommand),
    #[command(subcommand)]
    Policy(PolicyCommand),
}
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Init,
    Show,
    Path,
}
#[derive(Debug, Subcommand)]
pub enum PolicyCommand {
    Status,
    /// Install validated bundled policies, a local registry, or a pinned HTTPS registry.
    Update {
        #[arg(long, conflicts_with = "url")]
        from: Option<PathBuf>,
        #[arg(long, requires = "sha256", conflicts_with = "from")]
        url: Option<String>,
        #[arg(long, requires = "url")]
        sha256: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ThreadCommand {
    /// Show one thread's timeline, seats and changed files.
    Show { id: String },
    /// Delete a thread and everything under it. This cannot be undone.
    Delete {
        id: String,
        /// Delete every thread instead of one.
        #[arg(long, conflicts_with = "id")]
        all: bool,
    },
}

pub async fn execute(mut cli: Cli) -> Result<u8> {
    // Counted from the start, so an interrupt is never lost between two waits.
    crate::interrupt::listen().await;
    if cli.always_approve {
        cli.permission = Some(PermissionMode::Allow);
    }
    if cli.status {
        ensure!(
            cli.command.is_none(),
            "--status cannot be combined with a subcommand"
        );
        cli.command = Some(Command::Status { discover: false });
    }
    ensure!(
        cli.task.is_none() || cli.command.is_none(),
        "a task and subcommand cannot be used together"
    );
    if cli.command.is_none()
        && cli.task.is_none()
        && !cli.dry_run
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
    {
        cli.command = Some(Command::Chat);
    }
    let paths = Paths::resolve(cli.config, cli.data_dir)?;
    if let Some(Command::Config(command)) = &cli.command {
        match command {
            ConfigCommand::Path => println!("{}", paths.config.display()),
            ConfigCommand::Init => {
                ensure!(
                    !paths.config.exists(),
                    "config already exists: {}",
                    paths.config.display()
                );
                if let Some(parent) = paths.config.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&paths.config)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                file.write_all(toml::to_string_pretty(&Config::default())?.as_bytes())?;
                println!("Created {}", paths.config.display());
            }
            ConfigCommand::Show => {
                let mut config = Config::load(&paths.config)?;
                for agent in &mut config.agents {
                    for value in agent.env.values_mut() {
                        *value = "[redacted]".into();
                    }
                }
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&config)?);
                } else {
                    println!("{}", toml::to_string_pretty(&config)?);
                }
            }
        }
        return Ok(0);
    }
    if let Some(Command::Policy(command)) = &cli.command {
        match command {
            PolicyCommand::Status => {
                let registry = Registry::load(&paths.data)?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&registry)?);
                } else {
                    for p in registry.policies {
                        println!("{}  v{}  updated {}", p.provider, p.version, p.updated_at);
                        for source in p.source {
                            println!("  {source}");
                        }
                    }
                }
            }
            PolicyCommand::Update { from, url, sha256 } => {
                let registry: Registry = if let Some(path) = from {
                    if path.is_dir() {
                        let policies = ["openai", "anthropic", "google"]
                            .iter()
                            .map(|name| {
                                let bytes = std::fs::read(path.join(format!("{name}.json")))?;
                                ensure!(bytes.len() <= 1_048_576, "policy file too large");
                                Ok(serde_json::from_slice(&bytes)?)
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Registry {
                            schema_version: 1,
                            policies,
                        }
                    } else {
                        ensure!(path.metadata()?.len() <= 1_048_576, "policy file too large");
                        serde_json::from_slice(&std::fs::read(path)?)?
                    }
                } else if let Some(url) = url {
                    let url = reqwest::Url::parse(url)?;
                    ensure!(
                        url.scheme() == "https"
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "policy URL must be HTTPS without credentials"
                    );
                    let expected = sha256
                        .as_deref()
                        .context("remote policy requires --sha256")?;
                    ensure!(
                        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
                        "invalid SHA-256 digest"
                    );
                    let client = reqwest::Client::builder()
                        .timeout(Duration::from_secs(20))
                        .redirect(reqwest::redirect::Policy::none())
                        .build()?;
                    let mut response = client.get(url).send().await?.error_for_status()?;
                    let mut bytes = Vec::new();
                    while let Some(chunk) = response.chunk().await? {
                        ensure!(
                            bytes.len() + chunk.len() <= 1_048_576,
                            "policy registry exceeds 1 MiB"
                        );
                        bytes.extend_from_slice(&chunk);
                    }
                    ensure!(
                        format!("{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(expected),
                        "policy checksum mismatch; previous registry preserved"
                    );
                    serde_json::from_slice(&bytes)?
                } else {
                    Registry::bundled()?
                };
                registry.validate()?;
                registry.install(&paths.data)?;
                println!(
                    "Installed policy registry: {}",
                    paths.data.join("policies.json").display()
                );
            }
        }
        return Ok(0);
    }
    let root = cli
        .cwd
        .canonicalize()
        .context("--cwd must be an existing directory")?;
    ensure!(root.is_dir(), "--cwd must be a directory");
    let mut config = Config::load(&paths.config)?;
    if cli.council {
        config.council.enabled = true;
        config.validate()?;
    }
    if cli.no_eval {
        config.evaluator.auto = false;
        config.evaluator.checks.clear();
    }
    if let Some(permission) = cli.permission {
        config.scheduler.permission = permission;
    }
    let store = Store::open(&paths.data)?;
    #[cfg(unix)]
    {
        let leases = paths.data.join("processes");
        crate::process::enable(std::env::current_exe()?, leases.clone())?;
        let reaped = crate::process::reap(&leases);
        if reaped > 0 {
            eprintln!(
                "Stopped {reaped} agent process(es) left running by an interrupted Orochi run"
            );
        }
    }
    if cli.continue_last {
        let repo = store.repository_id(&root)?;
        match store.sessions(Some(&repo))?.into_iter().next() {
            Some(session) => cli.resume = Some(session.session_id),
            None if cli.task.is_none() => {
                eprintln!("No earlier conversation in this directory; starting a new one.")
            }
            None => bail!("--continue found no recorded session in this repository"),
        }
    }
    let shared = config.scheduler.shared_workspace;
    let joins = matches!(
        cli.command,
        Some(Command::Chat | Command::Collaborate { .. } | Command::CollaborateResume { .. })
    ) || (cli.command.is_none() && cli.task.is_some() && !cli.dry_run);
    let _membership = if joins && config.mailbox.enabled {
        let membership = crate::mailbox::join(
            std::env::current_exe()?,
            &paths.data,
            &config.mailbox,
            &root,
            &store.salt()?,
            cli.peer_name.as_deref(),
        )?;
        if !matches!(cli.command, Some(Command::Chat))
            && let Some(name) = crate::mailbox::name_prefix()
        {
            eprintln!("Mailbox peer: {name}");
        }
        Some(membership)
    } else {
        None
    };
    match cli.command {
        Some(Command::Chat) => {
            ensure!(
                !cli.json && !cli.dry_run,
                "--json and --dry-run need a task"
            );
            check_overrides(&config, &cli.agent, &cli.model, &cli.reasoning)?;
            return crate::chat::run(
                &config,
                &paths.data,
                &store,
                &root,
                crate::chat::Options {
                    overrides: Overrides {
                        agent: cli.agent,
                        model: cli.model,
                        reasoning: cli.reasoning,
                        mode: cli.mode,
                    },
                    resume: cli.resume,
                    // The chat answers permission requests itself unless asked to stop and
                    // ask; Shift-Tab and /confirm change it while it runs.
                    permission: cli.permission.unwrap_or(match config.scheduler.permission {
                        PermissionMode::Ask => PermissionMode::Allow,
                        chosen => chosen,
                    }),
                },
            )
            .await;
        }
        Some(Command::Peers { messages }) => {
            let mailbox = crate::mailbox::Mailbox::open(&paths.data, &config.mailbox)?;
            let channel = crate::mailbox::channel(&root, &store.salt()?);
            let peers = mailbox.peers(&channel)?;
            let history = if messages {
                mailbox.history(&channel, 50)?
            } else {
                vec![]
            };
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"peers": peers, "messages": history}))?
                );
            } else {
                if peers.is_empty() {
                    println!("No Orochi agents are running in this repository.");
                }
                for peer in &peers {
                    println!(
                        "{:<24} {:<28} {}",
                        peer.name,
                        peer.route.as_deref().unwrap_or("-"),
                        peer.worktree
                    );
                    if !peer.status.is_empty() {
                        println!("  {}", peer.status);
                    }
                }
                for message in &history {
                    println!(
                        "\n[{}] {} -> {}\n{}",
                        message.id, message.from, message.to, message.body
                    );
                }
            }
        }
        Some(Command::Status { discover }) => {
            let mut capabilities = std::collections::BTreeMap::new();
            let mut failures = Vec::new();
            if discover {
                let repo = store.repository_id(&root)?;
                let _lock = workspace_lock(&paths.data, &repo, shared)?;
                let (mut clients, discovered_failures) = tokio::select! {
                    result = scheduler::discover(&config, &root, &store, None, PermissionMode::Deny) => result?,
                    _ = tokio::signal::ctrl_c() => return Ok(130),
                };
                failures = discovered_failures;
                for client in &mut clients {
                    capabilities.insert(
                        client.config.id.clone(),
                        serde_json::to_value(&client.capabilities)?,
                    );
                    client.stop().await;
                }
            }
            let runtime = store.runtime()?;
            let time = now();
            let mut agents = inventory(&config, &paths.data);
            for agent in &mut agents {
                let id = agent["agent"].as_str().expect("inventory agent ID");
                let failure = failures.iter().find(|failure| failure.agent == id);
                let capability = capabilities.get(id);
                let account = runtime.get(&(id.into(), "*".into()));
                let status = agent["availability"]["status"]
                    .as_str()
                    .unwrap_or("unknown");
                let status = if matches!(status, "ready" | "adapter_required") {
                    if account.is_some_and(|state| !state.available(time)) {
                        "cooldown"
                    } else if failure.is_some() {
                        "connection_failed"
                    } else if capability.is_some() {
                        "connected"
                    } else {
                        status
                    }
                } else {
                    status
                };
                let status = status.to_owned();
                agent["discovery_checked"] = json!(discover && agent["enabled"] == true);
                agent["capabilities"] = capability.cloned().unwrap_or(serde_json::Value::Null);
                agent["connection_error"] = json!(failure.map(|failure| &failure.error));
                agent["runtime_status"] = json!(account.map(|state| state.effective_status(time)));
                agent["status"] = json!(status);
            }
            let states: Vec<_> = runtime.values().map(|s| json!({"agent":s.agent,"model":s.model,"status":s.effective_status(now()),"quota_estimate":s.quota_estimate,"reset_at":s.reset_at,"cooldown_until":s.cooldown_until,"consecutive_failures":s.consecutive_failures})).collect();
            let last = store
                .recent_runs(100)?
                .into_iter()
                .find(|r| r.purpose == "execution");
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"runtime":states,"last_run":last,"configured_agents":agents,"discovered":discover,"discovery_failures":failures})
                    )?
                );
            } else {
                println!("Agents");
                println!("{:<16} {:<12} {:<20} CLI", "AGENT", "PROVIDER", "STATUS");
                for agent in &agents {
                    let availability = &agent["availability"];
                    println!(
                        "{:<16} {:<12} {:<20} {}",
                        agent["agent"].as_str().unwrap_or(""),
                        agent["provider"].as_str().unwrap_or(""),
                        agent["status"].as_str().unwrap_or(""),
                        availability["native_cli"]
                            .as_str()
                            .or(availability["executable"].as_str())
                            .or(agent["command"].as_str())
                            .unwrap_or("")
                    );
                    if let Some(detail) = agent["connection_error"]
                        .as_str()
                        .or(availability["detail"].as_str())
                    {
                        println!("  {detail}");
                    }
                    if let Some(models) = agent["capabilities"]["models"].as_array() {
                        println!(
                            "  Models: {}",
                            models
                                .iter()
                                .filter_map(|model| model["model"].as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
                if !discover {
                    println!(
                        "\nready = local executable found; adapter_required = automatic setup pending."
                    );
                    println!(
                        "Authentication and connections are unchecked. Run `orochi status --discover` to verify and list models."
                    );
                }
                if !runtime.is_empty() {
                    println!("\nSaved runtime state");
                    println!("{:<16} {:<28} {:<14} QUOTA", "AGENT", "MODEL", "STATUS");
                }
                for s in runtime.values() {
                    println!(
                        "{:<16} {:<28} {:<14} {}",
                        s.agent,
                        s.model,
                        serde_json::to_value(s.effective_status(now()))?
                            .as_str()
                            .unwrap_or("unknown"),
                        s.quota_estimate
                            .map(|q| format!("{:.0}% remaining", q * 100.0))
                            .unwrap_or_else(|| "unknown".into())
                    );
                }
                if let Some(last) = last {
                    println!(
                        "\nLast route: {} / {} / {} ({:?})",
                        last.candidate.agent,
                        last.candidate.model,
                        last.candidate
                            .reasoning_level
                            .as_deref()
                            .unwrap_or("agent-default"),
                        last.outcome
                    );
                    for reason in last.candidate.reasons {
                        println!("  - {reason}");
                    }
                }
            }
        }
        Some(Command::Agents { discover }) => {
            if discover {
                let repo = store.repository_id(&root)?;
                let _lock = workspace_lock(&paths.data, &repo, shared)?;
                let (clients, failures) = tokio::select! {
                    result = scheduler::discover(&config, &root, &store, None, PermissionMode::Deny) => result?,
                    _ = tokio::signal::ctrl_c() => return Ok(130),
                };
                let values: Vec<_> = clients.iter().map(|c| json!({"agent":c.config.id,"provider":c.config.provider,"capabilities":c.capabilities})).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"agents":values,"failures":failures,"inventory":inventory(&config, &paths.data)})
                    )?
                );
            } else if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&inventory(&config, &paths.data))?
                );
            } else {
                for agent in &config.agents {
                    let availability =
                        crate::discovery::inspect(agent, &config.discovery, &paths.data);
                    println!(
                        "{:<16} {:<18} {}",
                        agent.id,
                        availability.status,
                        availability
                            .native_cli
                            .as_deref()
                            .or(availability.executable.as_deref())
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| agent.command.clone())
                    );
                    if let Some(detail) = availability.detail {
                        println!("  {detail}");
                    }
                }
            }
        }
        Some(Command::Sessions { all }) => {
            let repo = store.repository_id(&root)?;
            let sessions = store.sessions(if all { None } else { Some(&repo) })?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
            } else {
                for s in sessions {
                    println!(
                        "{}  {} / {}  {:?}",
                        s.session_id, s.agent, s.model, s.outcome
                    );
                }
            }
        }
        Some(Command::Threads {
            command,
            all,
            limit,
        }) => {
            ensure!(
                config.activity.enabled,
                "activity.enabled is false, so no conversation is recorded"
            );
            let activity =
                crate::activity::Activity::open(&paths.data, config.activity.retention_days)?;
            match command {
                None => {
                    let projects = activity.sidebar(limit as usize, all)?;
                    if cli.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "projects": projects
                            }))?
                        );
                    } else {
                        for project in projects {
                            println!("{}  {}", project.name, project.root);
                            for thread in project.threads {
                                println!(
                                    "  {}  {}  {}  {}",
                                    thread_mark(thread.status),
                                    &thread.id[..8],
                                    thread.title,
                                    thread.status
                                );
                            }
                        }
                    }
                }
                Some(ThreadCommand::Show { id }) => {
                    let thread = activity
                        .thread(&id)?
                        .with_context(|| format!("no thread {id}"))?;
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&thread)?);
                    } else {
                        println!("{}  ({})", thread.thread.title, thread.thread.status);
                        for seat in &thread.seats {
                            println!(
                                "  seat {} {}{}  {}",
                                seat.ordinal,
                                seat.role,
                                if seat.read_only { " (read-only)" } else { "" },
                                seat.model.as_deref().unwrap_or("choosing")
                            );
                        }
                        for item in &thread.items {
                            let text = item.text.lines().next().unwrap_or("");
                            println!("  {:<14} {}", item.kind, crate::context::bounded(text, 100));
                        }
                        for file in &thread.files {
                            println!("  {} +{} -{}", file.path, file.added, file.removed);
                        }
                    }
                }
                Some(ThreadCommand::Delete { id, all }) => {
                    if all {
                        for project in activity.sidebar(usize::MAX, true)? {
                            for thread in project.threads {
                                activity.delete_thread(&thread.id)?;
                            }
                        }
                    } else {
                        ensure!(activity.delete_thread(&id)?, "no thread {id}");
                    }
                }
            }
        }
        Some(Command::CollaborateResume { output, apply }) => {
            let report = crate::collaboration::resume(
                &config,
                &Registry::load(&paths.data)?,
                &store,
                &root,
                &output,
                apply,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(collaboration_code(&report));
        }
        Some(Command::Collaborate {
            task,
            plan,
            output,
            apply,
            dry_run,
        }) => {
            let policies = Registry::load(&paths.data)?;
            let plan = match plan {
                Some(path) => {
                    ensure!(
                        path.metadata()?.len() <= 65536,
                        "collaboration plan exceeds 64 KiB"
                    );
                    serde_json::from_slice(&std::fs::read(path)?)?
                }
                None => {
                    let mut descriptor = crate::router::profiler::profile(&task, &root);
                    // A dry run never sends `session/prompt`, so it cannot ask the classifier
                    // either; the team it prints is the one the local profile alone derives.
                    if !dry_run {
                        crate::router::classifier::refine(
                            &config,
                            &policies,
                            &store,
                            &root,
                            &task,
                            &mut descriptor,
                            |progress| {
                                if let crate::acp::Progress::Note(note) = progress {
                                    eprintln!("{note}");
                                }
                            },
                        )
                        .await;
                    }
                    let plan = crate::collaboration::plan::derive(&descriptor, &root);
                    eprintln!(
                        "Team: {} ({} / {}{})",
                        plan.summary(),
                        descriptor.task_type,
                        descriptor.complexity.key(),
                        if dry_run { ", local profile only" } else { "" }
                    );
                    plan
                }
            };
            if dry_run {
                // Printed as a plan file so it can be saved, edited and passed to `--plan`.
                plan.validate(&config)?;
                if let Some(waves) = plan.waves() {
                    eprintln!("Order: {}", crate::collaboration::graph::summary(&waves));
                }
                println!("{}", serde_json::to_string_pretty(&plan)?);
                return Ok(0);
            }
            let report = crate::collaboration::run(
                &config,
                &policies,
                &store,
                &root,
                &task,
                &plan,
                &output.expect("output is required without --dry-run"),
                apply,
                None,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(collaboration_code(&report));
        }
        Some(Command::Serve) => {
            crate::gateway::serve(config, paths.data).await?;
        }
        Some(Command::QuotaIngest { agent }) => {
            use std::io::Read;
            ensure!(
                config.agents.iter().any(|a| a.id == agent
                    && a.provider == crate::types::Provider::Anthropic
                    && a.enabled),
                "quota-ingest requires an enabled Anthropic agent"
            );
            let mut bytes = Vec::new();
            std::io::stdin().take(1_048_577).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 1_048_576, "quota payload exceeds 1 MiB");
            let value = serde_json::from_slice(&bytes).context("invalid statusline JSON")?;
            let time = now();
            let snapshot = crate::quota_sources::Snapshot {
                agent,
                source: "claude_statusline".into(),
                observed_at: time,
                valid_until: time.saturating_add(config.quota.max_age_secs),
                windows: crate::quota_sources::parse_claude_statusline(&value, time)?,
            };
            store.save_quota_snapshot(&snapshot)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            }
        }
        Some(Command::Quota { refresh }) => {
            let probes = if refresh {
                crate::quota_sources::refresh(&config, &store).await?
            } else {
                vec![]
            };
            let snapshots: Vec<_> = store
                .quota_snapshots()?
                .into_iter()
                .map(|s| json!({"stale":s.valid_until <= now(), "snapshot":s}))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"probes":probes,"snapshots":snapshots}))?
            );
        }
        Some(Command::Memory { forget, json }) => {
            use crate::memory::{Memory, Scope};
            let memory = Memory::open(&paths.data, &config.memory)
                .context("memory is off (memory.enabled = false)")?;
            let repository = store.repository_id(&root)?;
            let time = now();
            if let Some(id) = forget {
                let (scope, position) = crate::memory::position(&id)
                    .with_context(|| format!("invalid memory id {id}; use u1, r2, ..."))?;
                let text = memory
                    .forget(scope, &repository, position, time)?
                    .with_context(|| format!("no memory {id}"))?;
                println!("forgot: {text}");
                return Ok(0);
            }
            let listing: Vec<_> = [(Scope::User, "u", "user"), (Scope::Repo, "r", "repository")]
                .into_iter()
                .map(|(scope, prefix, name)| {
                    let items: Vec<_> = memory
                        .items(scope, &repository, time)
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            json!({"id": format!("{prefix}{}", index + 1), "text": item.text,
                                "yours": item.auto.is_none(),
                                "seen": item.auto.map(|s| s.count), "last": item.auto.map(|s| s.last)})
                        })
                        .collect();
                    json!({"scope": name, "path": memory.path(scope, &repository), "items": items})
                })
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&listing)?);
            } else {
                for scope in &listing {
                    println!(
                        "{} ({})",
                        scope["scope"].as_str().unwrap_or(""),
                        scope["path"].as_str().unwrap_or("")
                    );
                    let items = scope["items"].as_array().cloned().unwrap_or_default();
                    if items.is_empty() {
                        println!("  nothing yet");
                    }
                    for item in items {
                        let origin = if item["yours"] == true {
                            "yours".to_owned()
                        } else {
                            format!("heard {}x", item["seen"])
                        };
                        println!(
                            "  {:<4} {}  ({origin})",
                            item["id"].as_str().unwrap_or(""),
                            crate::memory::clean(item["text"].as_str().unwrap_or(""), 400)
                        );
                    }
                }
            }
        }
        Some(Command::Calibrate { limit }) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::learning::calibrate(
                    &store.recent_runs(limit as usize)?
                ))?
            );
        }
        Some(Command::Benchmark {
            input,
            seed,
            failure_penalty,
        }) => {
            ensure!(
                input.metadata()?.len() <= 32 * 1024 * 1024,
                "benchmark input exceeds 32 MiB"
            );
            let dataset = serde_json::from_slice(&std::fs::read(input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::benchmark::replay(
                    dataset,
                    &config.learning,
                    config.scheduler.required_success,
                    seed,
                    failure_penalty
                )?)?
            );
        }
        Some(Command::BenchmarkTune {
            input,
            training_cases,
            seed,
            failure_penalty,
        }) => {
            ensure!(
                input.metadata()?.len() <= 32 * 1024 * 1024,
                "benchmark input exceeds 32 MiB"
            );
            let dataset = serde_json::from_slice(&std::fs::read(input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::benchmark::tune(
                    dataset,
                    &config.learning,
                    config.scheduler.required_success,
                    seed,
                    failure_penalty,
                    training_cases
                )?)?
            );
        }
        Some(Command::Runs { limit }) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.recent_runs(limit as usize)?)?
            );
        }
        Some(Command::Config(_) | Command::Policy(_)) => unreachable!(),
        None => {
            let Some(task) = cli.task else {
                use clap::CommandFactory;
                Cli::command().print_help()?;
                println!();
                return Ok(0);
            };
            ensure!(
                !cli.json || cli.dry_run,
                "--json is supported with --dry-run and inspection commands"
            );
            check_overrides(&config, &cli.agent, &cli.model, &cli.reasoning)?;
            let repo = store.repository_id(&root)?;
            let _lock = workspace_lock(&paths.data, &repo, shared)?;
            let policies = Registry::load(&paths.data)?;
            let overrides = Overrides {
                agent: cli.agent,
                model: cli.model,
                reasoning: cli.reasoning,
                mode: cli.mode,
            };
            let permission = cli.permission.unwrap_or(match config.scheduler.permission {
                PermissionMode::Ask => PermissionMode::Allow,
                chosen => chosen,
            });
            // A one-shot run is a thread of one turn, so it appears beside conversations in a
            // client rather than being invisible to it. A dry run decides nothing and records
            // nothing.
            let recorded = (config.activity.enabled && !cli.dry_run)
                .then(|| -> Result<_> {
                    let activity = crate::activity::share(crate::activity::Activity::open(
                        &paths.data,
                        config.activity.retention_days,
                    )?);
                    let (seat, host) = crate::activity::start_turn(
                        &activity,
                        &root,
                        &store.salt()?,
                        &repo,
                        crate::activity::Origin::Run,
                        &task,
                        &[],
                        &overrides,
                        permission.key(),
                        "implementer",
                    )?;
                    Ok((activity, seat, host))
                })
                .transpose()
                .unwrap_or_else(|error| {
                    tracing::debug!(%error, "run not recorded");
                    None
                });
            let code = scheduler::run(
                &config,
                &policies,
                &store,
                &root,
                RunOptions {
                    task,
                    descriptor: None,
                    overrides,
                    dry_run: cli.dry_run,
                    json: cli.json,
                    resume: cli.resume,
                    // The chat answers permission requests itself unless asked to stop and
                    // ask; Shift-Tab and /confirm change it while it runs.
                    permission,
                    interactive: false,
                    attachments: vec![],
                    peer: None,
                    verify: true,
                    read_only: false,
                    place: None,
                    seat: recorded.as_ref().map(|(_, seat, _)| seat.clone()),
                },
            )
            .await;
            if let Some((activity, seat, host)) = &recorded {
                let state = match &code {
                    Ok(0) => crate::activity::TurnState::Completed,
                    Ok(130) => crate::activity::TurnState::Interrupted,
                    _ => crate::activity::TurnState::Failed,
                };
                if let Err(error) = activity
                    .lock()
                    .expect("activity store")
                    .end_turn(seat, host, state)
                {
                    tracing::debug!(%error, "turn not closed");
                }
            }
            return code;
        }
    }
    Ok(0)
}

/// The sidebar's status marks, as `docs/desktop-app-design.md` §6.2 lists them.
fn thread_mark(status: &str) -> &'static str {
    match status {
        "needs_you" => "!",
        "working" => "◉",
        "queued" => "⋯",
        "interrupted" => "⏸",
        "failed" => "×",
        "unread" => "●",
        _ => "○",
    }
}

fn check_overrides(
    config: &Config,
    agent: &Option<String>,
    model: &Option<String>,
    reasoning: &Option<String>,
) -> Result<()> {
    if let Some(id) = agent {
        ensure!(
            config
                .agents
                .iter()
                .any(|a| &a.id == id && a.enabled && !a.routing_only),
            "unknown, disabled or routing-only agent: {id}"
        );
    }
    if model.as_deref() == Some("") || reasoning.as_deref() == Some("") {
        bail!("model/reasoning overrides must not be empty");
    }
    Ok(())
}

fn collaboration_code(report: &crate::collaboration::Report) -> u8 {
    if report.finished() {
        0
    } else if report.status == crate::collaboration::RunStatus::Cancelled {
        130
    } else {
        1
    }
}

fn inventory(config: &Config, data: &std::path::Path) -> Vec<serde_json::Value> {
    config.agents.iter().map(|a| {
        let availability = crate::discovery::inspect(a, &config.discovery, data);
        json!({"agent":a.id,"provider":a.provider,"command":a.command,"enabled":a.enabled,"routing_only":a.routing_only,"installed":availability.installed,"availability":availability,"browser":a.browser,"web":a.web})
    }).collect()
}
