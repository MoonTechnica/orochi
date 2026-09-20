use crate::{
    agents::{AgentAdapter, AgentError, ErrorKind, ProviderAdapter, QuotaObservation},
    config::{AgentConfig, PermissionMode},
    types::{ExecutionCandidate, Usage},
};
use agent_client_protocol::schema::{ProtocolVersion, v1::*};
use agent_client_protocol::{
    AcpAgent, AcpAgentConfig, Agent, ConnectionTo, JsonRpcRequest, JsonRpcResponse, LineDirection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selector {
    pub id: String,
    pub current: String,
    pub values: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub model: String,
    pub reasoning: Option<Selector>,
    pub modes: Option<Selector>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub session_id: String,
    pub models: Vec<ModelCapabilities>,
    pub load_session: bool,
    /// The agent accepts images in prompts.
    pub image: bool,
    /// The agent accepts embedded file contents in prompts.
    pub embedded: bool,
    pub model_selector: Option<Selector>,
    pub legacy_models: bool,
}

/// Flatten ACP select groups without losing opaque IDs.
pub fn option_values(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| {
            if let Some(value) = v["value"].as_str() {
                vec![value.to_owned()]
            } else {
                option_values(&v["options"])
            }
        })
        .collect()
}
pub fn selector(state: &Value, category: &str) -> Option<Selector> {
    state["configOptions"]
        .as_array()?
        .iter()
        .find_map(|option| {
            let id = option["id"].as_str()?;
            let matches = option["category"].as_str() == Some(category)
                || id == category
                || (category == "thought_level"
                    && matches!(
                        id,
                        "reasoning" | "reasoning_effort" | "thinking_level" | "effort"
                    ));
            if !matches || option["type"].as_str() != Some("select") {
                return None;
            }
            let current = option["currentValue"].as_str()?.to_owned();
            let mut values = option_values(&option["options"]);
            if !values.contains(&current) {
                values.push(current.clone());
            }
            Some(Selector {
                id: id.to_owned(),
                current,
                values,
            })
        })
}

#[derive(Default)]
struct Observation {
    session: Value,
    last_error: Value,
    usage: Usage,
    quota: Option<QuotaObservation>,
    stream: bool,
    denied: bool,
}

struct Lifecycle(Option<JoinHandle<()>>);
impl Drop for Lifecycle {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcRequest)]
#[request(method = "session/set_model", response = RawResponse)]
#[serde(rename_all = "camelCase")]
struct LegacySetModel {
    session_id: String,
    model_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcResponse)]
#[serde(transparent)]
struct RawResponse(Value);

pub enum ExecutionEvent {
    /// A chunk of the agent's reply, and the `messageId` the chunk belongs to when the agent
    /// sends one — what tells two interleaved messages apart in a stored timeline. Without
    /// one, a reader treats a contiguous run of chunks as one message.
    Text(String, Option<String>),
    Permission(Value, oneshot::Sender<Option<String>>),
    /// The prompt turn ended; no further text follows for it.
    Finished,
    /// Presentation-only progress; consumers may ignore it.
    Progress(Progress),
}

#[derive(Debug, Clone)]
pub enum Progress {
    /// A chunk of the agent's reasoning text.
    Thinking(String),
    /// A tool call started or changed. Fields absent from an update stay `None`/empty.
    /// Boxed: it is by far the largest thing a progress event carries, and every other
    /// variant would otherwise pay for it.
    Tool(Box<ToolUpdate>),
    /// Plan entries as (status, content).
    Plan(Vec<(String, String)>),
    Route {
        agent: String,
        provider: crate::types::Provider,
        model: String,
        reasoning: Option<String>,
        resumed: bool,
    },
    Unavailable {
        agent: String,
        error: String,
    },
    /// Something worth one muted line in the transcript, such as a model left alone because
    /// its account is nearly spent.
    Note(String),
    Checking,
    Attempt {
        outcome: crate::types::Outcome,
        checks: Vec<crate::types::CheckResult>,
        error: Option<String>,
        usage: Usage,
        /// The tail of each failed check's output, by check name. It exists for the one place
        /// check output is kept — the conversation store — and never reaches telemetry.
        output: std::collections::BTreeMap<String, String>,
    },
    /// The agent's own session mode changed (`current_mode_update`), by its doing or ours.
    Mode(String),
    /// `usage_update`: how much of the model's context this session is holding.
    Context {
        used: u64,
        size: Option<u64>,
    },
    /// `available_commands_update`: the agent's own slash commands, as (name, description).
    Commands(Vec<(String, String)>),
    /// `session_info_update`: a title the agent offers for the conversation. Orochi never
    /// asks for one, so this is a title that costs nothing.
    Info(String),
    /// An update kind this version does not know, kept as received so that a conversation
    /// recorded today still renders when the protocol grows a kind tomorrow.
    Other(Value),
}

