//! The OneSignal REST API key: `notify.onesignal.api_key_env` or `ONESIGNAL_REST_API_KEY`, else
//! the macOS keychain (service `orochi-notify`), else a `0600` file beside the data. Written
//! through `security -i` on stdin (argv is visible to `ps`) and read back, since `security -i`
//! exits 0 on a failed command; the same care as `decision/key.rs`. The key can push anything
//! to every phone of the app, so it is never printed or put in a log. Tests turn
//! `notify.keychain` off: no test may touch the keychain of the machine it runs on.
use crate::config::NotifyConfig;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SERVICE: &str = "orochi-notify";
const DEFAULT_VARIABLE: &str = "ONESIGNAL_REST_API_KEY";

fn keychain(config: &NotifyConfig) -> bool {
    config.keychain && cfg!(target_os = "macos")
}
fn file(data: &Path) -> PathBuf {
    data.join("notify").join("onesignal.key")
}
fn account(data: &Path) -> String {
    let data = data.canonicalize().unwrap_or_else(|_| data.to_path_buf());
    let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
    format!("{}/onesignal", &digest[..16])
}

fn stored(data: &Path, config: &NotifyConfig) -> Option<String> {
    if keychain(config) {
        let output = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE, "-a"])
            .arg(account(data))
            .arg("-w")
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        return Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|k| !k.is_empty());
    }
    std::fs::read_to_string(file(data))
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
}

pub fn get(data: &Path, config: &NotifyConfig) -> Option<String> {
    let variable = Some(config.onesignal.api_key_env.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_VARIABLE);
    std::env::var(variable)
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| stored(data, config))
}

/// Whether a key is held, and where it came from, without saying what it is.
pub fn source(data: &Path, config: &NotifyConfig) -> Option<&'static str> {
    let variable = Some(config.onesignal.api_key_env.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_VARIABLE);
    if std::env::var(variable).is_ok_and(|k| !k.trim().is_empty()) {
        Some("environment")
    } else if stored(data, config).is_some() {
        Some(if keychain(config) { "keychain" } else { "file" })
    } else {
        None
    }
}

pub fn put(data: &Path, config: &NotifyConfig, key: &str) -> Result<()> {
    let key = key.trim();
    ensure!(
        !key.is_empty() && key.len() <= 512 && key.chars().all(|c| c.is_ascii_graphic()),
        "that does not look like an API key"
    );
    if !keychain(config) {
        let path = file(data);
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
        account(data)
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
        stored(data, config).as_deref() == Some(key),
        "the keychain did not take the key: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// `false` when there was nothing to forget.
pub fn forget(data: &Path, config: &NotifyConfig) -> Result<bool> {
    let mut removed = match std::fs::remove_file(file(data)) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if keychain(config) {
        removed |= std::process::Command::new("/usr/bin/security")
            .args(["delete-generic-password", "-s", SERVICE, "-a"])
            .arg(account(data))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    Ok(removed)
}
