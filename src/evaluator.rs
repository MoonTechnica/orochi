use crate::{
    config::{CheckCommand, EvaluatorConfig},
    types::{CheckResult, Outcome},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, BufReader};

/// The last lines of a failed check, kept for the one place check output is allowed to live:
/// the conversation store (§10.3 of `docs/desktop-app-design.md`). Telemetry never sees it.
pub const TAIL_LINES: usize = 200;

/// What the checks produced, beside their results: the tail of each **failed** check's output.
/// A check that passed leaves nothing — nobody reads the output of a green test run.
#[derive(Debug, Default, Clone)]
pub struct Output(pub BTreeMap<String, String>);

pub fn checks(config: &EvaluatorConfig, root: &Path) -> Vec<CheckCommand> {
    if !config.checks.is_empty() {
        return config.checks.clone();
    }
    if !config.auto {
        return vec![];
    }
    let mut checks = Vec::new();
    let mut add = |name: &str, command: &str, args: &[&str]| {
        checks.push(CheckCommand {
            name: name.into(),
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        })
    };
    if root.join("Cargo.toml").is_file() {
        add("tests", "cargo", &["test", "--quiet"]);
    } else if root.join("go.mod").is_file() {
        add("tests", "go", &["test", "./..."]);
    } else if let Ok(bytes) = std::fs::read(root.join("package.json"))
        && let Ok(package) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        let runner = if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
            "bun"
        } else {
            "npm"
        };
        for name in ["test", "typecheck", "lint", "build"] {
            if package["scripts"][name].is_string() {
                add(name, runner, &["run", name]);
            }
        }
    }
    if crate::context::git(root, &["rev-parse", "--git-dir"]).is_some() {
        add("git_diff", "git", &["diff", "--check"]);
        add("git_staged_diff", "git", &["diff", "--cached", "--check"]);
    }
    checks
}

struct ProcessGuard(Option<u32>);
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            // Each evaluator is a new process group; kill descendants on cancellation too.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

pub async fn evaluate(config: &EvaluatorConfig, root: &Path) -> Vec<CheckResult> {
    evaluate_with(config, root, true).await
}

pub async fn evaluate_with(
    config: &EvaluatorConfig,
    root: &Path,
    announce: bool,
) -> Vec<CheckResult> {
    detailed(config, root, announce).await.0
}

/// The same run, also reporting what the failed checks printed.
pub async fn detailed(
    config: &EvaluatorConfig,
    root: &Path,
    announce: bool,
) -> (Vec<CheckResult>, Output) {
    let mut results = Vec::new();
    let mut output = Output::default();
    for check in checks(config, root) {
        let start = Instant::now();
        if announce {
            eprintln!("  check: {}", check.name);
        }
        let mut command = tokio::process::Command::new(&check.command);
        command
            .args(&check.args)
            .current_dir(root)
            .env("CI", "true")
            .stdin(Stdio::null())
            // Captured rather than discarded, and echoed line by line below, so a terminal
            // still watches a long check run while its tail is kept for a failure.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut tail: VecDeque<String> = VecDeque::new();
        let (passed, exit_code, timed_out) = match command.spawn() {
            Ok(mut child) => {
                let _guard = ProcessGuard(child.id());
                let mut out = child.stdout.take().map(|o| BufReader::new(o).lines());
                let mut err = child.stderr.take().map(|e| BufReader::new(e).lines());
                let wait = async {
                    loop {
                        let line = tokio::select! {
                            Ok(Some(line)) = async { match &mut out {
                                Some(reader) => reader.next_line().await,
                                None => std::future::pending().await,
                            } } => line,
                            Ok(Some(line)) = async { match &mut err {
                                Some(reader) => reader.next_line().await,
                                None => std::future::pending().await,
                            } } => line,
                            status = child.wait() => return status,
                        };
                        if announce {
                            eprintln!("{line}");
                        }
                        tail.push_back(line);
                        while tail.len() > TAIL_LINES {
                            tail.pop_front();
                        }
                    }
                };
                match tokio::time::timeout(Duration::from_secs(config.timeout_secs), wait).await {
                    Ok(Ok(status)) => (status.success(), status.code(), false),
                    Ok(Err(_)) => (false, None, false),
                    Err(_) => {
                        let _ = child.kill().await;
                        (false, None, true)
                    }
                }
            }
            Err(_) => (false, None, false),
        };
        if !passed && !tail.is_empty() {
            output.0.insert(
                check.name.clone(),
                tail.into_iter().collect::<Vec<_>>().join("\n"),
            );
        }
        results.push(CheckResult {
            name: check.name,
            passed,
            exit_code,
            duration_ms: start.elapsed().as_millis() as u64,
            timed_out,
        });
    }
    (results, output)
}

pub fn outcome(completed: bool, checks: &[CheckResult]) -> Outcome {
    if !completed || checks.iter().any(|c| !c.passed) {
        return Outcome::Failure;
    }
    if checks
        .iter()
        .any(|c| c.passed && !c.name.starts_with("git_"))
    {
        Outcome::Success
    } else {
        Outcome::PartialSuccess
    }
}