#[derive(Debug, Clone, Default)]
pub struct ToolUpdate {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    /// What the tool acts on: `$ command`, a path, a pattern or a URL.
    pub detail: Option<String>,
    /// Text output lines.
    pub output: Option<String>,
    pub diffs: Vec<FileDiff>,
    /// ACP's own classification: read, edit, delete, move, search, execute, think, fetch,
    /// switch_mode, other. It is what a permission decision already turns on, and what lets a
    /// stored timeline group a turn's work without re-reading the title.
    pub kind: Option<String>,
    /// Files the call touched, as (path, line).
    pub locations: Vec<(String, Option<u64>)>,
    /// The call's arguments and result as the agent sent them, for a detail pane. Bounded
    /// where they are stored, not here.
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub old: Option<String>,
    pub new: String,
}

fn tool_detail(update: &Value) -> Option<String> {
    let input = &update["rawInput"];
    let command = match &input["command"] {
        Value::String(command) => Some(command.clone()),
        Value::Array(parts) => {
            let parts: Vec<_> = parts.iter().filter_map(Value::as_str).collect();
            // Shell wrappers add nothing a reader needs.
            Some(match parts.as_slice() {
                [shell, flag, script] if flag.starts_with('-') && shell.ends_with("sh") => {
                    (*script).to_owned()
                }
                _ => parts.join(" "),
            })
        }
        _ => None,
    };
    command.map(|c| format!("$ {c}")).or_else(|| {
        [
            "file_path",
            "path",
            "notebook_path",
            "url",
            "pattern",
            "query",
        ]
        .iter()
        .find_map(|key| input[key].as_str().map(str::to_owned))
    })
}

pub(crate) fn tool_update(update: &Value) -> Option<ToolUpdate> {
    let text = |key: &str| update[key].as_str().map(str::to_owned);
    let mut tool = ToolUpdate {
        id: text("toolCallId")?,
        title: text("title"),
        status: text("status"),
        detail: tool_detail(update),
        kind: text("kind"),
        locations: update["locations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| Some((l["path"].as_str()?.to_owned(), l["line"].as_u64())))
            .collect(),
        raw_input: update.get("rawInput").filter(|v| !v.is_null()).cloned(),
        raw_output: update.get("rawOutput").filter(|v| !v.is_null()).cloned(),
        ..Default::default()
    };
    let mut lines = Vec::new();
    for item in update["content"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("content") => {
                if let Some(text) = item["content"]["text"].as_str() {
                    lines.push(text);
                }
            }
            Some("diff") => {
                if let (Some(path), Some(new)) = (item["path"].as_str(), item["newText"].as_str()) {
                    tool.diffs.push(FileDiff {
                        path: path.into(),
                        old: item["oldText"].as_str().map(str::to_owned),
                        new: new.into(),
                    });
                }
            }
            _ => {}
        }
    }
    tool.output = (!lines.is_empty()).then(|| lines.join("\n"));
    Some(tool)
}

