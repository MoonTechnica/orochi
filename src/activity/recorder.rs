//! Turns the one event stream Orochi already has into rows.
//!
//! It is a **tee**, not a replacement: every event is forwarded to whoever asked for it (the
//! console's screen, the gateway, a collaboration turn) exactly as before, and a copy is
//! written down on the way past. A failure to write never fails a turn — the work is what
//! matters, and a store that cannot be written degrades to the behavior Orochi had before one
//! existed.
use super::{Activity, ItemKind, SeatState, Shared};
use crate::acp::{EventSink, ExecutionEvent, Progress, ToolUpdate};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Streamed text is flushed no more often than this. A token at a time would be one
/// transaction per token; a reader polling at 100 ms cannot see the difference.
const FLUSH: Duration = Duration::from_millis(80);

/// Where a recorder is writing: one seat of one turn, and the attempt currently filling it.
pub struct Seat {
    pub thread: String,
    pub turn: String,
    pub seat: String,
    pub attempt: Option<String>,
}

impl From<&super::SeatRef> for Seat {
    fn from(seat: &super::SeatRef) -> Self {
        Self {
            thread: seat.thread.clone(),
            turn: seat.turn.clone(),
            seat: seat.seat.clone(),
            attempt: None,
        }
    }
}

/// The open `agent_message` row and what has not been written to it yet.
#[derive(Default)]
struct Stream {
    item: Option<i64>,
    /// The `messageId` the open row belongs to, when the agent sends one.
    message_id: Option<String>,
    buffer: String,
    since: Option<Instant>,
}

/// Which attempt the rows being written belong to. It is shared rather than sent down the
/// event channel because the scheduler only ever changes it at a quiet point — the previous
/// attempt's client has been stopped and the next one has not been prompted — so there is no
/// event in flight to attribute to the wrong attempt.
type Current = Arc<Mutex<Option<String>>>;

pub struct Recorder {
    store: Shared,
    place: Seat,
    current: Current,
    thinking: bool,
    message: Stream,
    thought: Stream,
}

impl Recorder {
    pub fn new(store: Shared, place: Seat, thinking: bool) -> Self {
        let current = Arc::new(Mutex::new(place.attempt.clone()));
        Self {
            store,
            place,
            current,
            thinking,
            message: Stream::default(),
            thought: Stream::default(),
        }
    }

    pub fn seat(&self) -> &str {
        &self.place.seat
    }

    /// Records one event. The connection is locked **once** here and handed down: taking the
    /// guard again inside would deadlock a plain mutex, and a deadlock here hangs every agent
    /// this recorder is writing for. Errors are swallowed on purpose: see the module note.
    pub fn record(&mut self, event: &ExecutionEvent) {
        let shared = self.store.clone();
        let db = shared.lock().expect("activity store");
        let current = self.current.lock().expect("recorder attempt").clone();
        if current != self.place.attempt {
            // A failover: whatever the previous attempt was streaming ends with it.
            self.finish_streams(&db);
            self.place.attempt = current;
        }
        if let Err(error) = self.write(&db, event) {
            tracing::debug!(%error, "activity not recorded");
        }
    }

    fn write(&mut self, db: &Activity, event: &ExecutionEvent) -> anyhow::Result<()> {
        match event {
            ExecutionEvent::Text(chunk, id) => {
                // A different `messageId` is a different message, even mid-stream; without
                // one, a contiguous run of chunks is one message.
                if self.message.item.is_some()
                    && id.is_some()
                    && self.message.message_id.as_deref() != id.as_deref()
                {
                    self.close(db, true)?;
                }
                self.message.message_id.clone_from(id);
                self.stream(db, true, chunk)?;
            }
            ExecutionEvent::Finished => {
                self.finish_streams(db);
                self.seat_state(db, SeatState::Done)?;
            }
            ExecutionEvent::Permission(..) => self.seat_state(db, SeatState::Asking)?,
            ExecutionEvent::Progress(progress) => self.progress(db, progress)?,
        }
        Ok(())
    }

