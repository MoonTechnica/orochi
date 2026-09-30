//! Seats that outlive the turn that started them (`docs/supervision-design.md` §4).
//!
//! The lead of a console turn asks for a helper with the mailbox's `start_agent`; the console
//! decides, and a granted helper runs as a read-only seat owned by the session rather than by
//! the turn. The lead answers and ends its turn; the helper keeps working, and when it ends its
//! result goes back to the lead as a turn of its own. A helper takes no workspace lock (it
//! writes nothing, and a lock held between turns would lock the user's own next turn out), and
//! the process's interrupt does not reach it: Esc stops the lead, a helper is stopped by name.
use super::{LANGUAGE, answerer};
use crate::{
    acp::ExecutionEvent,
    config::{Config, PermissionMode},
    policy::Registry,
    scheduler::{self, Continuation, RunOptions},
    storage::Store,
    types::{Overrides, TaskDescriptor},
};
use futures::{
    StreamExt,
    future::{AbortHandle, Aborted, abortable},
    stream::FuturesUnordered,
};
use std::{
    future::Future,
    path::Path,
    pin::Pin,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// What a helper hands back to the lead, at most.
pub const RESULT_CHARS: usize = 4_000;

/// What a helper needs from the session, without borrowing the session.
#[derive(Clone, Copy)]
pub struct Env<'a> {
    pub config: &'a Config,
    pub store: &'a Store,
    pub root: &'a Path,
    pub data: &'a Path,
    pub headless: bool,
}

pub struct Helper {
    pub name: String,
    pub title: String,
    pub started: Instant,
    /// The same moment on the wall clock, for the host's pane report.
    pub started_at: u64,
    /// `agent · model`, once routed.
    pub route: Option<String>,
    /// The tool it is running, while it runs one.
    pub doing: Option<String>,
    pub seat: Option<crate::activity::SeatRef>,
    /// The descriptor of the turn that asked for it, which the completion turn is routed by.
    pub descriptor: TaskDescriptor,
    reply: String,
    receiver: mpsc::UnboundedReceiver<ExecutionEvent>,
    abort: AbortHandle,
}

/// How a helper ended.
pub enum Ending {
    Finished,
    Failed(String),
    Stopped,
}

/// A helper that has ended, with what it said.
pub struct Finished {
    pub name: String,
    pub title: String,
    pub elapsed: Duration,
    pub route: Option<String>,
    pub ending: Ending,
    pub result: String,
    pub descriptor: TaskDescriptor,
}

pub enum Event {
    Said(String, ExecutionEvent),
    Ended(String, Ending),
}

type Run<'a> = Pin<Box<dyn Future<Output = (String, Result<anyhow::Result<u8>, Aborted>)> + 'a>>;

/// Grants this close together are one launch: a lead asks for helpers one call at a time, and
/// they read as one block, as Claude Code shows its own.
const BURST: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct Background<'a> {
    pub helpers: Vec<Helper>,
    runs: FuturesUnordered<Run<'a>>,
    /// Started but not yet announced, and when the last of them was.
    announce: Vec<(String, String)>,
    announced_at: Option<Instant>,
    /// How many have started since none were running: what "N agents finished" counts.
    pub group: usize,
}

impl<'a> Background<'a> {
    pub fn is_empty(&self) -> bool {
        self.helpers.is_empty()
    }
    pub fn len(&self) -> usize {
        self.helpers.len()
    }
    /// A name no live helper has, from the one asked for.
    pub fn free_name(&self, wanted: &str, taken: &dyn Fn(&str) -> bool) -> String {
        let clash = |name: &str| taken(name) || self.helpers.iter().any(|h| h.name == name);
        if !clash(wanted) {
            return wanted.to_owned();
        }
        (2..)
            .map(|n| format!("{wanted}-{n}"))
            .find(|name| !clash(name))
            .expect("some suffix is free")
    }

