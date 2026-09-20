//! Routing advisers: coding agents asked over ACP to choose one supplied candidate ID.
//! Each session starts in an empty temporary directory with every permission denied.
use crate::{
    acp::{Client, ExecutionEvent},
    agents::{AgentError, ErrorKind},
    config::{AgentConfig, Config, LearningStrategy, RouterConfig},
    policy::Registry,
    router::scorer::{self, ScoringContext},
    storage::Store,
    types::{Complexity, ExecutionCandidate, Overrides, TaskDescriptor, Usage, now},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::Path, time::Duration};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

const INSTRUCTION: &str = "You are a routing adviser, not a coding agent. Do not read files, run commands or use any tool. Choose one supplied candidate to minimize expected resources to successful completion. Reply ONLY with a JSON object: {\"candidate_id\":\"<supplied id>\"}.";
const MAX_REPLY: usize = 65_536;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advice {
    pub candidate_id: String,
}

/// Only classification features and opaque candidates: no task text, filenames or repository paths.
pub fn request(task: &TaskDescriptor, candidates: &[ExecutionCandidate]) -> String {
    let descriptor = json!({"task_type": task.task_type, "language": task.language, "framework": task.framework,
        "scope": task.estimated_scope, "context_size": task.estimated_context, "complexity": task.complexity,
        "ambiguity": task.ambiguity, "long_horizon": task.long_horizon, "tests_available": task.tests_available});
    let choices: Vec<_> = candidates.iter().take(12).map(|c| json!({"id":c.id,"agent":c.agent,"model":c.model,"reasoning":c.reasoning_level,"success":c.success_probability,"cost":c.expected_cost})).collect();
    format!(
        "{INSTRUCTION}\n\n{}",
        json!({"task": descriptor, "candidates": choices})
    )
}

pub fn review(votes: &[String]) -> String {
    json!({"peer_candidate_votes": votes, "instruction": "Review these peer votes independently. Keep or revise your candidate choice; reply with the same JSON schema."}).to_string()
}

/// Conservative tokens for one turn: text as UTF-8 bytes (not assuming English
/// tokenization), the agent's session overhead and the expected reply.
pub fn turn_estimate(config: &RouterConfig, text: usize) -> usize {
    text + 128 + config.overhead() + config.max_output_tokens as usize
}

pub fn parse(reply: &str, candidates: &[ExecutionCandidate]) -> Option<Advice> {
    crate::context::trailing_json::<Advice>(reply)
        .filter(|a| candidates.iter().take(12).any(|c| c.id == a.candidate_id))
}

/// What an adviser session needs to launch its agent and choose a model for its role.
pub struct Selection<'a> {
    pub config: &'a Config,
    pub policies: &'a Registry,
    pub store: &'a Store,
    /// Difficulty of the routing decision, used only to choose a default model.
    pub complexity: Complexity,
}

/// Without an explicit model, score the agent's advertised models like execution candidates
/// for a routing decision of the seat's difficulty: the provider policy's success floor, hard
/// constraints, quota and cost apply. Coding history is ignored (static estimates).
fn choose(
    client: &Client,
    agent: &AgentConfig,
    choice: &RouterConfig,
    selection: &Selection<'_>,
    root: &Path,
) -> Result<(String, Option<String>)> {
    let mut scoring = selection.config.clone();
    scoring.learning.strategy = LearningStrategy::Static;
    let task = TaskDescriptor {
        task_type: "routing".into(),
        language: "none".into(),
        framework: None,
        repo_size: 0,
        candidate_files: vec![],
        estimated_scope: 1,
        estimated_context: 2000,
        complexity: selection.complexity,
        requires_architecture_change: false,
        requires_browser: false,
        requires_web: false,
        requires_image: false,
        collaborative: false,
        seats: None,
        tests_available: false,
        ambiguity: 0.0,
        long_horizon: false,
        preferred: None,
    };
    let overrides = Overrides {
        reasoning: choice.reasoning.clone(),
        ..Overrides::default()
    };
    let runtime = selection.store.runtime()?;
    let ranked = scorer::candidates(
        &[(agent, &client.capabilities)],
        &ScoringContext {
            task: &task,
            overrides: &overrides,
            config: &scoring,
            policies: selection.policies,
            store: selection.store,
            runtime: &runtime,
            sessions: &[],
            root,
            time: now(),
            busy: &[],
            taken: &[],
        },
    )?;
    let models = &client.capabilities.models;
    match ranked.into_iter().next() {
        Some(best) => Ok((best.model, best.reasoning_level)),
        // A single advertised model leaves nothing to choose.
        None if models.len() == 1 => Ok((models[0].model.clone(), choice.reasoning.clone())),
        None => Err(AgentError::new(
            ErrorKind::Configuration,
            "no eligible available model for routing advice; set `model`",
        )
        .into()),
    }
}

