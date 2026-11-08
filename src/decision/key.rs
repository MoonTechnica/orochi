//! A cloud decision provider's API key, one per provider: from `decision.api_key_env` or the
//! provider's own variable (`TYPESAFE_API_KEY`, `OPENAI_API_KEY`), else the macOS keychain
//! (service `orochi-decision`, one item per provider), else a `0600` file beside the data. Written
//! through `security -i` on stdin, since argv is visible to `ps`, and read back, since
//! `security -i` exits 0 on a failed command (both learned in `mcp/oauth.rs`). Tests turn
//! `decision.keychain` off: no test may touch the keychain of the machine it runs on.
use crate::config::DecisionConfig;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SERVICE: &str = "orochi-decision";

fn keychain(config: &DecisionConfig) -> bool {
    config.keychain && cfg!(target_os = "macos")
}
fn file(data: &Path, provider: &str) -> PathBuf {
    data.join("decision").join("keys").join(provider)
}
/// The data directory is part of the account, so two data directories keep separate keys.
fn account(data: &Path, provider: &str) -> String {
    let data = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
    format!("{}/{provider}", &digest[..16])
}
/// Only a provider Orochi knows, or `custom`, names a key: the name becomes a file name.
fn named(provider: &str) -> Result<()> {
    ensure!(
        provider == "custom" || super::provider(provider).is_some(),
        "no such provider: {provider}"
    );
    Ok(())
}
fn stored(data: &Path, config: &DecisionConfig, provider: &str) -> Option<String> {
    if keychain(config) {
        let output = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE, "-a"])
            .arg(account(data, provider))
            .arg("-w")
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        return Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|k| !k.is_empty());
    }
    std::fs::read_to_string(file(data, provider))
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
}

pub fn get(data: &Path, config: &DecisionConfig, provider: &str) -> Option<String> {
    named(provider).ok()?;
    let variable = config
        .api_key_env
        .clone()
        .or_else(|| super::provider(provider).map(|p| p.key_env.to_owned()));
    variable
        .and_then(|v| std::env::var(v).ok())
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| stored(data, config, provider))
}

pub fn put(data: &Path, config: &DecisionConfig, provider: &str, key: &str) -> Result<()> {
    named(provider)?;
    let key = key.trim();
    ensure!(
        !key.is_empty() && key.len() <= 512 && key.chars().all(|c| c.is_ascii_graphic()),
        "that does not look like an API key"
    );
    if !keychain(config) {
        let path = file(data, provider);
        std::fs::create_dir_all(path.parent().expect("key directory"))?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(&path)?.write_all(key.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        return Ok(());
    }
    use std::io::Write;
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    let line = format!(
        "add-generic-password -U -a \"{}\" -s \"{SERVICE}\" -X \"{hex}\"\n",
        account(data, provider)
    );
    let mut child = std::process::Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("security stdin")
        .write_all(line.as_bytes())?;
    let output = child.wait_with_output()?;
    ensure!(
        stored(data, config, provider).as_deref() == Some(key),
        "the keychain did not take the key: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// `false` when there was nothing to forget.
pub fn forget(data: &Path, config: &DecisionConfig, provider: &str) -> Result<bool> {
    named(provider)?;
    let mut removed = match std::fs::remove_file(file(data, provider)) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if keychain(config) {
        removed |= std::process::Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", SERVICE, "-a"])
            .arg(account(data, provider))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    Ok(removed)
}
