//! `orochi resident` and `orochi notices` / `orochi notify` (`docs/resident-agent-design.md` §14.4).
use crate::{
    activity::Activity,
    config::{Config, Paths},
    routines::{Zone, cli::stamp},
    storage::Store,
};
use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub enum ResidentCommand {
    /// Make a resident: a thread with a folder of its own, for routines to wake.
    New {
        name: String,
        /// Who it is and what it is for; shown to it at every wake.
        #[arg(
            long,
            default_value = "You help the person keep track of what matters to them."
        )]
        instruction: String,
    },
    List,
    Show {
        name: String,
    },
    /// Stop its routines waking it, without deleting anything.
    Pause {
        name: String,
    },
    Resume {
        name: String,
    },
    /// Delete the resident, its thread and its routines. Its folder is kept unless --folder.
    Rm {
        name: String,
        #[arg(long)]
        folder: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum NoticesCommand {
    /// Mark one notice read, or all of them.
    Read { id: Option<i64> },
}

#[derive(Debug, Subcommand)]
pub enum NotifyCommand {
    /// Send a test notice through every sink that is on, and say what happened to each.
    Test,
    /// The phones this machine may push to.
    Phones,
    /// Register a phone's push id by hand (the phone app does it when it pairs).
    PhoneAdd {
        id: String,
        name: String,
    },
    PhoneRm {
        id: String,
    },
    /// Keep the OneSignal REST API key (read from stdin, never from an argument).
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    Set,
    Forget,
    Status,
}

fn open(config: &Config, data: &Path) -> Result<Activity> {
    ensure!(config.activity.enabled, "activity.enabled is false");
    Activity::open(data, config.activity.retention_days)
}

pub fn run_resident(
    command: ResidentCommand,
    config: &Config,
    paths: &Paths,
    json: bool,
) -> Result<u8> {
    let activity = open(config, &paths.data)?;
    let named = |name: &str| {
        activity
            .resident_named(name)?
            .with_context(|| format!("no resident {name}"))
    };
    match command {
        ResidentCommand::New { name, instruction } => {
            let store = Store::open(&paths.data)?;
            let (thread, home) = super::create(
                config,
                &store,
                &activity,
                &paths.data,
                &name,
                &instruction,
                "read",
            )?;
            if json {
                println!("{}", json!({ "thread": thread, "home": home }));
            } else {
                println!("{thread}");
                eprintln!("{name} lives in {}", home.display());
            }
        }
        ResidentCommand::List => {
            let all = activity.residents()?;
            if json {
                println!("{}", serde_json::to_string(&all)?);
            } else if all.is_empty() {
                println!("No residents. `orochi resident new <name>`");
            }
            if !json {
                for r in all {
                    println!(
                        "{:<16} {}  {}  {}",
                        r.name,
                        &r.thread_id[..8],
                        if r.paused { "paused" } else { "" },
                        r.home
                    );
                }
            }
        }
        ResidentCommand::Show { name } => {
            let r = named(&name)?;
            let routines = activity.routines(Some(&r.thread_id))?;
            let notes = super::notes(Path::new(&r.home), config.residents.notes_chars);
            if json {
                println!(
                    "{}",
                    json!({ "resident": r, "routines": routines, "notes": notes })
                );
            } else {
                println!("{} ({})", r.name, r.thread_id);
                println!("  home:        {}", r.home);
                println!(
                    "  access:      {}{}",
                    r.access,
                    if r.paused { " (paused)" } else { "" }
                );
                println!("  instruction: {}", r.instruction);
                for routine in routines {
                    println!(
                        "  routine {}  {}  next {}",
                        &routine.id[..8],
                        routine.name,
                        routine
                            .next_at
                            .map_or("—".into(), |t| stamp(Zone::Local, t))
                    );
                }
            }
        }
        ResidentCommand::Pause { name } => {
            activity.set_resident_paused(&named(&name)?.thread_id, true)?;
        }
        ResidentCommand::Resume { name } => {
            activity.set_resident_paused(&named(&name)?.thread_id, false)?;
        }
        ResidentCommand::Rm { name, folder } => {
            let r = named(&name)?;
            activity.delete_thread(&r.thread_id)?;
            if folder {
                std::fs::remove_dir_all(&r.home)
                    .with_context(|| format!("could not remove {}", r.home))?;
            }
        }
    }
    Ok(0)
}

pub fn run_notices(
    command: Option<NoticesCommand>,
    unread: bool,
    config: &Config,
    paths: &Paths,
    json: bool,
) -> Result<u8> {
    let activity = open(config, &paths.data)?;
    if let Some(NoticesCommand::Read { id }) = command {
        activity.read_notices(id)?;
        return Ok(0);
    }
    let all = activity.notices(unread, 50)?;
    if json {
        println!("{}", serde_json::to_string(&all)?);
        return Ok(0);
    }
    if all.is_empty() {
        println!("Nothing to read.");
    }
    for n in all {
        println!(
            "{:>4} {} {:<9} {}  [mac {} · phone {}]",
            n.id,
            if n.read_at.is_none() { "●" } else { " " },
            n.kind,
            n.title,
            n.mac.as_deref().unwrap_or("—"),
            n.phone.as_deref().unwrap_or("—"),
        );
    }
    Ok(0)
}

pub async fn run_notify(
    command: NotifyCommand,
    config: &Config,
    paths: &Paths,
    json: bool,
) -> Result<u8> {
    match command {
        NotifyCommand::Test => {
            let (id, sent) = crate::notify::send_test(config, &paths.data).await?;
            if json {
                println!(
                    "{}",
                    json!({ "notice": id, "mac": sent.mac, "phone": sent.phone })
                );
            } else {
                println!(
                    "mac: {}  phone: {}",
                    sent.mac.as_deref().unwrap_or("—"),
                    sent.phone.as_deref().unwrap_or("—")
                );
            }
        }
        NotifyCommand::Phones => {
            let all = crate::notify::phones(&paths.data);
            if json {
                println!("{}", serde_json::to_string(&all)?);
            } else if all.is_empty() {
                println!("No phones. The Orochi phone app registers one when it pairs.");
            }
            if !json {
                for p in all {
                    println!("{}  {}", p.name, p.id);
                }
            }
        }
        NotifyCommand::PhoneAdd { id, name } => crate::notify::add_phone(&paths.data, &id, &name)?,
        NotifyCommand::PhoneRm { id } => {
            if !crate::notify::remove_phone(&paths.data, &id)? {
                bail!("no phone with that id");
            }
        }
        NotifyCommand::Key { command } => match command {
            KeyCommand::Set => {
                let mut key = String::new();
                std::io::stdin().read_line(&mut key)?;
                crate::notify::key::put(&paths.data, &config.notify, &key)?;
            }
            KeyCommand::Forget => {
                crate::notify::key::forget(&paths.data, &config.notify)?;
            }
            KeyCommand::Status => {
                let source = crate::notify::key::source(&paths.data, &config.notify);
                if json {
                    println!("{}", json!({ "held": source.is_some(), "source": source }));
                } else {
                    println!(
                        "{}",
                        source.map_or("no key".to_owned(), |s| format!("held ({s})"))
                    );
                }
            }
        },
    }
    Ok(0)
}
