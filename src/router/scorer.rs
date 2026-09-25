use crate::{
    acp::Capabilities,
    config::{AgentConfig, Config},
    context,
    policy::Registry,
    scheduler::quota,
    storage::Store,
    types::*,
};
use anyhow::Result;
use std::{collections::BTreeMap, path::Path};

/// Cost multiplier for a candidate the user said they want. An Orochi heuristic, not measured.
const PREFERENCE: f64 = 0.8;
/// Cost multiplier for the candidate the classifier read the work onto. Sized to cross the
/// price gap between two models of one tier -- 2.5x on Anthropic, where Claude Fable 5.1 is
/// $10/MTok against Claude Opus 5.5's $4 and the two score identically here -- and no
/// further: a judgment about what suits the work may pick within the capable field, not leap
/// out of it. The provider says the more capable model is what you reach for when the one
/// below it falls short, so reaching has to be possible. An Orochi heuristic, not measured.
const SUITED: f64 = 0.35;

pub struct ScoringContext<'a> {
    pub task: &'a TaskDescriptor,
    pub overrides: &'a Overrides,
    pub config: &'a Config,
    pub policies: &'a Registry,
    pub store: &'a Store,
    pub runtime: &'a RuntimeMap,
    pub sessions: &'a [SessionRecord],
    pub root: &'a Path,
    pub time: i64,
    /// Agents other Orochi runs in this repository are using right now.
    pub busy: &'a [String],
    /// The (agent, model) every other seat is running: a seat of its own is worth more than
    /// the same model twice, so these are dropped whenever anything else is left.
    pub taken: &'a [(String, String)],
    /// What to ask of the model for this seat's part of the work, where that is not the whole
    /// task's complexity: carrying out a settled plan asks less than arriving at one. It picks
    /// the success prior and the reasoning level, and nothing else — the descriptor keeps the
    /// task's own complexity, which is what seating, escalation and the learning strata read.
    pub difficulty: Option<Complexity>,
}