    fn progress(&mut self, db: &Activity, progress: &Progress) -> anyhow::Result<()> {
        match progress {
            Progress::Thinking(text) => {
                if self.thinking {
                    self.close(db, true)?;
                    self.stream(db, false, text)?;
                }
            }
            Progress::Tool(update) => {
                self.finish_streams(db);
                self.tool(db, update)?;
            }
            Progress::Plan(entries) => {
                let entries: Vec<_> = entries
                    .iter()
                    .map(|(status, content)| json!({"status": status, "content": content}))
                    .collect();
                self.item(db, ItemKind::Plan, None, None, "", Some(&json!(entries)))?;
            }
            Progress::Route {
                agent,
                provider,
                model,
                reasoning,
                resumed,
            } => {
                self.item(
                    db,
                    ItemKind::Route,
                    None,
                    None,
                    "",
                    Some(&json!({"agent": agent, "provider": provider.to_string(),
                        "model": model, "reasoning": reasoning, "resumed": resumed})),
                )?;
                self.seat_state(db, SeatState::Working)?;
            }
            Progress::Unavailable { agent, error } => {
                self.item(
                    db,
                    ItemKind::Unavailable,
                    None,
                    None,
                    error,
                    Some(&json!({ "agent": agent })),
                )?;
            }
            Progress::Note(text) => {
                self.item(db, ItemKind::Note, None, None, text, None)?;
            }
            Progress::Checking => self.seat_state(db, SeatState::Checking)?,
            Progress::Attempt {
                outcome,
                checks,
                error,
                usage,
                output,
            } => {
                self.finish_streams(db);
                // A failed check's last lines, beside its result: the first thing a reviewer
                // wants, and the one place check output is kept.
                let checks: Vec<_> = checks
                    .iter()
                    .map(|check| {
                        let mut value = json!(check);
                        if let Some(tail) = output.get(&check.name) {
                            value["tail"] = json!(tail);
                        }
                        value
                    })
                    .collect();
                self.item(
                    db,
                    ItemKind::Checks,
                    None,
                    None,
                    "",
                    Some(&json!({"outcome": outcome, "checks": checks,
                        "error": error, "usage": usage})),
                )?;
            }
            Progress::Mode(mode) => {
                self.item(
                    db,
                    ItemKind::Mode,
                    None,
                    None,
                    "",
                    Some(&json!({"mode": mode})),
                )?;
            }
            Progress::Context { used, size } => {
                self.item(
                    db,
                    ItemKind::Context,
                    None,
                    None,
                    "",
                    Some(&json!({"used": used, "size": size})),
                )?;
            }
            Progress::Commands(commands) => {
                let commands: Vec<_> = commands
                    .iter()
                    .map(|(name, description)| json!({"name": name, "description": description}))
                    .collect();
                self.item(
                    db,
                    ItemKind::Commands,
                    None,
                    None,
                    "",
                    Some(&json!(commands)),
                )?;
            }
            // A title the agent offered, which the user's own name for the thread outranks.
            Progress::Info(title) => db.title(&self.place.thread, title, false)?,
            Progress::Other(value) => {
                self.item(db, ItemKind::Acp, None, None, "", Some(value))?;
            }
        }
        Ok(())
    }

    /// One row per tool call, found again by its ACP id so an update merges into the call it
    /// belongs to. Diffs become patches under it.
    fn tool(&mut self, db: &Activity, update: &ToolUpdate) -> anyhow::Result<()> {
        let Some(attempt) = self.place.attempt.clone() else {
            return Ok(());
        };
        let raw = |value: &Option<serde_json::Value>| {
            value.as_ref().map(|v| {
                let text = v.to_string();
                if text.len() > super::MAX_RAW {
                    json!({"truncated": true})
                } else {
                    v.clone()
                }
            })
        };
        // Only what this update carried: a field it left out keeps the value the call already
        // had, which is the protocol's rule and what `Activity::update_tool` merges by.
        let mut data = serde_json::Map::new();
        let mut set = |key: &str, value: Option<serde_json::Value>| {
            if let Some(value) = value {
                data.insert(key.into(), value);
            }
        };
        set("title", update.title.clone().map(Into::into));
        set("tool_kind", update.kind.clone().map(Into::into));
        set("detail", update.detail.clone().map(Into::into));
        set(
            "locations",
            (!update.locations.is_empty()).then(|| {
                update
                    .locations
                    .iter()
                    .map(|(path, line)| json!({"path": path, "line": line}))
                    .collect::<Vec<_>>()
                    .into()
            }),
        );
        set("raw_input", raw(&update.raw_input));
        set("raw_output", raw(&update.raw_output));
        let data = serde_json::Value::Object(data);
        // Bound first: a guard in a `match` scrutinee lives for the whole match, and the
        // arms below lock again.
        let existing = db.tool_item(&attempt, &update.id)?;
        let item = match existing {
            Some(item) => {
                db.update_tool(
                    item,
                    update.status.as_deref(),
                    update.output.as_deref(),
                    Some(&data.to_string()),
                )?;
                item
            }
            None => db.item(
                &self.place.thread,
                &self.place.turn,
                Some(&attempt),
                ItemKind::ToolCall,
                update.status.as_deref(),
                Some(&update.id),
                update.output.as_deref().unwrap_or_default(),
                Some(&data.to_string()),
            )?,
        };
        for diff in &update.diffs {
            db.patch(
                item,
                &self.place.turn,
                &diff.path,
                diff.old.as_deref(),
                &diff.new,
            )?;
        }
        Ok(())
    }

