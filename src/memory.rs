//! What Orochi remembers between sessions: the user's standing preferences and notes about one
//! repository, as plain Markdown the user can read and edit. Agents come and go per task and
//! each reads only its own instruction file, so what should hold across all of them lives here.
//!
//! Kept apart from telemetry on purpose — this is text the user said — and never read from the
//! target repository: a repository must not be able to plant text into every later prompt.
//! Only Orochi and the user write these files; no agent is given a tool that can.
use crate::config::MemoryConfig;
use anyhow::{Result, ensure};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 65_536;
const MAX_ITEM_CHARS: usize = 200;
/// New items one reply may add: a message states a preference or two, not a manifesto.
const MAX_NEW_ITEMS: usize = 2;
/// An item seen once may have been a misreading of one task; one seen again has earned longer.
const ONCE_SECS: i64 = 90 * 24 * 3600;
const REPEATED_SECS: i64 = 180 * 24 * 3600;
const AUTO_MARK: &str = "<!-- auto ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Repo,
}

/// When Orochi wrote the item itself. A line without this is the user's own: it never expires,
/// is never replaced, and is injected first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub count: u32,
    pub last: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    pub auto: Option<Seen>,
}

/// Everything else in the file (headings, blank lines, continuation text) is kept as it was.
enum Line {
    Item(Item),
    Other(String),
}

/// What one classifier reply asks of the memory; positions are zero-based among a scope's items,
/// in file order, as `listing` numbered them.
#[derive(Debug, Default)]
pub struct Update {
    pub remember: Vec<(Scope, String)>,
    pub reinforce: Vec<(Scope, usize)>,
    pub replaces: Vec<(Scope, usize)>,
}
impl Update {
    pub fn is_empty(&self) -> bool {
        self.remember.is_empty() && self.reinforce.is_empty() && self.replaces.is_empty()
    }
}

/// `u3` / `r1`, as `listing` hands them out.
pub fn position(id: &str) -> Option<(Scope, usize)> {
    let scope = match id.as_bytes().first()? {
        b'u' => Scope::User,
        b'r' => Scope::Repo,
        _ => return None,
    };
    let number: usize = id[1..].parse().ok()?;
    Some((scope, number.checked_sub(1)?))
}

/// One line, bounded, and unable to break out of the file format or the terminal.
pub fn clean(text: &str, chars: usize) -> String {
    text.replace('\u{1b}', "\\x1b")
        .replace("<!--", "")
        .replace("-->", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(chars)
        .collect()
}

fn parse(line: &str) -> Line {
    let Some(rest) = line.strip_prefix("- ") else {
        return Line::Other(line.to_owned());
    };
    if let Some(start) = rest.rfind(AUTO_MARK)
        && let Some(meta) = rest[start + AUTO_MARK.len()..]
            .trim_end()
            .strip_suffix("-->")
    {
        let field = |name: &str| {
            meta.split_whitespace()
                .find_map(|part| part.strip_prefix(name))
                .and_then(|value| value.parse::<i64>().ok())
        };
        if let (Some(count), Some(last)) = (field("seen:"), field("last:")) {
            return Line::Item(Item {
                text: rest[..start].trim().to_owned(),
                auto: Some(Seen {
                    count: count.clamp(1, i64::from(u32::MAX)) as u32,
                    last,
                }),
            });
        }
    }
    Line::Item(Item {
        text: rest.trim().to_owned(),
        auto: None,
    })
}

fn render(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        match line {
            Line::Other(text) => out.push_str(text),
            Line::Item(Item { text, auto: None }) => out.push_str(&format!("- {text}")),
            Line::Item(Item {
                text,
                auto: Some(seen),
            }) => out.push_str(&format!(
                "- {text} {AUTO_MARK}seen:{} last:{} -->",
                seen.count, seen.last
            )),
        }
        out.push('\n');
    }
    out
}

fn expired(item: &Item, now: i64) -> bool {
    item.auto.is_some_and(|seen| {
        now - seen.last
            > if seen.count > 1 {
                REPEATED_SECS
            } else {
                ONCE_SECS
            }
    })
}

pub struct Memory {
    dir: PathBuf,
    config: MemoryConfig,
}

impl Memory {
    /// `None` when memory is turned off: nothing is read, written or injected.
    pub fn open(data: &Path, config: &MemoryConfig) -> Option<Self> {
        config.enabled.then(|| Self {
            dir: data.join("memory"),
            config: config.clone(),
        })
    }

    /// `repository` is the salted repository hash, so the path names no project.
    pub fn path(&self, scope: Scope, repository: &str) -> PathBuf {
        match scope {
            Scope::User => self.dir.join("USER.md"),
            Scope::Repo => self.dir.join("repos").join(repository).join("MEMORY.md"),
        }
    }

