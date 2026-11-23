//! Residents, routines, notices and the keeper, for the window and (the reading ones and a
//! few actions) the phone. Every call goes through the same library functions the command
//! line uses (`orochi::routines::api`, `orochi::resident`, `orochi::notify`), so a routine
//! made here is the one `orochi routines add` would have made.
use super::Client;
use anyhow::{Context, Result, ensure};
use orochi::{
    activity::{residents::Notice, routines::Firing},
    config::Config,
    routines::{Zone, api, now},
};
use serde_json::{Value, json};
use std::path::Path;

impl Client {
    /// Opens a page in the browser only if the agent read it in this conversation: the address
    /// is one of the thread's own fetches, so a link in a reply cannot send the window anywhere.
    pub fn open_source(&self, thread: &str, url: &str) -> Result<()> {
        ensure!(
            url.starts_with("https://") || url.starts_with("http://"),
            "only a web page opens"
        );
        let fetched: bool = self.activity.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM items WHERE thread_id=?1 AND kind='tool_call'
               AND json_extract(data, '$.raw_input.url')=?2)",
            rusqlite::params![thread, url],
            |r| r.get(0),
        )?;
        ensure!(fetched, "that page was not read in this conversation");
        let mut command = if cfg!(target_os = "macos") {
            std::process::Command::new("/usr/bin/open")
        } else {
            std::process::Command::new("xdg-open")
        };
        command
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("could not open it")?;
        Ok(())
    }

    pub fn residents(&self) -> Result<Vec<Value>> {
        let mut statement = self.activity.connection().prepare(
            "SELECT thread_id, name, home, instruction, access, paused, created_at, next_at,
                    routines, asking, working FROM v_residents ORDER BY created_at, name",
        )?;
        let rows = statement.query_map([], |r| {
            Ok(json!({
                "thread_id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?,
                "home": r.get::<_, String>(2)?, "instruction": r.get::<_, String>(3)?,
                "access": r.get::<_, String>(4)?, "paused": r.get::<_, i64>(5)? != 0,
                "created_at": r.get::<_, i64>(6)?, "next_at": r.get::<_, Option<i64>>(7)?,
                "routines": r.get::<_, i64>(8)?, "asking": r.get::<_, i64>(9)?,
                "working": r.get::<_, i64>(10)? != 0,
            }))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn routines(&self) -> Result<Vec<Value>> {
        let mut statement = self.activity.connection().prepare(
            "SELECT routine_id, thread_id, thread_title, name, instruction, schedule, watch,
                    access, notify, catch_up, made_by, expires_at, paused, next_at, last_at,
                    last_skipped, last_state,
                    (SELECT COALESCE(sum(f.tokens),0) FROM firings f
                      WHERE f.routine_id=v_routines.routine_id AND f.started_at>=?1),
                    proposed, every_secs, member
             FROM v_routines ORDER BY created_at, name",
        )?;
        let today = day_start(now());
        let rows = statement.query_map([today], |r| {
            Ok(json!({
                "id": r.get::<_, String>(0)?, "thread_id": r.get::<_, String>(1)?,
                "thread_title": r.get::<_, Option<String>>(2)?, "name": r.get::<_, String>(3)?,
                "instruction": r.get::<_, String>(4)?, "schedule": r.get::<_, Option<String>>(5)?,
                "watch": r.get::<_, Option<String>>(6)?, "access": r.get::<_, String>(7)?,
                "notify": r.get::<_, String>(8)?, "catch_up": r.get::<_, String>(9)?,
                "made_by": r.get::<_, String>(10)?, "expires_at": r.get::<_, Option<i64>>(11)?,
                "paused": r.get::<_, i64>(12)? != 0, "next_at": r.get::<_, Option<i64>>(13)?,
                "last_at": r.get::<_, Option<i64>>(14)?,
                "last_skipped": r.get::<_, Option<String>>(15)?,
                "last_state": r.get::<_, Option<String>>(16)?,
                "tokens_today": r.get::<_, i64>(17)?,
                "proposed": r.get::<_, i64>(18)? != 0,
                "every_secs": r.get::<_, Option<i64>>(19)?,
                "member": r.get::<_, Option<String>>(20)?,
            }))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn routine_firings(&self, id: &str) -> Result<Vec<Firing>> {
        self.activity.firings(id, 20)
    }

    pub fn routine_preview(&self, when: &str, count: usize) -> Result<Value> {
        let (words, times) = api::preview(when, now(), Zone::Local, count)?;
        Ok(json!({ "words": words, "next": times }))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn routine_add(
        &self,
        binary: &Path,
        thread: &str,
        when: &str,
        name: &str,
        instruction: &str,
        notify: &str,
        catch_up: &str,
        watch: &str,
        every: &str,
    ) -> Result<Value> {
        let config = Config::load(&self.config)?;
        let thread = self
            .activity
            .resolve_thread(thread)?
            .with_context(|| format!("no thread {thread}"))?;
        let name = Some(name).filter(|n| !n.trim().is_empty());
        // A watch is a command the person wrote here, never one an agent asked for.
        let made = if watch.trim().is_empty() {
            api::add(
                &config,
                &self.activity,
                &thread,
                when,
                name,
                instruction,
                notify,
                catch_up,
                false,
                now(),
                Zone::Local,
            )?
        } else {
            api::add_watch(
                &config,
                &self.activity,
                &thread,
                watch,
                every,
                name,
                instruction,
                notify,
                false,
                now(),
            )?
        };
        let _ = orochi::keeper::ensure_with(binary, &config, &self.config, &self.data);
        Ok(json!({ "id": made.id, "next_at": made.next_at, "words": made.words }))
    }

    /// `run`, `pause`, `resume`, `approve`, `decline` or `delete`.
    pub fn routine_action(&self, binary: &Path, id: &str, action: &str) -> Result<Value> {
        let config = Config::load(&self.config)?;
        let routine = api::named(&self.activity, id)?;
        match action {
            "run" => {
                let turn = orochi::keeper::run_now(
                    binary,
                    &self.activity,
                    &config,
                    &self.config,
                    &self.data,
                    &routine,
                    now(),
                    Zone::Local,
                )?;
                ensure!(
                    turn.is_some(),
                    "the last run of {} has not finished",
                    routine.name
                );
            }
            "pause" => {
                self.activity.set_routine_paused(&routine.id, true)?;
            }
            "resume" => {
                api::resume(&self.activity, &routine, now(), Zone::Local)?;
                let _ = orochi::keeper::ensure_with(binary, &config, &self.config, &self.data);
            }
            "approve" => {
                api::approve(&self.activity, &routine, now(), Zone::Local)?;
                let _ = orochi::keeper::ensure_with(binary, &config, &self.config, &self.data);
            }
            "decline" => {
                api::decline(&self.activity, &routine)?;
            }
            "delete" => {
                self.activity.delete_routine(&routine.id)?;
            }
            other => anyhow::bail!("no routine action called {other}"),
        }
        Ok(json!(true))
    }

    pub fn resident_add(&self, name: &str, instruction: &str) -> Result<Value> {
        let config = Config::load(&self.config)?;
        let (thread, home) = orochi::resident::create(
            &config,
            &self.store,
            &self.activity,
            &self.data,
            name,
            instruction,
            "read",
        )?;
        Ok(json!({ "thread": thread, "home": home }))
    }

    /// `pause`, `resume` or `delete`.
    pub fn resident_action(&self, name: &str, action: &str) -> Result<Value> {
        let resident = self
            .activity
            .resident_named(name)?
            .with_context(|| format!("no resident {name}"))?;
        match action {
            "pause" => {
                self.activity
                    .set_resident_paused(&resident.thread_id, true)?;
            }
            "resume" => {
                self.activity
                    .set_resident_paused(&resident.thread_id, false)?;
            }
            "delete" => {
                self.activity.delete_thread(&resident.thread_id)?;
            }
            other => anyhow::bail!("no resident action called {other}"),
        }
        Ok(json!(true))
    }

    pub fn resident_notes(&self, name: &str) -> Result<Value> {
        let resident = self
            .activity
            .resident_named(name)?
            .with_context(|| format!("no resident {name}"))?;
        let home = Path::new(&resident.home);
        Ok(json!({
            "notes": std::fs::read_to_string(home.join(orochi::resident::NOTES)).unwrap_or_default(),
            "previous": std::fs::read_to_string(home.join(format!("{}.prev", orochi::resident::NOTES))).ok(),
            "home": resident.home,
            "instruction": resident.instruction,
        }))
    }

    pub fn save_resident(&self, name: &str, notes: &str, instruction: &str) -> Result<()> {
        let resident = self
            .activity
            .resident_named(name)?
            .with_context(|| format!("no resident {name}"))?;
        ensure!(notes.len() <= 64 * 1024, "the notes are over 64 KiB");
        std::fs::write(
            Path::new(&resident.home).join(orochi::resident::NOTES),
            notes,
        )?;
        if !instruction.trim().is_empty() {
            self.activity
                .set_resident_instruction(&resident.thread_id, instruction.trim())?;
        }
        Ok(())
    }

    pub fn notices(&self, unread: bool) -> Result<Vec<Notice>> {
        self.activity.notices(unread, 100)
    }

    pub fn read_notices(&self, id: Option<i64>) -> Result<()> {
        self.activity.read_notices(id)?;
        Ok(())
    }

    /// What the keeper is doing, what starts it at login, and today's tokens against the budget.
    pub fn keeper(&self) -> Result<Value> {
        let config = Config::load(&self.config).unwrap_or_default();
        let status = orochi::keeper::status(&self.data)?;
        Ok(json!({
            "running": status.is_some(), "status": status,
            "login": orochi::keeper::login_installed(),
            "login_supported": cfg!(target_os = "macos"),
            "tokens_today": self.activity.routine_tokens_since(day_start(now()))?,
            "daily_tokens": config.routines.daily_tokens,
            "enabled": config.routines.enabled,
        }))
    }

    pub fn keeper_login(&self, binary: &Path, on: bool) -> Result<Value> {
        if on {
            orochi::keeper::login_install(binary, &self.config, &self.data)?;
        } else {
            orochi::keeper::login_remove()?;
        }
        self.keeper()
    }

    /// The notification settings with no secret in them: the key is only ever said to be held.
    pub fn notify_settings(&self) -> Result<Value> {
        let config = Config::load(&self.config).unwrap_or_default();
        Ok(json!({
            "notify": config.notify,
            "routines": config.routines,
            "residents": config.residents,
            "key": orochi::notify::key::source(&self.data, &config.notify),
            "phones": orochi::notify::phones(&self.data),
        }))
    }

    /// Writes one section of the configuration: `notify`, `routines` or `residents`.
    pub fn save_section(&self, section: &str, value: &Value) -> Result<()> {
        let mut config = Config::load(&self.config).unwrap_or_default();
        match section {
            "notify" => config.notify = serde_json::from_value(value.clone())?,
            "routines" => config.routines = serde_json::from_value(value.clone())?,
            "residents" => config.residents = serde_json::from_value(value.clone())?,
            other => anyhow::bail!("no settings section called {other}"),
        }
        config.save(&self.config)
    }

    /// `set` (with the key), `forget`.
    pub fn notify_key(&self, action: &str, key: Option<&str>) -> Result<Value> {
        let config = Config::load(&self.config).unwrap_or_default();
        match action {
            "set" => orochi::notify::key::put(&self.data, &config.notify, key.unwrap_or(""))?,
            "forget" => {
                orochi::notify::key::forget(&self.data, &config.notify)?;
            }
            other => anyhow::bail!("no key action called {other}"),
        }
        Ok(json!(orochi::notify::key::source(
            &self.data,
            &config.notify
        )))
    }

    pub fn phone_forget(&self, id: &str) -> Result<bool> {
        orochi::notify::remove_phone(&self.data, id)
    }

    /// A person is at the window: what decides whether the phone is buzzed.
    pub fn touch_active(&self) -> Result<()> {
        self.activity.touch_active()
    }
}

/// Local midnight before `at`, in unix seconds.
fn day_start(at: i64) -> i64 {
    let p = Zone::Local.parts(at);
    Zone::Local
        .at(p.year, p.month, p.day, 0, 0)
        .unwrap_or(at - at % 86_400)
}
