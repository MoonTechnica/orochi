//! `orochi routines` and `orochi keeper` (`docs/resident-agent-design.md` §14.4).
use super::{Schedule, Zone, api, now};
use crate::{
    activity::{Activity, routines::Routine},
    config::{Config, Paths},
};
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub enum RoutinesCommand {
    /// Make a routine: the instruction runs in the thread whenever the schedule says.
    Add {
        /// The thread the routine wakes.
        thread: String,
        /// When: `daily at 08:00`, `weekdays at 09:00`, `every 2h from 09:00 to 19:00`,
        /// `every mon,thu at 10:00`, `at 15:00`, `on 2026-11-24 at 09:30`, `in 90m`,
        /// `cron 0 8 * * 1-5`. Times are this machine's local time.
        #[arg(long, required_unless_present = "watch", conflicts_with = "watch")]
        when: Option<String>,
        /// Instead of a time: a command to look at every `--every`, which wakes the thread
        /// only when what it prints changes (`gh run list --limit 5`). It runs in the
        /// thread's folder with nobody present, so only you can write one.
        #[arg(long, requires = "every")]
        watch: Option<String>,
        /// How often a `--watch` looks: `15m`, `2h`.
        #[arg(long)]
        every: Option<String>,
        /// A name for lists; the instruction's first words by default.
        #[arg(long)]
        name: Option<String>,
        /// Tell the user when it finishes: `always`, `mention` (when the agent mentions
        /// them) or `never`.
        #[arg(long, default_value = "mention")]
        notify: String,
        /// What to do when the machine was asleep at its time: run once, or skip it.
        #[arg(long, default_value = "once")]
        catch_up: String,
        /// What the instruction asks the agent to do.
        #[arg(last = true, required = true)]
        instruction: Vec<String>,
    },
    /// Routines, with when each is next due and how the last one went.
    List {
        #[arg(long)]
        thread: Option<String>,
    },
    /// One routine and its latest firings.
    Show {
        id: String,
    },
    /// Run it now, through the same checks a due time goes through.
    Run {
        id: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    /// Allow a routine an agent proposed, so it runs.
    Approve {
        id: String,
    },
    /// Decline a routine an agent proposed; it is removed.
    Decline {
        id: String,
    },
    Rm {
        id: String,
    },
    /// The next times a schedule is due, as the window shows them while it is typed.
    Preview {
        when: String,
        #[arg(long, default_value_t = 3)]
        count: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum KeeperCommand {
    /// Whether the keeper is running, and since when.
    Status,
    /// Start the keeper at login (macOS): a user LaunchAgent, no root. `off` removes it.
    Login {
        #[arg(value_parser = ["on", "off", "status"])]
        state: String,
    },
}

pub fn stamp(zone: Zone, at: i64) -> String {
    let p = zone.parts(at);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        p.year, p.month, p.day, p.hour, p.minute
    )
}

fn open(config: &Config, data: &Path) -> Result<Activity> {
    ensure!(
        config.activity.enabled,
        "routines are kept in the conversation store, and activity.enabled is false"
    );
    Activity::open(data, config.activity.retention_days)
}

fn routine_named(activity: &Activity, id: &str) -> Result<Routine> {
    activity
        .routine(id)?
        .with_context(|| format!("no routine {id}"))
}

pub fn run_routines(
    command: RoutinesCommand,
    config: &Config,
    paths: &Paths,
    json: bool,
) -> Result<u8> {
    let zone = Zone::Local;
    let at = now();
    if let RoutinesCommand::Preview { when, count } = &command {
        let (words, times) = api::preview(when, at, zone, *count)?;
        if json {
            println!(
                "{}",
                json!({ "when": words, "next": times, "text": times.iter().map(|t| stamp(zone, *t)).collect::<Vec<_>>() })
            );
        } else {
            println!("{words}");
            for time in times {
                println!("  {}", stamp(zone, time));
            }
        }
        return Ok(0);
    }
    ensure!(
        config.routines.enabled,
        "routines are off (routines.enabled)"
    );
    let activity = open(config, &paths.data)?;
    match command {
        RoutinesCommand::Preview { .. } => unreachable!("answered above"),
        RoutinesCommand::Add {
            thread,
            when,
            watch,
            every,
            name,
            notify,
            catch_up,
            instruction,
        } => {
            let thread = activity
                .resolve_thread(&thread)?
                .with_context(|| format!("no thread {thread}"))?;
            let made = match (&when, &watch) {
                (Some(when), _) => api::add(
                    config,
                    &activity,
                    &thread,
                    when,
                    name.as_deref(),
                    &instruction.join(" "),
                    &notify,
                    &catch_up,
                    false,
                    at,
                    zone,
                )?,
                (None, Some(command)) => api::add_watch(
                    config,
                    &activity,
                    &thread,
                    command,
                    every.as_deref().unwrap_or(""),
                    name.as_deref(),
                    &instruction.join(" "),
                    &notify,
                    false,
                    at,
                )?,
                (None, None) => bail!("give --when, or --watch with --every"),
            };
            let (id, next) = (made.id.clone(), made.next_at);
            let kept = crate::keeper::ensure(config, &paths.config, &paths.data)?;
            if json {
                println!(
                    "{}",
                    json!({ "routine": id, "next_at": next, "keeper_started": kept })
                );
            } else {
                println!("{id}");
                eprintln!("{}; next {}", made.words, stamp(zone, next.unwrap_or(at)));
                if cfg!(not(unix)) {
                    eprintln!("The keeper does not run on this platform yet, so nothing wakes it.");
                }
            }
        }
        RoutinesCommand::List { thread } => {
            let thread = thread
                .map(|t| {
                    activity
                        .resolve_thread(&t)?
                        .with_context(|| format!("no thread {t}"))
                })
                .transpose()?;
            let routines = activity.routines(thread.as_deref())?;
            if json {
                println!("{}", serde_json::to_string(&routines)?);
            } else if routines.is_empty() {
                println!(
                    "No routines. `orochi routines add <thread> --when \"daily at 08:00\" -- <what to do>`"
                );
            }
            if !json {
                for r in routines {
                    println!(
                        "{}  {:<24}  {:<28}  next {}  {}",
                        &r.id[..8],
                        r.name,
                        words(&r, zone),
                        if r.proposed {
                            "proposed — `orochi routines approve` to allow".into()
                        } else if r.paused {
                            "paused".into()
                        } else {
                            r.next_at.map_or("—".into(), |t| stamp(zone, t))
                        },
                        activity
                            .firings(&r.id, 1)?
                            .first()
                            .map(|f| f
                                .skipped
                                .clone()
                                .or(f.turn_state.clone())
                                .unwrap_or_default())
                            .unwrap_or_default()
                    );
                }
            }
        }
        RoutinesCommand::Show { id } => {
            let r = routine_named(&activity, &id)?;
            let firings = activity.firings(&r.id, 10)?;
            if json {
                println!("{}", json!({ "routine": r, "firings": firings }));
            } else {
                println!("{} ({})", r.name, r.id);
                println!("  when:    {}", words(&r, zone));
                println!(
                    "  next:    {}",
                    r.next_at.map_or("—".into(), |t| stamp(zone, t))
                );
                println!(
                    "  access:  {}  notify: {}  made by: {}",
                    r.access, r.notify, r.made_by
                );
                println!("  say:     {}", r.instruction);
                for f in firings {
                    println!(
                        "  {}  {}",
                        stamp(zone, f.due_at),
                        f.skipped
                            .map(|s| format!("skipped: {s}"))
                            .or(f.turn_state)
                            .unwrap_or_else(|| "started".into())
                    );
                }
            }
        }
        RoutinesCommand::Run { id } => {
            let r = routine_named(&activity, &id)?;
            let turn = crate::keeper::run_now(
                &std::env::current_exe()?,
                &activity,
                config,
                &paths.config,
                &paths.data,
                &r,
                at,
                zone,
            )?;
            match turn {
                Some(turn) if json => println!("{}", json!({ "turn": turn })),
                Some(turn) => println!("{turn}"),
                None => bail!("the last run of {} has not finished", r.name),
            }
        }
        RoutinesCommand::Pause { id } => {
            let r = routine_named(&activity, &id)?;
            activity.set_routine_paused(&r.id, true)?;
        }
        RoutinesCommand::Resume { id } => {
            let r = routine_named(&activity, &id)?;
            api::resume(&activity, &r, at, zone)?;
            crate::keeper::ensure(config, &paths.config, &paths.data)?;
        }
        RoutinesCommand::Approve { id } => {
            let r = routine_named(&activity, &id)?;
            api::approve(&activity, &r, at, zone)?;
            crate::keeper::ensure(config, &paths.config, &paths.data)?;
        }
        RoutinesCommand::Decline { id } => {
            let r = routine_named(&activity, &id)?;
            api::decline(&activity, &r)?;
        }
        RoutinesCommand::Rm { id } => {
            let r = routine_named(&activity, &id)?;
            activity.delete_routine(&r.id)?;
        }
    }
    Ok(0)
}

fn words(r: &Routine, zone: Zone) -> String {
    r.schedule_parsed
        .as_deref()
        .and_then(|s| serde_json::from_str::<Schedule>(s).ok())
        .map(|s| s.describe(zone))
        .or_else(|| r.watch.as_ref().map(|_| "watch".to_owned()))
        .unwrap_or_default()
}

pub fn run_keeper(command: KeeperCommand, paths: &Paths, json: bool) -> Result<u8> {
    match command {
        KeeperCommand::Status => {
            let status = crate::keeper::status(&paths.data)?;
            if json {
                println!(
                    "{}",
                    json!({ "running": status.is_some(), "status": status })
                );
            } else if let Some(s) = status {
                println!(
                    "running (pid {}, since {})",
                    s.pid,
                    stamp(Zone::Local, s.started_at)
                );
            } else {
                println!("not running");
            }
        }
        KeeperCommand::Login { state } => match state.as_str() {
            "on" => {
                crate::keeper::login_install(
                    &std::env::current_exe()?,
                    &paths.config,
                    &paths.data,
                )?;
                println!("on");
            }
            "off" => {
                crate::keeper::login_remove()?;
                println!("off");
            }
            _ => println!(
                "{}",
                if crate::keeper::login_installed() {
                    "on"
                } else {
                    "off"
                }
            ),
        },
    }
    Ok(0)
}
