//! Initial sandbox setup, shared by the CLI and desktop's background job.
use crate::config::{SandboxClient, SandboxConfig};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

/// GUI launches do not necessarily inherit Homebrew's PATH. Respect explicit executable
/// paths, and only supplement the search for bare names.
pub fn locate(command: &str) -> Option<PathBuf> {
    crate::discovery::locate(command).or_else(|| {
        if Path::new(command).components().count() != 1 {
            return None;
        }
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .into_iter()
            .find_map(|dir| {
                crate::discovery::locate(&Path::new(dir).join(command).to_string_lossy())
            })
    })
}

pub fn ensure_lima(config: &SandboxConfig) -> Result<()> {
    ensure_lima_with(config, locate)
}

fn ensure_lima_with(
    config: &SandboxConfig,
    locate: impl Fn(&str) -> Option<PathBuf>,
) -> Result<()> {
    if locate(&config.limactl).is_some() {
        println!("Lima is already installed; reusing it");
        return Ok(());
    }
    ensure!(
        config.limactl == "limactl",
        "configured Lima executable {} was not found; correct sandbox.limactl and retry",
        config.limactl
    );
    let brew = locate("brew").context(
        "Homebrew was not found. Install it from https://brew.sh, then retry `orochi init`; alternatively install Lima and set sandbox.limactl",
    )?;
    println!("Installing Lima with Homebrew…");
    let status = std::process::Command::new(brew)
        .args(["install", "lima"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stdin(std::process::Stdio::null())
        .status()
        .context("could not start Homebrew")?;
    ensure!(
        status.success(),
        "Lima installation failed; fix the Homebrew error above and retry `orochi init`"
    );
    ensure!(
        locate(&config.limactl).is_some(),
        "Homebrew finished but limactl was not found; set sandbox.limactl to its installed path and retry"
    );
    Ok(())
}

/// Set up the host and build only missing images. Running this again resumes an incomplete
/// setup, without updating installed tools or replacing an existing golden image.
pub fn run(config: &SandboxConfig, data: &Path, host_only: bool) -> Result<()> {
    std::fs::create_dir_all(data.join("sandbox"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(std::env::temp_dir().join(format!(
            "orochi-setup-{}-{}.lock",
            config.lima_instance, config.project
        )))?;
    lock.lock()?;
    if config.client == SandboxClient::Incus
        && (!config.remote.is_empty() || config.incus != "incus")
    {
        ensure!(
            locate(&config.incus).is_some(),
            "Configured Incus client not found; correct sandbox.incus. Remote hosts must be prepared with `orochi sandbox host-script`"
        );
    }
    super::ops::up(config, data)?;
    if !host_only {
        let incus = super::Incus::new(config);
        let tier = super::Mode::Container;
        if incus
            .run(&["image", "show", super::ops::image(tier)])
            .is_ok()
        {
            println!("Sandbox image already exists; reusing it");
        } else {
            super::ops::build_image(config, tier)?;
        }
    }
    println!("Sandbox initialization complete");
    Ok(())
}

/// Prepare a local Linux host, including WSL2. Package installation is privileged, but
/// Orochi and its agents keep running as the original user.
pub(super) fn prepare_linux(config: &SandboxConfig, data: &Path) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "Local Incus setup requires Linux; on Windows use tools/windows/orochi.ps1 -Setup (WSL2)"
    );
    std::fs::create_dir_all(data.join("sandbox"))?;
    let directory = tempfile::tempdir_in(data.join("sandbox"))?;
    for (name, contents) in [
        ("host.sh", super::ops::HOST_SCRIPT),
        ("gateway.py", super::ops::GATEWAY_SCRIPT),
        ("idle.py", super::ops::IDLE_SCRIPT),
        ("prepare.sh", include_str!("linux-host.sh")),
    ] {
        std::fs::write(directory.path().join(name), contents)?;
    }
    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal();
    let mut command = if super::ids().0 == 0 {
        std::process::Command::new("env")
    } else if interactive {
        let mut command = std::process::Command::new(
            locate("sudo").context("Install sudo to authorize Linux host setup")?,
        );
        command.arg("env");
        command
    } else if let Some(pkexec) = locate("pkexec") {
        let mut command = std::process::Command::new(pkexec);
        command.arg("env");
        command
    } else {
        let mut command = std::process::Command::new(
            locate("sudo")
                .context("Run `orochi init` in a terminal to authorize Linux host setup")?,
        );
        command.args(["-n", "env"]);
        command
    };
    let mut env = super::ops::host_env(config);
    // Never allow the native Linux machine (or a WSL distribution) to power itself off.
    env.push("SBX_MANAGED_VM=0".into());
    command
        .args(env)
        .arg("bash")
        .arg(directory.path().join("prepare.sh"))
        .arg(directory.path().join("host.sh"))
        .arg(directory.path().join("gateway.py"))
        .arg(directory.path().join("idle.py"));
    println!("Preparing local Incus (administrator authorization may be required)…");
    ensure!(
        command.status()?.success(),
        "Local Incus setup failed; see the output above. If authorization was unavailable, run `orochi init` in a terminal"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_lima_never_invokes_a_package_manager() {
        ensure_lima_with(&SandboxConfig::default(), |name| {
            assert_eq!(name, "limactl");
            Some("/already/limactl".into())
        })
        .unwrap();
    }

    #[test]
    fn missing_custom_executable_does_not_install_a_different_one() {
        let config = SandboxConfig {
            limactl: "/missing/lima".into(),
            ..Default::default()
        };
        assert!(
            ensure_lima_with(&config, |_| None)
                .unwrap_err()
                .to_string()
                .contains("sandbox.limactl")
        );
    }

    #[test]
    fn missing_homebrew_explains_how_to_retry() {
        assert!(
            ensure_lima_with(&SandboxConfig::default(), |_| None)
                .unwrap_err()
                .to_string()
                .contains("https://brew.sh")
        );
    }

    #[test]
    fn a_missing_lima_is_installed_once_then_reused() {
        let dir = tempfile::tempdir().unwrap();
        let brew = dir.path().join("brew");
        let installed = dir.path().join("installed");
        std::fs::write(&brew, "#!/bin/sh\n[ \"$1 $2\" = 'install lima' ] || exit 9\n[ ! -e \"$(dirname \"$0\")/installed\" ] || exit 8\ntouch \"$(dirname \"$0\")/installed\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
        let locate = |name: &str| match name {
            "brew" => Some(brew.clone()),
            "limactl" if installed.exists() => Some(installed.clone()),
            _ => None,
        };
        ensure_lima_with(&SandboxConfig::default(), locate).unwrap();
        ensure_lima_with(&SandboxConfig::default(), locate).unwrap();
    }

    #[test]
    fn installer_failure_is_reported_and_success_is_verified() {
        let dir = tempfile::tempdir().unwrap();
        let brew = dir.path().join("brew");
        use std::os::unix::fs::PermissionsExt;
        for exit in [1, 0] {
            std::fs::write(
                &brew,
                format!("#!/bin/sh\n[ \"$1 $2\" = 'install lima' ] || exit 9\nexit {exit}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
            let error = ensure_lima_with(&SandboxConfig::default(), |name| {
                (name == "brew").then(|| brew.clone())
            })
            .unwrap_err()
            .to_string();
            assert!(error.contains(if exit == 1 {
                "installation failed"
            } else {
                "limactl was not found"
            }));
        }
    }
}