pub fn candidates(
    agents: &[(&AgentConfig, &Capabilities)],
    cx: &ScoringContext<'_>,
) -> Result<Vec<ExecutionCandidate>> {
    let mut candidates = Vec::new();
    // One query per seat, not per reasoning level and mode of it.
    let mut floors: BTreeMap<(String, String), f64> = BTreeMap::new();
    let task = cx.task;
    let difficulty = cx.difficulty.unwrap_or(task.complexity);
    let overrides = cx.overrides;
    // ACP advertises nothing about browsing, web access or image generation, so what is known
    // about them is what somebody wrote down. A requirement nobody here claims to meet
    // therefore says more about the declarations than about the agents, and enforcing it would
    // empty the field over a blank config field: it is dropped, and the run says so. Where
    // somebody does claim it, it is a real gate and the ones who cannot are left out.
    // Read over the agents this route may actually use: an agent the user pinned is the whole
    // field, and what some other agent claims is then beside the point.
    let offered = agents
        .iter()
        .filter(|(config, _)| overrides.agent.as_ref().is_none_or(|a| a == &config.id))
        .fold(Abilities::default(), |all, (config, capabilities)| {
            all.or(config.abilities().or(capabilities.declared))
        });
    let needed = Abilities::needed(task);
    let gate = Abilities {
        browser: needed.browser && offered.browser,
        web: needed.web && offered.web,
        image: needed.image && offered.image,
    };
    let ungated: Vec<&str> = Abilities {
        browser: needed.browser && !gate.browser,
        web: needed.web && !gate.web,
        image: needed.image && !gate.image,
    }
    .names();
    for (agent, capabilities) in agents {
        if overrides.agent.as_ref().is_some_and(|a| a != &agent.id) {
            continue;
        }
        let abilities = agent.abilities().or(capabilities.declared);
        if gate.browser && !abilities.browser
            || gate.web && !abilities.web
            || gate.image && !abilities.image
        {
            continue;
        }
        let policy = cx.policies.get(agent.provider);
        let key = context::cache_key(cx.root, &cx.store.salt()?, &serde_json::to_string(agent)?);
        for model in &capabilities.models {
            if overrides.model.as_ref().is_some_and(|m| m != &model.model)
                || !quota::available(cx.runtime, &agent.id, &model.model, cx.time)
            {
                continue;
            }
            let reasoning = if let Some(explicit) = &overrides.reasoning {
                if model
                    .reasoning
                    .as_ref()
                    .is_none_or(|s| !s.values.contains(explicit))
                {
                    continue;
                }
                Some(explicit.clone())
            } else {
                model.reasoning.as_ref().and_then(|s| {
                    policy.reasoning(difficulty, &s.values, Some(&s.current), &model.model)
                })
            };
            if !policy.permits(&model.model, reasoning.as_deref()) {
                continue;
            }
            if model.reasoning.is_some() && reasoning.is_none() {
                continue;
            }
            let mode = if let Some(explicit) = &overrides.mode {
                if model
                    .modes
                    .as_ref()
                    .is_none_or(|s| !s.values.contains(explicit))
                {
                    continue;
                }
                Some(explicit.clone())
            } else {
                model.modes.as_ref().map(|s| s.current.clone())
            };
            let rule = policy.model_rule(&model.model);
            let prior = rule.success_prior[difficulty.index()];
            let affinity = policy.cache_rules.stable_prefix
                && cx.sessions.iter().any(|s| {
                    s.agent == agent.id
                        && s.model == model.model
                        && s.reasoning == reasoning
                        && s.mode == mode
                        && s.cache_key == key
                        && s.updated_at <= cx.time
                        && s.updated_at > cx.time - cx.config.scheduler.cache_ttl_secs
                        && matches!(s.outcome, Outcome::Success | Outcome::PartialSuccess)
                });
            let mut c = ExecutionCandidate {
                id: String::new(),
                prediction: None,
                agent: agent.id.clone(),
                provider: agent.provider,
                model: model.model.clone(),
                reasoning_level: reasoning,
                mode,
                session_strategy: "fresh".into(),
                context_strategy: if policy.context_rules.prefer_envelope {
                    "task_envelope"
                } else {
                    "filesystem"
                }
                .into(),
                success_probability: prior,
                expected_tokens: 0.0,
                expected_cost: 0.0,
                confidence: 0.0,
                reasons: vec![
                    if difficulty == task.complexity {
                        format!("{} {} task", task.complexity.key(), task.task_type)
                    } else {
                        format!(
                            "{} {} task, asked of the model as {}",
                            task.complexity.key(),
                            task.task_type,
                            difficulty.key()
                        )
                    },
                    format!(
                        "{} policy v{}; {} model prior",
                        agent.provider, policy.version, rule.tier
                    ),
                ],
            };
            c.id = context::hash(&[
                c.agent.as_bytes(),
                c.model.as_bytes(),
                c.reasoning_level.as_deref().unwrap_or("").as_bytes(),
                c.mode.as_deref().unwrap_or("").as_bytes(),
            ])[..16]
                .into();
            // What more thought costs, read off the same ladder that chose the level, in this
            // agent's own words. A model with no effort axis, and a level no ladder can place,
            // are priced at the middle rung.
            let reasoning_factor = match (&model.reasoning, c.reasoning_level.as_deref()) {
                (Some(selector), Some(level)) => {
                    crate::effort::Ladder::new(&selector.values).factor(level)
                }
                _ => crate::effort::factor(None),
            };
            // What a session there costs before any work, measured, plus what the work looks
            // like. The size term is common to every candidate and so cannot tilt the choice;
            // the floor is the seat's own and can.
            let session = match floors.get(&(agent.id.clone(), model.model.clone())) {
                Some(floor) => *floor,
                None => {
                    let floor = cx
                        .store
                        .session_floor(&agent.id, &model.model, 64)?
                        .unwrap_or(0.0);
                    floors.insert((agent.id.clone(), model.model.clone()), floor);
                    floor
                }
            };
            let initial_tokens =
                session + task.size_prior() * rule.relative_tokens * reasoning_factor;
            let runs = cx.store.learning_runs(&c, task)?;
            let pooled = if cx.config.learning.pooling > 0.0 {
                cx.store.pooled_runs(&c, task)?
            } else {
                vec![]
            };
            // "unknown" is every ID no pattern matched; pooling those would mix unrelated models.
            let tiered = if cx.config.learning.tier_pooling > 0.0 && rule.tier != "unknown" {
                cx.store
                    .tier_runs(&c, task, |model| policy.model_rule(model).tier == rule.tier)?
            } else {
                vec![]
            };
            let estimate = crate::learning::estimate_evidence(
                &cx.config.learning,
                prior,
                initial_tokens,
                &runs,
                &pooled,
                &tiered,
            );
            c.success_probability = estimate.success;
            c.expected_tokens = estimate.tokens;
            if c.success_probability < cx.config.scheduler.required_success {
                continue;
            }
            c.prediction = Some(Prediction {
                candidate_id: c.id.clone(),
                model: c.model.clone(),
                reasoning: c.reasoning_level.clone(),
                mode: c.mode.clone(),
                prior_success: prior,
                prior_tokens: initial_tokens,
                success: c.success_probability,
                tokens: c.expected_tokens,
                strategy: format!("{:?}", cx.config.learning.strategy).to_lowercase(),
                selection_probability: Some(1.0),
                cost_features: None,
                prior_basis: Some(crate::learning::PRIOR_BASIS.into()),
            });
            let cache_discount = if affinity {
                policy.cache_rules.affinity_discount
            } else {
                0.0
            };
            let shadow = quota::shadow_price(cx.runtime, &c.agent, &c.model, cx.time);
            let rehydration = if affinity {
                0.0
            } else {
                task.estimated_context as f64 * 0.1
            };
            let features = CostFeatures {
                cache_discount,
                context_restore_tokens: rehydration,
                quota_multiplier: shadow,
                session_tokens: session,
                price_multiplier: rule.price(),
            };
            c.expected_cost = crate::learning::resource_cost(
                c.expected_tokens,
                c.success_probability,
                &features,
                estimate.latency_ms,
            );
            // What the user said they want for this kind of work tips the balance and no more:
            // it cannot bring back a candidate that failed a gate, and a much cheaper or much
            // likelier one still wins.
            if let Some(name) = &task.preferred
                && (c.model.to_lowercase().contains(name.as_str())
                    || c.agent.to_lowercase().contains(name.as_str()))
            {
                c.expected_cost *= PREFERENCE;
                c.reasons
                    .push(format!("you said you want {name} for this kind of work"));
            }
            // What the work itself suits, read from what each model says it is for. Same shape
            // as a preference: it reorders the capable field and opens nothing.
            if let Some(name) = &task.suited
                && (c.model.to_lowercase().contains(name.as_str())
                    || c.agent.to_lowercase().contains(name.as_str()))
            {
                c.expected_cost *= SUITED;
                c.reasons
                    .push(format!("{name} is what this kind of work is for"));
            }
            if cx.busy.contains(&c.agent) {
                // Another run holds this account; leave it room instead of racing for quota.
                c.expected_cost *= 1.4;
                c.reasons
                    .push("another agent here is using this account; leaving it room".into());
            }
            if let Some(p) = &mut c.prediction {
                p.cost_features = Some(features);
            }
            c.confidence = (0.72
                + if task.tests_available { 0.12 } else { 0.0 }
                + (estimate.samples as f64 / 100.0).min(0.15)
                - task.ambiguity * 0.2
                + if rule.tier != "unknown" { 0.08 } else { 0.0 })
            .clamp(0.0, 0.99);
            c.reasons.push(format!(
                "estimated success {:.0}% ({} evaluated samples)",
                c.success_probability * 100.0,
                estimate.samples
            ));
            c.reasons.push(format!(
                "quota resource multiplier {shadow:.2}; unavailable candidates excluded"
            ));
            if affinity {
                c.reasons
                    .push("recent compatible session; potential cache affinity".into());
            }
            if !ungated.is_empty() {
                c.reasons.push(format!(
                    "no agent here advertises {}; asked anyway rather than excluding every agent",
                    ungated.join(", ")
                ));
            }
            candidates.push(c);
        }
    }
    // A table of agents is only worth having if they are not all the same model.
    if candidates
        .iter()
        .any(|c| !cx.taken.contains(&(c.agent.clone(), c.model.clone())))
    {
        candidates.retain(|c| !cx.taken.contains(&(c.agent.clone(), c.model.clone())));
    }
    candidates.sort_by(|a, b| {
        a.expected_cost
            .total_cmp(&b.expected_cost)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(candidates)
}
