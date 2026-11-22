//! Residents, notices and the settling of a routine's finished turn
//! (`docs/resident-agent-design.md` §6, §13, §15).
use super::{Activity, millis};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Resident {
    pub thread_id: String,
    pub name: String,
    pub home: String,
    pub instruction: String,
    pub access: String,
    pub paused: bool,
    pub created_at: i64,
}

fn resident_of(r: &rusqlite::Row) -> rusqlite::Result<Resident> {
    Ok(Resident {
        thread_id: r.get(0)?,
        name: r.get(1)?,
        home: r.get(2)?,
        instruction: r.get(3)?,
        access: r.get(4)?,
        paused: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
    })
}
const RESIDENT: &str = "thread_id, name, home, instruction, access, paused, created_at";

#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub id: i64,
    pub kind: String,
    pub thread_id: Option<String>,
    pub routine_id: Option<String>,
    pub prompt_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub mac: Option<String>,
    pub phone: Option<String>,
    pub created_at: i64,
    pub read_at: Option<i64>,
}

pub type Unannounced = (String, String, String, Option<String>);

/// A routine's turn that has ended and not yet been settled.
#[derive(Debug, Clone)]
pub struct Finished {
    pub firing: i64,
    pub routine_id: String,
    pub routine: String,
    pub notify: String,
    pub thread_id: String,
    pub turn_id: String,
    pub state: String,
}

impl Activity {
    pub fn add_resident(
        &self,
        thread: &str,
        name: &str,
        home: &str,
        instruction: &str,
        access: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO residents (thread_id, name, home, instruction, access, created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![thread, name, home, instruction, access, millis() / 1000],
        )?;
        Ok(())
    }

