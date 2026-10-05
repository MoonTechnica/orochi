//! A project in `runner` mode: the agent runs on this machine, signed in as it is here, and
//! the project's runtime — tests, builds, servers, Docker, Supabase — runs in its sandbox. The
//! agent reaches the sandbox through the MCP server here (`orochi --internal-sandbox-runner`,
//! spawned by the agent like the mailbox), and Orochi's own checks run there too.
//!
//! The tree is the same path on both sides, so the agent edits files as it always does; only
//! what it *runs* moves. What the agent's own shell runs still runs here — the prompt note says
//! which tools to use instead, and that is the one thing this mode asks of the agent.
use super::{Incus, Mode, Placement, ops, placement};
use crate::config::{McpServerConfig, McpTransport, SandboxConfig};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
    time::{Duration, Instant},
};

pub const SERVE_FLAG: &str = "--internal-sandbox-runner";
pub const SERVER_NAME: &str = "orochi-sandbox";
const OUTPUT_LIMIT: usize = 20_000;
const DEFAULT_TIMEOUT: u64 = 600;
const MAX_TIMEOUT: u64 = 3600;

/// The MCP server a runner-mode session is given.
pub fn server(config: &SandboxConfig, data: &Path, root: &Path) -> Result<McpServerConfig> {
    Ok(McpServerConfig {
        name: SERVER_NAME.into(),
        transport: McpTransport::Stdio,
        command: std::env::current_exe()?.to_string_lossy().into_owned(),
        args: vec![
            SERVE_FLAG.into(),
            data.to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
            config
                .session_root
                .as_deref()
                .unwrap_or(root)
                .to_string_lossy()
                .into_owned(),
            serde_json::to_string(config)?,
        ],
        ..McpServerConfig::default()
    })
}

/// What the agent is told at the start of a runner-mode session.
pub fn prompt_note(config: &SandboxConfig, name: &str) -> String {
    format!(
        "Sandbox: this project's runtime is a Linux sandbox, not this machine. Run its tests, \
         builds, package installs, dev servers, Docker and Supabase with the `{SERVER_NAME}` MCP \
         tools — `sandbox_exec` for a command that finishes, `sandbox_start` / `sandbox_logs` / \
         `sandbox_stop` for one that keeps running, `sandbox_ports` for what listens — and not \
         with your own shell, which runs on the user's machine. Read and edit files as usual: the \
         sandbox sees this same directory at this same path. Its services open from here at {}.",
        ops::url(config, name, None)
    )
}

fn tools() -> Value {
    json!([
        {"name": "sandbox_exec",
         "description": "Run a shell command in this project's sandbox and wait for it: tests, builds, package installs, docker, supabase. Returns its exit code and the end of its output.",
         "inputSchema": {"type": "object", "properties": {
            "command": {"type": "string", "description": "Shell command, run with sh -lc"},
            "cwd": {"type": "string", "description": "Directory relative to the project root (default: the root)"},
            "timeout_secs": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT}},
          "required": ["command"]}},
        {"name": "sandbox_start",
         "description": "Start a long-running command (a dev server, a watcher) in the sandbox in the background, under a name. It keeps running after this call returns.",
         "inputSchema": {"type": "object", "properties": {
            "name": {"type": "string", "pattern": "^[a-z0-9-]{1,32}$"},
            "command": {"type": "string"},
            "cwd": {"type": "string"}},
          "required": ["name", "command"]}},
        {"name": "sandbox_logs",
         "description": "The last lines a background command started with sandbox_start printed.",
         "inputSchema": {"type": "object", "properties": {
            "name": {"type": "string", "pattern": "^[a-z0-9-]{1,32}$"},
            "lines": {"type": "integer", "minimum": 1, "maximum": 2000}},
          "required": ["name"]}},
        {"name": "sandbox_stop",
         "description": "Stop a background command started with sandbox_start.",
         "inputSchema": {"type": "object", "properties": {
            "name": {"type": "string", "pattern": "^[a-z0-9-]{1,32}$"}},
          "required": ["name"]}},
        {"name": "sandbox_ports",
         "description": "What listens in the sandbox, and the address each port opens at from the user's machine.",
         "inputSchema": {"type": "object", "properties": {}}}
    ])
}

