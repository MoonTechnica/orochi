use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Openai,
    Anthropic,
    Google,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
        })
    }
}

/// Ordered: `Ord` is what lets the classifier raise a complexity and never lower it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Simple,
    Normal,
    Complex,
    Extreme,
}
impl Complexity {
    pub fn key(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Normal => "normal",
            Self::Complex => "complex",
            Self::Extreme => "extreme",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Simple => 0,
            Self::Normal => 1,
            Self::Complex => 2,
            Self::Extreme => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDescriptor {
    pub task_type: String,
    pub language: String,
    pub framework: Option<String>,
    pub repo_size: usize,
    pub candidate_files: Vec<String>,
    pub estimated_scope: usize,
    pub estimated_context: u64,
    pub complexity: Complexity,
    pub requires_architecture_change: bool,
    pub requires_browser: bool,
    pub requires_web: bool,
    #[serde(default)]
    pub requires_image: bool,
    /// The user asked for the agents themselves to work together on this.
    #[serde(default)]
    pub collaborative: bool,
    /// How many agents the user asked for, when they said.
    #[serde(default)]
    pub seats: Option<usize>,
    pub tests_available: bool,
    pub ambiguity: f64,
    pub long_horizon: bool,
    /// An agent or model the user said they want for this kind of work, as the classifier read
    /// it from what is remembered: a name fragment matched against discovered IDs, never used
    /// to make one up. It lowers a candidate's cost; it never lets one past a gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub total_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
}
impl Usage {
    // Input includes cache reads, output includes reasoning: never double count them.
    pub fn total(&self) -> Option<u64> {
        self.total_tokens
            .or_else(|| Some(self.input_tokens?.saturating_add(self.output_tokens?)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Available,
    SoftLimit,
    Cooldown,
    Probe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub agent: String,
    /// `*` denotes an agent/account-wide failure.
    pub model: String,
    pub status: RuntimeStatus,
    pub quota_estimate: Option<f64>,
    pub reset_at: Option<i64>,
    pub cooldown_until: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_failure_at: Option<i64>,
    pub consecutive_failures: u32,
    pub rate_limit_count: u32,
}
impl RuntimeState {
    pub fn new(agent: &str, model: &str) -> Self {
        Self {
            agent: agent.into(),
            model: model.into(),
            status: RuntimeStatus::Available,
            quota_estimate: None,
            reset_at: None,
            cooldown_until: None,
            last_success_at: None,
            last_failure_at: None,
            consecutive_failures: 0,
            rate_limit_count: 0,
        }
    }
    pub fn effective_status(&self, time: i64) -> RuntimeStatus {
        if self.status == RuntimeStatus::Cooldown && self.cooldown_until.is_some_and(|t| t <= time)
        {
            RuntimeStatus::Probe
        } else if self.status == RuntimeStatus::SoftLimit
            && self.reset_at.is_some_and(|t| t <= time)
        {
            RuntimeStatus::Available
        } else {
            self.status
        }
    }
    pub fn available(&self, time: i64) -> bool {
        self.effective_status(time) != RuntimeStatus::Cooldown
    }
    pub fn shadow_price(&self, time: i64) -> f64 {
        if self.reset_at.is_some_and(|t| t <= time) {
            return 1.0;
        }
        let Some(remaining) = self.quota_estimate else {
            return 1.0;
        };
        let urgency = self
            .reset_at
            .map(|t| ((t - time).max(0) as f64 / 14_400.0).min(1.0))
            .unwrap_or(0.5);
        1.0 + (1.0 - remaining.clamp(0.0, 1.0)).powi(3) * 8.0 * urgency
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCandidate {
    pub id: String,
    pub agent: String,
    pub provider: Provider,
    pub model: String,
    pub reasoning_level: Option<String>,
    pub mode: Option<String>,
    pub session_strategy: String,
    pub context_strategy: String,
    pub success_probability: f64,
    pub expected_tokens: f64,
    pub expected_cost: f64,
    pub confidence: f64,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub prediction: Option<Prediction>,
}

/// A file a chat message carries: an image or a document the agent should look at.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub tag: String,
    pub path: std::path::PathBuf,
    pub name: String,
    pub bytes: u64,
    pub image: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Overrides {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    PartialSuccess,
    Failure,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub repository_id: String,
    pub task_type: String,
    pub language: String,
    pub framework: Option<String>,
    pub scope: usize,
    pub context_size: u64,
    pub candidate: ExecutionCandidate,
    pub usage: Usage,
    pub duration_ms: u64,
    pub attempt: usize,
    pub outcome: Outcome,
    pub checks: Vec<CheckResult>,
    pub error_kind: Option<String>,
    pub started_at: i64,
    pub purpose: String,
    #[serde(default)]
    pub complexity: Option<Complexity>,
    /// Frozen predictions made before execution; actual model changes never rewrite these.
    #[serde(default)]
    pub prediction: Option<Prediction>,
    /// What the user did next, added after the run (`Store::feedback`); the only part of a
    /// record ever written later. It labels a run nothing verified, and only weakly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<Feedback>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Feedback {
    /// `continued` or `rerouted`.
    pub signal: String,
    pub success: bool,
    /// How much this counts against a verified outcome's 1.0.
    pub evidence: f64,
    pub at: i64,
}
impl Feedback {
    /// The weights are Orochi heuristics, not measured; `orochi calibrate` shows how predictions
    /// fare against them next to verified outcomes.
    pub fn continued() -> Self {
        Self::new("continued", true, 0.2)
    }
    /// Also after an interruption: stopping a turn alone is no verdict, stopping it and asking
    /// for another agent is.
    pub fn rerouted() -> Self {
        Self::new("rerouted", false, 0.3)
    }
    fn new(signal: &str, success: bool, evidence: f64) -> Self {
        Self {
            signal: signal.into(),
            success,
            evidence,
            at: now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prediction {
    pub candidate_id: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub mode: Option<String>,
    pub prior_success: f64,
    pub prior_tokens: f64,
    pub success: f64,
    pub tokens: f64,
    pub strategy: String,
    pub selection_probability: Option<f64>,
    #[serde(default)]
    pub cost_features: Option<CostFeatures>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostFeatures {
    pub cache_discount: f64,
    pub context_restore_tokens: f64,
    pub quota_multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub repository_id: String,
    pub agent: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub mode: Option<String>,
    pub cache_key: String,
    pub updated_at: i64,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    pub samples: usize,
    pub successes: usize,
    pub token_samples: usize,
    pub total_tokens: u64,
    pub duration_ms: u64,
}

pub type RuntimeMap = BTreeMap<(String, String), RuntimeState>;
