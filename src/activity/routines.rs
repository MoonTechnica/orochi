//! Routines and their firings (`docs/resident-agent-design.md` §6, §15).
//!
//! A routine is a reason to wake a thread; a firing is one due time, whether it ran or was
//! skipped, so a skipped one costs nothing and is still a row to read. Instants here are unix
//! **seconds**, unlike the rest of the store's milliseconds, because a schedule is read in
//! minutes.
use super::Activity;
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

pub const ACCESS: [&str; 3] = ["read", "home", "worktree"];

#[derive(Debug, Clone, Serialize)]
pub struct Routine {
    pub id: String,
    pub thread_id: String,
    pub name: String,
    pub instruction: String,
    pub schedule: Option<String>,
    pub schedule_parsed: Option<String>,
    pub watch: Option<String>,
    pub every_secs: Option<i64>,
    pub watch_hash: Option<String>,
    pub watch_failures: i64,
    pub access: String,
    pub notify: String,
    pub catch_up: String,
    pub descriptor: Option<String>,
    pub made_by: String,
    pub expires_at: Option<i64>,
    pub paused: bool,
    pub next_at: Option<i64>,
    pub last_at: Option<i64>,
    pub created_at: i64,
    /// An agent proposed it and the person has not allowed it yet; it does not run.
    pub proposed: bool,
    /// The member of the team that runs it, by name; none: the lead runs it.
    pub member: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Firing {
    pub id: i64,
    pub routine_id: String,
    pub due_at: i64,
    pub started_at: Option<i64>,
    pub turn_id: Option<String>,
    pub skipped: Option<String>,
    pub turn_state: Option<String>,
}

pub struct NewRoutine<'a> {
    pub thread: &'a str,
    pub name: &'a str,
    pub instruction: &'a str,
    /// The words and what they parsed to.
    pub schedule: Option<(&'a str, String)>,
    /// A command and how often to run it, in seconds.
    pub watch: Option<(&'a str, i64)>,
    pub access: &'a str,
    pub notify: &'a str,
    pub catch_up: &'a str,
    pub descriptor: Option<String>,
    pub made_by: &'a str,
    pub expires_at: Option<i64>,
    pub next_at: Option<i64>,
    pub now: i64,
    pub proposed: bool,
}

const COLUMNS: &str = "id, thread_id, name, instruction, schedule, schedule_parsed, watch,
    every_secs, watch_hash, watch_failures, access, notify, catch_up, descriptor, made_by,
    expires_at, paused, next_at, last_at, created_at, proposed, member";

fn routine_of(r: &rusqlite::Row) -> rusqlite::Result<Routine> {
    Ok(Routine {
        id: r.get(0)?,
        thread_id: r.get(1)?,
        name: r.get(2)?,
        instruction: r.get(3)?,
        schedule: r.get(4)?,
        schedule_parsed: r.get(5)?,
        watch: r.get(6)?,
        every_secs: r.get(7)?,
        watch_hash: r.get(8)?,
        watch_failures: r.get(9)?,
        access: r.get(10)?,
        notify: r.get(11)?,
        catch_up: r.get(12)?,
        descriptor: r.get(13)?,
        made_by: r.get(14)?,
        expires_at: r.get(15)?,
        paused: r.get::<_, i64>(16)? != 0,
        next_at: r.get(17)?,
        last_at: r.get(18)?,
        created_at: r.get(19)?,
        proposed: r.get::<_, i64>(20)? != 0,
        member: r.get(21)?,
    })
}

impl Activity {
    pub fn add_routine(&self, new: &NewRoutine) -> Result<String> {
        ensure!(
            new.schedule.is_some() != new.watch.is_some(),
            "a routine has a schedule or a watch, not both and not neither"
        );
        let id = uuid::Uuid::new_v4().to_string();
        self.connection.execute(
            "INSERT INTO routines (id, thread_id, name, instruction, schedule, schedule_parsed,
                watch, every_secs, access, notify, catch_up, descriptor, made_by, expires_at,
                next_at, created_at, proposed, paused)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?17)",
            params![
                id,
                new.thread,
                new.name,
                new.instruction,
                new.schedule.as_ref().map(|s| s.0),
                new.schedule.as_ref().map(|s| &s.1),
                new.watch.map(|w| w.0),
                new.watch.map(|w| w.1),
                new.access,
                new.notify,
                new.catch_up,
                new.descriptor,
                new.made_by,
                new.expires_at,
                new.next_at,
                new.now,
                i64::from(new.proposed)
            ],
        )?;
        Ok(id)
    }

