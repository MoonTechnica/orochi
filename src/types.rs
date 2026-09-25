use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Who is behind the model. A name, and an open set: an ACP agent may be driven by a vendor
/// nobody here has written a policy for, and making it claim to be one of three would put its
/// runs in another vendor's learning strata and price them by another vendor's tiers. The name
/// is held inline so the type stays `Copy` -- it is threaded through every candidate, record,
/// closure and map key there is -- and short, because it is an identifier and nothing else. It
/// serializes as the plain string it always was, so records written before the set opened
/// deserialize unchanged.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Provider([u8; Provider::MAX]);

impl Provider {
    pub const MAX: usize = 24;
    pub const OPENAI: Self = Self::literal("openai");
    pub const ANTHROPIC: Self = Self::literal("anthropic");
    pub const GOOGLE: Self = Self::literal("google");
    /// A name written in this source; invalid ones fail to compile.
    const fn literal(name: &str) -> Self {
        let bytes = name.as_bytes();
        assert!(
            !bytes.is_empty() && bytes.len() <= Self::MAX,
            "provider name"
        );
        let mut out = [0u8; Self::MAX];
        let mut i = 0;
        while i < bytes.len() {
            assert!(valid(bytes[i], i == 0), "provider name");
            out[i] = bytes[i];
            i += 1;
        }
        Self(out)
    }
    /// A name from config, a stored record or an agent: lowercase ASCII, starting with a letter.
    pub fn new(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > Self::MAX {
            return None;
        }
        let mut out = [0u8; Self::MAX];
        for (index, byte) in bytes.iter().enumerate() {
            if !valid(*byte, index == 0) {
                return None;
            }
            out[index] = *byte;
        }
        Some(Self(out))
    }
    pub fn as_str(&self) -> &str {
        let len = self.0.iter().position(|b| *b == 0).unwrap_or(Self::MAX);
        std::str::from_utf8(&self.0[..len]).unwrap_or_default()
    }
}
const fn valid(byte: u8, first: bool) -> bool {
    byte.is_ascii_lowercase() || !first && (byte.is_ascii_digit() || byte == b'-' || byte == b'_')
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl Serialize for Provider {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Provider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::new(&name).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid provider name (lowercase ASCII, at most {} characters): {name}",
                Self::MAX
            ))
        })
    }
}

