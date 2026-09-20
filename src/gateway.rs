//! Orochi as a stdio ACP v1 agent. Each prompt runs the existing scheduler on an isolated runtime.
use crate::{
    acp::ExecutionEvent,
    config::Config,
    policy::Registry,
    scheduler::{self, RunOptions},
    storage::{Store, workspace_lock},
    types::Overrides,
};
use agent_client_protocol::schema::{ProtocolVersion, v1::*};
use agent_client_protocol::{Agent, Client, ConnectionTo, Responder, Stdio};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot, watch};

struct Session {
    root: PathBuf,
    history: String,
    cancel: Option<watch::Sender<bool>>,
    /// The thread this session is in the conversation store, opened by its first prompt.
    thread: Option<String>,
}
type Sessions = Arc<Mutex<BTreeMap<String, Session>>>;
struct Running {
    sessions: Sessions,
    id: String,
    cancel: watch::Sender<bool>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        if let Some(session) = self.sessions.lock().unwrap().get_mut(&self.id) {
            session.cancel = None;
        }
    }
}
fn invalid() -> agent_client_protocol::Error {
    Error::invalid_params()
}

/// Opens (or continues) the thread a gateway session is, and the turn this prompt is. A
/// failure to record never stops the prompt from running.
fn open_turn(
    config: &Config,
    store: &Store,
    root: &Path,
    thread: Option<String>,
    text: &str,
) -> Option<(crate::activity::Shared, crate::activity::SeatRef, String)> {
    if !config.activity.enabled {
        return None;
    }
    // The gateway joins prompt blocks with newlines; what is recorded is the message, not
    // that joining.
    let text = text.trim_end();
    let opened = (|| -> anyhow::Result<_> {
        let activity = crate::activity::share(crate::activity::Activity::open(
            store.data_dir(),
            config.activity.retention_days,
        )?);
        let repository_id = store.repository_id(root)?;
        let opened = activity.lock().expect("activity store");
        let (thread, turn, host) = match thread {
            Some(thread) => {
                let host = opened.register_host(Some(&thread), "serve")?;
                let turn = opened.queue_turn(&thread, text, &[], "solo", "acp")?;
                opened.claim_turn(&turn, &host)?;
                opened.turn_shape(&turn, Some("solo"), None)?;
                (thread, turn, host)
            }
            None => opened.start_thread(
                root,
                &store.salt()?,
                &repository_id,
                crate::activity::Origin::Serve,
                text,
                &[],
                &Overrides::default(),
                config.scheduler.permission.key(),
                "solo",
            )?,
        };
        let seat = opened.create_seat(&turn, 0, "implementer", true, false, None, None)?;
        drop(opened);
        Ok((
            activity.clone(),
            crate::activity::SeatRef {
                thread,
                turn,
                seat,
                store: activity,
            },
            host,
        ))
    })();
    opened
        .map_err(|error| tracing::debug!(%error, "gateway turn not recorded"))
        .ok()
}

