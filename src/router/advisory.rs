//! Lightweight routing, frontier escalation and a two-round council. Every seat is a coding
//! agent consulted over ACP, with bounded replacements that receive the same routing state.
use crate::{
    agents::{AgentError, ErrorKind},
    config::{Config, RouterConfig},
    policy::Registry,
    router::adviser::{self, Selection, Session},
    scheduler::{self, quota},
    storage::Store,
    types::*,
};
use std::time::Instant;

pub struct Consultation {
    pub agent: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub purpose: &'static str,
    pub usage: Usage,
    pub duration_ms: u64,
    pub candidate_id: Option<String>,
    /// Why no valid vote was obtained: an agent error kind, `cooldown`, `request_failed`
    /// or `invalid_advice`.
    pub error: Option<String>,
}
pub struct Decision {
    pub candidate_id: Option<String>,
    pub purpose: &'static str,
    pub consultations: Vec<Consultation>,
}
fn constrained_vote(result: Option<String>, candidates: &[ExecutionCandidate]) -> Option<String> {
    result.filter(|id| {
        candidates
            .iter()
            .take(12)
            .any(|c| &c.id == id && c.expected_cost <= candidates[0].expected_cost * 1.25)
    })
}
/// Worst case for one seat: every replacement, and for a council a review turn in a reused
/// session that also carries the first request and reply.
fn seat_estimate(config: &RouterConfig, request: usize, review: Option<usize>) -> usize {
    config
        .choices()
        .map(|r| {
            let first = adviser::turn_estimate(r, request);
            first
                + review.map_or(0, |review| {
                    adviser::turn_estimate(r, request + r.max_output_tokens as usize + review)
                })
        })
        .sum()
}
fn fraction<'a>(configs: impl Iterator<Item = &'a RouterConfig>) -> f64 {
    configs
        .flat_map(RouterConfig::choices)
        .map(|r| r.max_resource_fraction)
        .fold(1.0, f64::min)
}

struct Cx<'a> {
    config: &'a Config,
    policies: &'a Registry,
    store: &'a Store,
    candidates: &'a [ExecutionCandidate],
    request: String,
}

struct Seat<'a> {
    config: &'a RouterConfig,
    /// Difficulty of this seat's routing decision; chooses default adviser models.
    complexity: Complexity,
    next: Option<usize>,
    session: Option<(usize, Session)>,
}
impl<'a> Seat<'a> {
    fn new(config: &'a RouterConfig, complexity: Complexity) -> Self {
        Self {
            config,
            complexity,
            next: Some(0),
            session: None,
        }
    }
    async fn close(&mut self) {
        if let Some((_, session)) = self.session.take() {
            session.stop().await;
        }
    }
    /// Every replacement receives the same descriptor, candidates and peer votes; no state is
    /// held solely by an unavailable session. A review turn reuses the seat's live session.
    async fn consult(
        &mut self,
        cx: &Cx<'_>,
        review: Option<&str>,
        purpose: &'static str,
    ) -> (Option<String>, Vec<Consultation>) {
        let mut calls = vec![];
        let Some(first) = self.next else {
            return (None, calls);
        };
        let mut responsive = None;
        for (index, choice) in self.config.choices().enumerate().skip(first) {
            let start = Instant::now();
            let reused = match self.session.take() {
                Some((i, session)) if i == index => Some(session),
                Some((_, session)) => {
                    session.stop().await;
                    None
                }
                None => None,
            };
            let (outcome, usage, model, reasoning) =
                turn(cx, choice, self.complexity, reused, review).await;
            let (candidate_id, error) = match outcome {
                Ok((reply, session)) => {
                    responsive = Some(index);
                    self.session = Some((index, session));
                    let id = constrained_vote(
                        adviser::parse(&reply, cx.candidates).map(|a| a.candidate_id),
                        cx.candidates,
                    );
                    let error = id.is_none().then(|| "invalid_advice".to_owned());
                    (id, error)
                }
                Err(error) => (None, Some(failure(cx, choice, &model, &error))),
            };
            let valid = candidate_id.is_some();
            calls.push(Consultation {
                agent: choice.agent.clone(),
                model,
                reasoning,
                purpose,
                usage,
                duration_ms: start.elapsed().as_millis() as u64,
                candidate_id: candidate_id.clone(),
                error,
            });
            if valid {
                self.next = Some(index);
                return (candidate_id, calls);
            }
        }
        self.next = responsive;
        (None, calls)
    }
}

fn failure(cx: &Cx<'_>, choice: &RouterConfig, model: &str, error: &anyhow::Error) -> String {
    let Some(agent_error) = error.downcast_ref::<AgentError>() else {
        eprintln!("Adviser {} / {model} unavailable: {error:#}", choice.agent);
        return "request_failed".into();
    };
    if agent_error.kind == ErrorKind::Unavailable && agent_error.message == "cooldown" {
        return "cooldown".into();
    }
    // The adviser shares the account with execution, so a real limit applies to both.
    if matches!(
        agent_error.kind,
        ErrorKind::RateLimit | ErrorKind::Authentication | ErrorKind::Unavailable
    ) {
        // An unknown default model can only be recorded agent-wide.
        let scope = if model == "agent-default" { "*" } else { model };
        let _ = scheduler::update_failure(cx.store, &choice.agent, scope, agent_error);
    }
    eprintln!("Adviser {} / {model} unavailable: {error:#}", choice.agent);
    agent_error.kind.key().into()
}

