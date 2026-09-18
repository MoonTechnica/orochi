//! Full-information, chronological replay. Every case must contain measured outcomes for every arm.
use crate::{
    config::{LearningConfig, LearningStrategy},
    learning,
    policy::Registry,
    types::*,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub schema_version: u32,
    pub provenance: String,
    pub synthetic: bool,
    pub cases: Vec<Case>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub descriptor: TaskDescriptor,
    pub arms: Vec<Arm>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    /// A policy/quota-eligible baseline candidate with uncalibrated priors.
    pub candidate: ExecutionCandidate,
    pub success: bool,
    pub tokens: u64,
    pub duration_ms: u64,
}
#[derive(Debug, Serialize)]
pub struct PolicyResult {
    pub strategy: LearningStrategy,
    pub selected: usize,
    pub abstained: usize,
    pub successes: usize,
    pub tokens: u64,
    pub tokens_per_success: Option<f64>,
    pub penalized_cost: f64,
    pub regret: f64,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub provenance: String,
    pub synthetic: bool,
    pub seed: u64,
    pub failure_penalty: f64,
    pub policies: Vec<PolicyResult>,
    pub limitation: &'static str,
}
fn key(candidate: &ExecutionCandidate, task: &TaskDescriptor) -> String {
    serde_json::to_string(&(
        candidate.agent.clone(),
        candidate.model.clone(),
        &candidate.reasoning_level,
        &candidate.mode,
        &task.task_type,
        &task.language,
        &task.framework,
        task.complexity,
    ))
    .unwrap()
}

fn arm_key(candidate: &ExecutionCandidate) -> String {
    serde_json::to_string(&(
        &candidate.agent,
        &candidate.model,
        &candidate.reasoning_level,
        &candidate.mode,
    ))
    .unwrap()
}
/// Replay reads tiers from the bundled policy, not the installed one: a result must not change
/// because policies were updated after the runs it replays.
fn same_tier_work(
    policies: &Registry,
    run: &RunRecord,
    candidate: &ExecutionCandidate,
    task: &TaskDescriptor,
) -> bool {
    let tier = |model: &str| &policies.get(candidate.provider).model_rule(model).tier;
    run.candidate.provider == candidate.provider
        && run.candidate.model != candidate.model
        && tier(&candidate.model) != "unknown"
        && tier(&run.candidate.model) == tier(&candidate.model)
        && run.task_type == task.task_type
        && run.complexity == Some(task.complexity)
        && run.candidate.reasoning_level == candidate.reasoning_level
        && run.candidate.mode == candidate.mode
}
fn same_context(run: &RunRecord, task: &TaskDescriptor) -> bool {
    run.task_type == task.task_type
        && run.language == task.language
        && run.framework == task.framework
        && run.complexity == Some(task.complexity)
}

