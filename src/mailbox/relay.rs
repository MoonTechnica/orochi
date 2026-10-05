//! File transport across the existing shared workspace mount: no host binary inside Linux,
//! no credential copies, listener or extra port. Uses the same RPC handler as host stdio.
use super::{Mailbox, SessionPeer};
use anyhow::{Result, ensure};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub struct Relay {
    directory: PathBuf,
    stop: Arc<AtomicBool>,
}
impl Relay {
    pub fn start(peer: &SessionPeer, root: &Path) -> Result<Self> {
        let active = super::ACTIVE.get().expect("registered mailbox");
        let parent = root.join(".orochi/mailbox");
        std::fs::create_dir_all(&parent)?;
        ensure!(
            parent.canonicalize()?.starts_with(root.canonicalize()?),
            "mailbox directory escapes the workspace"
        );
        let directory = tempfile::Builder::new()
            .prefix("relay-")
            .tempdir_in(&parent)?;
        let path = directory.path().to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let data = active.data.clone();
        let config = active.config.clone();
        let id = peer.id.clone();
        let delegate = peer.delegate;
        std::thread::Builder::new()
            .name("mailbox-relay".into())
            .spawn(move || {
                let Ok(mailbox) = Mailbox::open(&data, &config) else {
                    return;
                };
                while !stopped.load(Ordering::Acquire) {
                    let Ok(entries) = std::fs::read_dir(directory.path()) else {
                        break;
                    };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_none_or(|e| e != "request") {
                            continue;
                        }
                        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                            continue;
                        };
                        if !metadata.is_file() || metadata.len() > 64 * 1024 {
                            continue;
                        }
                        let Some(request) = std::fs::read(&path)
                            .ok()
                            .and_then(|b| serde_json::from_slice(&b).ok())
                        else {
                            continue;
                        };
                        if let Some(reply) = super::rpc(&mailbox, &id, delegate, &request) {
                            let result = (|| -> Result<()> {
                                let mut temporary =
                                    tempfile::NamedTempFile::new_in(directory.path())?;
                                std::io::Write::write_all(
                                    &mut temporary,
                                    &serde_json::to_vec(&reply)?,
                                )?;
                                temporary.persist(path.with_extension("response"))?;
                                Ok(())
                            })();
                            if result.is_err() {
                                return;
                            }
                        }
                        let _ = std::fs::remove_file(path);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })?;
        Ok(Self {
            directory: path,
            stop,
        })
    }
    pub fn server(&self) -> (&'static str, PathBuf, Vec<String>) {
        (
            super::SERVER_NAME,
            "python3".into(),
            vec![
                "-c".into(),
                include_str!("relay.py").into(),
                self.directory.to_string_lossy().into_owned(),
            ],
        )
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