fn progress(update: &Value) -> Option<Progress> {
    match update["sessionUpdate"].as_str()? {
        "agent_thought_chunk" => Some(Progress::Thinking(
            update["content"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )),
        "tool_call" | "tool_call_update" => {
            tool_update(update).map(|t| Progress::Tool(Box::new(t)))
        }
        "plan" => Some(Progress::Plan(
            update["entries"]
                .as_array()?
                .iter()
                .filter_map(|e| Some((e["status"].as_str()?.into(), e["content"].as_str()?.into())))
                .collect(),
        )),
        "current_mode_update" => Some(Progress::Mode(update["currentModeId"].as_str()?.into())),
        "usage_update" => Some(Progress::Context {
            used: update["used"].as_u64()?,
            size: update["size"].as_u64(),
        }),
        "available_commands_update" => Some(Progress::Commands(
            update["availableCommands"]
                .as_array()?
                .iter()
                .filter_map(|c| {
                    Some((
                        c["name"].as_str()?.to_owned(),
                        c["description"].as_str().unwrap_or_default().to_owned(),
                    ))
                })
                .collect(),
        )),
        "session_info_update" => Some(Progress::Info(update["title"].as_str()?.into())),
        // Chunks and `config_option_update` are handled by the caller, which has the state
        // they change; anything else is kept verbatim rather than dropped.
        "agent_message_chunk" | "user_message_chunk" | "config_option_update" => None,
        _ => Some(Progress::Other(update.clone())),
    }
}

pub type EventSink = tokio::sync::mpsc::UnboundedSender<ExecutionEvent>;

pub struct Client {
    events: Option<EventSink>,
    /// This session's mailbox identity; agents in one process are separate peers.
    peer: Option<crate::mailbox::SessionPeer>,
    pub config: AgentConfig,
    discovery_model: Option<String>,
    connection: ConnectionTo<Agent>,
    observations: Arc<Mutex<Observation>>,
    lifecycle: Lifecycle,
    shutdown: Option<oneshot::Sender<()>>,
    pub capabilities: Capabilities,
    timeout: Duration,
    lease: Option<std::path::PathBuf>,
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        // SDK's AcpAgent guard kills the entire process group when the task drops.
        if let Some(task) = &self.lifecycle.0 {
            task.abort();
        }
        self.release();
    }
}

