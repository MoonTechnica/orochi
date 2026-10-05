//! Durable workspaces owned by one Orochi conversation. Never merge or remove them implicitly.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspaces {
    pub source: PathBuf,
    pub directory: PathBuf,
    pub workspaces: Vec<Workspace>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root: PathBuf,
    pub base: String,
}

pub fn directory(root: &Path, id: &str) -> PathBuf {
    root.join(".orochi/sessions")
        .join(crate::context::hash(&[id.as_bytes()]))
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "git {}: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
impl Workspaces {
    pub fn open(source: &Path, directory: &Path) -> Result<Self> {
        let source = source.canonicalize()?;
        std::fs::create_dir_all(directory)?;
        let directory = directory.canonicalize()?;
        let path = directory.join("workspaces.json");
        if path.exists() {
            let state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
            ensure!(
                state.source == source && state.directory == directory,
                "session workspace source changed"
            );
            return Ok(state);
        }
        let state = Self {
            source,
            directory,
            workspaces: vec![],
        };
        state.save()?;
        Ok(state)
    }
    fn save(&self) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(self)?)?;
        file.persist(self.directory.join("workspaces.json"))?;
        Ok(())
    }
    /// Snapshot tracked edits and nonignored new files using a private index. The user's HEAD,
    /// branch and index are untouched. Every worker starts with the state visible at launch.
    pub fn create(&mut self, name: &str) -> Result<PathBuf> {
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "invalid workspace name"
        );
        let mut ordinal = 1;
        let root = loop {
            let label = if ordinal == 1 {
                name.to_owned()
            } else {
                format!("{name}-{ordinal}")
            };
            let root = self.directory.join(&label);
            if !root.exists() {
                break (label, root);
            }
            ordinal += 1;
        };
        let repository = PathBuf::from(git(&self.source, &["rev-parse", "--show-toplevel"])?);
        let relative = self.source.strip_prefix(&repository)?;
        let head = git(&self.source, &["rev-parse", "HEAD"])
            .context("writable helpers require a Git repository with a commit")?;
        let index_dir = tempfile::tempdir_in(&self.directory)?;
        let index = index_dir.path().join("index");
        let indexed = |args: &[&str]| -> Result<String> {
            let output = Command::new("git")
                .current_dir(&repository)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .output()?;
            ensure!(
                output.status.success(),
                "cannot snapshot workspace: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_owned())
        };
        indexed(&["read-tree", &head])?;
        indexed(&[
            "add",
            "-A",
            "--",
            ".",
            ":(glob,exclude)**/.orochi",
            ":(glob,exclude)**/.orochi/**",
        ])?;
        let tree = indexed(&["write-tree"])?;
        let base = git(
            &self.source,
            &[
                "-c",
                "user.name=Orochi",
                "-c",
                "user.email=orochi@localhost",
                "commit-tree",
                &tree,
                "-p",
                &head,
                "-m",
                "Orochi workspace starting state",
            ],
        )?;
        git(
            &self.source,
            &[
                "worktree",
                "add",
                "--detach",
                root.1.to_str().context("non-UTF8 workspace path")?,
                &base,
            ],
        )?;
        self.workspaces.push(Workspace {
            name: root.0,
            root: root.1.join(relative),
            base,
        });
        self.save()?;
        Ok(root.1.join(relative))
    }
    pub fn git_common(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(git(
            &self.source,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?))
    }
}