    pub fn residents(&self) -> Result<Vec<Resident>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT {RESIDENT} FROM residents ORDER BY created_at, name"
            ))?
            .query_map([], resident_of)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn resident_for(&self, thread: &str) -> Result<Option<Resident>> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {RESIDENT} FROM residents WHERE thread_id=?1"),
                [thread],
                resident_of,
            )
            .optional()?)
    }

    pub fn resident_named(&self, name: &str) -> Result<Option<Resident>> {
        Ok(self
            .connection
            .query_row(
                &format!(
                    "SELECT {RESIDENT} FROM residents WHERE name=?1 OR thread_id LIKE ?1 || '%'"
                ),
                [name],
                resident_of,
            )
            .optional()?)
    }

    pub fn set_resident_paused(&self, thread: &str, paused: bool) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE residents SET paused=?2 WHERE thread_id=?1",
            params![thread, i64::from(paused)],
        )? == 1)
    }

    pub fn set_resident_instruction(&self, thread: &str, instruction: &str) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE residents SET instruction=?2 WHERE thread_id=?1",
            params![thread, instruction],
        )? == 1)
    }

    /// The last `n` replies a routine's finished turns ended with, oldest first.
    pub fn reports(&self, routine: &str, n: usize) -> Result<Vec<(i64, String)>> {
        let mut found: Vec<(i64, String)> = self
            .connection
            .prepare(
                "SELECT f.due_at,
                        COALESCE((SELECT i.text FROM items i WHERE i.turn_id=f.turn_id
                                  AND i.kind='agent_message' ORDER BY i.id DESC LIMIT 1), '')
                 FROM firings f JOIN turns t ON t.id=f.turn_id
                 WHERE f.routine_id=?1 AND t.state NOT IN ('queued','running')
                 ORDER BY f.id DESC LIMIT ?2",
            )?
            .query_map(params![routine, n as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        found.reverse();
        Ok(found)
    }

    /// What the turn's agent said last: a routine's report.
    pub fn last_reply(&self, turn: &str) -> Result<String> {
        Ok(self
            .connection
            .query_row(
                "SELECT text FROM items WHERE turn_id=?1 AND kind='agent_message'
                 ORDER BY id DESC LIMIT 1",
                [turn],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Firings whose turn has ended and which nobody has settled yet.
    pub fn unsettled(&self) -> Result<Vec<Finished>> {
        Ok(self
            .connection
            .prepare(
                "SELECT f.id, f.routine_id, r.name, r.notify, r.thread_id, t.id, t.state
                 FROM firings f JOIN routines r ON r.id=f.routine_id JOIN turns t ON t.id=f.turn_id
                 WHERE f.settled=0 AND t.state NOT IN ('queued','running') ORDER BY f.id",
            )?
            .query_map([], |r| {
                Ok(Finished {
                    firing: r.get(0)?,
                    routine_id: r.get(1)?,
                    routine: r.get(2)?,
                    notify: r.get(3)?,
                    thread_id: r.get(4)?,
                    turn_id: r.get(5)?,
                    state: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Firings with a turn still to finish: the keeper stays for them.
    pub fn awaiting(&self) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM firings WHERE settled=0 AND turn_id IS NOT NULL",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn settle(&self, firing: i64) -> Result<()> {
        self.connection
            .execute("UPDATE firings SET settled=1 WHERE id=?1", [firing])?;
        Ok(())
    }

    pub fn add_notice(
        &self,
        kind: &str,
        thread: Option<&str>,
        routine: Option<&str>,
        prompt: Option<&str>,
        title: &str,
        body: Option<&str>,
    ) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO notices (kind, thread_id, routine_id, prompt_id, title, body, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![kind, thread, routine, prompt, title, body, millis()],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub fn notice_delivery(&self, id: i64, mac: Option<&str>, phone: Option<&str>) -> Result<()> {
        self.connection.execute(
            "UPDATE notices SET mac=COALESCE(?2,mac), phone=COALESCE(?3,phone) WHERE id=?1",
            params![id, mac, phone],
        )?;
        Ok(())
    }

    pub fn notices(&self, unread_only: bool, limit: i64) -> Result<Vec<Notice>> {
        Ok(self
            .connection
            .prepare(
                "SELECT id, kind, thread_id, routine_id, prompt_id, title, body, mac, phone,
                        created_at, read_at
                 FROM notices WHERE (?1=0 OR read_at IS NULL) ORDER BY id DESC LIMIT ?2",
            )?
            .query_map(params![i64::from(unread_only), limit], |r| {
                Ok(Notice {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    thread_id: r.get(2)?,
                    routine_id: r.get(3)?,
                    prompt_id: r.get(4)?,
                    title: r.get(5)?,
                    body: r.get(6)?,
                    mac: r.get(7)?,
                    phone: r.get(8)?,
                    created_at: r.get(9)?,
                    read_at: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Marks one notice read, or every one with `None`.
    pub fn read_notices(&self, id: Option<i64>) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE notices SET read_at=?2 WHERE read_at IS NULL AND (?1 IS NULL OR id=?1)",
            params![id, millis()],
        )?)
    }

    /// When the window last had a person at it, in milliseconds; 0 if never.
    pub fn active_at(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM app_state WHERE key='active_at'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn touch_active(&self) -> Result<()> {
        self.connection.execute(
            "INSERT INTO app_state (key, value) VALUES ('active_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [millis()],
        )?;
        Ok(())
    }

    /// What a turn cost, weighted as ranking weighs it (a cache read is a tenth of a fresh
    /// token), summed over its attempts. Zero where nothing was reported.
    pub fn turn_tokens(&self, turn: &str) -> Result<i64> {
        let mut total = 0.0;
        let mut statement = self.connection.prepare(
            "SELECT a.usage FROM attempts a JOIN seats s ON s.id=a.seat_id
             WHERE s.turn_id=?1 AND a.usage IS NOT NULL",
        )?;
        for usage in statement.query_map([turn], |r| r.get::<_, String>(0))? {
            if let Ok(usage) = serde_json::from_str::<crate::types::Usage>(&usage?) {
                total += usage.weighted().unwrap_or(0.0);
            }
        }
        Ok(total.round() as i64)
    }

    pub fn set_firing_tokens(&self, firing: i64, tokens: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE firings SET tokens=?2 WHERE id=?1",
            params![firing, tokens],
        )?;
        Ok(())
    }

    /// Tokens routines have spent since `since` (unix seconds).
    pub fn routine_tokens_since(&self, since: i64) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(sum(tokens),0) FROM firings WHERE started_at>=?1",
            [since],
            |r| r.get(0),
        )?)
    }

    /// Questions on a routine's turn that no notice has been made for: the prompt, its thread,
    /// its request and the routine whose turn it is.
    pub fn unannounced_prompts(&self) -> Result<Vec<Unannounced>> {
        Ok(self
            .connection
            .prepare(
                "SELECT pr.id, pr.thread_id, pr.request,
                        (SELECT f.routine_id FROM firings f WHERE f.turn_id=pr.turn_id LIMIT 1)
                 FROM prompts pr JOIN turns t ON t.id=pr.turn_id
                 WHERE t.origin='routine' AND pr.answer IS NULL
                   AND NOT EXISTS (SELECT 1 FROM notices n WHERE n.prompt_id=pr.id)",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Notices no sink has looked at yet.
    pub fn pending_notices(&self) -> Result<Vec<Notice>> {
        self.notice_where("mac IS NULL AND phone IS NULL")
    }

    /// Notices kept back for quiet hours.
    pub fn held_notices(&self) -> Result<Vec<Notice>> {
        self.notice_where("mac='held' OR phone='held'")
    }

    fn notice_where(&self, condition: &str) -> Result<Vec<Notice>> {
        Ok(self
            .connection
            .prepare(&format!(
                "SELECT id, kind, thread_id, routine_id, prompt_id, title, body, mac, phone,
                        created_at, read_at FROM notices WHERE {condition} ORDER BY id"
            ))?
            .query_map([], |r| {
                Ok(Notice {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    thread_id: r.get(2)?,
                    routine_id: r.get(3)?,
                    prompt_id: r.get(4)?,
                    title: r.get(5)?,
                    body: r.get(6)?,
                    mac: r.get(7)?,
                    phone: r.get(8)?,
                    created_at: r.get(9)?,
                    read_at: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Whether a notice with this title was made since `since` (milliseconds).
    pub fn notice_since(&self, kind: &str, title: &str, since: i64) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM notices WHERE kind=?1 AND title=?2 AND created_at>=?3)",
            params![kind, title, since],
            |r| r.get(0),
        )?)
    }
}