struct Runner {
    config: SandboxConfig,
    placement: Placement,
}

fn name_ok(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A directory the agent named, kept inside the project.
fn within(root: &Path, cwd: Option<&str>) -> Result<PathBuf> {
    let Some(cwd) = cwd.filter(|c| !c.is_empty() && *c != ".") else {
        return Ok(root.to_path_buf());
    };
    let plain = Path::new(cwd)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    ensure!(plain, "cwd must be a directory inside the project");
    Ok(root.join(cwd))
}

fn tail(text: &str) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text.to_owned();
    }
    let mut start = text.len() - OUTPUT_LIMIT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…(earlier output cut)\n{}", &text[start..])
}

impl Runner {
    fn ready(&self, data: &Path) -> Result<()> {
        self.placement.ready(&self.config, data)
    }

    /// Runs `script` with `sh -lc` inside, as the agent's work (it carries the mark the VM's
    /// idle check reads), and returns (exit code, output). `hold` keeps stdin open for as long
    /// as it runs, so the command ends if this server is killed.
    fn run(
        &self,
        cwd: &Path,
        script: &str,
        timeout: Duration,
        hold: bool,
    ) -> Result<(i32, String)> {
        let mut env = vec![];
        if hold {
            env.push(("SBX_HOLD".to_owned(), "1".to_owned()));
        }
        let args = self.placement.exec_args(
            &self.config,
            cwd,
            &env,
            "sh",
            &["-lc".into(), script.into()],
            false,
        );
        let mut child = Incus::new(&self.config)
            .command(&args)?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let held = child.stdin.take();
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut text);
            text
        });
        let errors = std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = std::io::Read::read_to_end(&mut err, &mut text);
            text
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if started.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        drop(held);
        let mut output = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
        let errors = String::from_utf8_lossy(&errors.join().unwrap_or_default()).into_owned();
        if !errors.is_empty() {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&errors);
        }
        match status {
            Some(status) => Ok((status.code().unwrap_or(-1), tail(&output))),
            None => Ok((
                -1,
                tail(&format!(
                    "{output}\n(stopped after {} s; use sandbox_start for a command that keeps running)",
                    timeout.as_secs()
                )),
            )),
        }
    }

    fn call(&self, data: &Path, tool: &str, args: &Value) -> Result<Value> {
        self.ready(data)?;
        let root = self.placement.root.clone();
        match tool {
            "sandbox_exec" => {
                let command = args["command"].as_str().context("command is required")?;
                let cwd = within(&root, args["cwd"].as_str())?;
                let timeout = args["timeout_secs"]
                    .as_u64()
                    .unwrap_or(DEFAULT_TIMEOUT)
                    .clamp(1, MAX_TIMEOUT);
                let (code, output) = self.run(&cwd, command, Duration::from_secs(timeout), true)?;
                Ok(json!({"exit_code": code, "output": output}))
            }
            "sandbox_start" => {
                let name = args["name"].as_str().context("name is required")?;
                ensure!(name_ok(name), "name must be 1-32 of a-z, 0-9 and -");
                let command = args["command"].as_str().context("command is required")?;
                let cwd = within(&root, args["cwd"].as_str())?;
                // Detached into a session of its own, logging to a file: it outlives this call
                // and the agent's turn, and as the agent's work keeps the VM up while it runs.
                let script = format!(
                    "mkdir -p /tmp/orochi-run && \
                     if [ -f /tmp/orochi-run/{name}.pid ] && kill -0 \"$(cat /tmp/orochi-run/{name}.pid)\" 2>/dev/null; then echo already-running; exit 3; fi; \
                     setsid sh -lc {} > /tmp/orochi-run/{name}.log 2>&1 < /dev/null & echo $! > /tmp/orochi-run/{name}.pid; echo started",
                    shell_quote(command)
                );
                let (code, output) = self.run(&cwd, &script, Duration::from_secs(30), false)?;
                if code == 3 {
                    bail!("{name} is already running; sandbox_stop it first");
                }
                ensure!(code == 0, "could not start {name}: {output}");
                Ok(
                    json!({"started": name, "logs": format!("sandbox_logs {{\"name\": \"{name}\"}}")}),
                )
            }
            "sandbox_logs" => {
                let name = args["name"].as_str().context("name is required")?;
                ensure!(name_ok(name), "name must be 1-32 of a-z, 0-9 and -");
                let lines = args["lines"].as_u64().unwrap_or(200).clamp(1, 2000);
                let script = format!(
                    "tail -n {lines} /tmp/orochi-run/{name}.log; \
                     if kill -0 \"$(cat /tmp/orochi-run/{name}.pid 2>/dev/null)\" 2>/dev/null; then echo '(running)'; else echo '(not running)'; fi"
                );
                let (_, output) = self.run(&root, &script, Duration::from_secs(30), true)?;
                Ok(json!({"name": name, "output": output}))
            }
            "sandbox_stop" => {
                let name = args["name"].as_str().context("name is required")?;
                ensure!(name_ok(name), "name must be 1-32 of a-z, 0-9 and -");
                let script = format!(
                    "pid=$(cat /tmp/orochi-run/{name}.pid 2>/dev/null) && kill -TERM -- \"-$pid\" 2>/dev/null; rm -f /tmp/orochi-run/{name}.pid; echo stopped"
                );
                self.run(&root, &script, Duration::from_secs(30), true)?;
                Ok(json!({"stopped": name}))
            }
            "sandbox_ports" => {
                let ports = ops::listening(&self.config, &self.placement.name)?;
                Ok(json!(
                    ports
                        .iter()
                        .map(|p| json!({"port": p, "url": ops::url(&self.config, &self.placement.name, Some(*p))}))
                        .collect::<Vec<_>>()
                ))
            }
            other => bail!("unknown tool {other}"),
        }
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn serve(args: &[std::ffi::OsString]) -> ExitCode {
    let (data, root, scope, config) = match args {
        [data, root, config] => (data, root, root, config),
        [data, root, scope, config] => (data, root, scope, config),
        _ => {
            eprintln!("orochi: invalid sandbox runner arguments");
            return ExitCode::from(2);
        }
    };
    let data = PathBuf::from(data);
    let root = PathBuf::from(root);
    let config: SandboxConfig = match serde_json::from_str(&config.to_string_lossy()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("orochi: invalid sandbox runner configuration: {error}");
            return ExitCode::from(2);
        }
    };
    let runner = match placement(&data, &PathBuf::from(scope), Some(Mode::Runner)) {
        Ok(Some(mut placement)) => {
            placement.root = root.clone();
            Runner { config, placement }
        }
        Ok(None) => {
            eprintln!("orochi: {} has no sandbox", root.display());
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("orochi: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let params = &request["params"];
        let reply = match request["method"].as_str().unwrap_or("") {
            "initialize" => json!({"result": {
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Run this project's commands in its sandbox."}}),
            "ping" => json!({"result": {}}),
            "tools/list" => json!({"result": {"tools": tools()}}),
            "tools/call" => {
                let result = runner.call(
                    &data,
                    params["name"].as_str().unwrap_or(""),
                    &params["arguments"],
                );
                let (text, error) = match result {
                    Ok(value) => (value.to_string(), false),
                    Err(error) => (format!("{error:#}"), true),
                };
                json!({"result": {"content": [{"type": "text", "text": text}], "isError": error}})
            }
            _ => json!({"error": {"code": -32601, "message": "method not found"}}),
        };
        let mut message = reply;
        message["jsonrpc"] = json!("2.0");
        message["id"] = id;
        if writeln!(stdout, "{message}")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            break;
        }
    }
    ExitCode::SUCCESS
}
