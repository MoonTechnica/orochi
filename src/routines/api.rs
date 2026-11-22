//! What making, previewing and resuming a routine means, shared by the command line, the
//! window and (for what an agent may ask) the mailbox, so one rule is not written three times.
use super::{Schedule, Zone};
use crate::{
    activity::{
        Activity,
        routines::{NewRoutine, Routine},
    },
    config::Config,
};
use anyhow::{Context, Result, ensure};
use std::path::Path;

pub struct Made {
    pub id: String,
    pub next_at: Option<i64>,
    pub words: String,
}

/// The floor on how often a routine may wake, and the room a thread has for one more. An
/// agent only proposes, and the person allows, so one floor holds for both; what an agent is
/// limited in is how many proposals it may leave waiting.
pub fn admit(
    config: &Config,
    activity: &Activity,
    thread: &str,
    schedule: Option<&Schedule>,
    by_agent: bool,
    now: i64,
    zone: Zone,
) -> Result<()> {
    let floor = config.routines.min_interval_secs;
    if let Some(gap) = schedule.and_then(|s| s.shortest_gap(now, zone)) {
        ensure!(
            gap >= floor,
            "that wakes every {} minute(s), and the shortest allowed is {} (routines.min_interval_secs)",
            gap / 60,
            floor / 60,
        );
    }
    ensure!(
        activity.routines_on(thread)? < config.routines.max_per_thread,
        "this thread already has {} routines (routines.max_per_thread)",
        config.routines.max_per_thread
    );
    if by_agent {
        ensure!(
            activity.proposals_on(thread)? < config.routines.max_proposals,
            "{} proposals are already waiting for the person's answer (routines.max_proposals)",
            config.routines.max_proposals
        );
    }
    Ok(())
}

/// How long a proposal waits for an answer before it is dropped.
fn proposal_expiry(config: &Config, by_agent: bool, now: i64) -> Option<i64> {
    by_agent.then(|| now + config.routines.proposal_days * 86_400)
}

#[allow(clippy::too_many_arguments)]
pub fn add(
    config: &Config,
    activity: &Activity,
    thread: &str,
    when: &str,
    name: Option<&str>,
    instruction: &str,
    notify: &str,
    catch_up: &str,
    by_agent: bool,
    now: i64,
    zone: Zone,
) -> Result<Made> {
    ensure!(
        config.routines.enabled,
        "routines are off (routines.enabled)"
    );
    ensure!(
        matches!(notify, "always" | "mention" | "never"),
        "notify is always, mention or never"
    );
    ensure!(
        matches!(catch_up, "once" | "skip"),
        "catch-up is once or skip"
    );
    ensure!(
        !instruction.trim().is_empty(),
        "the routine needs an instruction"
    );
    let schedule = Schedule::parse(when, now, zone)?;
    admit(
        config,
        activity,
        thread,
        Some(&schedule),
        by_agent,
        now,
        zone,
    )?;
    let next = schedule.next_after(now, zone);
    ensure!(next.is_some(), "that is never due");
    let name = name.map(str::to_owned).unwrap_or_else(|| {
        instruction
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
    });
    // Labelled once, here, from the words alone: the same instruction is the same work every
    // time it wakes, so it is not read again at each wake.
    let descriptor = activity
        .thread_settings(thread)?
        .map(|(cwd, ..)| crate::router::profiler::profile(instruction, Path::new(&cwd)))
        .and_then(|d| serde_json::to_string(&d).ok());
    let expires_at = proposal_expiry(config, by_agent, now);
    let id = activity.add_routine(&NewRoutine {
        thread,
        name: &name,
        instruction,
        schedule: Some((when, serde_json::to_string(&schedule)?)),
        watch: None,
        access: "read",
        notify,
        catch_up,
        descriptor,
        made_by: if by_agent { "agent" } else { "user" },
        expires_at,
        next_at: next,
        now,
        proposed: by_agent,
    })?;
    Ok(Made {
        id,
        next_at: next,
        words: schedule.describe(zone),
    })
}

/// The next `count` times a schedule is due, and the schedule in words.
pub fn preview(when: &str, now: i64, zone: Zone, count: usize) -> Result<(String, Vec<i64>)> {
    let schedule = Schedule::parse(when, now, zone)?;
    let mut times = vec![];
    let mut after = now;
    while times.len() < count.clamp(1, 20)
        && let Some(next) = schedule.next_after(after, zone)
    {
        times.push(next);
        after = next;
    }
    Ok((schedule.describe(zone), times))
}