    fn item(
        &self,
        db: &Activity,
        kind: ItemKind,
        status: Option<&str>,
        key: Option<&str>,
        text: &str,
        data: Option<&serde_json::Value>,
    ) -> anyhow::Result<i64> {
        db.item(
            &self.place.thread,
            &self.place.turn,
            self.place.attempt.as_deref(),
            kind,
            status,
            key,
            text,
            data.map(|d| d.to_string()).as_deref(),
        )
    }

    fn seat_state(&self, db: &Activity, state: SeatState) -> anyhow::Result<()> {
        db.seat_state(&self.place.seat, state)
    }

    fn stream_of(&mut self, message: bool) -> &mut Stream {
        if message {
            &mut self.message
        } else {
            &mut self.thought
        }
    }

    /// Opens the row if it is not open, buffers the chunk, and writes when the buffer has
    /// been waiting long enough.
    fn stream(&mut self, db: &Activity, message: bool, chunk: &str) -> anyhow::Result<()> {
        if self.stream_of(message).item.is_none() {
            let kind = if message {
                ItemKind::AgentMessage
            } else {
                ItemKind::Thought
            };
            let item = self.item(db, kind, Some("streaming"), None, "", None)?;
            self.stream_of(message).item = Some(item);
            self.stream_of(message).since = Some(Instant::now());
        }
        let stream = self.stream_of(message);
        stream.buffer.push_str(chunk);
        let due = stream.since.is_none_or(|since| since.elapsed() >= FLUSH);
        if due {
            self.flush(db, message)?;
        }
        Ok(())
    }

    fn flush(&mut self, db: &Activity, message: bool) -> anyhow::Result<()> {
        let stream = self.stream_of(message);
        let (Some(item), false) = (stream.item, stream.buffer.is_empty()) else {
            return Ok(());
        };
        let chunk = std::mem::take(&mut stream.buffer);
        stream.since = Some(Instant::now());
        db.append(item, &chunk)
    }

    /// Writes what is buffered and marks the row finished.
    fn close(&mut self, db: &Activity, message: bool) -> anyhow::Result<()> {
        self.flush(db, message)?;
        let stream = self.stream_of(message);
        let item = stream.item.take();
        stream.message_id = None;
        stream.since = None;
        if let Some(item) = item {
            db.item_status(item, "completed")?;
        }
        Ok(())
    }

    fn finish_streams(&mut self, db: &Activity) {
        for message in [true, false] {
            if let Err(error) = self.close(db, message) {
                tracing::debug!(%error, "activity stream not closed");
            }
        }
    }
}

impl Drop for Recorder {
    /// A turn that ends without `Finished` — interrupted, or its agent gone — still leaves
    /// what was streamed before it stopped.
    fn drop(&mut self) {
        let shared = self.store.clone();
        let db = shared.lock().expect("activity store");
        self.finish_streams(&db);
    }
}

/// Puts the store in the path of an event stream without changing what the stream delivers.
///
/// Everything the scheduler and `acp.rs` already send goes to whoever asked for it exactly as
/// before; a copy is written down on the way past, on a task of its own so that no agent ever
/// waits for a disk write. Dropping the tee ends that task once what is queued has been
/// written.
pub struct Tee {
    sink: EventSink,
    current: Current,
    close: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Tee {
    /// `echo` takes over what `acp.rs` does when nothing is listening: a one-shot run prints
    /// the agent's reply to stdout. Without it, putting the store in the path would silence
    /// the very output the run exists to produce.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        store: Shared,
        place: Seat,
        thinking: bool,
        downstream: Option<EventSink>,
        echo: bool,
        answerer: Answerer,
    ) -> Self {
        let (sink, mut events) = tokio::sync::mpsc::unbounded_channel();
        let asking = Asking {
            store: store.clone(),
            thread: place.thread.clone(),
            turn: place.turn.clone(),
            answerer,
        };
        let mut recorder = Recorder::new(store, place, thinking);
        let current = recorder.current.clone();
        let (close, mut closed) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    event = events.recv() => event,
                    // Asked to stop: write what is already queued, then leave. The agents'
                    // senders may outlive this, so waiting for the channel to close is not
                    // an option.
                    _ = &mut closed => {
                        while let Ok(event) = events.try_recv() {
                            recorder.record(&event);
                            if echo && let ExecutionEvent::Text(text, _) = &event {
                                use std::io::Write;
                                print!("{}", text.replace('\u{1b}', "\\x1b"));
                                let _ = std::io::stdout().flush();
                            }
                            if let Some(downstream) = &downstream {
                                let _ = downstream.send(event);
                            }
                        }
                        return;
                    }
                };
                let Some(event) = event else { return };
                recorder.record(&event);
                if echo && let ExecutionEvent::Text(text, _) = &event {
                    use std::io::Write;
                    print!("{}", text.replace('\u{1b}', "\\x1b"));
                    let _ = std::io::stdout().flush();
                }
                // A question is a row as well as an event, so a client can see it and answer
                // it. Whoever answers first wins; the agent waits for exactly one answer.
                if let ExecutionEvent::Permission(request, reply) = event {
                    asking.open(recorder.place.attempt.clone(), request, reply, &downstream);
                    continue;
                }
                if let Some(downstream) = &downstream {
                    let _ = downstream.send(event);
                }
            }
        });
        Self {
            sink,
            current,
            close: Some(close),
            task: Some(task),
        }
    }

    /// The sink to hand to `acp` and to report progress through, in place of the caller's.
    pub fn sink(&self) -> EventSink {
        self.sink.clone()
    }

    /// Names the attempt the rows that follow belong to.
    pub fn attempt(&self, attempt: Option<String>) {
        *self.current.lock().expect("recorder attempt") = attempt;
    }

    /// Waits for what has been sent to be written. A turn is not finished until its last row
    /// is in the store, or a client reading it would see a truncated conversation.
    pub async fn finish(mut self) {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Tee {
    fn drop(&mut self) {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
    }
}

/// Who settles a permission question when nobody is listening on the event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answerer {
    /// This run decides for itself, as it did before a store sat in the path: the terminal is
    /// asked, or the configured mode answers.
    Local(crate::config::PermissionMode),
    /// A headless host: the question stays open until a client answers it. Nobody here can,
    /// so guessing would be worse than waiting.
    Store,
}

