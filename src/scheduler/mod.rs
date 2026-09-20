pub mod quota;

use crate::{
    acp::{Client, ExecutionEvent, Progress},
    agents::{AgentError, ErrorKind},
    config::{Config, PermissionMode},
    context::{self, TaskEnvelope},
    discovery, evaluator,
    policy::Registry,
    router::{
        advisory, profiler,
        scorer::{self, ScoringContext},
    },
    storage::Store,
    types::*,
};
use anyhow::{Result, bail, ensure};
use futures::{StreamExt, stream};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct DiscoveryFailure {
    pub agent: String,
    pub error: String,
    /// The agent's CLI is not installed (an unused preset rather than a problem).
    #[serde(skip)]
    pub missing: bool,
}
pub async fn discover(
    config: &Config,
    root: &Path,
    store: &Store,
    only_agent: Option<&str>,
    permission: PermissionMode,
) -> Result<(Vec<Client>, Vec<DiscoveryFailure>)> {
    discover_agents(
        config, root, store, only_agent, permission, None, None, true, None, false,
    )
    .await
}
/// Execution discovery: routing-only advisers are never started here. `peer` names every
/// session it starts in the mailbox (a collaboration role or pipeline phase).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn discover_as(
    config: &Config,
    root: &Path,
    store: &Store,
    only_agent: Option<&str>,
    permission: PermissionMode,
    events: Option<crate::acp::EventSink>,
    model: Option<&str>,
    peer: Option<&str>,
    read_only: bool,
) -> Result<(Vec<Client>, Vec<DiscoveryFailure>)> {
    discover_agents(
        config, root, store, only_agent, permission, events, model, false, peer, read_only,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn discover_agents(
    config: &Config,
    root: &Path,
    store: &Store,
    only_agent: Option<&str>,
    permission: PermissionMode,
    events: Option<crate::acp::EventSink>,
    model: Option<&str>,
    include_advisers: bool,
    peer: Option<&str>,
    read_only: bool,
) -> Result<(Vec<Client>, Vec<DiscoveryFailure>)> {
    let runtime = store.runtime()?;
    let timeout = Duration::from_secs(config.scheduler.discovery_timeout_secs);
    let mut failures = Vec::new();
    let mut configs = Vec::new();
    for agent in config.agents.iter().filter(|a| {
        a.enabled && (include_advisers || !a.routing_only) && only_agent.is_none_or(|id| id == a.id)
    }) {
        let availability = discovery::inspect(agent, &config.discovery, store.data_dir());
        if !matches!(availability.status, "ready" | "adapter_required") {
            failures.push(DiscoveryFailure {
                agent: agent.id.clone(),
                missing: availability.status == "not_installed",
                error: availability
                    .detail
                    .unwrap_or_else(|| availability.status.into()),
            });
        } else if !quota::available(&runtime, &agent.id, "*", now()) {
            failures.push(DiscoveryFailure {
                agent: agent.id.clone(),
                error: "agent/account is in cooldown".into(),
                missing: false,
            });
        } else {
            configs.push(agent.clone());
        }
    }
    let results: Vec<_> = stream::iter(configs.into_iter().map(|agent| {
        let events = events.clone();
        async move {
            let launch = match discovery::prepare(&agent, &config.discovery, store.data_dir()).await
            {
                Ok(launch) => launch,
                Err(error) => return (agent, Err(error)),
            };
            let result = tokio::time::timeout(
                timeout,
                Client::start_named(
                    launch, root, permission, timeout, events, model, peer, read_only,
                ),
            )
            .await
            .unwrap_or_else(|_| {
                Err(AgentError::new(
                    ErrorKind::Timeout,
                    "agent discovery timed out",
                ))
            });
            (agent, result)
        }
    }))
    .buffer_unordered(4)
    .collect()
    .await;
    let mut clients = Vec::new();
    for (agent, result) in results {
        match result {
            Ok(client) => {
                save_quota(store, &client)?;
                clients.push(client);
            }
            Err(error) => {
                update_failure(store, &agent.id, "*", &error)?;
                failures.push(DiscoveryFailure {
                    agent: agent.id,
                    error: error.to_string(),
                    missing: false,
                });
            }
        }
    }
    clients.sort_by(|a, b| a.config.id.cmp(&b.config.id));
    failures.sort_by(|a, b| a.agent.cmp(&b.agent));
    Ok((clients, failures))
}
pub(crate) fn save_quota(store: &Store, client: &Client) -> Result<()> {
    if let Some(observation) = client.quota() {
        let model = observation.model.as_deref().unwrap_or("*");
        store.update_runtime(&client.config.id, model, |state| {
            quota::observe(state, &observation, now())
        })?;
    }
    Ok(())
}
pub(crate) fn update_failure(
    store: &Store,
    agent: &str,
    model: &str,
    error: &AgentError,
) -> Result<()> {
    let scope = if error.model_scoped
        || !matches!(
            error.kind,
            ErrorKind::RateLimit | ErrorKind::Authentication | ErrorKind::Unavailable
        ) {
        model
    } else {
        "*"
    };
    store.update_runtime(agent, scope, |state| quota::fail(state, error, now()))
}
pub(crate) fn update_success(store: &Store, agent: &str, model: &str) -> Result<()> {
    for scope in ["*", model] {
        store.update_runtime(agent, scope, |state| {
            // A concurrent run may have observed a new limit. Keep its active cooldown.
            if state.available(now()) {
                quota::succeed(state, now());
            }
        })?;
    }
    Ok(())
}

pub struct RunOptions {
    pub task: String,
    pub overrides: Overrides,
    /// Already classified by the caller (the console classifies before it decides seats and
    /// phases). Passed on so one turn is classified once and every decision reads the same
    /// answer, rather than the scheduler asking again about text the console rewrote.
    pub descriptor: Option<TaskDescriptor>,
    pub dry_run: bool,
    pub json: bool,
    pub resume: Option<String>,
    pub permission: PermissionMode,
    /// A chat message: report progress as events instead of stderr, send the text as typed,
    /// evaluate only when files changed, and never hand a finished turn to another agent.
    pub interactive: bool,
    /// Files the message carries (images and documents).
    pub attachments: Vec<Attachment>,
    /// Names this run's agent session in the mailbox (a collaboration role, say).
    pub peer: Option<String>,
    /// Run the evaluator's checks once the agent finishes.
    pub verify: bool,
    /// Refuse this run the tools that would change the workspace (an advisory role).
    pub read_only: bool,
    /// Where this run chooses among the seats of one turn; it waits for the seats before it.
    pub place: Option<crate::mailbox::Place>,
    /// The seat this run fills in the conversation store. `None` records nothing, which is
    /// what an adviser, a classifier and `activity.enabled = false` all are.
    pub seat: Option<crate::activity::SeatRef>,
}
#[derive(Debug, Serialize)]
pub struct RoutePlan {
    pub descriptor: TaskDescriptor,
    pub candidates: Vec<ExecutionCandidate>,
    pub discovery_failures: Vec<DiscoveryFailure>,
}

pub async fn run(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    options: RunOptions,
) -> Result<u8> {
    run_with_events(config, policies, store, root, options, None).await
}
pub async fn run_with_events(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    options: RunOptions,
    events: Option<crate::acp::EventSink>,
) -> Result<u8> {
    run_turn(config, policies, store, root, options, events, &mut None).await
}

/// The session an execution ran in, so a conversation can continue it.
#[derive(Debug, Clone)]
pub struct Continuation {
    pub session: SessionRecord,
    /// The run this turn recorded, so what the user does next can be added to it.
    pub run_id: String,
    /// The agent advertised `session/load`.
    pub loadable: bool,
    /// Session modes the agent offers for this model (its own approval modes).
    pub modes: Vec<String>,
}

/// Like `run_with_events`, also reporting the last recorded execution session.
pub async fn run_turn(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    options: RunOptions,
    events: Option<crate::acp::EventSink>,
    continuation: &mut Option<Continuation>,
) -> Result<u8> {
    // With a seat to fill, everything this run emits passes through the store on its way to
    // the caller. Without one nothing is written, which is what an adviser, a classifier and
    // `activity.enabled = false` all are.
    let tee = options.seat.clone().map(|seat| {
        crate::activity::recorder::Tee::start(
            seat.store.clone(),
            crate::activity::recorder::Seat::from(&seat),
            config.activity.thinking,
            events.clone(),
            // Nothing is listening: this run's reply belongs on stdout, as it did before the
            // store sat in the path.
            events.is_none(),
        )
    });
    let downstream = match &tee {
        Some(tee) => Some(tee.sink()),
        None => events,
    };
    let result = run_recorded(
        config,
        policies,
        store,
        root,
        options,
        downstream,
        continuation,
        tee.as_ref(),
    )
    .await;
    // A turn is not over until its last row is written, or a client reading the store would
    // see a conversation that stops mid-sentence.
    if let Some(tee) = tee {
        tee.finish().await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_recorded(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    mut options: RunOptions,
    events: Option<crate::acp::EventSink>,
    continuation: &mut Option<Continuation>,
    tee: Option<&crate::activity::recorder::Tee>,
) -> Result<u8> {
    ensure!(!options.task.trim().is_empty(), "task must not be empty");
    ensure!(
        options.task.len() <= 1_048_576,
        "task exceeds the 1 MiB input limit"
    );
    let loud = !options.interactive;
    // Every wait below hears an interrupt that came since this run began, including one that
    // arrived while nothing was waiting.
    let since = crate::interrupt::mark();
    let report = |progress: Progress| {
        if let Some(events) = &events {
            let _ = events.send(ExecutionEvent::Progress(progress));
        }
    };
    if config.quota.refresh_before_run {
        let results = tokio::select! {
            result = crate::quota_sources::refresh(config, store) => result?,
            _ = since.wait() => return Ok(130),
        };
        for result in results.iter().filter(|r| loud && r.status == "unavailable") {
            eprintln!(
                "Quota {}: {}; retaining unexpired observations",
                result.agent,
                result.detail.as_deref().unwrap_or("unknown")
            );
        }
    }
    let repository_id = store.repository_id(root)?;
    let descriptor = match options.descriptor.take() {
        Some(descriptor) => descriptor,
        None => {
            let mut descriptor = profiler::profile(&options.task, root);
            // Classification exists to choose. A read-only seat is handed a prompt Orochi
            // wrote for it; a dry run is an inspection; and a route pinned to one agent and
            // model has nothing left to decide, so none of them may start an agent to ask.
            let decided = options.dry_run
                || (options.overrides.agent.is_some() && options.overrides.model.is_some());
            if !options.read_only && !decided {
                crate::router::classifier::refine(
                    config,
                    policies,
                    store,
                    root,
                    &options.task,
                    &mut descriptor,
                    |progress| {
                        // A one-shot run has no event sink to show what was just remembered.
                        if loud && let Progress::Note(note) = &progress {
                            eprintln!("{note}");
                        }
                        report(progress)
                    },
                )
                .await;
            }
            descriptor
        }
    };
    let previous = store.sessions(Some(&repository_id))?;
    if let Some(id) = &options.resume {
        let records: Vec<_> = previous
            .iter()
            .filter(|s| {
                s.session_id == *id
                    && options
                        .overrides
                        .agent
                        .as_ref()
                        .is_none_or(|a| a == &s.agent)
            })
            .collect();
        ensure!(
            records.len() == 1,
            "--resume must identify one recorded session in this repository (add --agent if ambiguous)"
        );
        let record = records[0];
        ensure!(
            options
                .overrides
                .model
                .as_ref()
                .is_none_or(|m| m == &record.model),
            "resume model differs from the recorded session"
        );
        options.overrides.agent = Some(record.agent.clone());
        options.overrides.model = Some(record.model.clone());
        if options.overrides.reasoning.is_none() {
            options.overrides.reasoning = record.reasoning.clone();
        }
        if options.overrides.mode.is_none() {
            options.overrides.mode = record.mode.clone();
        }
    }
    if loud {
        eprintln!(
            "Profiling: {} / {} / {}",
            descriptor.task_type,
            descriptor.language,
            descriptor.complexity.key()
        );
    }
    let (mut clients, failures) = tokio::select! {
        result = discover_as(config, root, store, options.overrides.agent.as_deref(), options.permission, events.clone(), options.overrides.model.as_deref(), options.peer.as_deref(), options.read_only) => result?,
        _ = since.wait() => return Ok(130),
    };
    if !options.dry_run {
        for failure in &failures {
            if loud {
                eprintln!("Unavailable: {}: {}", failure.agent, failure.error);
            } else if !failure.missing {
                report(Progress::Unavailable {
                    agent: failure.agent.clone(),
                    error: failure.error.clone(),
                });
            }
        }
    }
    let mut attempted = BTreeSet::new();
    let mut retryable = BTreeSet::new();
    let mut envelope = TaskEnvelope::new(&options.task, root);
    let task_id = uuid::Uuid::new_v4().to_string();
    let mut last_failure = None;
    for attempt in 0..config.scheduler.max_attempts {
        let runtime = store.runtime()?;
        let refs: Vec<_> = clients
            .iter()
            .map(|c| (&c.config, &c.capabilities))
            .collect();
        if let Some(place) = &options.place {
            tokio::select! {
                _ = place.turn() => {}
                _ = since.wait() => return Ok(130),
            }
        }
        let taken = crate::mailbox::busy_routes(options.peer.as_deref());
        let busy: Vec<String> = taken.iter().map(|(agent, _)| agent.clone()).collect();
        let score = |overrides: &Overrides| {
            scorer::candidates(
                &refs,
                &ScoringContext {
                    task: &descriptor,
                    overrides,
                    config,
                    policies,
                    store,
                    runtime: &runtime,
                    sessions: &previous,
                    root,
                    time: now(),
                    busy: &busy,
                    taken: &taken,
                },
            )
        };
        let mut ranked = score(&options.overrides)?;
        // A session mode carried over from an earlier turn may not exist for this model.
        if ranked.is_empty() && options.overrides.mode.take().is_some() {
            if loud {
                eprintln!("The chosen mode is unavailable here; continuing without it.");
            } else {
                report(Progress::Unavailable {
                    agent: "mode".into(),
                    error: "not available for this model; continuing without it".into(),
                });
            }
            ranked = score(&options.overrides)?;
        }
        if ranked.iter().any(|c| !attempted.contains(&c.id)) {
            ranked.retain(|c| !attempted.contains(&c.id));
        } else {
            ranked.retain(|c| retryable.contains(&c.id));
            for candidate in &mut ranked {
                candidate.reasons.push(
                    "bounded retry with a fresh TaskEnvelope after an unfinished or failed attempt"
                        .into(),
                );
            }
        }
        if ranked.is_empty() {
            for failure in failures.iter().filter(|_| loud) {
                eprintln!("  {}: {}", failure.agent, failure.error);
            }
            if attempt == 0 {
                bail!(
                    "no eligible execution candidate: check adapter installation/login, quota, capability flags, explicit overrides, and scheduler.required_success"
                );
            }
            break;
        }
        if options.dry_run {
            let plan = RoutePlan {
                descriptor,
                candidates: ranked,
                discovery_failures: failures,
            };
            if options.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print_route(&plan.candidates[0]);
                for candidate in &plan.candidates {
                    eprintln!(
                        "  {} / {} / {}: expected cost {:.0}",
                        candidate.agent,
                        candidate.model,
                        candidate
                            .reasoning_level
                            .as_deref()
                            .unwrap_or("agent-default"),
                        candidate.expected_cost
                    );
                }
                for failure in &plan.discovery_failures {
                    eprintln!("  {}: {}", failure.agent, failure.error);
                }
            }
            return Ok(0);
        }
        let mut advised = false;
        if attempt == 0 {
            let started = now();
            let decision = tokio::select! {
                decision = advisory::advise(config, policies, &descriptor, &ranked, store) => decision,
                _ = since.wait() => return Ok(130),
            };
            for consultation in &decision.consultations {
                let mut adviser = ranked[0].clone();
                if let Some(agent) = config.agents.iter().find(|a| a.id == consultation.agent) {
                    adviser.provider = agent.provider;
                }
                adviser.agent = consultation.agent.clone();
                adviser.model = consultation.model.clone();
                adviser.reasoning_level = consultation.reasoning.clone();
                adviser.mode = None;
                adviser.prediction = None;
                store.record(&make_record(
                    &task_id,
                    &repository_id,
                    &descriptor,
                    adviser,
                    consultation.usage.clone(),
                    Duration::from_millis(consultation.duration_ms),
                    0,
                    Outcome::PartialSuccess,
                    vec![],
                    consultation.error.clone(),
                    started,
                    consultation.purpose,
                ))?;
            }
            if let Some(id) = decision.candidate_id
                && let Some(index) = ranked.iter().position(|c| c.id == id)
            {
                ranked.swap(0, index);
                if let Some(p) = &mut ranked[0].prediction {
                    p.strategy = decision.purpose.into();
                    p.selection_probability = None;
                }
                ranked[0].reasons.push(format!(
                    "{} recommendation passed scheduler constraints",
                    decision.purpose
                ));
                advised = true;
            }
        }
        if !advised {
            crate::learning::select(
                &mut ranked,
                &config.learning,
                crate::learning::random_draw(),
            );
        }
        let mut candidate = ranked.remove(0);
        // Quota can quietly demote the model a task deserves. Say so once, rather than
        // leaving "why didn't it use the good model" unanswered.
        if let Some(rested) = ranked.iter().chain([&candidate]).min_by(|a, b| {
            let bare = |c: &ExecutionCandidate| {
                c.expected_cost
                    / c.prediction
                        .as_ref()
                        .and_then(|p| p.cost_features.as_ref())
                        .map_or(1.0, |f| f.quota_multiplier.max(1.0))
            };
            bare(a).total_cmp(&bare(b))
        }) && rested.model != candidate.model
            && rested
                .prediction
                .as_ref()
                .and_then(|p| p.cost_features.as_ref())
                .is_some_and(|f| f.quota_multiplier > 1.2)
        {
            let note = format!(
                "{} is close to its limit here; using {} instead",
                rested.model, candidate.model
            );
            if loud {
                eprintln!("  {note}");
            } else {
                report(Progress::Note(note));
            }
        }
        // Check persisted quota again after a potentially slow router request.
        if !quota::available(&store.runtime()?, &candidate.agent, &candidate.model, now()) {
            attempted.insert(candidate.id);
            continue;
        }
        attempted.insert(candidate.id.clone());
        // Taken at the choice itself, before anything is awaited: the next seat to choose sees
        // it at once, where the mailbox would show it only after this session is set up.
        let _claim =
            crate::mailbox::claim(options.peer.as_deref(), &candidate.agent, &candidate.model);
        let index = clients
            .iter()
            .position(|c| c.config.id == candidate.agent)
            .expect("candidate's live client");
        let resume = options.resume.take();
        // A resumed session already carries what was remembered when it began.
        let fresh = resume.is_none();
        if resume.is_some() {
            candidate.session_strategy = "resume".into();
            candidate
                .reasons
                .push("continues the recorded session".into());
        }
        // One attempt per pass of this loop, opened before the route is reported so that the
        // report is filed under the attempt it describes. A failover is the next attempt in
        // the same seat, not a second turn.
        let recorded_attempt = match (tee, &options.seat) {
            (Some(tee), Some(seat)) => {
                let considered: Vec<_> = ranked
                    .iter()
                    .take(5)
                    .map(|c| {
                        serde_json::json!({"agent": c.agent, "model": c.model,
                            "reasoning": c.reasoning_level, "cost": c.expected_cost,
                            "success": c.success_probability, "reasons": c.reasons})
                    })
                    .collect();
                let opened = seat.store.lock().expect("activity store").create_attempt(
                    &seat.seat,
                    &candidate,
                    resume.is_some(),
                    Some(&serde_json::json!(considered).to_string()),
                );
                match opened {
                    Ok(id) => {
                        tee.attempt(Some(id.clone()));
                        Some(id)
                    }
                    Err(error) => {
                        tracing::debug!(%error, "attempt not recorded");
                        None
                    }
                }
            }
            _ => None,
        };
        if loud {
            print_route(&candidate);
        }
        // Reported whether or not a terminal printed it: which agent took the work is part of
        // the record, and a one-shot run has no console listening to tell it.
        report(Progress::Route {
            agent: candidate.agent.clone(),
            provider: candidate.provider,
            model: candidate.model.clone(),
            reasoning: candidate.reasoning_level.clone(),
            resumed: resume.is_some(),
        });
        if options.read_only
            && let Some(mode) = clients[index]
                .capabilities
                .models
                .iter()
                .find(|m| m.model == candidate.model)
                .and_then(|m| m.modes.as_ref())
                .and_then(|modes| {
                    modes.values.iter().find(|value| {
                        let value = value.to_ascii_lowercase().replace(['-', '_', ' '], "");
                        value.contains("readonly") || value.contains("plan")
                    })
                })
        {
            candidate.mode = Some(mode.clone());
            candidate
                .reasons
                .push("a seat that only reads takes the agent's own read-only mode".into());
        }
        let started = now();
        let clock = Instant::now();
        let mut checks = Vec::new();
        // What the failed checks printed. It reaches the conversation store and nothing else.
        let mut check_output = evaluator::Output::default();
        let before = if options.interactive {
            context::tree_fingerprint(root)
        } else {
            None
        };
        let setup = tokio::select! {
            result = async {
                if let Some(id) = resume { clients[index].load_session(&id, root).await?; }
                clients[index].configure(&candidate).await
            } => result,
            _ = since.wait() => Err(AgentError::new(ErrorKind::Cancelled, "interrupted during session setup")),
        };
        let result = match setup {
            Ok(()) => {
                let mut text = if options.interactive && attempt == 0 {
                    options.task.clone()
                } else {
                    envelope.prompt(attempt > 0)
                };
                // Joining the mailbox first means the note names this session correctly.
                clients[index].announce_route(&candidate.agent, &candidate.model);
                // A seat that started before the one it answers to had joined would find
                // nobody to talk to.
                if let Some(place) = &options.place {
                    place.seated();
                }
                if let Some(note) = clients[index].peer_note() {
                    text = format!("{note}\n\n{text}");
                }
                if fresh
                    && let Some(note) =
                        crate::memory::Memory::open(store.data_dir(), &config.memory)
                            .and_then(|memory| memory.note(&repository_id, now()))
                {
                    text = format!("{note}\n\n{text}");
                }
                clients[index]
                    .prompt_with(
                        text,
                        &options.attachments,
                        Duration::from_secs(config.scheduler.prompt_timeout_secs),
                    )
                    .await
            }
            Err(error) => Err(error),
        };
        // Freeze agent-side writes before running checks or starting a fallback.
        clients[index].stop().await;
        let outcome = match &result {
            Ok(completed) => {
                // A chat turn that changed nothing has nothing to verify; it stays unlabeled.
                let unchanged = || before.is_some() && before == context::tree_fingerprint(root);
                if *completed && options.verify && !unchanged() {
                    if !loud && !evaluator::checks(&config.evaluator, root).is_empty() {
                        report(Progress::Checking);
                    }
                    (checks, check_output) = tokio::select! {
                        result = evaluator::detailed(&config.evaluator, root, loud) => result,
                        _ = since.wait() => {
                            store.record(&make_record(&task_id, &repository_id, &descriptor, candidate.clone(), clients[index].usage(), clock.elapsed(), attempt, Outcome::Cancelled, vec![], Some("cancelled".into()), started, "execution"))?;
                            return Ok(130);
                        }
                    };
                }
                evaluator::outcome(*completed, &checks)
            }
            Err(error) if error.kind == ErrorKind::Cancelled => Outcome::Cancelled,
            Err(_) => Outcome::Failure,
        };
        let error_kind = result.as_ref().err().map(|e| e.kind.key().into());
        report(Progress::Attempt {
            outcome: outcome.clone(),
            checks: checks.clone(),
            error: result.as_ref().err().map(|e| e.to_string()),
            usage: clients[index].usage(),
            output: std::mem::take(&mut check_output.0),
        });
        let session_id = clients[index].capabilities.session_id.clone();
        let cache_key = context::cache_key(
            root,
            &store.salt()?,
            &serde_json::to_string(&clients[index].config)?,
        );
        // Agents may announce a provider-side model switch while executing. Attribute actual usage correctly.
        if let Some(actual) = clients[index].current_model()
            && actual != candidate.model
        {
            candidate.reasons.push(format!(
                "agent switched from {} to {actual}",
                candidate.model
            ));
            candidate.model = actual;
        }
        candidate.reasoning_level = clients[index].current_reasoning();
        candidate.mode = clients[index].current_mode();
        let record = make_record(
            &task_id,
            &repository_id,
            &descriptor,
            candidate.clone(),
            clients[index].usage(),
            clock.elapsed(),
            attempt,
            outcome.clone(),
            checks.clone(),
            error_kind,
            started,
            "execution",
        );
        store.record(&record)?;
        // The one-way link between the two files, plus the labels and numbers a thread's UI
        // needs, so reading a conversation never has to open the telemetry file.
        if let (Some(attempt), Some(seat)) = (&recorded_attempt, &options.seat)
            && let Err(error) = seat.store.lock().expect("activity store").attempt_finished(
                attempt,
                Some(&record.id),
                Some(match record.outcome {
                    Outcome::Success => "success",
                    Outcome::PartialSuccess => "partial_success",
                    Outcome::Failure => "failure",
                    Outcome::Cancelled => "cancelled",
                }),
                record.error_kind.as_deref(),
                result.as_ref().err().map(|e| e.to_string()).as_deref(),
                serde_json::to_string(&checks).ok().as_deref(),
                Some(&record.usage),
            )
        {
            tracing::debug!(%error, "attempt outcome not recorded");
        }
        let session = SessionRecord {
            session_id,
            repository_id: repository_id.clone(),
            agent: candidate.agent.clone(),
            model: candidate.model.clone(),
            reasoning: candidate.reasoning_level.clone(),
            mode: candidate.mode.clone(),
            cache_key,
            updated_at: now(),
            outcome: outcome.clone(),
        };
        store.session(&session)?;
        *continuation = Some(Continuation {
            session,
            run_id: record.id,
            loadable: clients[index].capabilities.load_session,
            modes: clients[index].modes(),
        });
        save_quota(store, &clients[index])?;
        match outcome {
            Outcome::Success | Outcome::PartialSuccess => {
                update_success(store, &candidate.agent, &candidate.model)?;
                if loud {
                    eprintln!(
                        "{} ({} attempt{}, telemetry recorded locally)",
                        if outcome == Outcome::Success {
                            "Completed; checks passed"
                        } else {
                            "Completed; no substantive verification available (partial_success)"
                        },
                        attempt + 1,
                        if attempt == 0 { "" } else { "s" }
                    );
                }
                return Ok(0);
            }
            Outcome::Cancelled => {
                if loud {
                    eprintln!("Stopped; current files preserved.");
                }
                return Ok(130);
            }
            Outcome::Failure => {
                let finished = result.is_ok();
                let error = result.err().unwrap_or_else(|| {
                    AgentError::new(
                        ErrorKind::Other,
                        "agent did not finish or evaluator checks failed",
                    )
                });
                update_failure(store, &candidate.agent, &candidate.model, &error)?;
                if error.kind == ErrorKind::Other {
                    retryable.insert(candidate.id.clone());
                }
                if loud {
                    eprintln!("Attempt failed: {error}");
                }
                // The user reviews a finished chat turn; another agent must not "fix" it unasked.
                if options.interactive && finished {
                    return Ok(1);
                }
                last_failure = Some(error.to_string());
                envelope.refresh(
                    root,
                    &format!("previous attempt: {}", error.kind.key()),
                    checks,
                );
                // Kill the failed process before any other agent touches the shared repository.
                let mut failed_client = clients.remove(index);
                let failed_config = failed_client.config.clone();
                failed_client.stop().await;
                if attempt + 1 < config.scheduler.max_attempts
                    && (matches!(error.kind, ErrorKind::Other | ErrorKind::Configuration)
                        || error.model_scoped)
                    && quota::available(&store.runtime()?, &failed_config.id, "*", now())
                {
                    let timeout = Duration::from_secs(config.scheduler.discovery_timeout_secs);
                    let restarted = tokio::select! {
                        result = tokio::time::timeout(timeout, Client::start_named(failed_config, root, options.permission, timeout, events.clone(), None, options.peer.as_deref(), options.read_only)) => result,
                        _ = since.wait() => return Ok(130),
                    };
                    if let Ok(Ok(client)) = restarted {
                        clients.push(client);
                    }
                }
            }
        }
    }
    if loud {
        eprintln!(
            "Task remains incomplete: {}. Current changes are preserved.",
            last_failure
                .as_deref()
                .unwrap_or("eligible candidates exhausted")
        );
    }
    Ok(1)
}

fn print_route(candidate: &ExecutionCandidate) {
    eprintln!(
        "Route: {} / {} / {}",
        candidate.agent,
        candidate.model,
        candidate
            .reasoning_level
            .as_deref()
            .unwrap_or("agent-default")
    );
    for reason in &candidate.reasons {
        eprintln!("  - {reason}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_record(
    task_id: &str,
    repo: &str,
    task: &TaskDescriptor,
    candidate: ExecutionCandidate,
    usage: Usage,
    duration: Duration,
    attempt: usize,
    outcome: Outcome,
    checks: Vec<CheckResult>,
    error_kind: Option<String>,
    started_at: i64,
    purpose: &str,
) -> RunRecord {
    RunRecord {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.into(),
        repository_id: repo.into(),
        task_type: task.task_type.clone(),
        language: task.language.clone(),
        framework: task.framework.clone(),
        scope: task.estimated_scope,
        context_size: task.estimated_context,
        prediction: candidate.prediction.clone(),
        complexity: Some(task.complexity),
        candidate,
        usage,
        duration_ms: duration.as_millis() as u64,
        attempt,
        outcome,
        checks,
        error_kind,
        started_at,
        purpose: purpose.into(),
        feedback: None,
    }
}
