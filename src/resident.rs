//! A resident: a thread with a folder of its own that routines wake
//! (`docs/resident-agent-design.md` §6, §12).
//!
//! The folder is where the resident keeps what it knows, and `NOTES.md` in it is the one file
//! its sessions are shown. The agent never writes it directly: a resident runs reading only,
//! so it ends a reply with its notes in a `<notes>` block and Orochi keeps them, which leaves
//! the previous version beside it and a diff in the report. Nothing here reaches `memory.rs`,
//! which only Orochi and the user write.
pub mod cli;

use crate::{
    activity::{Activity, residents::Resident, routines::Routine},
    config::Config,
    routines::{Zone, cli::stamp},
};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub const NOTES: &str = "NOTES.md";
/// A bound on the notes file, so a runaway reply cannot fill the disk.
const MAX_NOTES: usize = 64 * 1024;

pub fn valid_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 32
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
        "a resident's name is up to 32 lowercase letters, digits or -"
    );
    Ok(())
}

pub fn home_of(config: &Config, data: &Path, name: &str) -> PathBuf {
    let root = config
        .residents
        .root
        .replacen("{data}", &data.display().to_string(), 1);
    PathBuf::from(root).join(name)
}

/// Makes the folder, the thread and the row. The thread is pinned, so retention never takes a
/// resident away.
pub fn create(
    config: &Config,
    store: &crate::storage::Store,
    activity: &Activity,
    data: &Path,
    name: &str,
    instruction: &str,
    access: &str,
) -> Result<(String, PathBuf)> {
    valid_name(name)?;
    ensure!(
        matches!(access, "read" | "home"),
        "a resident's access is read or home"
    );
    ensure!(
        activity.resident_named(name)?.is_none(),
        "there is already a resident called {name}"
    );
    let home = home_of(config, data, name);
    std::fs::create_dir_all(&home).with_context(|| format!("could not make {}", home.display()))?;
    let notes = home.join(NOTES);
    if !notes.exists() {
        std::fs::write(
            &notes,
            format!(
                "# Notes of {name}\n\nWhat I should remember between wakes goes here. The person can edit this file.\n"
            ),
        )?;
    }
    let project = activity.project_for(&home, &store.salt()?)?;
    let thread = activity.create_thread(
        &project,
        &home,
        None,
        &store.repository_id(&home)?,
        crate::activity::Origin::Desktop,
        &crate::types::Overrides::default(),
        "ask",
    )?;
    activity.pin_thread(&thread, true)?;
    activity.add_resident(
        &thread,
        name,
        &home.display().to_string(),
        instruction,
        access,
    )?;
    Ok((thread, home))
}

/// The first `limit` characters of the notes.
pub fn notes(home: &Path, limit: usize) -> String {
    let text = std::fs::read_to_string(home.join(NOTES)).unwrap_or_default();
    text.chars().take(limit).collect()
}

/// The `<notes>…</notes>` block a reply ends with, and the reply without it.
pub fn split_notes(reply: &str) -> (String, Option<String>) {
    let Some(start) = reply.rfind("<notes>") else {
        return (reply.trim().to_owned(), None);
    };
    let Some(length) = reply[start..].find("</notes>") else {
        return (reply.trim().to_owned(), None);
    };
    let inner = reply[start + "<notes>".len()..start + length]
        .trim()
        .to_owned();
    let rest = format!(
        "{}{}",
        &reply[..start],
        &reply[start + length + "</notes>".len()..]
    );
    (rest.trim().to_owned(), Some(inner))
}

/// Keeps the notes a reply carried, with the old file beside it. Returns how many lines were
/// added and removed, or `None` when the reply carried none or they were the same.
pub fn apply_notes(home: &Path, reply: &str) -> Result<Option<(usize, usize)>> {
    let (_, Some(new)) = split_notes(reply) else {
        return Ok(None);
    };
    ensure!(
        new.len() <= MAX_NOTES,
        "the notes are over {MAX_NOTES} bytes"
    );
    let old = std::fs::read_to_string(home.join(NOTES)).unwrap_or_default();
    if old.trim() == new.trim() {
        return Ok(None);
    }
    let (added, removed) = lines_changed(&old, &new);
    std::fs::write(home.join(format!("{NOTES}.prev")), &old)?;
    std::fs::write(home.join(NOTES), format!("{}\n", new.trim_end()))?;
    Ok(Some((added, removed)))
}