/// Opens permission questions as rows and settles them.
struct Asking {
    store: Shared,
    thread: String,
    turn: String,
    answerer: Answerer,
}

impl Asking {
    fn open(
        &self,
        attempt: Option<String>,
        request: serde_json::Value,
        reply: tokio::sync::oneshot::Sender<Option<String>>,
        downstream: &Option<EventSink>,
    ) {
        let row = self
            .store
            .lock()
            .expect("activity store")
            .open_prompt(
                &self.thread,
                &self.turn,
                attempt.as_deref(),
                None,
                "permission",
                &request.to_string(),
            )
            .map_err(|error| tracing::debug!(%error, "permission not recorded"))
            .ok();
        let downstream = match (downstream, self.answerer) {
            // The row is the only answerer, whether or not anything is watching this run: a
            // watcher with no terminal cannot answer, and holding a channel it never answers
            // on would read as a refusal the moment the sender dropped.
            (_, Answerer::Store) => None,
            (Some(downstream), _) => Some(downstream),
            (None, Answerer::Local(permission)) => {
                let store = self.store.clone();
                tokio::spawn(async move {
                    let answer = crate::acp::choose_permission(&request, permission).await;
                    if let Some(row) = row {
                        let _ = store.lock().expect("activity store").answer_prompt(
                            &row,
                            answer.as_deref(),
                            "policy",
                        );
                    }
                    let _ = reply.send(answer);
                });
                return;
            }
        };
        // With someone listening, they and the store race, and the loser's dialog closes when
        // it sees the row answered. With nobody, the row is the only answerer — and then there
        // must be no terminal channel at all, or dropping its unused sender would read as an
        // instant refusal.
        let mut terminal = None;
        if let Some(downstream) = downstream {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if downstream
                .send(ExecutionEvent::Permission(request, tx))
                .is_err()
            {
                let _ = reply.send(None);
                return;
            }
            terminal = Some(rx);
        }
        let Some(row) = row else {
            tokio::spawn(async move {
                let answer = match terminal {
                    Some(rx) => rx.await.ok().flatten(),
                    None => None,
                };
                let _ = reply.send(answer);
            });
            return;
        };
        let store = self.store.clone();
        tokio::spawn(async move {
            let mut poll = tokio::time::interval(std::time::Duration::from_millis(150));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let answer = loop {
                tokio::select! {
                    answered = async {
                        match &mut terminal {
                            Some(rx) => rx.await.ok().flatten(),
                            // Nobody to hear from: wait for the row instead of resolving.
                            None => std::future::pending().await,
                        }
                    } => {
                        let _ = store.lock().expect("activity store").answer_prompt(
                            &row,
                            answered.as_deref(),
                            "terminal",
                        );
                        break answered;
                    }
                    _ = poll.tick() => {
                        let stored = store
                            .lock()
                            .expect("activity store")
                            .prompt_answer(&row)
                            .ok()
                            .flatten();
                        if let Some(answer) = stored {
                            break answer;
                        }
                    }
                }
            };
            let _ = reply.send(answer);
        });
    }
}