    /// Starts one helper. `lead` is who asked, for its prompt.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        env: Env<'a>,
        lead: &str,
        name: String,
        title: String,
        task: &str,
        descriptor: TaskDescriptor,
        seat: Option<crate::activity::SeatRef>,
    ) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let options = RunOptions {
            task: format!("{LANGUAGE}\n\n{}", prompt(lead, &name, &title, task)),
            descriptor: Some(descriptor.clone()),
            // A helper is asked for a part of the task, not the whole of it.
            difficulty: Some(descriptor.complexity.eased()),
            overrides: Overrides::default(),
            dry_run: false,
            json: false,
            resume: None,
            permission: PermissionMode::Allow,
            interactive: true,
            attachments: vec![],
            peer: Some(name.clone()),
            verify: false,
            read_only: true,
            place: None,
            seat: seat.clone(),
            answerer: answerer(PermissionMode::Allow, env.headless),
            background: true,
        };
        let (run, abort) = abortable(async move {
            let policies = Registry::load(env.data)?;
            let mut slot: Option<Continuation> = None;
            scheduler::run_turn(
                env.config,
                &policies,
                env.store,
                env.root,
                options,
                Some(sender),
                &mut slot,
            )
            .await
        });
        let label = name.clone();
        self.runs.push(Box::pin(async move { (label, run.await) }));
        self.group += 1;
        self.helpers.push(Helper {
            name,
            title,
            started: Instant::now(),
            started_at: crate::pane::now_millis(),
            route: None,
            doing: None,
            seat,
            descriptor,
            reply: String::new(),
            receiver,
            abort,
        });
    }

    /// The next thing a helper said or did, or its end. Pending for ever when there are none,
    /// so a `select!` arm guards it with `!is_empty()`.
    pub async fn next(&mut self) -> Event {
        let helpers = &mut self.helpers;
        let runs = &mut self.runs;
        std::future::poll_fn(|cx| {
            for helper in helpers.iter_mut() {
                if let std::task::Poll::Ready(Some(event)) = helper.receiver.poll_recv(cx) {
                    return std::task::Poll::Ready(Event::Said(helper.name.clone(), event));
                }
            }
            match runs.poll_next_unpin(cx) {
                std::task::Poll::Ready(Some((name, result))) => {
                    let ending = match result {
                        Err(Aborted) => Ending::Stopped,
                        Ok(Ok(0)) => Ending::Finished,
                        Ok(Ok(130)) => Ending::Stopped,
                        Ok(Ok(code)) => Ending::Failed(format!("exit {code}")),
                        Ok(Err(error)) => Ending::Failed(first_line(&format!("{error:#}"))),
                    };
                    std::task::Poll::Ready(Event::Ended(name, ending))
                }
                _ => std::task::Poll::Pending,
            }
        })
        .await
    }

    /// Holds started helpers back for one launch block.
    pub fn launched(&mut self, started: Vec<(String, String)>) {
        if !started.is_empty() {
            self.announce.extend(started);
            self.announced_at = Some(Instant::now());
        }
    }

    /// The launch block's helpers, once the burst is over, or at once with `now`.
    pub fn announcement(&mut self, now: bool) -> Option<Vec<(String, String)>> {
        let over = self.announced_at.is_some_and(|at| at.elapsed() >= BURST);
        (!self.announce.is_empty() && (now || over)).then(|| std::mem::take(&mut self.announce))
    }

    /// The last of what a helper has said so far, for the manager's view of it.
    pub fn said_so_far(&self, name: &str, chars: usize) -> String {
        let Some(helper) = self.helpers.iter().find(|h| h.name == name) else {
            return String::new();
        };
        let count = helper.reply.chars().count();
        helper
            .reply
            .chars()
            .skip(count.saturating_sub(chars))
            .collect()
    }

    /// Records what a helper said: its reply text is the result the lead gets.
    pub fn heard(&mut self, name: &str, event: &ExecutionEvent) {
        let Some(helper) = self.helpers.iter_mut().find(|h| h.name == name) else {
            return;
        };
        match event {
            ExecutionEvent::Text(text, _) if helper.reply.len() < RESULT_CHARS * 4 => {
                helper.reply.push_str(text);
            }
            ExecutionEvent::Progress(crate::acp::Progress::Route { agent, model, .. }) => {
                helper.route = Some(format!("{agent} · {model}"));
            }
            ExecutionEvent::Progress(crate::acp::Progress::Tool(update)) => {
                match update.status.as_deref() {
                    Some("completed" | "failed") => helper.doing = None,
                    _ => {
                        let what = update.detail.clone().or_else(|| update.title.clone());
                        if what.is_some() {
                            helper.doing = what;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Takes an ended helper out, with everything it said that was still queued.
    pub fn ended(&mut self, name: &str, ending: Ending) -> Option<Finished> {
        let index = self.helpers.iter().position(|h| h.name == name)?;
        let mut helper = self.helpers.remove(index);
        while let Ok(event) = helper.receiver.try_recv() {
            if let ExecutionEvent::Text(text, _) = event {
                helper.reply.push_str(&text);
            }
        }
        if let Some(seat) = &helper.seat {
            let store = seat.store.lock().expect("activity store");
            let result = bounded_result(&helper.reply);
            if !result.is_empty() {
                let _ = store.seat_result(&seat.seat, &result);
            }
            if matches!(ending, Ending::Stopped) {
                let _ = store.seat_state(&seat.seat, crate::activity::SeatState::Cancelled);
            }
        }
        Some(Finished {
            result: bounded_result(&helper.reply),
            name: helper.name,
            title: helper.title,
            elapsed: helper.started.elapsed(),
            route: helper.route,
            ending,
            descriptor: helper.descriptor,
        })
    }

    /// Stops one helper by name; its end arrives through `next` as `Stopped`.
    pub fn stop(&mut self, name: &str) -> bool {
        match self.helpers.iter().find(|h| h.name == name) {
            Some(helper) => {
                helper.abort.abort();
                true
            }
            None => false,
        }
    }

    pub fn stop_all(&mut self) {
        for helper in &self.helpers {
            helper.abort.abort();
        }
    }

    /// Drops every helper at once, recording each as cancelled: `/new` leaves them behind, and
    /// nothing they would say belongs to the conversation that follows.
    pub fn abandon(&mut self) -> usize {
        let count = self.helpers.len();
        for helper in self.helpers.drain(..) {
            helper.abort.abort();
            if let Some(seat) = &helper.seat {
                let _ = seat
                    .store
                    .lock()
                    .expect("activity store")
                    .seat_state(&seat.seat, crate::activity::SeatState::Cancelled);
            }
        }
        self.runs = FuturesUnordered::new();
        self.announce.clear();
        self.group = 0;
        count
    }

    /// One line for the lead's next turn, so it does not start what is already running.
    pub fn running_note(&self) -> Option<String> {
        if self.helpers.is_empty() {
            return None;
        }
        let names: Vec<String> = self
            .helpers
            .iter()
            .map(|h| format!("`{}` ({})", h.name, h.title))
            .collect();
        Some(format!(
            "Background agents still running: {}. Their results arrive as their own messages; do not start them again.",
            names.join(", ")
        ))
    }
}

/// The message that hands helpers' results back to the lead: Orochi's words, not the user's.
pub fn completion(finished: &[Finished]) -> String {
    finished
        .iter()
        .map(|f| {
            let head = format!("Background agent `{}` ({})", f.name, f.title);
            let elapsed = super::span(f.elapsed);
            match &f.ending {
                Ending::Finished if f.result.is_empty() => {
                    format!("{head} finished after {elapsed} without reporting anything.")
                }
                Ending::Finished => format!("{head} finished after {elapsed}:\n{}", f.result),
                Ending::Stopped => format!("{head} was stopped by the user after {elapsed}."),
                Ending::Failed(why) => format!("{head} failed after {elapsed}: {why}."),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn bounded_result(reply: &str) -> String {
    let reply = reply.trim();
    if reply.chars().count() <= RESULT_CHARS {
        return reply.to_owned();
    }
    // The end of a report is its conclusion; keep that.
    let skip = reply.chars().count() - RESULT_CHARS;
    format!("…{}", reply.chars().skip(skip).collect::<String>())
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

/// A helper's own instructions: one part of the task, read only, and a report at the end.
fn prompt(lead: &str, name: &str, title: &str, task: &str) -> String {
    format!(
        "You are `{name}`, working in the background for `{lead}` on one part of what the user \
         asked: {title}.\n\nYou can read everything and change nothing: write tools are refused \
         for this session. `{lead}` is not waiting on you turn by turn; your final reply is what \
         it receives, so end with the result — what you found, where, and what it means — in a \
         few short paragraphs. Use send_message only for something `{lead}` needs before you \
         finish. Never start another agent or another Orochi.\n\nYour task:\n{task}"
    )
}