    /// Names the member that runs a routine (none: the lead).
    pub fn set_routine_member(&self, id: &str, member: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE routines SET member=?2 WHERE id=?1",
            params![id, member],
        )?;
        Ok(())
    }

    /// A routine by its id or by the start of it, as long as the start names one.
    pub fn routine(&self, id: &str) -> Result<Option<Routine>> {
        let mut found = self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM routines WHERE id LIKE ?1 || '%' LIMIT 2"
            ))?
            .query_map([id], routine_of)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(match found.len() {
            1 => found.pop(),
            _ => self
                .connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM routines WHERE id=?1"),
                    [id],
                    routine_of,
                )
                .optional()?,
        })
    }

    pub fn routines(&self, thread: Option<&str>) -> Result<Vec<Routine>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM routines WHERE (?1 IS NULL OR thread_id=?1)
                 ORDER BY created_at, id"
            ))?
            .query_map([thread], routine_of)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn routines_on(&self, thread: &str) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM routines WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )?)
    }

    /// Scheduled routines that are due, oldest first. A watch is run by its own clock.
    pub fn due_routines(&self, now: i64) -> Result<Vec<Routine>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM routines
                 WHERE paused=0 AND next_at IS NOT NULL AND next_at<=?1 ORDER BY next_at, id"
            ))?
            .query_map([now], routine_of)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// How many routines the keeper has any reason to stay for.
    pub fn active_routines(&self) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM routines WHERE paused=0 AND (next_at IS NOT NULL OR watch IS NOT NULL)",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn next_routine_at(&self) -> Result<Option<i64>> {
        Ok(self.connection.query_row(
            "SELECT min(next_at) FROM routines WHERE paused=0",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn set_routine_paused(&self, id: &str, paused: bool) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE routines SET paused=?2 WHERE id=?1",
            params![id, i64::from(paused)],
        )? == 1)
    }

    /// Sets when it is next due, as resuming does after a pause.
    pub fn set_routine_next(&self, id: &str, next: Option<i64>) -> Result<()> {
        self.connection.execute(
            "UPDATE routines SET next_at=?2 WHERE id=?1",
            params![id, next],
        )?;
        Ok(())
    }

    /// The person allows a proposal: it runs from `next_at` on and no longer expires.
    pub fn approve_routine(&self, id: &str, next_at: Option<i64>) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE routines SET proposed=0, paused=0, expires_at=NULL, next_at=?2
             WHERE id=?1 AND proposed=1",
            params![id, next_at],
        )? == 1)
    }

    /// Proposals waiting on a thread.
    pub fn proposals_on(&self, thread: &str) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM routines WHERE thread_id=?1 AND proposed=1",
            [thread],
            |r| r.get(0),
        )?)
    }

    pub fn delete_routine(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM routines WHERE id=?1", [id])?
            == 1)
    }

    /// Proposals nobody answered in time, removed; what was removed.
    pub fn expire_routines(&self, now: i64) -> Result<Vec<Routine>> {
        let expired: Vec<Routine> = self
            .connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM routines WHERE expires_at IS NOT NULL AND expires_at<=?1"
            ))?
            .query_map([now], routine_of)?
            .collect::<rusqlite::Result<_>>()?;
        for routine in &expired {
            self.delete_routine(&routine.id)?;
        }
        Ok(expired)
    }

    /// Records that a due time has been taken and moves the routine on, in one step: the
    /// next due time is stored with the firing, so a crash between the two cannot run one
    /// due time twice. A missed wake is the cheaper mistake than a repeated one.
    pub fn begin_firing(
        &self,
        routine: &str,
        due_at: i64,
        skipped: Option<&str>,
        next_at: Option<i64>,
        now: i64,
    ) -> Result<i64> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        tx.execute(
            "INSERT INTO firings (routine_id, due_at, started_at, skipped) VALUES (?1,?2,?3,?4)",
            params![routine, due_at, skipped.is_none().then_some(now), skipped],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE routines SET next_at=?2, last_at=?3 WHERE id=?1",
            params![routine, next_at, now],
        )?;
        // A watch looks every few minutes; only the latest of its looks are kept.
        tx.execute(
            "DELETE FROM firings WHERE routine_id=?1 AND id NOT IN
               (SELECT id FROM firings WHERE routine_id=?1 ORDER BY id DESC LIMIT 200)",
            [routine],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// What a watch last saw (a hash of its output) and how many looks in a row have failed.
    pub fn set_watch_state(&self, routine: &str, hash: Option<&str>, failures: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE routines SET watch_hash=COALESCE(?2, watch_hash), watch_failures=?3 WHERE id=?1",
            params![routine, hash, failures],
        )?;
        Ok(())
    }

    /// Ties a firing to the turn it queued.
    pub fn firing_turn(&self, firing: i64, turn: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE firings SET turn_id=?2 WHERE id=?1",
            params![firing, turn],
        )?;
        Ok(())
    }

    pub fn firing_failed(&self, firing: i64) -> Result<()> {
        self.connection
            .execute("UPDATE firings SET skipped='failed' WHERE id=?1", [firing])?;
        Ok(())
    }

    pub fn firings(&self, routine: &str, limit: i64) -> Result<Vec<Firing>> {
        Ok(self
            .connection
            .prepare(
                "SELECT f.id, f.routine_id, f.due_at, f.started_at, f.turn_id, f.skipped, t.state
                 FROM firings f LEFT JOIN turns t ON t.id=f.turn_id
                 WHERE f.routine_id=?1 ORDER BY f.id DESC LIMIT ?2",
            )?
            .query_map(params![routine, limit], |r| {
                Ok(Firing {
                    id: r.get(0)?,
                    routine_id: r.get(1)?,
                    due_at: r.get(2)?,
                    started_at: r.get(3)?,
                    turn_id: r.get(4)?,
                    skipped: r.get(5)?,
                    turn_state: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Whether the routine's last turn is still queued or running.
    pub fn routine_busy(&self, routine: &str) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM firings f JOIN turns t ON t.id=f.turn_id
                WHERE f.routine_id=?1 AND t.state IN ('queued','running'))",
            [routine],
            |r| r.get(0),
        )?)
    }
}