impl Client {
    pub async fn start(
        config: AgentConfig,
        root: &Path,
        permission: PermissionMode,
        timeout: Duration,
    ) -> Result<Self, AgentError> {
        Self::start_with_events(config, root, permission, timeout, None).await
    }
    pub async fn start_with_events(
        config: AgentConfig,
        root: &Path,
        permission: PermissionMode,
        timeout: Duration,
        events: Option<EventSink>,
    ) -> Result<Self, AgentError> {
        Self::start_with_model(config, root, permission, timeout, events, None).await
    }
    pub async fn start_with_model(
        config: AgentConfig,
        root: &Path,
        permission: PermissionMode,
        timeout: Duration,
        events: Option<EventSink>,
        model: Option<&str>,
    ) -> Result<Self, AgentError> {
        Self::start_named(
            config, root, permission, timeout, events, model, None, false,
        )
        .await
    }
    /// `peer` names this session in the mailbox (a collaboration role, say); a `read_only`
    /// session is refused the tools that would change the workspace.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_named(
        config: AgentConfig,
        root: &Path,
        permission: PermissionMode,
        timeout: Duration,
        events: Option<EventSink>,
        model: Option<&str>,
        peer: Option<&str>,
        read_only: bool,
    ) -> Result<Self, AgentError> {
        Self::start_inner(
            config, root, permission, timeout, events, model, false, peer, read_only,
        )
        .await
    }
    /// Start without Orochi's coordination tools (routing advisers).
    pub async fn start_isolated(
        config: AgentConfig,
        root: &Path,
        timeout: Duration,
        events: Option<EventSink>,
        model: Option<&str>,
    ) -> Result<Self, AgentError> {
        Self::start_inner(
            config,
            root,
            PermissionMode::Deny,
            timeout,
            events,
            model,
            true,
            None,
            false,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn start_inner(
        config: AgentConfig,
        root: &Path,
        permission: PermissionMode,
        timeout: Duration,
        events: Option<EventSink>,
        model: Option<&str>,
        isolated: bool,
        peer: Option<&str>,
        read_only: bool,
    ) -> Result<Self, AgentError> {
        let observations = Arc::new(Mutex::new(Observation::default()));
        let debug_state = observations.clone();
        let provider = config.provider;
        let (mut command, mut args) = (config.command.clone(), config.args.clone());
        #[cfg(unix)]
        let lease = crate::process::wrap(&mut command, &mut args);
        #[cfg(not(unix))]
        let lease = None;
        let agent = AcpAgent::new(
            AcpAgentConfig::new(command)
                .args(args)
                .envs(config.env.clone()),
        )
        .with_debug(move |line, direction| {
            if direction != LineDirection::Stdout {
                return;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                return;
            };
            let mut state = debug_state.lock().expect("observation mutex");
            if let Some(error) = value.get("error") {
                state.last_error = error.clone();
            }
            if let Some(result) = value.get("result") {
                if result.get("sessionId").is_some() {
                    state.session = result.clone();
                } else if result.get("configOptions").is_some() {
                    state.session["configOptions"] = result["configOptions"].clone();
                }
                let adapter = ProviderAdapter(provider);
                if let Some(usage) = adapter.get_usage(result) {
                    state.usage = usage;
                }
                if let Some(quota) = adapter.get_quota(result) {
                    state.quota = Some(quota);
                }
            }
        });
        let notification_events = events.clone();
        let permission_events = events.clone();
        let notification_state = observations.clone();
        let permission_state = observations.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let lifecycle = Lifecycle(Some(tokio::spawn(async move {
            let result = agent_client_protocol::Client
                .builder()
                .on_receive_notification(
                    async move |notification: SessionNotification, _cx| {
                        let value = serde_json::to_value(&notification).unwrap_or_default();
                        let mut state = notification_state.lock().expect("observation mutex");
                        if value["sessionId"] != state.session["sessionId"] {
                            return Ok(());
                        }
                        let update = &value["update"];
                        if update["sessionUpdate"] == "config_option_update" {
                            state.session["configOptions"] = update["configOptions"].clone();
                        }
                        if state.stream
                            && let Some(events) = &notification_events
                            && let Some(progress) = progress(update)
                        {
                            let _ = events.send(ExecutionEvent::Progress(progress));
                        }
                        if state.stream
                            && update["sessionUpdate"] == "agent_message_chunk"
                            && let Some(text) = update["content"]["text"].as_str()
                        {
                            if let Some(events) = &notification_events {
                                let _ = events.send(ExecutionEvent::Text(
                                    text.to_owned(),
                                    update["messageId"].as_str().map(str::to_owned),
                                ));
                            } else {
                                print!("{}", text.replace('\u{1b}', "\\x1b"));
                                let _ = std::io::stdout().flush();
                            }
                        }
                        if let Some(usage) = ProviderAdapter(provider).get_usage(update) {
                            state.usage = usage;
                        }
                        if let Some(quota) = ProviderAdapter(provider).get_quota(update) {
                            state.quota = Some(quota);
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        let value = serde_json::to_value(&request).unwrap_or_default();
                        let active = {
                            let state = permission_state.lock().expect("observation mutex");
                            state.stream && value["sessionId"] == state.session["sessionId"]
                        };
                        // A read-only session may look at anything and change nothing.
                        let writes = !matches!(
                            value["toolCall"]["kind"].as_str(),
                            Some("read" | "search" | "fetch" | "think") | None
                        );
                        let decision = if active {
                            if read_only && writes && !coordination_tool(&value) {
                                None
                            } else if coordination_tool(&value) {
                                allow_once_option(&value)
                            } else if let Some(events) = &permission_events
                                && permission == PermissionMode::Ask
                            {
                                let (tx, rx) = oneshot::channel();
                                if events
                                    .send(ExecutionEvent::Permission(value.clone(), tx))
                                    .is_ok()
                                {
                                    rx.await.ok().flatten()
                                } else {
                                    None
                                }
                            } else {
                                choose_permission(&value, permission).await
                            }
                        } else {
                            None
                        };
                        let outcome = if let Some(id) = decision {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id))
                        } else {
                            permission_state.lock().expect("observation mutex").denied = true;
                            RequestPermissionOutcome::Cancelled
                        };
                        responder.respond(RequestPermissionResponse::new(outcome))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
                    let _ = ready_tx.send(connection);
                    let _ = shutdown_rx.await;
                    Ok(())
                })
                .await;
            if let Err(error) = result {
                tracing::debug!(%error, "ACP connection ended");
            }
        })));
        let connection = match tokio::time::timeout(timeout, ready_rx).await {
            Ok(Ok(connection)) => connection,
            other => {
                if let Some(task) = lifecycle.0.as_ref() {
                    task.abort();
                }
                #[cfg(unix)]
                if let Some(lease) = &lease {
                    crate::process::release(lease);
                }
                return Err(AgentError::new(
                    ErrorKind::Unavailable,
                    format!("could not start {}: {other:?}", config.id),
                ));
            }
        };
        let mut client = Self {
            events,
            // Routing advisers get no tools beyond the agent's own.
            peer: (!isolated)
                .then(|| crate::mailbox::register_session(peer))
                .flatten(),
            discovery_model: model.map(str::to_owned),
            config,
            connection,
            observations,
            lifecycle,
            shutdown: Some(shutdown_tx),
            capabilities: Capabilities {
                session_id: String::new(),
                models: vec![],
                load_session: false,
                image: false,
                embedded: false,
                model_selector: None,
                legacy_models: false,
            },
            timeout,
            lease,
        };
        let initialized = client
            .request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("orochi", env!("CARGO_PKG_VERSION"))),
            )
            .await?;
        if initialized.protocol_version != ProtocolVersion::V1 {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent did not negotiate ACP v1",
            ));
        }
        let advertised = serde_json::to_value(initialized).unwrap_or_default();
        let agent_capabilities = &advertised["agentCapabilities"];
        client.capabilities.load_session =
            agent_capabilities["loadSession"].as_bool().unwrap_or(false);
        let prompt = &agent_capabilities["promptCapabilities"];
        client.capabilities.image = prompt["image"].as_bool().unwrap_or(false);
        client.capabilities.embedded = prompt["embeddedContext"].as_bool().unwrap_or(false);
        client.new_session(root).await?;
        Ok(client)
    }
    async fn request<R: JsonRpcRequest>(&self, request: R) -> Result<R::Response, AgentError> {
        self.observations
            .lock()
            .expect("observation mutex")
            .last_error = Value::Null;
        match tokio::time::timeout(
            self.timeout,
            self.connection.send_request(request).block_task(),
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(self.classify(&error.to_string())),
            Err(_) => Err(AgentError::new(ErrorKind::Timeout, "ACP request timed out")),
        }
    }
    fn classify(&self, error: &str) -> AgentError {
        ProviderAdapter(self.config.provider).classify_error(
            &self
                .observations
                .lock()
                .expect("observation mutex")
                .last_error,
            error,
            crate::types::now(),
        )
    }
    fn state(&self) -> Value {
        self.observations
            .lock()
            .expect("observation mutex")
            .session
            .clone()
    }
    pub fn quota(&self) -> Option<QuotaObservation> {
        self.observations
            .lock()
            .expect("observation mutex")
            .quota
            .clone()
    }
    pub fn usage(&self) -> Usage {
        self.observations
            .lock()
            .expect("observation mutex")
            .usage
            .clone()
    }
    pub fn current_model(&self) -> Option<String> {
        let state = self.state();
        selector(&state, "model")
            .map(|s| s.current)
            .or_else(|| state["models"]["currentModelId"].as_str().map(String::from))
    }
    pub fn current_reasoning(&self) -> Option<String> {
        selector(&self.state(), "thought_level").map(|s| s.current)
    }
    pub fn current_mode(&self) -> Option<String> {
        self.mode_selector().map(|s| s.current)
    }
    fn mode_selector(&self) -> Option<Selector> {
        let state = self.state();
        selector(&state, "mode").or_else(|| {
            let modes = &state["modes"];
            Some(Selector {
                id: "mode".into(),
                current: modes["currentModeId"].as_str()?.into(),
                values: modes["availableModes"]
                    .as_array()?
                    .iter()
                    .filter_map(|v| v["id"].as_str().map(String::from))
                    .collect(),
            })
        })
    }
    /// The mailbox server for this session's own peer, unless this is a routing adviser.
    fn coordination(&self) -> Vec<McpServer> {
        match self.peer.as_ref().and_then(crate::mailbox::server_for) {
            Some((name, command, args)) => vec![McpServer::Stdio(
                McpServerStdio::new(name, command).args(args),
            )],
            None => vec![],
        }
    }
    /// Coordination instructions naming this session's peer.
    pub fn peer_note(&self) -> Option<String> {
        self.peer.as_ref().and_then(crate::mailbox::prompt_note_for)
    }
    /// Joins the mailbox as this session and tells other agents what it is running.
    pub fn announce_route(&mut self, agent: &str, model: &str) {
        if let Some(peer) = &mut self.peer {
            crate::mailbox::announce_route(peer, agent, model);
        }
    }
    pub async fn new_session(&mut self, root: &Path) -> Result<(), AgentError> {
        let request = NewSessionRequest::new(root).mcp_servers(self.coordination());
        let session = self.request(request).await?;
        self.capabilities.session_id = session.session_id.to_string();
        // Raw response retained by the SDK observer also preserves legacy model fields.
        self.discover().await
    }
    pub async fn load_session(&mut self, session: &str, root: &Path) -> Result<(), AgentError> {
        if !self.capabilities.load_session {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent does not support session/load",
            ));
        }
        self.observations.lock().expect("observation mutex").session =
            json!({"sessionId": session});
        // A reloaded session needs the mailbox server again; it is not carried over.
        let response = self
            .request(
                LoadSessionRequest::new(session.to_owned(), root).mcp_servers(self.coordination()),
            )
            .await?;
        let mut state = serde_json::to_value(response).unwrap_or_default();
        state["sessionId"] = json!(session);
        self.observations.lock().expect("observation mutex").session = state;
        self.capabilities.session_id = session.into();
        self.discover().await
    }
    async fn set_config(&self, id: &str, value: &str) -> Result<(), AgentError> {
        let response = self
            .request(SetSessionConfigOptionRequest::new(
                self.capabilities.session_id.clone(),
                id.to_owned(),
                value,
            ))
            .await?;
        self.observations.lock().expect("observation mutex").session["configOptions"] =
            serde_json::to_value(response.config_options).unwrap_or_default();
        Ok(())
    }
    async fn set_model(&self, model: &str) -> Result<(), AgentError> {
        if model == "agent-default" {
            return Ok(());
        }
        if let Some(select) = selector(&self.state(), "model") {
            if !select.values.iter().any(|v| v == model) {
                return Err(AgentError::new(
                    ErrorKind::Configuration,
                    format!("model no longer advertised: {model}"),
                ));
            }
            if select.current != model {
                self.set_config(&select.id, model).await?;
            }
        } else if self.capabilities.legacy_models {
            self.request(LegacySetModel {
                session_id: self.capabilities.session_id.clone(),
                model_id: model.into(),
            })
            .await?;
            self.observations.lock().expect("observation mutex").session["models"]["currentModelId"] =
                json!(model);
        } else {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent has no model selector",
            ));
        }
        if self.current_model().as_deref() != Some(model) {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent did not apply selected model",
            ));
        }
        Ok(())
    }
    pub async fn use_model(&self, model: &str) -> Result<(), AgentError> {
        self.set_model(model).await
    }
    pub async fn use_reasoning(&self, level: &str) -> Result<(), AgentError> {
        let Some(select) = selector(&self.state(), "thought_level") else {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "selected model does not expose reasoning",
            ));
        };
        if !select.values.iter().any(|v| v == level) {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "reasoning is not supported by this model",
            ));
        }
        if select.current != level {
            self.set_config(&select.id, level).await?;
        }
        if self.current_reasoning().as_deref() != Some(level) {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent did not apply selected reasoning",
            ));
        }
        Ok(())
    }
    pub async fn discover(&mut self) -> Result<(), AgentError> {
        let state = self.state();
        let model_selector = selector(&state, "model");
        self.capabilities.legacy_models =
            model_selector.is_none() && state["models"]["availableModels"].is_array();
        let original = self.current_model();
        let models = if let Some(s) = &model_selector {
            s.values.clone()
        } else if self.capabilities.legacy_models {
            state["models"]["availableModels"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v["modelId"].as_str().map(String::from))
                .collect()
        } else {
            vec!["agent-default".into()]
        };
        if models.len() > 128 {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent advertised too many models",
            ));
        }
        let mut discovered = Vec::new();
        for model in models
            .into_iter()
            .filter(|m| {
                self.discovery_model
                    .as_ref()
                    .is_none_or(|wanted| wanted == m)
            })
            .collect::<Vec<_>>()
        {
            // ACP configuration is model-dependent: inspect every model's returned selectors.
            if let Err(error) = self.set_model(&model).await {
                if error.kind == ErrorKind::Configuration
                    || (self.discovery_model.is_none()
                        && error.model_scoped
                        && error.kind == ErrorKind::RateLimit)
                {
                    continue;
                }
                return Err(error);
            }
            discovered.push(ModelCapabilities {
                model,
                reasoning: selector(&self.state(), "thought_level"),
                modes: self.mode_selector(),
            });
        }
        if let Some(original) = original
            .filter(|m| self.discovery_model.is_none() && discovered.iter().any(|d| &d.model == m))
        {
            self.set_model(&original).await?;
        }
        self.capabilities.model_selector = model_selector;
        self.capabilities.models = discovered;
        Ok(())
    }
    pub async fn configure(&self, candidate: &ExecutionCandidate) -> Result<(), AgentError> {
        self.set_model(&candidate.model).await?;
        if let Some(mode) = &candidate.mode {
            if let Some(select) = selector(&self.state(), "mode") {
                if !select.values.contains(mode) {
                    return Err(AgentError::new(
                        ErrorKind::Configuration,
                        "mode is no longer available",
                    ));
                }
                if select.current != *mode {
                    self.set_config(&select.id, mode).await?;
                }
            } else if let Some(select) = self.mode_selector() {
                if !select.values.contains(mode) {
                    return Err(AgentError::new(
                        ErrorKind::Configuration,
                        "unknown session mode",
                    ));
                }
                if select.current != *mode {
                    let response = self
                        .request(SetSessionModeRequest::new(
                            self.capabilities.session_id.clone(),
                            mode.clone(),
                        ))
                        .await?;
                    let _ = response;
                    self.observations.lock().expect("observation mutex").session["modes"]["currentModeId"] =
                        json!(mode);
                }
            } else {
                return Err(AgentError::new(
                    ErrorKind::Configuration,
                    "agent does not expose session modes",
                ));
            }
        }
        if let Some(reasoning) = &candidate.reasoning_level {
            let Some(select) = selector(&self.state(), "thought_level") else {
                return Err(AgentError::new(
                    ErrorKind::Configuration,
                    "selected model does not expose reasoning",
                ));
            };
            if !select.values.contains(reasoning) {
                return Err(AgentError::new(
                    ErrorKind::Configuration,
                    "reasoning is no longer supported by this model",
                ));
            }
            if select.current != *reasoning {
                self.set_config(&select.id, reasoning).await?;
            }
        }
        if candidate.model != "agent-default"
            && self.current_model().as_deref() != Some(&candidate.model)
            || self.current_reasoning() != candidate.reasoning_level
            || self.current_mode() != candidate.mode
        {
            return Err(AgentError::new(
                ErrorKind::Configuration,
                "agent changed dependent configuration; route must be rebuilt",
            ));
        }
        Ok(())
    }
    /// Session modes the agent offers for the model in use (its own approval modes).
    pub fn modes(&self) -> Vec<String> {
        self.mode_selector().map(|s| s.values).unwrap_or_default()
    }

    pub async fn prompt(&mut self, prompt: String, timeout: Duration) -> Result<bool, AgentError> {
        self.prompt_with(prompt, &[], timeout).await
    }

    /// Sends the prompt with the files it carries: images inline when the agent takes them,
    /// text embedded when it takes embedded context, and a resource link otherwise.
    pub async fn prompt_with(
        &mut self,
        prompt: String,
        attachments: &[crate::types::Attachment],
        timeout: Duration,
    ) -> Result<bool, AgentError> {
        {
            let mut o = self.observations.lock().expect("observation mutex");
            o.stream = true;
            o.denied = false;
            o.usage = Usage::default();
            o.last_error = Value::Null;
        }
        let mut blocks = vec![ContentBlock::Text(TextContent::new(prompt))];
        for attachment in attachments {
            blocks.push(self.attachment_block(attachment));
        }
        let request = PromptRequest::new(self.capabilities.session_id.clone(), blocks);
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, self.connection.send_request(request).block_task()) => match result {
                Ok(Ok(response)) if response.stop_reason == StopReason::Cancelled => Err(AgentError::new(ErrorKind::Cancelled, "agent cancelled the turn")),
                Ok(Ok(response)) => Ok(response.stop_reason == StopReason::EndTurn),
                Ok(Err(error)) => Err(self.classify(&error.to_string())),
                Err(_) => { self.cancel(); Err(AgentError::new(ErrorKind::Timeout, "agent prompt timed out")) }
            },
            _ = tokio::signal::ctrl_c() => { self.cancel(); Err(AgentError::new(ErrorKind::Cancelled, "interrupted")) }
        };
        let denied = {
            let mut state = self.observations.lock().expect("observation mutex");
            state.stream = false;
            state.denied
        };
        if let Some(events) = &self.events {
            // Let a consumer polling in this task close the stream before callers report on it.
            if events.send(ExecutionEvent::Finished).is_ok() {
                tokio::task::yield_now().await;
            }
        } else {
            println!();
        }
        if denied && result.is_ok() {
            return Err(AgentError::new(
                ErrorKind::Cancelled,
                "permission denied; execution stopped",
            ));
        }
        result
    }
    fn attachment_block(&self, attachment: &crate::types::Attachment) -> ContentBlock {
        let uri = format!("file://{}", attachment.path.display());
        let link =
            || ContentBlock::ResourceLink(ResourceLink::new(attachment.name.clone(), uri.clone()));
        if attachment.bytes > MAX_ATTACHMENT_BYTES {
            return link();
        }
        let mime = attachment
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match (attachment.image, self.capabilities.image) {
            (true, true) => match std::fs::read(&attachment.path) {
                Ok(bytes) => ContentBlock::Image(ImageContent::new(
                    base64(&bytes),
                    match mime.as_deref() {
                        Some("png") => "image/png",
                        Some("gif") => "image/gif",
                        Some("webp") => "image/webp",
                        Some("bmp") => "image/bmp",
                        _ => "image/jpeg",
                    },
                )),
                Err(_) => link(),
            },
            (true, false) => link(),
            (false, _) if self.capabilities.embedded => match std::fs::read(&attachment.path) {
                // Only text is embedded; anything else is a link the agent can open itself.
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => ContentBlock::Resource(EmbeddedResource::new(
                        EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(
                            text, uri,
                        )),
                    )),
                    Err(_) => link(),
                },
                Err(_) => link(),
            },
            (false, _) => link(),
        }
    }

    pub fn cancel(&self) {
        let _ = self.connection.send_notification(CancelNotification::new(
            self.capabilities.session_id.clone(),
        ));
    }
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.lifecycle.0.take() {
            task.abort();
            let _ = task.await;
        }
        self.release();
    }
    fn release(&mut self) {
        #[cfg(unix)]
        if let Some(lease) = self.lease.take() {
            crate::process::release(&lease);
        }
    }
}