fn cooldown(cx: &Cx<'_>, agent: &str, model: &str) -> Option<anyhow::Error> {
    match cx.store.runtime() {
        Err(error) => Some(error),
        Ok(runtime) if quota::available(&runtime, agent, model, now()) => None,
        Ok(_) => Some(AgentError::new(ErrorKind::Unavailable, "cooldown").into()),
    }
}

/// One adviser turn. Returns the reply with its live session, the usage and the model in use.
async fn turn(
    cx: &Cx<'_>,
    choice: &RouterConfig,
    complexity: Complexity,
    reused: Option<Session>,
    review: Option<&str>,
) -> (
    anyhow::Result<(String, Session)>,
    Usage,
    String,
    Option<String>,
) {
    let configured = choice.model.as_deref().unwrap_or("agent-default");
    if let Some(error) = cooldown(cx, &choice.agent, choice.model.as_deref().unwrap_or("*")) {
        if let Some(session) = reused {
            session.stop().await;
        }
        return (Err(error), Usage::default(), configured.into(), None);
    }
    let (mut session, text) = match (reused, review) {
        (Some(session), Some(review)) => (session, review.to_owned()),
        (reused, review) => {
            if let Some(session) = reused {
                session.stop().await;
            }
            let selection = Selection {
                config: cx.config,
                policies: cx.policies,
                store: cx.store,
                complexity,
            };
            let started = Session::start(choice, &selection).await;
            match started {
                Ok(session) => (
                    session,
                    match review {
                        Some(review) => format!("{}\n\n{review}", cx.request),
                        None => cx.request.clone(),
                    },
                ),
                Err(error) => return (Err(error), Usage::default(), configured.into(), None),
            }
        }
    };
    let (model, reasoning) = (session.model(), session.reasoning());
    let (result, usage) = session.ask(text).await;
    match result {
        Ok(reply) => (Ok((reply, session)), usage, model, reasoning),
        Err(error) => {
            session.stop().await;
            (Err(error), usage, model, reasoning)
        }
    }
}

/// Routing decisions are small classification tasks. Default adviser models scale with the
/// seat: a router is simple, council seats are normal, a frontier judge is complex. These
/// levels are Orochi heuristics over the provider policy.
pub async fn advise(
    config: &Config,
    policies: &Registry,
    task: &TaskDescriptor,
    candidates: &[ExecutionCandidate],
    store: &Store,
) -> Decision {
    let mut decision = Decision {
        candidate_id: None,
        purpose: "routing",
        consultations: vec![],
    };
    if candidates.len() < 2 {
        return decision;
    }
    let cx = Cx {
        config,
        policies,
        store,
        candidates,
        request: adviser::request(task, candidates),
    };
    if config.council.enabled {
        decision.purpose = "council";
        let n = config.council.members.len();
        if !(2..=4).contains(&n) {
            return decision;
        }
        let longest = candidates
            .iter()
            .take(12)
            .max_by_key(|c| c.id.len())
            .unwrap()
            .id
            .clone();
        let worst_review = adviser::review(&vec![longest; n]).len();
        // Reserve both rounds and every possible replacement before starting any adviser.
        let total: usize = config
            .council
            .members
            .iter()
            .map(|m| seat_estimate(m, cx.request.len(), Some(worst_review)))
            .sum();
        if total as f64 > candidates[0].expected_tokens * fraction(config.council.members.iter()) {
            return decision;
        }
        // Replacements keep their logical seat for round two. Failed seats remain in the
        // majority denominator; replacements never add votes.
        let mut seats: Vec<Seat> = config
            .council
            .members
            .iter()
            .map(|m| Seat::new(m, Complexity::Normal))
            .collect();
        let mut votes = vec![];
        for round in 0..2 {
            let purpose = if round == 0 {
                "council_initial"
            } else {
                "council_review"
            };
            let review = (round == 1).then(|| adviser::review(&votes));
            let results = futures::future::join_all(
                seats
                    .iter_mut()
                    .map(|seat| seat.consult(&cx, review.as_deref(), purpose)),
            )
            .await;
            let mut next_votes = vec![];
            for (vote, calls) in results {
                next_votes.extend(vote);
                decision.consultations.extend(calls);
            }
            votes = next_votes;
            if votes.is_empty() {
                break;
            }
        }
        for seat in &mut seats {
            seat.close().await;
        }
        decision.candidate_id = votes
            .iter()
            .find(|id| votes.iter().filter(|v| *v == *id).count() > n / 2)
            .cloned();
        return decision;
    }
    let difficult = matches!(task.complexity, Complexity::Extreme) || task.ambiguity >= 0.7;
    let (router, purpose, complexity) = if difficult && config.frontier.is_some() {
        (
            config.frontier.as_ref(),
            "frontier_routing",
            Complexity::Complex,
        )
    } else if candidates[0].confidence < config.scheduler.routing_confidence {
        (config.router.as_ref(), "routing", Complexity::Simple)
    } else {
        (None, "routing", Complexity::Simple)
    };
    decision.purpose = purpose;
    if let Some(router) = router
        && seat_estimate(router, cx.request.len(), None) as f64
            <= candidates[0].expected_tokens * fraction(std::iter::once(router))
    {
        let mut seat = Seat::new(router, complexity);
        let (vote, calls) = seat.consult(&cx, None, purpose).await;
        seat.close().await;
        decision.candidate_id = vote;
        decision.consultations = calls;
    }
    decision
}
