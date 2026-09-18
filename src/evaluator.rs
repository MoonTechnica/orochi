use crate::{
    config::{CheckCommand, EvaluatorConfig},
    types::{CheckResult, Outcome},
};
use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

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
    let mut results = Vec::new();
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
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let (passed, exit_code, timed_out) = match command.spawn() {
            Ok(mut child) => {
                let _guard = ProcessGuard(child.id());
                match tokio::time::timeout(Duration::from_secs(config.timeout_secs), child.wait())
                    .await
                {
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
        results.push(CheckResult {
            name: check.name,
            passed,
            exit_code,
            duration_ms: start.elapsed().as_millis() as u64,
            timed_out,
        });
    }
    results
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
