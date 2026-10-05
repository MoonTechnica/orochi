//! Sandbox operations started from somewhere with no terminal (the desktop window): each is the
//! same `orochi sandbox …` a person would type, run in the background with its output kept in
//! `<data>/sandbox/jobs/`, so a long image build neither blocks the window nor needs a second
//! implementation.
use super::Mode;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What may be asked for. Only these become arguments; nothing a caller sends is passed through.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Setup,
    Up,
    Down,
    Image {
        vm: bool,
    },
    Create {
        root: PathBuf,
        mode: Mode,
        docker: bool,
    },
    Mode {
        root: PathBuf,
        mode: Mode,
    },
    Focus {
        root: PathBuf,
    },
    Unfocus,
    Snapshot {
        root: PathBuf,
    },
    Reset {
        root: PathBuf,
    },
    Remove {
        root: PathBuf,
    },
    Gc,
}
impl Request {
    pub fn args(&self) -> Vec<String> {
        if matches!(self, Request::Setup) {
            return vec!["init".into()];
        }
        let path = |p: &PathBuf| p.to_string_lossy().into_owned();
        let mut args = vec!["sandbox".to_owned()];
        match self {
            Request::Setup => unreachable!(),
            Request::Up => args.push("up".into()),
            Request::Down => args.push("down".into()),
            Request::Image { vm } => {
                args.push("image".into());
                if *vm {
                    args.push("--vm".into());
                }
            }
            Request::Create { root, mode, docker } => {
                args.extend([
                    "create".into(),
                    path(root),
                    "--mode".into(),
                    mode.key().into(),
                ]);
                if !docker {
                    args.push("--no-docker".into());
                }
            }
            Request::Mode { root, mode } => {
                args.extend(["mode".into(), mode.key().into(), path(root), "--yes".into()])
            }
            Request::Focus { root } => args.extend(["focus".into(), path(root)]),
            Request::Unfocus => args.push("unfocus".into()),
            Request::Snapshot { root } => args.extend(["snapshot".into(), path(root)]),
            Request::Reset { root } => args.extend(["reset".into(), path(root), "--yes".into()]),
            Request::Remove { root } => args.extend(["rm".into(), path(root), "--yes".into()]),
            Request::Gc => args.push("gc".into()),
        }
        args
    }
    /// The project it concerns, so the window can show a row as busy.
    pub fn root(&self) -> Option<&Path> {
        match self {
            Request::Create { root, .. }
            | Request::Mode { root, .. }
            | Request::Focus { root }
            | Request::Snapshot { root }
            | Request::Reset { root }
            | Request::Remove { root } => Some(root),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub request: Request,
    pub started_at: i64,
    /// The exit code once it has finished.
    pub exit: Option<i32>,
    /// The end of what it printed.
    #[serde(default)]
    pub tail: String,
}

fn dir(data: &Path) -> PathBuf {
    data.join("sandbox/jobs")
}

/// Starts `orochi sandbox …` in the background and returns its id at once.
pub fn start(binary: &Path, config: &Path, data: &Path, request: Request) -> Result<String> {
    if let Some(root) = request.root() {
        ensure!(
            root.is_absolute(),
            "a project is named by its absolute path"
        );
    }
    let dir = dir(data);
    std::fs::create_dir_all(&dir)?;
    prune(&dir, 50);
    let id = format!(
        "{}-{}",
        crate::types::now(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let job = Job {
        id: id.clone(),
        request: request.clone(),
        started_at: crate::types::now(),
        exit: None,
        tail: String::new(),
    };
    std::fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&job)?)?;
    let log = dir.join(format!("{id}.log"));
    let exit = dir.join(format!("{id}.exit"));
    // `sh` only records the exit code; every argument reaches orochi as its own word.
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("log=$1; exit_file=$2; shift 2; \"$@\" >\"$log\" 2>&1 </dev/null; echo $? >\"$exit_file\"")
        .arg("orochi-sandbox-job")
        .arg(&log)
        .arg(&exit)
        .arg(binary)
        .arg("--config")
        .arg(config)
        .arg("--data-dir")
        .arg(data)
        .args(request.args())
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own group, so closing the window does not take a half-built image with it.
        command.process_group(0);
    }
    command
        .spawn()
        .with_context(|| format!("could not start {}", binary.display()))?;
    Ok(id)
}

/// The most recent jobs, newest first, each with the end of its output.
pub fn list(data: &Path, limit: usize) -> Result<Vec<Job>> {
    let dir = dir(data);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(vec![]);
    };
    let mut jobs: Vec<Job> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    jobs.sort_by(|a: &Job, b: &Job| b.id.cmp(&a.id));
    jobs.truncate(limit);
    for job in &mut jobs {
        job.exit = std::fs::read_to_string(dir.join(format!("{}.exit", job.id)))
            .ok()
            .and_then(|s| s.trim().parse().ok());
        let log = std::fs::read_to_string(dir.join(format!("{}.log", job.id))).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        job.tail = lines[lines.len().saturating_sub(20)..].join("\n");
    }
    Ok(jobs)
}

/// Keeps the newest `keep` jobs' files and removes the rest.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    for id in ids.into_iter().skip(keep) {
        for ext in ["json", "log", "exit"] {
            let _ = std::fs::remove_file(dir.join(format!("{id}.{ext}")));
        }
    }
}