/// What an agent can do that ACP says nothing about: drive a browser, reach the open internet,
/// make an image. The protocol advertises none of the three -- `promptCapabilities` says only
/// what a *prompt* may carry, which is whether an image can be sent to the agent and not
/// whether one can be asked of it -- so they come from what the user configured for the agent
/// and from what the agent itself declares in `_meta["orochi.dev/capabilities"]` on
/// `initialize`, in the shape Orochi's other extensions already use. Undeclared is not the same
/// as absent: where nobody claims a capability the requirement is dropped rather than left to
/// empty the field in silence (`scorer::candidates`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Abilities {
    pub browser: bool,
    pub web: bool,
    pub image: bool,
}
impl Abilities {
    pub fn or(self, other: Self) -> Self {
        Self {
            browser: self.browser || other.browser,
            web: self.web || other.web,
            image: self.image || other.image,
        }
    }
    /// Everything this task needs is here.
    pub fn covers(self, task: &TaskDescriptor) -> bool {
        (!task.requires_browser || self.browser)
            && (!task.requires_web || self.web)
            && (!task.requires_image || self.image)
    }
    /// What this task needs and nothing else, for saying which requirement stood in the way.
    pub fn needed(task: &TaskDescriptor) -> Self {
        Self {
            browser: task.requires_browser,
            web: task.requires_web,
            image: task.requires_image,
        }
    }
    pub fn names(self) -> Vec<&'static str> {
        [
            (self.browser, "browser"),
            (self.web, "web"),
            (self.image, "image"),
        ]
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect()
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
    /// One step less to ask of a model, for the part of a task that is not where the thinking
    /// is. It never goes below Normal: carrying out a plan for cross-cutting work is still
    /// not a one-line edit, and Simple has nothing left to give up.
    pub fn eased(self) -> Self {
        match self {
            Self::Extreme => Self::Complex,
            Self::Complex => Self::Normal,
            other => other,
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

/// How big a piece of work looks before anything has run: the context it would carry plus a
/// constant for each place it is expected to touch. A heuristic, and the only measure of this
/// task's own size that is in hand before an agent is asked anything about it.
pub fn size_prior(context: u64, scope: usize) -> f64 {
    context as f64 + 1500.0 * scope as f64
}
/// What work of a kind has cost here, beside how big that work looked before it ran.
#[derive(Debug, Clone, Copy)]
pub struct Typical {
    pub tokens: f64,
    pub size: f64,
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
    /// The model the classifier judged this kind of work suits, read from what each model
    /// says it is for. It is the one thing the numbers cannot carry: two models of one tier,
    /// with the same prior and the same price, still differ in what they are good at. Like
    /// `preferred` it only reorders candidates that already passed every gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suited: Option<String>,
}
impl TaskDescriptor {
    pub fn size_prior(&self) -> f64 {
        size_prior(self.estimated_context, self.estimated_scope)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub total_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    /// Tokens written to the cache this run. The dearest kind there is, and until it had a
    /// field of its own it was only an unexplained residual inside `total_tokens`. Records
    /// written before it existed have none.
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
    /// How many `usage_update` notifications the agent sent while the turn ran — roughly one
    /// per model request, for both adapters measured. It is not part of any total and nothing
    /// is estimated from it. It is here because **what a reported total covers is the
    /// adapter's choice, and the two disagree**: `claude-agent-acp` accumulates every assistant
    /// message into the turn's figure, while `codex-acp` reports `tokenUsage.last` — the final
    /// model request alone — and keeps the turn's `total` for its own `/status` (read from
    /// 1.10.0 and from the latest, 1.13.1, on 2026-09-25). A turn that made one request and a
    /// turn that made seventeen are then not comparable, and nothing in the record said which
    /// this was. Now it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests: Option<u32>,
}
/// What a token of each kind costs, against one fresh input token. Cache reads are the cheap
/// ones, a cache write costs more than the input it stores, and output is the dear one. The
/// exact ratio is a provider's and a model's -- that is `ModelRule::price_premium`'s job --
/// but the shape holds wherever there is a cache, and counting all four as one token does not:
/// measured on one Anthropic run, 93% of the total was cache reads.
const CACHE_READ: f64 = 0.1;
const CACHE_WRITE: f64 = 1.25;
const OUTPUT: f64 = 5.0;
impl Usage {
    // Input includes cache reads, output includes reasoning: never double count them.
    pub fn total(&self) -> Option<u64> {
        self.total_tokens
            .or_else(|| Some(self.input_tokens?.saturating_add(self.output_tokens?)))
    }
    /// The same run counted by what it costs rather than by how many tokens passed. This is
    /// what ranks one seat against another; `total()` stays what a run is *reported* as, so
    /// `orochi runs` and the views keep showing what was actually spent.
    pub fn weighted(&self) -> Option<f64> {
        let total = self.total()? as f64;
        // Without the cache breakdown there is nothing to weigh by, and guessing one would
        // make an old record dearer than the same run recorded today: it reads as its total.
        let Some(cached) = self.cached_tokens else {
            return Some(total);
        };
        let cached = cached as f64;
        let output = self.output_tokens.unwrap_or(0) as f64;
        let created = self.cache_creation_tokens.unwrap_or(0) as f64;
        let fresh = (total - output - cached - created).max(0.0);
        Some(fresh + created * CACHE_WRITE + cached * CACHE_READ + output * OUTPUT)
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
    /// What `prior_tokens` is made of. `learning` reads a run's token ratio only against a
    /// prior made the same way; records from before this existed carry `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_basis: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostFeatures {
    pub cache_discount: f64,
    pub context_restore_tokens: f64,
    pub quota_multiplier: f64,
    /// What opening a session on this seat costs before any work: measured, never discounted,
    /// and 0.0 where nothing has been measured yet. Records written before it existed have none.
    #[serde(default)]
    pub session_tokens: f64,
    /// What a token on this model costs relative to the provider's baseline. Price is not a
    /// token count: it belongs to what a candidate costs and never to `Prediction::tokens`,
    /// which calibration compares against. Records written before it existed read as 1.0.
    #[serde(default = "unit")]
    pub price_multiplier: f64,
}
fn unit() -> f64 {
    1.0
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