fn replay_from(
    dataset: Dataset,
    config: &LearningConfig,
    required_success: f64,
    seed: u64,
    penalty: f64,
    score_start: usize,
) -> Result<Report> {
    ensure!(
        dataset.schema_version == 1 && !dataset.cases.is_empty(),
        "expected schema_version=1 and nonempty cases"
    );
    ensure!(
        penalty.is_finite() && penalty > 0.0,
        "failure penalty must be positive"
    );
    let mut cases = BTreeSet::new();
    for case in &dataset.cases {
        ensure!(
            cases.insert(&case.id) && !case.id.is_empty(),
            "duplicate/empty case ID"
        );
        ensure!(!case.arms.is_empty(), "case has no measured arms");
        let mut ids = BTreeSet::new();
        let mut keys = BTreeSet::new();
        for arm in &case.arms {
            let c = &arm.candidate;
            ensure!(
                ids.insert(&c.id) && keys.insert(key(c, &case.descriptor)),
                "duplicate arm in case {}",
                case.id
            );
            if let Some(features) = c.prediction.as_ref().and_then(|p| p.cost_features.as_ref()) {
                ensure!(
                    (0.0..1.0).contains(&features.cache_discount)
                        && features.context_restore_tokens.is_finite()
                        && features.context_restore_tokens >= 0.0
                        && features.quota_multiplier.is_finite()
                        && features.quota_multiplier > 0.0,
                    "invalid cost features"
                );
            }
            ensure!(
                !c.id.is_empty()
                    && c.success_probability.is_finite()
                    && c.success_probability > 0.0
                    && c.success_probability <= 1.0
                    && c.expected_tokens.is_finite()
                    && c.expected_tokens > 0.0
                    && c.expected_cost.is_finite()
                    && c.expected_cost > 0.0,
                "invalid baseline prediction"
            );
        }
    }
    let registry = Registry::bundled()?;
    let mut policies = vec![];
    for strategy in [
        LearningStrategy::Static,
        LearningStrategy::Ewma,
        LearningStrategy::Bandit,
    ] {
        let mut config = config.clone();
        config.strategy = strategy;
        let mut history: BTreeMap<String, Vec<RunRecord>> = BTreeMap::new();
        let mut arms: BTreeMap<String, Vec<RunRecord>> = BTreeMap::new();
        let mut recorded: Vec<RunRecord> = vec![];
        let mut metric = PolicyResult {
            strategy,
            selected: 0,
            abstained: 0,
            successes: 0,
            tokens: 0,
            tokens_per_success: None,
            penalized_cost: 0.0,
            regret: 0.0,
        };
        let mut rng = seed;
        for (time, case) in dataset.cases.iter().enumerate() {
            let oracle = case
                .arms
                .iter()
                .map(|a| a.tokens as f64 + if a.success { 0.0 } else { penalty })
                .fold(penalty, f64::min);
            let mut candidates: Vec<_> = case
                .arms
                .iter()
                .filter_map(|arm| {
                    let mut c = arm.candidate.clone();
                    let pooled: Vec<RunRecord> = arms
                        .get(&arm_key(&c))
                        .into_iter()
                        .flatten()
                        .filter(|r| !same_context(r, &case.descriptor))
                        .cloned()
                        .collect();
                    let tiered: Vec<RunRecord> = if config.tier_pooling > 0.0 {
                        recorded
                            .iter()
                            .filter(|r| same_tier_work(&registry, r, &c, &case.descriptor))
                            .cloned()
                            .collect()
                    } else {
                        vec![]
                    };
                    let estimate = learning::estimate_evidence(
                        &config,
                        c.success_probability,
                        c.expected_tokens,
                        history
                            .get(&key(&c, &case.descriptor))
                            .map(Vec::as_slice)
                            .unwrap_or(&[]),
                        &pooled,
                        &tiered,
                    );
                    let features = c
                        .prediction
                        .as_ref()
                        .and_then(|p| p.cost_features.clone())
                        .unwrap_or(CostFeatures {
                            cache_discount: 0.0,
                            context_restore_tokens: 0.0,
                            quota_multiplier: c.expected_cost * c.success_probability
                                / c.expected_tokens,
                        });
                    c.prediction = Some(Prediction {
                        candidate_id: c.id.clone(),
                        model: c.model.clone(),
                        reasoning: c.reasoning_level.clone(),
                        mode: c.mode.clone(),
                        prior_success: c.success_probability,
                        prior_tokens: c.expected_tokens,
                        success: estimate.success,
                        tokens: estimate.tokens,
                        strategy: format!("{strategy:?}"),
                        selection_probability: Some(1.0),
                        cost_features: Some(features.clone()),
                    });
                    c.expected_cost = learning::resource_cost(
                        estimate.tokens,
                        estimate.success,
                        &features,
                        estimate.latency_ms,
                    );
                    c.success_probability = estimate.success;
                    c.expected_tokens = estimate.tokens;
                    (c.success_probability >= required_success).then_some(c)
                })
                .collect();
            candidates.sort_by(|a, b| {
                a.expected_cost
                    .total_cmp(&b.expected_cost)
                    .then_with(|| a.id.cmp(&b.id))
            });
            // SplitMix64: stable across platforms and independent of map iteration order.
            rng = rng.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            let draw = ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64;
            learning::select(&mut candidates, &config, draw);
            if candidates.is_empty() {
                if time >= score_start {
                    metric.abstained += 1;
                    metric.penalized_cost += penalty;
                    metric.regret += penalty - oracle;
                }
                continue;
            }
            let c = candidates.remove(0);
            let arm = case.arms.iter().find(|a| a.candidate.id == c.id).unwrap();
            if time >= score_start {
                metric.selected += 1;
                metric.successes += usize::from(arm.success);
                metric.tokens = metric.tokens.saturating_add(arm.tokens);
                let cost = arm.tokens as f64 + if arm.success { 0.0 } else { penalty };
                metric.penalized_cost += cost;
                metric.regret += cost - oracle;
            }
            let task = &case.descriptor;
            let arm_history = arms.entry(arm_key(&c)).or_default();
            let record = RunRecord {
                id: format!("{time}"),
                task_id: case.id.clone(),
                repository_id: "benchmark".into(),
                task_type: task.task_type.clone(),
                language: task.language.clone(),
                framework: task.framework.clone(),
                scope: task.estimated_scope,
                context_size: task.estimated_context,
                complexity: Some(task.complexity),
                prediction: c.prediction.clone(),
                candidate: c,
                usage: Usage {
                    total_tokens: Some(arm.tokens),
                    ..Default::default()
                },
                duration_ms: arm.duration_ms,
                attempt: 0,
                outcome: if arm.success {
                    Outcome::Success
                } else {
                    Outcome::Failure
                },
                checks: vec![],
                error_kind: None,
                started_at: time as i64,
                purpose: "execution".into(),
                feedback: None,
            };
            arm_history.push(record.clone());
            recorded.push(record.clone());
            history
                .entry(key(&record.candidate, task))
                .or_default()
                .push(record);
        }
        metric.tokens_per_success =
            (metric.successes > 0).then(|| metric.tokens as f64 / metric.successes as f64);
        policies.push(metric);
    }
    Ok(Report {
        provenance: dataset.provenance,
        synthetic: dataset.synthetic,
        seed,
        failure_penalty: penalty,
        policies,
        limitation: "Replay assumes each arm was evaluated independently on the same starting state. Only chosen-arm feedback trains each policy; input order is chronological. Eligibility is supplied by the dataset. Synthetic data cannot establish provider quality or optimality.",
    })
}