fn bounded_bytes(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub async fn serve(config: Config, data: PathBuf) -> anyhow::Result<()> {
    let sessions: Sessions = Arc::new(Mutex::new(BTreeMap::new()));
    let initialized = Arc::new(AtomicBool::new(false));
    let init = initialized.clone();
    let new_init = initialized.clone();
    let new_sessions = sessions.clone();
    let prompt_sessions = sessions.clone();
    let cancel_sessions = sessions.clone();
    let workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    let worker_handles = workers.clone();
    let result = Agent.builder().name("orochi")
        .on_receive_request(async move |_: InitializeRequest, responder, _cx| {
            init.store(true, Ordering::Release);
            responder.respond(InitializeResponse::new(ProtocolVersion::V1)
                .agent_capabilities(AgentCapabilities::new())
                .agent_info(Implementation::new("orochi", env!("CARGO_PKG_VERSION"))))
        }, agent_client_protocol::on_receive_request!())
        .on_receive_request(async move |request: NewSessionRequest, responder, _cx| {
            if !new_init.load(Ordering::Acquire) || !request.cwd.is_absolute() || !request.mcp_servers.is_empty() || !request.additional_directories.is_empty() {
                return responder.respond_with_error(invalid());
            }
            let root = match request.cwd.canonicalize() { Ok(p) if p.is_dir() => p, _ => return responder.respond_with_error(invalid()) };
            let mut sessions = new_sessions.lock().unwrap();
            if sessions.len() >= 128 { return responder.respond_with_internal_error("session limit reached; restart the gateway"); }
            let id = uuid::Uuid::new_v4().to_string();
            sessions.insert(id.clone(), Session { root, history: String::new(), cancel: None, thread: None });
            responder.respond(NewSessionResponse::new(id))
        }, agent_client_protocol::on_receive_request!())
        .on_receive_notification(async move |request: CancelNotification, _cx| {
            if let Some(session) = cancel_sessions.lock().unwrap().get(&request.session_id.to_string())
                && let Some(cancel) = &session.cancel { let _ = cancel.send(true); }
            Ok(())
        }, agent_client_protocol::on_receive_notification!())
        .on_receive_request(async move |request: PromptRequest, responder: Responder<PromptResponse>, cx: ConnectionTo<Client>| {
            if !initialized.load(Ordering::Acquire) { return responder.respond_with_error(invalid()); }
            let mut text = String::new();
            for block in request.prompt {
                match block {
                    ContentBlock::Text(content) => text.push_str(&content.text),
                    ContentBlock::ResourceLink(link) => { text.push_str("\nReferenced resource: "); text.push_str(&link.uri); }
                    _ => return responder.respond_with_error(invalid()),
                }
                text.push('\n');
                if text.len() > 1_000_000 { return responder.respond_with_error(invalid()); }
            }
            if text.trim().is_empty() { return responder.respond_with_error(invalid()); }
            let id = request.session_id.to_string();
            let (cancel, cancellation) = watch::channel(false);
            let (root, history, thread) = {
                let mut sessions = prompt_sessions.lock().unwrap();
                let Some(session) = sessions.get_mut(&id) else { return responder.respond_with_error(invalid()); };
                if session.cancel.is_some() { return responder.respond_with_error(invalid()); }
                session.cancel = Some(cancel.clone());
                (session.root.clone(), session.history.clone(), session.thread.clone())
            };
            let running = Running { sessions: prompt_sessions.clone(), id: id.clone(), cancel };
            let task = if history.is_empty() { text.clone() } else { format!("Previous conversation (context):\n{history}\n\nCurrent user request:\n{text}") };
            let config = config.clone(); let data = data.clone();
            let (events, receiver) = mpsc::unbounded_channel();
            let (finished, done) = oneshot::channel();
            let (opened_tx, opened_rx) = oneshot::channel();
            let prompt_text = text.clone();
            let mut worker_cancel = cancellation.clone();
            let worker = std::thread::Builder::new().name("orochi-acp-run".into()).spawn(move || {
                let result = (|| -> anyhow::Result<u8> {
                    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                    runtime.block_on(async {
                        let store = Store::open(&data)?;
                        let _lock = workspace_lock(&data, &store.repository_id(&root)?, config.scheduler.shared_workspace)?;
                        let policies = Registry::load(&data)?;
                        // A session is a thread and each prompt a turn in it, as a console
                        // conversation is, so gateway work appears beside terminal work.
                        let recorded = open_turn(&config, &store, &root, thread, &prompt_text);
                        let _ = opened_tx.send(recorded.as_ref().map(|(_, seat, _)| seat.thread.clone()));
                        let options = RunOptions { task, descriptor: None, overrides: Overrides::default(), dry_run: false, json: false, resume: None, permission: config.scheduler.permission, interactive: false, attachments: vec![], peer: None, verify: true, read_only: false, place: None, seat: recorded.as_ref().map(|(_, seat, _)| seat.clone()), answerer: crate::activity::recorder::Answerer::Local(config.scheduler.permission) };
                        let code = tokio::select! {
                            result = scheduler::run_with_events(&config, &policies, &store, &root, options, Some(events)) => result,
                            _ = worker_cancel.wait_for(|v| *v) => Ok(130),
                        };
                        if let Some((activity, seat, host)) = &recorded {
                            let state = match &code {
                                Ok(0) => crate::activity::TurnState::Completed,
                                Ok(130) => crate::activity::TurnState::Interrupted,
                                _ => crate::activity::TurnState::Failed,
                            };
                            let _ = activity
                                .lock()
                                .expect("activity store")
                                .end_turn(seat, host, state);
                        }
                        code
                    })
                })();
                let _ = finished.send(result.map_err(|e| e.to_string()));
            });
            match worker {
                Ok(worker) => {
                    let mut handles = worker_handles.lock().unwrap();
                    let mut pending = vec![];
                    for handle in handles.drain(..) {
                        if handle.is_finished() { let _ = handle.join(); } else { pending.push(handle); }
                    }
                    pending.push(worker); *handles = pending;
                },
                Err(error) => { drop(running); return responder.respond_with_internal_error(error); }
            }
            let output = cx.clone();
            cx.spawn(async move { forward(output, responder, id, text, running, cancellation, receiver, done, opened_rx).await })?;
            Ok(())
        }, agent_client_protocol::on_receive_request!())
        .connect_to(Stdio::new()).await;
    for session in sessions.lock().unwrap().values() {
        if let Some(cancel) = &session.cancel {
            let _ = cancel.send(true);
        }
    }
    let handles = std::mem::take(&mut *workers.lock().unwrap());
    tokio::task::spawn_blocking(move || {
        for handle in handles {
            let _ = handle.join();
        }
    })
    .await?;
    result?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn forward(
    cx: ConnectionTo<Client>,
    responder: Responder<PromptResponse>,
    id: String,
    text: String,
    running: Running,
    mut cancel: watch::Receiver<bool>,
    mut events: mpsc::UnboundedReceiver<ExecutionEvent>,
    mut done: oneshot::Receiver<Result<u8, String>>,
    opened_rx: oneshot::Receiver<Option<String>>,
) -> agent_client_protocol::Result<()> {
    let request_cancel = responder.cancellation();
    let mut requested_cancel = false;
    let mut response_text = String::new();
    let code = loop {
        tokio::select! {
            biased;
            event = events.recv(), if !events.is_closed() || !events.is_empty() => {
                match event {
                    Some(ExecutionEvent::Text(text, _)) => {
                        if response_text.len() < 16384 { response_text.push_str(bounded_bytes(&text, 16384 - response_text.len())); }
                        cx.send_notification(SessionNotification::new(id.clone(), SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(text))))))?;
                    }
                    Some(ExecutionEvent::Permission(mut value, answer)) => {
                        value["sessionId"] = serde_json::Value::String(id.clone());
                        let request: RequestPermissionRequest = serde_json::from_value(value).map_err(|_| invalid())?;
                        let allowed: Vec<_> = request.options.iter().filter(|o| o.kind == PermissionOptionKind::AllowOnce).map(|o| o.option_id.to_string()).collect();
                        let response = tokio::select! {
                            result = cx.send_request(request).block_task() => result.ok(),
                            _ = cancel.wait_for(|v| *v) => None,
                            _ = request_cancel.cancelled() => { let _ = running.cancel.send(true); None },
                        };
                        let choice = response.and_then(|r| match r.outcome { RequestPermissionOutcome::Selected(s) => Some(s.option_id.to_string()), _ => None }).filter(|id| allowed.contains(id));
                        let _ = answer.send(choice);
                    }
                    Some(ExecutionEvent::Finished | ExecutionEvent::Progress(_)) | None => {}
                }
            }
            _ = request_cancel.cancelled(), if !requested_cancel => {
                requested_cancel = true; let _ = running.cancel.send(true);
            }
            result = &mut done => break result.unwrap_or_else(|_| Err("scheduler worker stopped".into())),
        }
    };
    // The worker has finished, and with it everything the store had in flight; what it
    // forwarded last is still in the channel.
    while let Ok(event) = events.try_recv() {
        if let ExecutionEvent::Text(text, _) = event {
            if response_text.len() < 16384 {
                response_text.push_str(bounded_bytes(&text, 16384 - response_text.len()));
            }
            cx.send_notification(SessionNotification::new(
                id.clone(),
                SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new(text),
                ))),
            ))?;
        }
    }
    let opened = opened_rx.await.ok().flatten();
    if let Some(session) = running.sessions.lock().unwrap().get_mut(&id) {
        session.history = format!(
            "User: {}\nAgent: {}",
            bounded_bytes(&text, 8192),
            response_text
        );
        // The first prompt opened the thread; the ones after it are turns in the same thread.
        if session.thread.is_none() {
            session.thread = opened;
        }
    }
    match code {
        Ok(0) => responder.respond(PromptResponse::new(StopReason::EndTurn)),
        Ok(130) => responder.respond(PromptResponse::new(StopReason::Cancelled)),
        Ok(_) => responder.respond_with_internal_error(
            "task incomplete; evaluator failed or eligible agents exhausted",
        ),
        Err(error) => responder.respond_with_internal_error(error),
    }
}