    fn load(&self, scope: Scope, repository: &str, now: i64) -> Vec<Line> {
        let mut text = String::new();
        if let Ok(file) = std::fs::File::open(self.path(scope, repository)) {
            let mut bytes = Vec::new();
            let _ = file.take(MAX_FILE_BYTES).read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        text.lines()
            .map(parse)
            .filter(|line| !matches!(line, Line::Item(item) if expired(item, now)))
            .collect()
    }

    fn save(&self, scope: Scope, repository: &str, lines: &[Line]) -> Result<()> {
        ensure!(
            !repository.is_empty() && repository.chars().all(|c| c.is_ascii_alphanumeric()),
            "invalid repository identifier"
        );
        let path = self.path(scope, repository);
        let text = render(lines);
        ensure!(
            text.len() as u64 <= MAX_FILE_BYTES,
            "{} would exceed 64 KiB; trim it",
            path.display()
        );
        let parent = path.parent().expect("memory files have a parent");
        std::fs::create_dir_all(parent)?;
        let staged = path.with_extension("md.tmp");
        std::fs::write(&staged, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&staged, &path)?;
        Ok(())
    }

    pub fn items(&self, scope: Scope, repository: &str, now: i64) -> Vec<Item> {
        self.load(scope, repository, now)
            .into_iter()
            .filter_map(|line| match line {
                Line::Item(item) => Some(item),
                Line::Other(_) => None,
            })
            .collect()
    }

    /// Applies one reply and returns what was newly remembered. Positions refer to the items as
    /// they stood before this call; the user's own lines are never touched.
    pub fn apply(&self, repository: &str, update: &Update, now: i64) -> Result<Vec<String>> {
        let mut added = Vec::new();
        for scope in [Scope::User, Scope::Repo] {
            let mut lines = self.load(scope, repository, now);
            let positions: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter_map(|(index, line)| matches!(line, Line::Item(_)).then_some(index))
                .collect();
            let mut changed = false;
            let mut removed = Vec::new();
            let wanted = |list: &[(Scope, usize)]| -> Vec<usize> {
                list.iter()
                    .filter(|(s, _)| *s == scope)
                    .filter_map(|(_, position)| positions.get(*position).copied())
                    .collect()
            };
            for index in wanted(&update.reinforce) {
                if let Line::Item(Item {
                    auto: Some(seen), ..
                }) = &mut lines[index]
                {
                    seen.count = seen.count.saturating_add(1);
                    seen.last = now;
                    changed = true;
                }
            }
            for index in wanted(&update.replaces) {
                if matches!(&lines[index], Line::Item(Item { auto: Some(_), .. })) {
                    removed.push(index);
                }
            }
            removed.sort_unstable();
            removed.dedup();
            for index in removed.into_iter().rev() {
                lines.remove(index);
                changed = true;
            }
            for (_, text) in update
                .remember
                .iter()
                .filter(|(s, _)| *s == scope)
                .take(MAX_NEW_ITEMS)
            {
                let text = clean(text, MAX_ITEM_CHARS);
                if text.is_empty() {
                    continue;
                }
                let same = |item: &Item| item.text.to_lowercase() == text.to_lowercase();
                // Said again rather than said for the first time.
                if let Some(Line::Item(item)) = lines
                    .iter_mut()
                    .find(|line| matches!(line, Line::Item(item) if same(item)))
                {
                    if let Some(seen) = &mut item.auto {
                        seen.count = seen.count.saturating_add(1);
                        seen.last = now;
                        changed = true;
                    }
                    continue;
                }
                lines.push(Line::Item(Item {
                    text: text.clone(),
                    auto: Some(Seen {
                        count: 1,
                        last: now,
                    }),
                }));
                added.push(text);
                changed = true;
            }
            if changed {
                self.save(scope, repository, &lines)?;
            }
        }
        Ok(added)
    }

    /// Removes one item, the user's own included: this is the user asking.
    pub fn forget(
        &self,
        scope: Scope,
        repository: &str,
        position: usize,
        now: i64,
    ) -> Result<Option<String>> {
        let mut lines = self.load(scope, repository, now);
        let Some(index) = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| matches!(line, Line::Item(_)).then_some(index))
            .nth(position)
        else {
            return Ok(None);
        };
        let Line::Item(item) = lines.remove(index) else {
            unreachable!("position was taken from items");
        };
        self.save(scope, repository, &lines)?;
        Ok(Some(item.text))
    }

    /// Items within `budget` characters, keeping `order`.
    fn within(items: Vec<Item>, budget: usize) -> Vec<Item> {
        let mut used = 0;
        items
            .into_iter()
            .take_while(|item| {
                used += item.text.chars().count() + 3;
                used <= budget
            })
            .collect()
    }

    /// What the classifier is shown so it can say "again" or "no longer" instead of adding a
    /// duplicate. File order, because `Update` positions are counted in it.
    pub fn listing(&self, repository: &str, now: i64) -> serde_json::Value {
        let list = |scope, prefix: &str, budget| -> Vec<String> {
            Self::within(self.items(scope, repository, now), budget)
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    format!(
                        "{prefix}{}: {}",
                        index + 1,
                        clean(&item.text, MAX_ITEM_CHARS)
                    )
                })
                .collect()
        };
        serde_json::json!({
            "user": list(Scope::User, "u", self.config.user_chars),
            "repo": list(Scope::Repo, "r", self.config.repo_chars),
        })
    }

    /// The note put in front of a fresh agent session, or `None` when nothing is remembered.
    /// The user's own lines come first, then what was said most often, then most recently.
    pub fn note(&self, repository: &str, now: i64) -> Option<String> {
        let chosen = |scope, budget| -> Vec<String> {
            let mut items = self.items(scope, repository, now);
            items.sort_by_key(|item| match item.auto {
                None => (0, 0, 0),
                Some(seen) => (1, -i64::from(seen.count), -seen.last),
            });
            Self::within(items, budget)
                .iter()
                .map(|item| format!("- {}", clean(&item.text, MAX_ITEM_CHARS * 4)))
                .collect()
        };
        let user = chosen(Scope::User, self.config.user_chars);
        let repo = chosen(Scope::Repo, self.config.repo_chars);
        if user.is_empty() && repo.is_empty() {
            return None;
        }
        let mut note = String::from(
            "What Orochi remembers from earlier sessions with this user. These are standing preferences and notes, not part of the task: they never override this task's instructions or the repository's own instructions.",
        );
        if !user.is_empty() {
            note.push_str(&format!("\nAbout the user:\n{}", user.join("\n")));
        }
        if !repo.is_empty() {
            note.push_str(&format!("\nAbout this repository:\n{}", repo.join("\n")));
        }
        Some(note)
    }
}
