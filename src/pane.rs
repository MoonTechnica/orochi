//! The terminal Orochi runs in may belong to an agent host that shows, per pane, what the
//! agent in it is doing. Orca (read at 1.4.215, `docs/supervision-design.md` §8) learns it two
//! ways: hooks it installs into each CLI's own settings, which post to a local port keyed by the
//! pane, and an `OSC 9999` sequence any program may write to its own terminal.
//!
//! Orochi opens many sessions of those CLIs — the lead, the seats beside it, the classifier,
//! advisers, quota probes — and every one of them would run the host's hooks with this pane's
//! key, each claiming to be *the* agent in it. So the hooks are silenced in everything Orochi
//! launches, and the console speaks for the pane itself.
use serde::Serialize;
use std::time::{Duration, Instant};

/// Orca's hook scripts exit without posting or spooling when any of these is empty.
const HOOK_VARIABLES: [&str; 3] = [
    "ORCA_AGENT_HOOK_PORT",
    "ORCA_AGENT_HOOK_TOKEN",
    "ORCA_AGENT_HOOK_ENDPOINT",
];
/// Set by the host in each pane it opens; without it there is nobody to report to.
const PANE_KEY: &str = "ORCA_PANE_KEY";
const PREFIX: &str = "\x1b]9999;";
const BEL: &str = "\x07";
/// A report is a state change, not a stream: at most this often, except that a change of
/// `state` itself is never held back.
const INTERVAL: Duration = Duration::from_millis(500);
/// The host keeps at most this many seats per pane.
const MAX_SEATS: usize = 32;

/// Environment entries for a launched agent that keep the host's hooks quiet in it. Empty
/// rather than removed: it goes through the one `envs` every launch passes, and the scripts test
/// for empty. Only variables that are set are touched, so outside such a host this is nothing.
pub fn quiet_hooks() -> Vec<(String, String)> {
    HOOK_VARIABLES
        .iter()
        .filter(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
        .map(|name| ((*name).to_owned(), String::new()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Working,
    /// A permission question waits on the user.
    Blocked,
    /// Orochi asked the user something of its own.
    Waiting,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SeatState {
    Working,
    Blocked,
    Idle,
}

/// One seat beside the lead, as the host lists it under the pane.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seat {
    pub id: String,
    pub state: SeatState,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What the pane says. It never carries the user's request or an agent's reply: the host keeps
/// what it is told and can hand it to a paired phone, and a title or a tool line is already on
/// the screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_mode: Option<&'static str>,
    pub agent_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive_prompt: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub interrupted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<Seat>,
}
impl Default for Report {
    fn default() -> Self {
        Self {
            state: State::Done,
            working_mode: None,
            agent_type: "orochi",
            model: None,
            tool_name: None,
            tool_input: None,
            interactive_prompt: None,
            interrupted: false,
            turn_completed_at: None,
            subagents: vec![],
        }
    }
}

fn cut(text: &mut Option<String>, limit: usize) {
    if let Some(value) = text.as_mut() {
        let clean: String = value
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(limit)
            .collect();
        *value = clean;
    }
}

/// The sequence for one report, within the host's own limits (it drops a field past them).
pub fn encode(report: &Report) -> String {
    let mut report = report.clone();
    cut(&mut report.model, 120);
    cut(&mut report.tool_name, 60);
    cut(&mut report.tool_input, 160);
    cut(&mut report.interactive_prompt, 16_000);
    report.subagents.truncate(MAX_SEATS);
    for seat in &mut report.subagents {
        seat.id = seat
            .id
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        cut(&mut seat.agent_type, 40);
        cut(&mut seat.model, 120);
        cut(&mut seat.description, 160);
    }
    let json = serde_json::to_string(&report).unwrap_or_default();
    format!("{PREFIX}{json}{BEL}")
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Holds what the pane last said and decides when to say it again.
pub struct Reporter {
    enabled: bool,
    sent: Option<String>,
    sent_state: Option<State>,
    sent_at: Option<Instant>,
    held: Option<String>,
    pub report: Report,
}
impl Reporter {
    /// On only in a pane the host opened, writing to a terminal.
    pub fn new(tty: bool) -> Self {
        Self::with(tty && std::env::var_os(PANE_KEY).is_some_and(|v| !v.is_empty()))
    }
    pub fn with(enabled: bool) -> Self {
        Self {
            enabled,
            sent: None,
            sent_state: None,
            sent_at: None,
            held: None,
            report: Report::default(),
        }
    }
    pub fn disable(&mut self) {
        self.enabled = false;
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    /// The sequence to write for the current report, if it changed and may be said now.
    /// A change that arrives too soon is kept and comes out of the next `due`.
    pub fn update(&mut self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let encoded = encode(&self.report);
        if self.sent.as_deref() == Some(encoded.as_str()) {
            self.held = None;
            return None;
        }
        let soon = self.sent_at.is_some_and(|at| at.elapsed() < INTERVAL);
        if soon && self.sent_state == Some(self.report.state) {
            self.held = Some(encoded);
            return None;
        }
        self.mark(encoded)
    }
    /// A held report whose interval has passed.
    pub fn due(&mut self) -> Option<String> {
        if self.sent_at.is_some_and(|at| at.elapsed() < INTERVAL) {
            return None;
        }
        let held = self.held.take()?;
        self.mark(held)
    }
    fn mark(&mut self, encoded: String) -> Option<String> {
        self.sent = Some(encoded.clone());
        self.sent_state = Some(self.report.state);
        self.sent_at = Some(Instant::now());
        self.held = None;
        Some(encoded)
    }
    /// Sets or replaces one seat by name.
    pub fn seat(&mut self, seat: Seat) {
        match self.report.subagents.iter_mut().find(|s| s.id == seat.id) {
            Some(existing) => *existing = seat,
            None => self.report.subagents.push(seat),
        }
    }
    pub fn seat_state(&mut self, id: &str, state: SeatState) {
        if let Some(seat) = self.report.subagents.iter_mut().find(|s| s.id == id) {
            seat.state = state;
        }
    }
    pub fn drop_seat(&mut self, id: &str) {
        self.report.subagents.retain(|s| s.id != id);
    }
}
