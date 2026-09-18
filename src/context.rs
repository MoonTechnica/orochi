use crate::types::CheckResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

pub fn hash(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.len().to_le_bytes());
        digest.update(part);
    }
    format!("{:x}", digest.finalize())
}
pub fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}
/// One schema-valid JSON object at the end of `text` (optionally fenced). Some ACP bridges
/// stream local CLI notices first; an object embedded mid-response is not accepted.
pub fn trailing_json<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    for (start, _) in text.match_indices('{') {
        let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<T>();
        if let Some(Ok(value)) = values.next() {
            let tail = text[start + values.byte_offset()..].trim();
            if tail.is_empty() || tail == "```" {
                return Some(value);
            }
        }
    }
    None
}

pub fn bounded(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

pub fn changed_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for args in [
        vec!["diff", "--name-only", "-z"],
        vec!["diff", "--cached", "--name-only", "-z"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        if let Some(value) = git(root, &args) {
            files.extend(
                value
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .take(200)
                    .map(String::from),
            );
        }
    }
    files.sort();
    files.dedup();
    files.truncate(200);
    files
}

/// Paths, sizes and mtimes of the files an agent could change: git-visible files when
/// available, otherwise a bounded walk. `None` means unknown (too many files).
pub fn tree_fingerprint(root: &Path) -> Option<String> {
    let mut files: Vec<std::path::PathBuf> = match git(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        Some(list) => list
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| root.join(s))
            .collect(),
        None => walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                e.depth() == 0
                    || !matches!(
                        e.file_name().to_str(),
                        Some(".git" | "node_modules" | "target")
                    )
            })
            .filter_map(Result::ok)
            .filter(|e| !e.file_type().is_dir())
            .map(walkdir::DirEntry::into_path)
            .take(50_001)
            .collect(),
    };
    if files.len() > 50_000 {
        return None;
    }
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        digest.update(path.as_os_str().as_encoded_bytes());
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                digest.update(meta.len().to_le_bytes());
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .unwrap_or_default();
                digest.update(modified.as_nanos().to_le_bytes());
            }
            Err(_) => digest.update(b"missing"),
        }
    }
    Some(format!("{:x}", digest.finalize()))
}

pub fn cache_key(root: &Path, salt: &str, agent: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(salt.as_bytes());
    digest.update(root.as_os_str().as_encoded_bytes());
    digest.update(agent.as_bytes());
    for name in [
        "AGENTS.md",
        "CLAUDE.md",
        "GEMINI.md",
        ".mcp.json",
        ".gemini/settings.json",
        ".claude/settings.json",
        ".codex/config.toml",
    ] {
        digest.update(name.as_bytes());
        if let Ok(content) = std::fs::read(root.join(name)) {
            digest.update(content);
        }
    }
    format!("{:x}", digest.finalize())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEnvelope {
    pub original_task: String,
    pub constraints: Vec<String>,
    pub current_status: String,
    pub preexisting_changes: Vec<String>,
    pub changed_files: Vec<String>,
    pub git_diff_summary: String,
    pub tests: Vec<CheckResult>,
    pub failed_tests: Vec<String>,
    pub completed_items: Vec<String>,
    pub remaining_items: Vec<String>,
}
impl TaskEnvelope {
    pub fn new(task: &str, root: &Path) -> Self {
        Self { original_task: task.into(), constraints: vec!["Preserve the user's existing changes. Read repository instructions before editing.".into()], current_status: "pending".into(), preexisting_changes: changed_files(root), changed_files: vec![], git_diff_summary: String::new(), tests: vec![], failed_tests: vec![], completed_items: vec![], remaining_items: vec!["Complete the original task and verify the result.".into()] }
    }
    pub fn refresh(&mut self, root: &Path, status: &str, checks: Vec<CheckResult>) {
        self.current_status = status.into();
        self.changed_files = changed_files(root);
        let unstaged = git(root, &["diff", "--stat", "--no-ext-diff"]).unwrap_or_default();
        let staged =
            git(root, &["diff", "--cached", "--stat", "--no-ext-diff"]).unwrap_or_default();
        self.git_diff_summary = bounded(&format!("Unstaged:\n{unstaged}\nStaged:\n{staged}"), 4000);
        self.failed_tests = checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.name.clone())
            .collect();
        self.completed_items = checks
            .iter()
            .filter(|c| c.passed)
            .map(|c| format!("Check passed: {}", c.name))
            .collect();
        self.tests = checks;
    }
    pub fn prompt(&self, handoff: bool) -> String {
        let prefix = "You are executing a coding task scheduled by Orochi. The filesystem is the source of truth. Follow repository instructions, preserve existing changes, and verify your work.\n\n";
        if handoff {
            format!(
                "{prefix}Continue the task using this compact handoff. A previous attempt stopped; changes may be incomplete. Inspect the filesystem and git before continuing. Do not assume a passed check proves the whole task is complete.\n{}",
                serde_json::to_string(self).expect("serializable envelope")
            )
        } else {
            format!("{prefix}{}", self.original_task)
        }
    }
}
