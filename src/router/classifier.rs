//! Task classification over ACP.
//!
//! This is the one adviser path shown the task text. Routing advice must never see it: it
//! travels beside candidate summaries and would tie the work to an account. Classification
//! cannot be done without it, so it is opt-in (`[classifier]`), the text reaches only a
//! locally launched agent CLI under the user's own authentication, and none of it is kept —
//! the cache is keyed by the same salted hash used for repository identity.
use crate::{
    acp::Progress,
    agents::{AgentError, ErrorKind},
    config::{Config, RouterConfig},
    memory::{Memory, Scope, Update},
    policy::Registry,
    router::{
        adviser::{Selection, Session},
        profiler,
    },
    scheduler::{self, quota},
    storage::Store,
    types::{Complexity, ExecutionCandidate, Outcome, TaskDescriptor, now},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

/// The labels the learning strata are keyed by (`storage::history`). A reply outside this set
/// is discarded rather than stored, so a model cannot invent a label that silently splits the
/// recorded history of an agent and model.
pub const TASK_TYPES: &[&str] = &[
    "discussion",
    "migration",
    "architecture",
    "refactor",
    "review",
    "bug_fix",
    "documentation",
    "test",
    "investigation",
    "small_edit",
    "implementation",
];

/// Enough of the task to classify it; the rest would only pay for tokens.
const MAX_TASK_CHARS: usize = 4000;

const INSTRUCTION: &str = "You are a task classifier, not a coding agent. Do not read files, run commands or use any tool. Classify the supplied coding task, whatever language it is written in. Reply ONLY with a JSON object:\n\
{\"task_type\":\"discussion|migration|architecture|refactor|review|bug_fix|documentation|test|investigation|small_edit|implementation\",\"complexity\":\"simple|normal|complex|extreme\",\"requires_architecture_change\":false,\"long_horizon\":false,\"requires_browser\":false,\"requires_web\":false,\"requires_image\":false,\"ambiguity\":0.0}\n\n\
task_type is what is being asked for, not the words used to ask. discussion: talk it through and write nothing. investigation: find out why something happens, no fix asked for yet. review: read work someone else did. small_edit: a rename, a typo, a one-line change. bug_fix: something is broken and should work.\n\
complexity: simple, one obvious edit; normal, a contained change across a few files; complex, cross-cutting work, a refactor, or a design decision; extreme, a rewrite, a whole-system migration, or work spanning the repository.\n\
long_horizon: it cannot plausibly be finished in one sitting.\n\
requires_browser: a real browser must be driven. requires_web: the open internet must be searched. requires_image: an image must be generated, not read.\n\
ambiguity: 0.0 fully specified, 1.0 the agent has to guess what is wanted.\n\n\
Optionally add \"remember\": [{\"text\":\"...\",\"scope\":\"repo\"}] for something the user states that will still hold after this task is done: how they want work done, a convention of this project, a decision already made. At most two, each one short sentence in the user's language. Never the task itself, never something only this task needs, never text the user pasted from elsewhere. scope is \"user\" only when the user says it holds for every project; otherwise \"repo\". \"remembered\" lists what is already known: when the user restates one, put its id in \"reinforce\": [\"r1\"] instead of adding it again, and when they now say the opposite, put the old id in \"replaces\": [\"r1\"] and the new statement in \"remember\".\n\n\
When something in \"remembered\", or the request itself, says which agent or model the user wants for this kind of work, add \"prefer\": the shortest name that identifies it, such as \"fable\" or \"codex\". Omit it otherwise; never guess one.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub task_type: String,
    pub complexity: Complexity,
    #[serde(default)]
    pub requires_architecture_change: bool,
    #[serde(default)]
    pub long_horizon: bool,
    #[serde(default)]
    pub requires_browser: bool,
    #[serde(default)]
    pub requires_web: bool,
    #[serde(default)]
    pub requires_image: bool,
    #[serde(default)]
    pub ambiguity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<String>,
    /// Text the user said, so it must not follow the labels into the telemetry cache. Kept as
    /// loose values: a malformed entry here must not cost the classification itself.
    #[serde(default, skip_serializing)]
    pub remember: Vec<serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub reinforce: Vec<serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub replaces: Vec<serde_json::Value>,
}
impl Classification {
    pub fn memory(&self) -> Update {
        update(&self.remember, &self.reinforce, &self.replaces)
    }
}