/// When a routine starting over from `now` is next due: a schedule's next time, a watch's
/// next look.
fn due_from(routine: &Routine, now: i64, zone: Zone) -> Option<i64> {
    match routine.every_secs {
        Some(secs) if routine.watch.is_some() => Some(now + secs),
        _ => routine
            .schedule_parsed
            .as_deref()
            .and_then(|s| serde_json::from_str::<Schedule>(s).ok())
            .and_then(|s| s.next_after(now, zone)),
    }
}

/// Starts a paused routine again from now: one paused for a week does not run a week's worth.
/// A proposal is not resumed; it is allowed.
pub fn resume(activity: &Activity, routine: &Routine, now: i64, zone: Zone) -> Result<()> {
    ensure!(
        !routine.proposed,
        "that routine is a proposal waiting for your answer; allow it instead"
    );
    let next = due_from(routine, now, zone);
    activity.set_routine_next(&routine.id, next)?;
    // A watch its failures paused starts its count again, or its next failure pauses it.
    activity.set_watch_state(&routine.id, None, 0)?;
    activity.set_routine_paused(&routine.id, false)?;
    Ok(())
}

/// The person allows an agent's proposal: from now on it runs like one they made.
pub fn approve(activity: &Activity, routine: &Routine, now: i64, zone: Zone) -> Result<()> {
    ensure!(
        routine.proposed,
        "that routine is not waiting for an answer"
    );
    let next = due_from(routine, now, zone);
    ensure!(
        activity.approve_routine(&routine.id, next)?,
        "that proposal was answered already"
    );
    Ok(())
}

/// The person declines a proposal: it is removed.
pub fn decline(activity: &Activity, routine: &Routine) -> Result<()> {
    ensure!(
        routine.proposed,
        "that routine is not waiting for an answer"
    );
    activity.delete_routine(&routine.id)?;
    Ok(())
}

pub fn named(activity: &Activity, id: &str) -> Result<Routine> {
    activity
        .routine(id)?
        .with_context(|| format!("no routine {id}"))
}

/// A watch: a command looked at every `every`, that wakes the thread only when what it prints
/// changes. An agent may propose one, but a command run with nobody present runs only once
/// the person has read it and allowed it.
#[allow(clippy::too_many_arguments)]
pub fn add_watch(
    config: &Config,
    activity: &Activity,
    thread: &str,
    command: &str,
    every: &str,
    name: Option<&str>,
    instruction: &str,
    notify: &str,
    by_agent: bool,
    now: i64,
) -> Result<Made> {
    ensure!(
        config.routines.enabled,
        "routines are off (routines.enabled)"
    );
    ensure!(
        matches!(notify, "always" | "mention" | "never"),
        "notify is always, mention or never"
    );
    ensure!(!command.trim().is_empty(), "a watch needs a command");
    ensure!(
        command.len() <= 2000,
        "that command is over 2000 characters"
    );
    ensure!(
        !instruction.trim().is_empty(),
        "the watch needs an instruction: what to do when it changes"
    );
    let secs = super::length(every.trim())?;
    ensure!(
        secs >= config.routines.min_interval_secs,
        "that looks every {} minute(s), and the shortest allowed is {} (routines.min_interval_secs)",
        secs / 60,
        config.routines.min_interval_secs / 60
    );
    admit(
        config,
        activity,
        thread,
        None,
        by_agent,
        now,
        super::Zone::Utc,
    )?;
    let name = name.map(str::to_owned).unwrap_or_else(|| {
        command
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" ")
    });
    let next = now + secs;
    let id = activity.add_routine(&NewRoutine {
        thread,
        name: &name,
        instruction,
        schedule: None,
        watch: Some((command, secs)),
        access: "read",
        notify,
        catch_up: "skip",
        descriptor: None,
        made_by: if by_agent { "agent" } else { "user" },
        expires_at: proposal_expiry(config, by_agent, now),
        next_at: Some(next),
        now,
        proposed: by_agent,
    })?;
    Ok(Made {
        id,
        next_at: Some(next),
        words: format!("watch every {}", every.trim()),
    })
}

/// Runs a watch's command once, as the keeper would, so it can be tried before it is made.
/// Returns the output (capped) and whether it succeeded.
pub async fn try_watch(
    config: &Config,
    data: &Path,
    thread: &str,
    activity: &Activity,
    command: &str,
) -> Result<(bool, String)> {
    let (cwd, ..) = activity
        .thread_settings(thread)?
        .with_context(|| format!("no thread {thread}"))?;
    crate::keeper::run_watch(
        command,
        Path::new(&cwd),
        data,
        config.routines.watch_timeout_secs,
    )
    .await
}