/// Only a one-time grant is ever selected, never an "always allow" option.
pub(crate) fn allow_once_option(request: &Value) -> Option<String> {
    request["options"]
        .as_array()?
        .iter()
        .find(|v| v["kind"] == "allow_once")?["optionId"]
        .as_str()
        .map(String::from)
}

/// Orochi's own mailbox tools only talk to other agents: they touch nothing to approve.
pub(crate) fn coordination_tool(request: &Value) -> bool {
    let title = request["toolCall"]["title"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    title.contains("orochi-mailbox") || title.contains("orochi_mailbox")
}

pub(crate) fn describe_permission(request: &Value) -> String {
    serde_json::to_string_pretty(&request["toolCall"])
        .unwrap_or_default()
        .replace('\u{1b}', "\\x1b")
}

const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let triple = chunk.iter().enumerate().fold(0u32, |value, (index, byte)| {
            value | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[(triple >> (18 - 6 * index) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// How a run with nobody listening answers a permission request. The conversation store sits
/// in the path of every event now, so it answers this way too rather than swallowing the
/// question: putting a recorder between an agent and its user must not change who decides.
pub(crate) async fn choose_permission(request: &Value, mode: PermissionMode) -> Option<String> {
    let id = allow_once_option(request)?;
    match mode {
        PermissionMode::Allow => Some(id),
        PermissionMode::Deny => None,
        PermissionMode::Ask => {
            if !std::io::stdin().is_terminal() {
                eprintln!(
                    "permission requested without a terminal; use --permission allow or deny explicitly"
                );
                return None;
            }
            eprintln!("\nPermission request:\n{}", describe_permission(request));
            eprint!("Allow once? [y/N] ");
            let _ = std::io::stderr().flush();
            let (tx, rx) = oneshot::channel();
            // Detached input thread avoids making Tokio shutdown wait on a blocked stdin read.
            std::thread::spawn(move || {
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                let _ = tx.send(input);
            });
            let input = rx.await.ok()?;
            matches!(input.trim().to_lowercase().as_str(), "y" | "yes").then_some(id)
        }
    }
}