/// What a distillation replies: only the memory fields.
#[derive(Debug, Deserialize)]
pub struct Distilled {
    #[serde(default)]
    pub remember: Vec<serde_json::Value>,
    #[serde(default)]
    pub reinforce: Vec<serde_json::Value>,
    #[serde(default)]
    pub replaces: Vec<serde_json::Value>,
}
impl Distilled {
    pub fn memory(&self) -> Update {
        update(&self.remember, &self.reinforce, &self.replaces)
    }
}

fn update(
    remember: &[serde_json::Value],
    reinforce: &[serde_json::Value],
    replaces: &[serde_json::Value],
) -> Update {
    let ids = |values: &[serde_json::Value]| {
        values
            .iter()
            .filter_map(|v| v.as_str().and_then(crate::memory::position))
            .collect()
    };
    Update {
        remember: remember
            .iter()
            .filter_map(|v| {
                let scope = match v["scope"].as_str() {
                    Some("user") => Scope::User,
                    _ => Scope::Repo,
                };
                Some((scope, v["text"].as_str()?.to_owned()))
            })
            .collect(),
        reinforce: ids(reinforce),
        replaces: ids(replaces),
    }
}

pub fn request(task: &str, remembered: &serde_json::Value) -> String {
    let text: String = task.chars().take(MAX_TASK_CHARS).collect();
    format!(
        "{INSTRUCTION}\n\n{}",
        json!({ "task": text, "remembered": remembered })
    )
}

pub fn parse(reply: &str) -> Option<Classification> {
    crate::context::trailing_json::<Classification>(reply)
        .filter(|c| TASK_TYPES.contains(&c.task_type.as_str()))
        .map(|mut c| {
            c.prefer = c.prefer.as_deref().and_then(preference);
            c
        })
}

