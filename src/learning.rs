//! Context-stratified EWMA and bounded epsilon-greedy exploration.
use crate::{
    config::{LearningConfig, LearningStrategy},
    types::*,
};
use serde::Serialize;

/// What `Prediction::prior_tokens` is made of. Bump this whenever the prior changes shape:
/// the EWMA learns `measured / predicted`, so ratios from a differently made prior are not
/// ratios it can use. "session" is the measured cost of opening the seat, which the prior
/// carried nothing of before.
/// Names the recipe a prediction was made with, so a run predicted another way trains
/// success and latency but not tokens. It changed when the session floor stopped counting
/// cache reads as if they cost what fresh input costs.
pub const PRIOR_BASIS: &str = "weighted-session+scope";

pub fn resource_cost(tokens: f64, success: f64, features: &CostFeatures, latency_ms: f64) -> f64 {
    // `tokens` already includes what opening the session costs, and that part is not
    // discounted: a cache hit saves re-reading the work, not the prompt the seat opens with.
    let work = (tokens - features.session_tokens).max(0.0);
    // Price multiplies the tokens and nothing else: waiting costs the same whoever answers.
    ((work * (1.0 - features.cache_discount)
        + features.session_tokens
        + features.context_restore_tokens)
        * features.quota_multiplier
        * features.price_multiplier
        + latency_ms / 1000.0 * 2.0)
        / success
}

pub fn labeled(run: &RunRecord) -> bool {
    label(run).is_some()
}