pub fn replay(
    dataset: Dataset,
    config: &LearningConfig,
    required_success: f64,
    seed: u64,
    penalty: f64,
) -> Result<Report> {
    replay_from(dataset, config, required_success, seed, penalty, 0)
}

#[derive(Debug, Serialize)]
pub struct TuningReport {
    pub training_cases: usize,
    pub validation_cases: usize,
    pub recommended: LearningConfig,
    pub training: Report,
    pub baseline_validation: Report,
    pub tuned_validation: Report,
    pub limitation: &'static str,
}

/// Select coefficients on the chronological prefix only. Warm up on that prefix when
/// evaluating the unseen suffix; never use validation labels for coefficient selection.
pub fn tune(
    dataset: Dataset,
    config: &LearningConfig,
    required_success: f64,
    seed: u64,
    penalty: f64,
    training_cases: usize,
) -> Result<TuningReport> {
    ensure!(
        training_cases >= 2 && dataset.cases.len() >= training_cases + 2,
        "tuning requires at least two training and two validation cases"
    );
    let train = Dataset {
        cases: dataset.cases[..training_cases].to_vec(),
        ..dataset.clone()
    };
    let mut best_config = config.clone();
    let mut best_report = replay(train.clone(), config, required_success, seed, penalty)?;
    let target = if config.strategy == LearningStrategy::Bandit {
        2
    } else {
        1
    };
    for alpha in [0.05, 0.1, 0.2, 0.4] {
        for prior_weight in [2.0, 8.0, 16.0, 32.0] {
            for (exploration, pooling, tier_pooling) in [0.05, 0.1, 0.2]
                .into_iter()
                .flat_map(|e| [0.0, 0.5, 1.0].map(|p| (e, p)))
                .flat_map(|(e, p)| [0.0, 0.5, 1.0].map(|t| (e, p, t)))
            {
                let trial = LearningConfig {
                    alpha,
                    prior_weight,
                    exploration,
                    pooling,
                    tier_pooling,
                    ..config.clone()
                };
                let report = replay(train.clone(), &trial, required_success, seed, penalty)?;
                let a = &report.policies[target];
                let b = &best_report.policies[target];
                // Do not trade away successful tasks just to reduce token consumption.
                if a.successes > b.successes
                    || (a.successes == b.successes && a.penalized_cost < b.penalized_cost)
                {
                    best_config = trial;
                    best_report = report;
                }
            }
        }
    }
    Ok(TuningReport {
        training_cases,
        validation_cases: dataset.cases.len() - training_cases,
        baseline_validation: replay_from(
            dataset.clone(),
            config,
            required_success,
            seed,
            penalty,
            training_cases,
        )?,
        tuned_validation: replay_from(
            dataset,
            &best_config,
            required_success,
            seed,
            penalty,
            training_cases,
        )?,
        recommended: best_config,
        training: best_report,
        limitation: "Coefficients are selected on the training prefix only; validation is a chronological holdout with chosen-arm warm-up. Small datasets and a single seed cannot establish general provider quality. Review validation before applying; configuration is not automatically changed.",
    })
}