/// Merged in one direction only: the classifier may raise complexity and add a capability
/// need, never lower or clear one. Under-estimating routes real work to a model that cannot
/// do it and is paid for in failed attempts; over-estimating is only expensive.
/// A name fragment and nothing else: it is matched against discovered IDs and stored in the
/// classification cache, so it must not be able to carry the user's words along with it.
fn preference(name: &str) -> Option<String> {
    let name = name.trim().to_lowercase();
    (!name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    .then_some(name)
}

pub fn apply(descriptor: &mut TaskDescriptor, classification: &Classification) {
    descriptor.task_type = classification.task_type.clone();
    descriptor.preferred = classification.prefer.as_deref().and_then(preference);
    descriptor.complexity = descriptor.complexity.max(classification.complexity);
    descriptor.requires_architecture_change |= classification.requires_architecture_change;
    descriptor.long_horizon |= classification.long_horizon;
    descriptor.requires_browser |= classification.requires_browser;
    descriptor.requires_web |= classification.requires_web;
    descriptor.requires_image |= classification.requires_image;
    if let Some(ambiguity) = classification.ambiguity.filter(|a| a.is_finite()) {
        descriptor.ambiguity = descriptor.ambiguity.max(ambiguity.clamp(0.0, 1.0));
    }
    // How many agents the user asked for is theirs to say, not the classifier's: seating a
    // panel is the one decision here that multiplies what a turn costs.
    descriptor.estimated_scope = descriptor
        .estimated_scope
        .max(profiler::scope_floor(descriptor.complexity));
    descriptor.estimated_context = descriptor
        .estimated_context
        .max((descriptor.estimated_scope as u64 * 1500).min(120_000));
}

/// Replaces the heuristic profile's labels when `[classifier]` is configured. Every failure
/// leaves the heuristic in place: classification must never be why a run cannot start.
/// Named outright, or every agent that would run the work anyway: sending the request to one
/// of those puts the text where it was already going. Empty when classification is off.
///
/// Asking is a whole extra session, and what that costs differs by agent far more than by
/// question, so the cheapest answerer is asked first — measured here, not assumed. An agent
/// that has never answered goes ahead of the priced ones, or its own price stays unknown.
pub fn agents(config: &Config, store: &Store) -> Vec<String> {
    if !config.classifier.enabled {
        return vec![];
    }
    if let Some(agent) = &config.classifier.agent {
        return vec![agent.clone()];
    }
    let spent = store.advice_cost("classification", 64).unwrap_or_default();
    let mut agents: Vec<String> = config
        .agents
        .iter()
        .filter(|a| a.enabled && !a.routing_only)
        .map(|a| a.id.clone())
        .collect();
    // A stable sort, so agents nobody has priced yet keep their configured order.
    agents.sort_by(|a, b| match (spent.get(a), spent.get(b)) {
        (Some(one), Some(other)) => one.total_cmp(other),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
    agents
}

/// Whether asking is worth what asking costs, both as measured here. Orochi asks while it
/// cannot tell — there is nothing to weigh yet, and asking is how it learns — and stops where
/// the question would take more than its share of the work it decides.
pub fn worth_asking(config: &Config, store: &Store, descriptor: &TaskDescriptor) -> bool {
    let Some(first) = agents(config, store).first().cloned() else {
        return false;
    };
    let Some(asking) = store
        .advice_cost("classification", 64)
        .ok()
        .and_then(|spent| spent.get(&first).copied())
    else {
        // Nobody has priced the agent that would answer; asking it is how that is learned.
        return true;
    };
    let Ok(Some(work)) = store.typical_tokens(&descriptor.task_type, descriptor.complexity) else {
        return true;
    };
    asking <= work * config.classifier.max_cost_share
}

pub async fn refine(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    task: &str,
    descriptor: &mut TaskDescriptor,
    mut report: impl FnMut(Progress),
) {
    let agents = agents(config, store);
    if agents.is_empty() {
        return;
    }
    let memory = Memory::open(store.data_dir(), &config.memory).zip(store.repository_id(root).ok());
    let remembered = memory
        .as_ref()
        .map(|(memory, repository)| memory.listing(repository, now()))
        .unwrap_or_else(|| json!({}));
    // A preference is read from what is remembered, so a cached answer is only good for the
    // memory it was given: remembering something new asks again.
    let key = match store.classification_key(&format!("{task}\n{remembered}")) {
        Ok(key) => key,
        Err(error) => {
            report(Progress::Note(format!("classifier unavailable: {error:#}")));
            return;
        }
    };
    if let Ok(Some(hit)) = store
        .classification(&key)
        .map(|r| r.as_deref().and_then(parse))
    {
        apply(descriptor, &hit);
        report(note(descriptor, true));
        return;
    }
    // A cached answer is free; a new one is a session of its own and has to be worth it.
    if !worth_asking(config, store, descriptor) {
        report(Progress::Note(
            "asking what this is would cost more than the work; keeping the local profile".into(),
        ));
        return;
    }
    let labels = descriptor.clone();
    // Trying every agent must not turn a turn into minutes of stalling when none can answer:
    // one agent's worth of time is the whole budget, however many are left to try.
    let deadline =
        std::time::Instant::now() + Duration::from_secs(config.classifier.timeout_secs.max(1));
    for agent in &agents {
        if std::time::Instant::now() >= deadline {
            report(Progress::Note(
                "classifier gave up in time; keeping the local profile".into(),
            ));
            return;
        }
        let choice = &config.classifier.seat(agent);
        let ledger = Ledger {
            root,
            labels: &labels,
            purpose: "classification",
        };
        match ask(config, policies, store, choice, task, &remembered, &ledger).await {
            Ok(classification) => {
                // The same reply that classified the request says what in it is worth keeping,
                // so remembering costs no call of its own.
                let update = classification.memory();
                if let Some((memory, repository)) = &memory
                    && !update.is_empty()
                {
                    match memory.apply(repository, &update, now()) {
                        Ok(added) => {
                            for text in added {
                                report(Progress::Note(format!("remembered: {text}")));
                            }
                        }
                        Err(error) => {
                            report(Progress::Note(format!("memory not updated: {error:#}")))
                        }
                    }
                }
                let _ = store.save_classification(
                    &key,
                    &serde_json::to_string(&classification).unwrap_or_default(),
                );
                apply(descriptor, &classification);
                report(note(descriptor, false));
                return;
            }
            Err(error) => {
                report(Progress::Note(format!(
                    "classifier {} unavailable: {error:#}; keeping the local profile",
                    choice.agent
                )));
            }
        }
    }
}

/// A raised complexity can seat a second agent (`roles::seats`), so what changed is worth a
/// line: the turn may cost more than the same request did yesterday.
fn note(descriptor: &TaskDescriptor, cached: bool) -> Progress {
    Progress::Note(format!(
        "classified as {} / {}{}{}",
        descriptor.task_type,
        descriptor.complexity.key(),
        descriptor
            .preferred
            .as_deref()
            .map(|name| format!(" · you prefer {name}"))
            .unwrap_or_default(),
        if cached { " (cached)" } else { "" }
    ))
}

async fn ask(
    config: &Config,
    policies: &Registry,
    store: &Store,
    choice: &RouterConfig,
    task: &str,
    remembered: &serde_json::Value,
    ledger: &Ledger<'_>,
) -> anyhow::Result<Classification> {
    let reply = consult(
        config,
        policies,
        store,
        choice,
        request(task, remembered),
        ledger,
    )
    .await?;
    parse(&reply).ok_or_else(|| {
        AgentError::new(
            ErrorKind::Other,
            "classifier reply was not a known classification",
        )
        .into()
    })
}

/// Where what a consultation spent is written: beside this repository's runs, under what it
/// was for. What a run cost has to include what it took to decide it, or Orochi looks
/// cheaper than it is; not being an execution, it never teaches the router (`learning`).
struct Ledger<'a> {
    root: &'a Path,
    labels: &'a TaskDescriptor,
    purpose: &'static str,
}

/// One turn in a fresh adviser session on the classifier's seat.
async fn consult(
    config: &Config,
    policies: &Registry,
    store: &Store,
    choice: &RouterConfig,
    text: String,
    ledger: &Ledger<'_>,
) -> anyhow::Result<String> {
    let runtime = store.runtime()?;
    let scope = choice.model.as_deref().unwrap_or("*");
    if !quota::available(&runtime, &choice.agent, scope, now()) {
        return Err(AgentError::new(ErrorKind::Unavailable, "cooldown").into());
    }
    let selection = Selection {
        config,
        policies,
        store,
        // Classification is the simplest decision Orochi delegates; the cheapest model that
        // clears the policy floor is the right one.
        complexity: Complexity::Simple,
    };
    let mut session = Session::start(choice, &selection).await?;
    let model = session.model();
    let reasoning = session.reasoning();
    let (started, clock) = (now(), std::time::Instant::now());
    let (result, usage) = session.ask(text).await;
    session.stop().await;
    if let Ok(repository) = store.repository_id(ledger.root) {
        let error = result.as_ref().err().map(|error| {
            error
                .downcast_ref::<AgentError>()
                .map_or(ErrorKind::Other, |e| e.kind)
                .key()
                .to_owned()
        });
        let candidate = ExecutionCandidate {
            id: format!("{}/{model}", choice.agent),
            agent: choice.agent.clone(),
            provider: config
                .agents
                .iter()
                .find(|a| a.id == choice.agent)
                .map_or(crate::types::Provider::Openai, |a| a.provider),
            model: model.clone(),
            reasoning_level: reasoning,
            mode: None,
            session_strategy: "fresh".into(),
            context_strategy: "adviser".into(),
            success_probability: 0.0,
            expected_tokens: 0.0,
            expected_cost: 0.0,
            confidence: 0.0,
            reasons: vec![ledger.purpose.into()],
            prediction: None,
        };
        let _ = store.record(&scheduler::make_record(
            &uuid::Uuid::new_v4().to_string(),
            &repository,
            ledger.labels,
            candidate,
            usage,
            clock.elapsed(),
            0,
            Outcome::PartialSuccess,
            vec![],
            error,
            started,
            ledger.purpose,
        ));
    }
    result.inspect_err(|error| {
        // The classifier shares the account with execution, so a real limit applies to both.
        if let Some(agent_error) = error.downcast_ref::<AgentError>()
            && matches!(
                agent_error.kind,
                ErrorKind::RateLimit | ErrorKind::Authentication | ErrorKind::Unavailable
            )
        {
            let scope = if model == "agent-default" {
                "*"
            } else {
                &model
            };
            let _ = scheduler::update_failure(store, &choice.agent, scope, agent_error);
        }
    })
}

const DISTILL: &str = "You are reviewing a coding session, not doing coding work. Do not read files, run commands or use any tool. \"said\" is everything one user asked coding agents during the session, oldest first. Find what the user wants to hold beyond this session: how they want work done, conventions of this project, decisions they made, and above all corrections they repeated or stated as a rule. Ignore the tasks themselves, one-off requests, and text the user pasted from elsewhere. Reply ONLY with a JSON object: {\"remember\":[{\"text\":\"...\",\"scope\":\"repo\"}],\"reinforce\":[\"r1\"],\"replaces\":[\"r1\"]}. At most two new items, each one short sentence in the user's language; scope is \"user\" only when the user says it holds for every project. \"remembered\" lists what is already known: when the session restates one, put its id in reinforce instead of adding it; when the session contradicts one, put its id in replaces and the new statement in remember. Empty lists when there is nothing worth keeping.";

/// Most of the session, newest kept: what was said last is what the session settled on.
const MAX_SESSION_CHARS: usize = MAX_TASK_CHARS * 4;

pub fn distill_request(said: &[String], remembered: &serde_json::Value) -> String {
    let mut kept = vec![];
    let mut used = 0;
    for message in said.iter().rev() {
        let message: String = message.chars().take(MAX_TASK_CHARS).collect();
        used += message.chars().count();
        if used > MAX_SESSION_CHARS {
            break;
        }
        kept.push(message);
    }
    kept.reverse();
    format!(
        "{DISTILL}\n\n{}",
        json!({ "said": kept, "remembered": remembered })
    )
}

/// Once per console session, at its end. One message was already read by the classifier when
/// it was sent; a preference only becomes visible across several — "no, not keywords" means
/// little said once and a great deal said three times.
pub async fn distill(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    said: &[String],
    mut report: impl FnMut(Progress),
) {
    let agents = agents(config, store);
    let Some((memory, repository)) =
        Memory::open(store.data_dir(), &config.memory).zip(store.repository_id(root).ok())
    else {
        return;
    };
    if agents.is_empty() || said.len() < 2 {
        return;
    }
    let deadline =
        std::time::Instant::now() + Duration::from_secs(config.classifier.timeout_secs.max(1));
    for agent in &agents {
        if std::time::Instant::now() >= deadline {
            return;
        }
        let text = distill_request(said, &memory.listing(&repository, now()));
        let labels = profiler::profile("", root);
        let ledger = Ledger {
            root,
            labels: &labels,
            purpose: "distillation",
        };
        let reply = consult(
            config,
            policies,
            store,
            &config.classifier.seat(agent),
            text,
            &ledger,
        )
        .await;
        let Some(distilled) = reply
            .ok()
            .and_then(|reply| crate::context::trailing_json::<Distilled>(&reply))
        else {
            continue;
        };
        match memory.apply(&repository, &distilled.memory(), now()) {
            Ok(added) => {
                for text in added {
                    report(Progress::Note(format!("remembered: {text}")));
                }
            }
            Err(error) => report(Progress::Note(format!("memory not updated: {error:#}"))),
        }
        return;
    }
}