pub struct Session {
    client: Client,
    events: UnboundedReceiver<ExecutionEvent>,
    timeout: Duration,
    _workspace: tempfile::TempDir,
}

impl Session {
    pub async fn start(config: &RouterConfig, selection: &Selection<'_>) -> Result<Self> {
        let agent = selection
            .config
            .agents
            .iter()
            .find(|a| a.id == config.agent && a.enabled)
            .with_context(|| format!("unknown or disabled adviser agent: {}", config.agent))?;
        let timeout = Duration::from_secs(config.timeout_secs);
        let start = async {
            let launch = crate::discovery::prepare(
                agent,
                &selection.config.discovery,
                selection.store.data_dir(),
            )
            .await?;
            let workspace = tempfile::tempdir()?;
            let (tx, events) = unbounded_channel();
            let client = Client::start_isolated(
                launch,
                workspace.path(),
                timeout,
                Some(tx),
                config.model.as_deref(),
            )
            .await?;
            let (model, reasoning) = match &config.model {
                Some(model) => (model.clone(), config.reasoning.clone()),
                None => choose(&client, agent, config, selection, workspace.path())?,
            };
            client.use_model(&model).await?;
            if let Some(level) = &reasoning {
                client.use_reasoning(level).await?;
            }
            anyhow::Ok(Self {
                client,
                events,
                timeout,
                _workspace: workspace,
            })
        };
        tokio::time::timeout(timeout, start)
            .await
            .map_err(|_| AgentError::new(ErrorKind::Timeout, "adviser session start timed out"))?
    }

    /// One turn. Usage is returned even when the turn fails.
    pub async fn ask(&mut self, text: String) -> (Result<String>, Usage) {
        let mut reply = String::new();
        let result = {
            let prompt = self.client.prompt(text, self.timeout);
            tokio::pin!(prompt);
            loop {
                tokio::select! {
                    done = &mut prompt => break done,
                    Some(event) = self.events.recv() => collect(event, &mut reply),
                }
            }
        };
        while let Ok(event) = self.events.try_recv() {
            collect(event, &mut reply);
        }
        let usage = self.client.usage();
        let result = match result {
            Ok(true) => Ok(reply),
            Ok(false) => {
                Err(AgentError::new(ErrorKind::Other, "adviser did not finish its turn").into())
            }
            Err(error) => Err(error.into()),
        };
        (result, usage)
    }

    /// The model actually in use, as reported by the agent.
    pub fn model(&self) -> String {
        self.client
            .current_model()
            .unwrap_or_else(|| "agent-default".into())
    }
    pub fn reasoning(&self) -> Option<String> {
        self.client.current_reasoning()
    }

    pub async fn stop(mut self) {
        self.client.stop().await;
    }
}

fn collect(event: ExecutionEvent, reply: &mut String) {
    match event {
        ExecutionEvent::Text(chunk, _) if reply.len() + chunk.len() <= MAX_REPLY => {
            reply.push_str(&chunk)
        }
        ExecutionEvent::Text(..) | ExecutionEvent::Finished | ExecutionEvent::Progress(_) => {}
        ExecutionEvent::Permission(_, answer) => {
            let _ = answer.send(None);
        }
    }
}