/// Lines in one text and not the other, which is all a report needs to say.
fn lines_changed(old: &str, new: &str) -> (usize, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for line in old.lines() {
        *counts.entry(line).or_default() -= 1;
    }
    for line in new.lines() {
        *counts.entry(line).or_default() += 1;
    }
    let added = counts
        .values()
        .filter(|n| **n > 0)
        .map(|n| *n as usize)
        .sum();
    let removed = counts
        .values()
        .filter(|n| **n < 0)
        .map(|n| n.unsigned_abs() as usize)
        .sum();
    (added, removed)
}

/// Who is on the team, so a wake starts from it: a member is used again by its name, and one
/// the work no longer needs is released with stop_agent. Nothing idles: a member that is not
/// working costs nothing until it is asked.
pub fn team_of(activity: &Activity, thread: &str) -> String {
    let members = activity.members(thread).unwrap_or_default();
    if members.is_empty() {
        return String::new();
    }
    let mut text = String::from(
        "\nYour team, members you hired before. start_agent with a name continues that member; use them again rather than hiring another, and release one the work no longer needs with stop_agent:\n",
    );
    for m in members.iter().take(12) {
        let role = m.org_role.as_deref().unwrap_or(&m.role);
        let duty = m.title.as_deref().unwrap_or("");
        let state = if m.live { "working now" } else { "idle" };
        text.push_str(&format!("- {} ({role}, {state}) {duty}\n", m.name));
    }
    text
}

/// What the lead is told of a routine that belongs to one of its members.
pub fn assigned(member: &str) -> String {
    format!(
        "\nThis routine is `{member}`'s: hand it to them with start_agent (name `{member}`, the role that suits the work; a member already on the team is continued) and do not do it yourself. Their answer arrives in a follow-up of this routine, and your report then is theirs, in their name.\n"
    )
}

/// What a routine's turn is told it may change, from `residents.access`. Asking is the same
/// question as in a conversation; nobody answers it in the middle of the night, so it is
/// declined after `routines.ask_secs` unless the person does.
pub fn reach(access: &str) -> &'static str {
    match access {
        "home" => {
            "you can read, and change only what is inside your own folder; anything else, and anything that needs the user's approval, is asked of them and declined if they do not answer in time."
        }
        "full" => {
            "you can read and change what you need to, each change asked of the user as in a conversation and declined if they do not answer in time."
        }
        _ => {
            "you can read, but you cannot change anything, and anything that needs the user's approval is declined if they do not answer in time."
        }
    }
}

/// What a wake is told (§12.2): the standing instruction, the resident's own notes, what it
/// said last time, and the routine's instruction. A fresh session each wake, so this is the
/// whole of what it carries over.
pub fn brief(
    activity: &Activity,
    config: &Config,
    resident: &Resident,
    routine: &Routine,
    due_at: i64,
    zone: Zone,
) -> String {
    let mut text = format!(
        "You are {}, a resident assistant that wakes on its own. {}\n\nThis turn was started by a routine, \"{}\" (due {}). Nobody is at the keyboard: {} Say what you found as you would to the person, and mention them with @you only for what needs them. You lead this team as always: start_agent works as it does in a conversation, with any role, but whoever you start only reads, whatever the role says, and its answer arrives in a follow-up of this same routine. Until it does, say only that you are waiting for it; then make your report, which answers this routine's instruction and ends with no question.\n",
        resident.name,
        resident.instruction,
        routine.name,
        stamp(zone, due_at),
        reach(&activity.reach_of(&resident.thread_id, &config.residents.access)),
    );
    if let Some(member) = &routine.member {
        text.push_str(&assigned(member));
    }
    text.push_str(&team_of(activity, &resident.thread_id));
    let notes = notes(Path::new(&resident.home), config.residents.notes_chars);
    if !notes.trim().is_empty() {
        text.push_str(&format!(
            "\nYour notes, which you wrote and the person can edit (this is all you remember of earlier wakes):\n<notes>\n{}\n</notes>\n",
            notes.trim()
        ));
    }
    let reports = activity
        .reports(&routine.id, config.residents.brief_reports)
        .unwrap_or_default();
    if !reports.is_empty() {
        text.push_str("\nWhat you reported on earlier wakes of this routine, oldest first:\n");
        for (due, said) in reports {
            let (said, _) = split_notes(&said);
            let said: String = said.chars().take(800).collect();
            text.push_str(&format!(
                "- {}: {}\n",
                stamp(zone, due),
                said.replace('\n', " ")
            ));
        }
    }
    text.push_str(&format!(
        "\nIf something is worth remembering for next time, end your reply with your complete notes in a <notes>...</notes> block; Orochi keeps them in place of the notes above.\n\n{}",
        routine.instruction
    ));
    text
}