/// What a run teaches, and how much it counts. A verified outcome counts fully and nothing the
/// user does afterwards changes it. An unverified completion, or a turn the user interrupted,
/// teaches only through what the user did next, and only as much as that signal is worth.
/// Infrastructure failures and runs that switched model mid-turn teach nothing.
pub fn label(run: &RunRecord) -> Option<(bool, f64)> {
    let comparable = run.purpose == "execution"
        && run.prediction.as_ref().is_none_or(|p| {
            p.model == run.candidate.model
                && p.reasoning == run.candidate.reasoning_level
                && p.mode == run.candidate.mode
        });
    if !comparable {
        return None;
    }
    match run.outcome {
        Outcome::Success | Outcome::Failure
            if !matches!(
                run.error_kind.as_deref(),
                Some(
                    "rate_limit" | "authentication" | "unavailable" | "configuration" | "cancelled"
                )
            ) =>
        {
            Some((run.outcome == Outcome::Success, 1.0))
        }
        Outcome::PartialSuccess | Outcome::Cancelled => run
            .feedback
            .as_ref()
            .filter(|f| f.evidence.is_finite() && f.evidence > 0.0 && f.evidence < 1.0)
            .map(|f| (f.success, f.evidence)),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct Estimate {
    pub success: f64,
    pub tokens: f64,
    pub latency_ms: f64,
    pub samples: usize,
}

pub fn estimate(
    config: &LearningConfig,
    prior: f64,
    initial_tokens: f64,
    runs: &[RunRecord],
) -> Estimate {
    estimate_pooled(config, prior, initial_tokens, runs, &[])
}

/// Same arm in other task contexts. Token use relative to the frozen prior and the success
/// residual are largely agent/model properties (system prompts, tool loops, caching), so with
/// `pooling > 0` they move this context's starting point before its own evidence is applied.
#[derive(Debug, Default)]
struct Pool {
    ratio: f64,
    residual: f64,
}
fn pool(config: &LearningConfig, weight: f64, runs: &[RunRecord]) -> Pool {
    let decay = 1.0 - config.alpha;
    let (mut mass, mut ratio, mut residual) = (0.0, 0.0, 0.0);
    let (mut token_mass, mut prior_mass, mut token_prior_mass) =
        (0.0, config.prior_weight, config.prior_weight);
    for (run, (success, weight)) in runs.iter().filter_map(|r| label(r).map(|l| (r, l))) {
        let Some(prediction) = &run.prediction else {
            continue;
        };
        // A run counting `weight` of a full one ages the rest by that much too.
        let decay = decay.powf(weight);
        prior_mass *= decay;
        mass = mass * decay + weight;
        residual = residual * decay + weight * (f64::from(success) - prediction.prior_success);
        if let Some(tokens) = run.usage.weighted()
            && prediction.prior_tokens.is_finite()
            && prediction.prior_tokens > 0.0
        {
            ratio = ratio * decay + weight * (tokens / prediction.prior_tokens).clamp(0.05, 20.0);
            token_mass = token_mass * decay + weight;
            token_prior_mass *= decay;
        }
    }
    let weight = weight.clamp(0.0, 1.0);
    Pool {
        ratio: if token_mass > 0.0 {
            1.0 + weight * (ratio / token_mass - 1.0) * token_mass / (token_mass + token_prior_mass)
        } else {
            1.0
        },
        residual: if mass > 0.0 {
            weight * residual / (mass + prior_mass)
        } else {
            0.0
        },
    }
}

pub fn estimate_pooled(
    config: &LearningConfig,
    prior: f64,
    initial_tokens: f64,
    runs: &[RunRecord],
    pooled: &[RunRecord],
) -> Estimate {
    estimate_evidence(config, prior, initial_tokens, runs, pooled, &[])
}

/// `tiered` is other models of the same provider and policy tier on this kind of work (task
/// type and complexity). Model IDs are replaced every few months while tiers persist; without
/// this each release starts again from the static prior and what was measured on its
/// predecessor is thrown away. Like `pooled` it only moves the starting point, and a model's own
/// evidence takes over as it accumulates.
pub fn estimate_evidence(
    config: &LearningConfig,
    prior: f64,
    initial_tokens: f64,
    runs: &[RunRecord],
    pooled: &[RunRecord],
    tiered: &[RunRecord],
) -> Estimate {
    let mut estimate = Estimate {
        success: prior,
        tokens: initial_tokens,
        ..Default::default()
    };
    if config.strategy == LearningStrategy::Static {
        return estimate;
    }
    let none = || Pool {
        ratio: 1.0,
        residual: 0.0,
    };
    let arm = if config.pooling > 0.0 {
        pool(config, config.pooling, pooled)
    } else {
        none()
    };
    let tier = if config.tier_pooling > 0.0 {
        pool(config, config.tier_pooling, tiered)
    } else {
        none()
    };
    // Disjoint evidence (this model elsewhere, other models here), so the shifts compose.
    let shared = Pool {
        ratio: arm.ratio * tier.ratio,
        residual: arm.residual + tier.residual,
    };
    let prior = (prior + shared.residual).clamp(0.01, 0.999);
    let initial_tokens = initial_tokens * shared.ratio;
    estimate.success = prior;
    estimate.tokens = initial_tokens;
    let decay = 1.0 - config.alpha;
    let (mut positive, mut mass, mut prior_mass) = (0.0, 0.0, config.prior_weight);
    let (mut token_mass, mut ratio_sum, mut latency) = (0.0, 0.0, 0.0);
    let mut token_prior_mass = config.prior_weight;
    for (run, (success, weight)) in runs.iter().filter_map(|r| label(r).map(|l| (r, l))) {
        // With weight 1 this is exactly the unweighted update.
        let decay = decay.powf(weight);
        prior_mass *= decay;
        positive = positive * decay + weight * f64::from(success);
        mass = mass * decay + weight;
        latency = latency * decay + weight * run.duration_ms as f64;
        if let Some((tokens, prediction)) = run.usage.weighted().zip(run.prediction.as_ref())
            && prediction.prior_basis.as_deref() == Some(PRIOR_BASIS)
            && prediction.prior_tokens.is_finite()
            && prediction.prior_tokens > 0.0
        {
            // Normalize by task size; bound the influence of single pathological turns.
            ratio_sum = ratio_sum * decay
                + weight * (tokens / (prediction.prior_tokens * shared.ratio)).clamp(0.05, 20.0);
            token_mass = token_mass * decay + weight;
            token_prior_mass *= decay;
        }
        // Confidence counts only what was verified.
        estimate.samples += usize::from(weight >= 1.0);
    }
    estimate.success = ((prior * prior_mass + positive) / (prior_mass + mass)).clamp(0.01, 0.999);
    if token_mass > 0.0 {
        let weight = token_mass / (token_mass + token_prior_mass);
        estimate.tokens = initial_tokens * (1.0 - weight + weight * ratio_sum / token_mass);
    }
    if mass > 0.0 {
        estimate.latency_ms = latency / mass;
    }
    estimate
}

/// `draw` is in [0,1); supplying it explicitly makes replay and probability tests deterministic.
/// Call only with candidates that already passed all hard constraints and the success floor.
pub fn select(candidates: &mut [ExecutionCandidate], config: &LearningConfig, draw: f64) {
    if candidates.is_empty() {
        return;
    }
    let pool = candidates
        .iter()
        .take_while(|c| c.expected_cost <= candidates[0].expected_cost * config.max_cost_ratio)
        .count();
    let epsilon = if config.strategy == LearningStrategy::Bandit {
        config.exploration
    } else {
        0.0
    };
    let explore_mass = epsilon / pool as f64;
    let best_probability = 1.0 - epsilon + explore_mass;
    let index = if draw < best_probability {
        0
    } else {
        (1 + ((draw - best_probability) / explore_mass) as usize).min(pool - 1)
    };
    let probability = if index == 0 {
        best_probability
    } else {
        explore_mass
    };
    candidates.swap(0, index);
    if let Some(prediction) = &mut candidates[0].prediction {
        prediction.selection_probability = Some(probability);
    }
    candidates[0].reasons.push(format!(
        "{:?} selection; probability {probability:.4}; {pool} candidates within cost bound",
        config.strategy
    ));
}

pub fn random_draw() -> f64 {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    // UUID's last 8 bytes retain 62 random bits; discard its variant bits.
    (u64::from_be_bytes(bytes[8..].try_into().unwrap()) & ((1u64 << 62) - 1)) as f64
        / (1u64 << 62) as f64
}

#[derive(Debug, Default, Serialize)]
pub struct CalibrationBin {
    pub count: usize,
    pub predicted: Option<f64>,
    pub observed: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct CalibrationReport {
    pub evaluated: usize,
    pub skipped: usize,
    pub token_samples: usize,
    pub brier: Option<f64>,
    pub prior_brier: Option<f64>,
    pub token_wape: Option<f64>,
    pub prior_token_wape: Option<f64>,
    pub bins: Vec<CalibrationBin>,
    /// Runs labeled only by what the user did next, scored apart so they never blur the
    /// verified numbers above. Per signal: how many, and how predictions fared against them.
    pub weak: Vec<WeakCalibration>,
    pub limitation: &'static str,
}
#[derive(Debug, Serialize)]
pub struct WeakCalibration {
    pub signal: String,
    pub evidence: f64,
    pub evaluated: usize,
    pub brier: f64,
}
/// Predictions are frozen before the run, so this is prequential, with no future-label leakage.
pub fn calibrate(runs: &[RunRecord]) -> CalibrationReport {
    let (mut n, mut tn) = (0, 0);
    let (mut brier, mut prior_brier, mut error, mut prior_error, mut tokens) =
        (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut bins: Vec<CalibrationBin> = (0..10).map(|_| CalibrationBin::default()).collect();
    let mut weak: Vec<WeakCalibration> = vec![];
    for (run, (success, weight)) in runs.iter().filter_map(|r| label(r).map(|l| (r, l))) {
        let Some(p) = &run.prediction else {
            continue;
        };
        let y = f64::from(success);
        if weight < 1.0 {
            let signal = run.feedback.as_ref().map_or("", |f| f.signal.as_str());
            let entry = match weak.iter().position(|w| w.signal == signal) {
                Some(index) => &mut weak[index],
                None => {
                    weak.push(WeakCalibration {
                        signal: signal.into(),
                        evidence: weight,
                        evaluated: 0,
                        brier: 0.0,
                    });
                    weak.last_mut().unwrap()
                }
            };
            entry.evaluated += 1;
            entry.brier += (p.success - y).powi(2);
            continue;
        }
        n += 1;
        brier += (p.success - y).powi(2);
        prior_brier += (p.prior_success - y).powi(2);
        let bin = &mut bins[((p.success * 10.0) as usize).min(9)];
        bin.count += 1;
        bin.predicted = Some(bin.predicted.unwrap_or(0.0) + p.success);
        bin.observed = Some(bin.observed.unwrap_or(0.0) + y);
        if let Some(actual) = run.usage.weighted() {
            tn += 1;
            tokens += actual;
            error += (p.tokens - actual).abs();
            prior_error += (p.prior_tokens - actual).abs();
        }
    }
    for bin in &mut bins {
        bin.predicted = bin.predicted.map(|v| v / bin.count as f64);
        bin.observed = bin.observed.map(|v| v / bin.count as f64);
    }
    for entry in &mut weak {
        entry.brier /= entry.evaluated as f64;
    }
    let weak_count: usize = weak.iter().map(|w| w.evaluated).sum();
    CalibrationReport {
        evaluated: n,
        skipped: runs.len() - n - weak_count,
        token_samples: tn,
        brier: (n > 0).then(|| brier / n as f64),
        prior_brier: (n > 0).then(|| prior_brier / n as f64),
        token_wape: (tokens > 0.0).then(|| error / tokens),
        prior_token_wape: (tokens > 0.0).then(|| prior_error / tokens),
        bins,
        weak,
        limitation: "Chosen-arm observations only; not a causal comparison of routing policies. Use paired benchmark cases to compare alternatives. Token error is not currency cost. Weak signals (what the user did next) are uncalibrated Orochi heuristics and are reported apart from verified outcomes; whether learning from them helps shows over time in the verified Brier score, not in theirs.",
    }
}
