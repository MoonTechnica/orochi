//! Where Orochi puts what is big: helpers' worktrees (`workspaces.root`), a local decision
//! model and llama.cpp (`decision.models_dir`), and the sandbox VM with every sandbox's disk
//! (`sandbox.lima_home`). Each is read and set here, by the window's Storage page and by
//! `orochi places`, so both say the same thing and refuse the same things.
//!
//! Nothing already there is moved. A worktree open in the old place stays where it is and is
//! still found (`workspaces::sessions_dirs`); a model downloaded to the old place is downloaded
//! again on next use; a VM made under the old Lima home stays there, which is why moving that
//! one while a VM exists is refused unless asked for outright.
use crate::config::{Config, SandboxClient};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Which {
    Worktrees,
    Models,
    Vm,
}
impl Which {
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "worktrees" => Self::Worktrees,
            "models" => Self::Models,
            "vm" => Self::Vm,
            other => bail!("no such place: {other} (worktrees, models or vm)"),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Place {
    pub which: Which,
    /// What is set, if anything: a path, or for worktrees a template with `{repository}`,
    /// `{name}` and `{hash}` (`workspaces::expand`).
    pub configured: Option<String>,
    /// Where it goes unset; for worktrees the default template.
    pub default: String,
    /// The variables a template may name, for worktrees.
    pub variables: Vec<&'static str>,
    /// Where it goes now, where that does not depend on the repository.
    pub effective: Option<PathBuf>,
    /// Free space on the disk it goes to, in GB.
    pub free_gb: Option<u64>,
    /// How much is there now, where that can be told cheaply.
    pub used_bytes: Option<u64>,
    /// Whether it applies on this machine at all (the VM only with Lima).
    pub applies: bool,
    /// The VM's state where one exists under the current Lima home.
    pub vm: Option<String>,
}

fn size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

/// The part of a template before its first variable: the folder it is certainly under.
fn fixed_prefix(template: &str) -> PathBuf {
    let fixed = template.split('{').next().unwrap_or_default();
    let fixed = fixed.trim_end_matches(|c| c != '/');
    PathBuf::from(if fixed.is_empty() { template } else { fixed })
}

fn writable(place: &Path) -> Result<()> {
    std::fs::create_dir_all(place).with_context(|| format!("cannot create {}", place.display()))?;
    let probe = place.join(".orochi-write-test");
    std::fs::write(&probe, b"").with_context(|| format!("cannot write in {}", place.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn read(config: &Config, data: &Path) -> Vec<Place> {
    let models = crate::decision::local::home(data, &config.decision);
    let lima = config.sandbox.client == SandboxClient::Lima;
    let vm_home = config.sandbox.lima_disk();
    vec![
        Place {
            which: Which::Worktrees,
            configured: config.workspaces.root.clone(),
            default: crate::workspaces::DEFAULT_TEMPLATE.into(),
            variables: vec!["{repository}", "{name}", "{hash}"],
            effective: None,
            free_gb: config
                .workspaces
                .root
                .as_deref()
                .map(fixed_prefix)
                .filter(|p| p.is_absolute())
                .and_then(|p| crate::decision::local::free_disk_gb(&p)),
            used_bytes: None,
            applies: true,
            vm: None,
        },
        Place {
            which: Which::Models,
            configured: config
                .decision
                .models_dir
                .as_ref()
                .map(|p| p.display().to_string()),
            variables: vec![],
            default: data.join("decision").display().to_string(),
            free_gb: crate::decision::local::free_disk_gb(&models),
            used_bytes: Some(size(&models)),
            effective: Some(models),
            applies: true,
            vm: None,
        },
        Place {
            which: Which::Vm,
            configured: config
                .sandbox
                .lima_home
                .as_ref()
                .map(|p| p.display().to_string()),
            variables: vec![],
            default: home_dir().join(".lima").display().to_string(),
            effective: Some(
                config
                    .sandbox
                    .lima_home
                    .clone()
                    .unwrap_or_else(|| home_dir().join(".lima")),
            ),
            free_gb: crate::decision::local::free_disk_gb(&vm_home),
            used_bytes: None,
            applies: lima,
            vm: lima
                .then(|| {
                    crate::sandbox::ops::vm_status(&config.sandbox)
                        .ok()
                        .flatten()
                })
                .flatten(),
        },
    ]
}

/// Sets one place (`None` puts it back to the default) and saves the configuration. A place
/// must be an absolute directory Orochi can create and write in; moving the VM's home while a
/// VM exists under the current one is refused without `force`, since the new place would
/// start without it.
pub fn set(
    config: &mut Config,
    path: &Path,
    data: &Path,
    which: Which,
    place: Option<String>,
    force: bool,
) -> Result<()> {
    let place = place.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
    match (&place, which) {
        (Some(template), Which::Worktrees) => {
            crate::workspaces::check_template(template)?;
            // The fixed part before any variable is a folder that will hold them all.
            let prefix = fixed_prefix(template);
            if prefix.is_absolute() {
                writable(&prefix)?;
            }
        }
        (Some(place), _) => {
            ensure!(
                Path::new(place).is_absolute(),
                "{place} is not an absolute path"
            );
            writable(Path::new(place))?;
        }
        (None, _) => {}
    }
    match which {
        Which::Worktrees => config.workspaces.root = place,
        Which::Models => {
            let before = crate::decision::local::home(data, &config.decision);
            config.decision.models_dir = place.map(PathBuf::from);
            // A server still running from the old place would keep answering from there.
            if crate::decision::local::home(data, &config.decision) != before {
                crate::decision::local::stop(&before);
            }
        }
        Which::Vm => {
            if config.sandbox.lima_home != place.as_ref().map(PathBuf::from)
                && config.sandbox.client == SandboxClient::Lima
                && !force
                && let Ok(Some(state)) = crate::sandbox::ops::vm_status(&config.sandbox)
            {
                bail!(
                    "the sandbox VM ({state}) lives under {}; it stays there and a new one would be made in the new place. Remove it first (`limactl delete {}`), or set it anyway",
                    config.sandbox.lima_disk().display(),
                    config.sandbox.lima_instance
                );
            }
            config.sandbox.lima_home = place.map(PathBuf::from);
        }
    }
    config.save(path)
}
