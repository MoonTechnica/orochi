use crate::types::Provider;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub agents: Vec<AgentConfig>,
    pub discovery: DiscoveryConfig,
    pub scheduler: SchedulerConfig,
    pub evaluator: EvaluatorConfig,
    pub router: Option<RouterConfig>,
    pub classifier: ClassifierConfig,
    pub roles: RolesConfig,
    pub memory: MemoryConfig,
    pub learning: LearningConfig,
    pub frontier: Option<RouterConfig>,
    pub council: CouncilConfig,
    pub quota: QuotaConfig,
    pub mailbox: MailboxConfig,
    pub mcp: McpConfig,
    pub activity: ActivityConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            agents: vec![
                AgentConfig::preset("codex", Provider::Openai, "codex-acp", &[]),
                AgentConfig::preset("claude", Provider::Anthropic, "claude-agent-acp", &[]),
                AgentConfig::preset("gemini", Provider::Google, "gemini", &["--acp"]),
                AgentConfig::preset("antigravity", Provider::Google, "agy_acp_server.par", &[]),
            ],
            discovery: DiscoveryConfig::default(),
            scheduler: SchedulerConfig::default(),
            evaluator: EvaluatorConfig::default(),
            router: None,
            classifier: ClassifierConfig::default(),
            roles: RolesConfig::default(),
            memory: MemoryConfig::default(),
            learning: LearningConfig::default(),
            frontier: None,
            council: CouncilConfig::default(),
            quota: QuotaConfig::default(),
            mailbox: MailboxConfig::default(),
            mcp: McpConfig::default(),
            activity: ActivityConfig::default(),
        }
    }
}

/// MCP servers every agent session is given, beside Orochi's own mailbox server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
    /// Also give them the servers the target repository names in its own `.mcp.json`, the way
    /// the CLIs themselves read it. On: a repository that ships one ships it for the agent
    /// working there. It is the one thing Orochi takes from the repository, so every entry
    /// still passes `mcp::check`, the console asks before using one, and a run nobody can be
    /// asked in says which servers came from it.
    pub trust_repository: bool,
    /// Use every repository's servers without asking: Claude Code's
    /// `enableAllProjectMcpServers`, for every project at once.
    pub enable_all_repository_servers: bool,
    /// Keep remote servers' OAuth tokens in the macOS keychain, as Claude Code keeps its own.
    /// Off, or on any other system, they are a `0600` file beside Orochi's data.
    pub keychain: bool,
}
impl Default for McpConfig {
    fn default() -> Self {
        Self {
            servers: vec![],
            trust_repository: true,
            enable_all_repository_servers: false,
            keychain: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
    Sse,
}

/// One MCP server, in the shape `.mcp.json` already uses: a command, or a URL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub transport: McpTransport,
    /// stdio: the executable, absolute or a bare name on PATH.
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// http/sse: the endpoint, and the headers each request to it carries.
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// http/sse: the OAuth client this machine was given by hand, for a server that registers
    /// none on the spot. Empty asks the server to register one (`orochi mcp login`).
    pub oauth_client_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningStrategy {
    Static,
    Ewma,
    Bandit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearningConfig {
    pub strategy: LearningStrategy,
    pub alpha: f64,
    pub prior_weight: f64,
    pub exploration: f64,
    pub max_cost_ratio: f64,
    /// Weight of the same arm's evidence from other task contexts (0 disables pooling).
    pub pooling: f64,
    /// Weight of other models of the same provider and tier on the same kind of work, so
    /// evidence survives a model being replaced (0 disables it). Off until a benchmark replay
    /// shows it helps.
    pub tier_pooling: f64,
}
impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            strategy: LearningStrategy::Ewma,
            alpha: 0.1,
            prior_weight: 16.0,
            exploration: 0.1,
            max_cost_ratio: 1.25,
            pooling: 0.0,
            tier_pooling: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CouncilConfig {
    pub enabled: bool,
    /// Explicit, tool-free OpenAI-compatible advisers. Replies contain candidate IDs only.
    pub members: Vec<RouterConfig>,
}

/// The conversation store a desktop client reads: what was asked, answered, changed and said
/// between agents. It is the one place Orochi keeps the user's own text, so it is also the one
/// place the user can bound or switch off.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityConfig {
    /// `false` keeps nothing: no `activity.sqlite3`, the console behaves as it did before one
    /// existed, and a client sees only telemetry and live peers.
    pub enabled: bool,
    /// Threads untouched for this long are deleted, pinned ones excepted. `0` keeps them
    /// until they are deleted by hand.
    pub retention_days: i64,
    /// Reasoning is the bulkiest thing an agent streams and the least often read back.
    pub thinking: bool,
    /// A headless host exits after this long with nothing queued; the next message starts
    /// another.
    pub host_idle_secs: u64,
}
impl Default for ActivityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
            thinking: true,
            host_idle_secs: 600,
        }
    }
}

/// Messages between agents that separate Orochi processes run in the same repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MailboxConfig {
    pub enabled: bool,
    /// Messages are deleted after this many seconds.
    pub retention_secs: i64,
    pub max_messages_per_hour: u32,
}
impl Default for MailboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_secs: 86_400,
            max_messages_per_hour: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuotaConfig {
    pub refresh_before_run: bool,
    pub timeout_secs: u64,
    pub max_age_secs: i64,
    pub probes: Vec<QuotaProbe>,
}
impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            refresh_before_run: false,
            timeout_secs: 30,
            max_age_secs: 300,
            probes: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaProbe {
    pub agent: String,
    /// Native CLI quota source, or command (versioned Orochi JSON contract).
    pub kind: QuotaProbeKind,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaProbeKind {
    CodexAppServer,
    ClaudeUsage,
    AntigravityUsage,
    Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub id: String,
    pub provider: Provider,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Explicit capabilities absent from ACP's standard schema.
    #[serde(default)]
    pub browser: bool,
    #[serde(default)]
    pub web: bool,
    /// The agent can produce images (not just read them).
    #[serde(default)]
    pub image: bool,
    /// Only consulted as a routing adviser; never discovered or selected to execute tasks.
    #[serde(default)]
    pub routing_only: bool,
}
fn yes() -> bool {
    true
}
impl AgentConfig {
    pub fn preset(id: &str, provider: Provider, command: &str, args: &[&str]) -> Self {
        Self {
            id: id.into(),
            provider,
            command: command.into(),
            args: args.iter().map(|a| (*a).into()).collect(),
            env: BTreeMap::new(),
            enabled: true,
            browser: false,
            web: false,
            image: false,
            routing_only: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscoveryConfig {
    /// Add known coding CLIs unless their agent ID is explicitly configured.
    pub auto_add: bool,
    /// Prepare missing, version-pinned ACP bridges in Orochi's data directory.
    pub auto_install: bool,
    pub setup_timeout_secs: u64,
}
impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            auto_add: true,
            auto_install: true,
            setup_timeout_secs: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulerConfig {
    pub required_success: f64,
    pub routing_confidence: f64,
    pub max_attempts: usize,
    pub discovery_timeout_secs: u64,
    pub prompt_timeout_secs: u64,
    pub cache_ttl_secs: i64,
    pub permission: PermissionMode,
    /// Let several Orochi runs share one working directory (shared instead of exclusive lock).
    pub shared_workspace: bool,
}
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            required_success: 0.7,
            routing_confidence: 0.8,
            max_attempts: 3,
            discovery_timeout_secs: 30,
            prompt_timeout_secs: 1800,
            cache_ttl_secs: 1800,
            permission: PermissionMode::Ask,
            shared_workspace: false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Ask,
    Deny,
    Allow,
}
impl PermissionMode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Deny => "deny",
            Self::Allow => "allow",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluatorConfig {
    pub auto: bool,
    pub timeout_secs: u64,
    pub checks: Vec<CheckCommand>,
}
impl Default for EvaluatorConfig {
    fn default() -> Self {
        Self {
            auto: true,
            timeout_secs: 300,
            checks: vec![],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckCommand {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A routing adviser: an `[[agents]]` entry asked over ACP, in a fresh session without
/// repository access, to choose one supplied candidate. Authentication is the agent CLI's own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterConfig {
    pub agent: String,
    /// Model ID advertised by the agent over ACP. Omitted: the agent's own default model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Includes starting the agent CLI.
    #[serde(default = "router_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "router_fraction")]
    pub max_resource_fraction: f64,
    /// Expected reply size for budgeting; agents cannot be capped mid-turn.
    #[serde(default = "router_output")]
    pub max_output_tokens: u32,
    /// Tokens a session spends beyond the routing request (system prompt, tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overhead_tokens: Option<u64>,
    /// Replacement advisers, in priority order. At most three, no nesting.
    #[serde(default)]
    pub fallbacks: Vec<RouterConfig>,
}
impl RouterConfig {
    pub fn choices(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self).chain(self.fallbacks.iter())
    }
    /// Conservative default for an agent session; measure and override per adviser.
    pub fn overhead(&self) -> usize {
        self.session_overhead_tokens
            .unwrap_or(20_000)
            .min(10_000_000) as usize
    }
}
/// Classifying the request is what makes routing read what was asked rather than which words
/// it happens to contain, so it runs by default. Without `agent` every enabled agent is tried
/// in configured order until one answers; a failure only ever leaves the local heuristic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClassifierConfig {
    pub enabled: bool,
    /// An `[[agents]]` ID. Omitted: whichever enabled agent answers first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    pub timeout_secs: u64,
    pub max_output_tokens: u32,
    /// The largest share of what work like this has cost here that asking what it is may take.
    /// Asking is a whole session; where it would cost as much as the work it decides, the
    /// local profile is kept instead. Orochi asks until it has measured both sides. Both sides
    /// are weighted costs. Asking costs what a cold session costs and no more can be done
    /// about it: measured on 2026-09-24, a classification on `codex` spent 24,262 weighted
    /// tokens of which 23,982 were uncached input, against 20,441 on `haiku` -- it is the
    /// system prompt, not the model, and an earlier guess that better routing would make the
    /// question cheap was wrong. What the share is taken against is what was wrong: see
    /// `classifier::worth_asking`, which weighs it against this task's own size.
    pub max_cost_share: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_overhead_tokens: Option<u64>,
}
impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            agent: None,
            model: None,
            reasoning: None,
            // Short: this runs before every turn, and giving up only costs the heuristic.
            timeout_secs: 60,
            max_output_tokens: 512,
            max_cost_share: 0.25,
            session_overhead_tokens: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RolesConfig {
    /// How much work like this must have cost here before a second seat joins it, as
    /// `Usage::weighted()` counts it. A seat is another session beside the first, so it roughly
    /// doubles the turn; below about twice a bare session there is too little work for a second
    /// reading to pay for that. Orochi's own heuristic, from sessions measured at 21,882 and
    /// 37,630 tokens on 2026-09-20 and carried over to weighted costs at the measured ratio
    /// (2026-09-22, 31 runs: raw is 3.73x weighted, so 60,000 raw is about 16,000).
    pub min_work_tokens: u64,
}
impl Default for RolesConfig {
    fn default() -> Self {
        Self {
            min_work_tokens: 16_000,
        }
    }
}

impl ClassifierConfig {
    /// One adviser seat for `agent`, carrying this section's settings.
    pub fn seat(&self, agent: &str) -> RouterConfig {
        RouterConfig {
            agent: agent.to_owned(),
            model: self.model.clone(),
            reasoning: self.reasoning.clone(),
            timeout_secs: self.timeout_secs,
            max_resource_fraction: 1.0,
            max_output_tokens: self.max_output_tokens,
            session_overhead_tokens: self.session_overhead_tokens,
            fallbacks: vec![],
        }
    }
}

/// What Orochi remembers between sessions (`memory.rs`). The budgets bound what is put in
/// front of every fresh agent session, which is paid for on every one of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub user_chars: usize,
    pub repo_chars: usize,
}
impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            user_chars: 1500,
            repo_chars: 2500,
        }
    }
}

fn router_timeout() -> u64 {
    120
}
fn router_fraction() -> f64 {
    1.0
}
fn router_output() -> u32 {
    512
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config: Self = if path.exists() {
            toml::from_str(&std::fs::read_to_string(path)?)
                .map_err(|error| {
                    let text = error.to_string();
                    let legacy = ["endpoint", "api_key_env", "acp"]
                        .iter()
                        .any(|f| text.contains(&format!("unknown field `{f}`")))
                        || text.contains("missing field `agent`");
                    if legacy {
                        anyhow::anyhow!(
                            "{error}\nHTTP routing advisers were removed: set `agent` (an [[agents]] ID) and `model` for [router], [frontier] and council members (see docs/adaptive-routing.md)"
                        )
                    } else {
                        error.into()
                    }
                })
                .with_context(|| format!("invalid config: {}", path.display()))?
        } else {
            Self::default()
        };
        if config.discovery.auto_add {
            for preset in Self::default().agents {
                if !config.agents.iter().any(|agent| agent.id == preset.id) {
                    config.agents.push(preset);
                }
            }
        }
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for agent in &self.agents {
            ensure!(
                !agent.id.is_empty()
                    && agent
                        .id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "invalid agent id"
            );
            ensure!(ids.insert(&agent.id), "duplicate agent id: {}", agent.id);
            ensure!(!agent.command.trim().is_empty(), "agent command is empty");
            ensure!(
                Path::new(&agent.command).is_absolute()
                    || Path::new(&agent.command).components().count() == 1,
                "use an absolute path for agent executables outside PATH"
            );
        }
        let s = &self.scheduler;
        ensure!(
            (0.0..=1.0).contains(&s.required_success) && s.required_success > 0.0,
            "required_success must be in (0, 1]"
        );
        ensure!(
            (0.0..=1.0).contains(&s.routing_confidence),
            "routing_confidence must be in [0, 1]"
        );
        ensure!(
            (1..=10).contains(&s.max_attempts),
            "max_attempts must be in 1..=10"
        );
        ensure!(
            s.discovery_timeout_secs > 0
                && s.prompt_timeout_secs > 0
                && s.cache_ttl_secs >= 0
                && self.evaluator.timeout_secs > 0,
            "invalid timeout"
        );
        ensure!(
            self.discovery.setup_timeout_secs > 0,
            "invalid adapter setup timeout"
        );
        ensure!(
            (0.0..=100.0).contains(&self.classifier.max_cost_share),
            "classifier.max_cost_share must be in [0, 100]"
        );
        ensure!(
            self.roles.min_work_tokens > 0,
            "roles.min_work_tokens must be above zero"
        );
        for check in &self.evaluator.checks {
            ensure!(
                !check.name.is_empty() && !check.command.is_empty(),
                "check name/command is empty"
            );
        }
        let learning = &self.learning;
        ensure!(
            learning.alpha.is_finite() && learning.alpha > 0.0 && learning.alpha <= 1.0,
            "learning.alpha must be in (0, 1]"
        );
        ensure!(
            learning.prior_weight.is_finite() && learning.prior_weight > 0.0,
            "invalid learning.prior_weight"
        );
        ensure!(
            (0.0..=1.0).contains(&learning.exploration),
            "invalid learning.exploration"
        );
        ensure!(
            (0.0..=1.0).contains(&learning.pooling) && (0.0..=1.0).contains(&learning.tier_pooling),
            "learning.pooling and learning.tier_pooling must be in [0, 1]"
        );
        ensure!(
            learning.max_cost_ratio.is_finite() && (1.0..=2.0).contains(&learning.max_cost_ratio),
            "learning.max_cost_ratio must be in [1, 2]"
        );
        ensure!(
            !self.council.enabled || self.council.members.len() >= 2,
            "enabled council requires at least two members"
        );
        ensure!(
            self.council.members.len() <= 4,
            "council supports at most four members"
        );
        ensure!(
            self.quota.timeout_secs > 0 && self.quota.max_age_secs > 0,
            "invalid quota timeout/age"
        );
        ensure!(
            (60..=30 * 86_400).contains(&self.mailbox.retention_secs)
                && (1..=1000).contains(&self.mailbox.max_messages_per_hour),
            "mailbox.retention_secs must be 60..=2592000 and max_messages_per_hour 1..=1000"
        );
        let mut servers = std::collections::BTreeSet::new();
        for server in &self.mcp.servers {
            crate::mcp::check(server)?;
            ensure!(
                servers.insert(&server.name),
                "duplicate MCP server: {}",
                server.name
            );
        }
        ensure!(
            (0..=3650).contains(&self.activity.retention_days),
            "activity.retention_days must be 0..=3650 (0 keeps threads until deleted)"
        );
        ensure!(
            (60..=86_400).contains(&self.activity.host_idle_secs),
            "activity.host_idle_secs must be 60..=86400"
        );
        let mut probes = std::collections::BTreeSet::new();
        for probe in &self.quota.probes {
            ensure!(
                ids.contains(&probe.agent),
                "unknown quota agent: {}",
                probe.agent
            );
            ensure!(probes.insert(&probe.agent), "duplicate quota probe");
            ensure!(
                !probe.command.trim().is_empty()
                    && (Path::new(&probe.command).is_absolute()
                        || Path::new(&probe.command).components().count() == 1),
                "invalid quota command"
            );
        }
        ensure!(
            self.memory.user_chars <= 20_000 && self.memory.repo_chars <= 20_000,
            "memory budgets must be at most 20000 characters"
        );
        let classifier = &self.classifier;
        ensure!(
            classifier
                .agent
                .as_ref()
                .is_none_or(|id| self.agents.iter().any(|a| &a.id == id && a.enabled)),
            "classifier must name an enabled [[agents]] entry"
        );
        ensure!(
            classifier.timeout_secs > 0
                && classifier.max_output_tokens > 0
                && classifier.max_output_tokens <= 4096
                && classifier.model.as_deref().is_none_or(|m| !m.is_empty()),
            "invalid classifier settings"
        );
        for primary in self
            .router
            .iter()
            .chain(self.frontier.iter())
            .chain(self.council.members.iter())
        {
            ensure!(
                primary.fallbacks.len() <= 3
                    && primary.fallbacks.iter().all(|r| r.fallbacks.is_empty()),
                "router supports at most three non-nested fallbacks"
            );
            for r in primary.choices() {
                ensure!(
                    self.agents.iter().any(|a| a.id == r.agent && a.enabled),
                    "routing adviser must name an enabled [[agents]] entry: {}",
                    r.agent
                );
                ensure!(
                    (0.0..=1.0).contains(&r.max_resource_fraction) && r.max_resource_fraction > 0.0,
                    "invalid router resource fraction"
                );
                ensure!(
                    r.max_output_tokens > 0
                        && r.max_output_tokens <= 4096
                        && r.timeout_secs > 0
                        && r.model.as_deref().is_none_or(|m| !m.is_empty()),
                    "invalid router settings"
                );
            }
        }
        Ok(())
    }
}

pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
}
impl Paths {
    pub fn resolve(config: Option<PathBuf>, data: Option<PathBuf>) -> Result<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let config_root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|p| p.join(".config")));
        let data_root = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|p| p.join(".local/share")));
        let config = config.or_else(|| config_root.map(|p| p.join("orochi/config.toml")));
        let data = data.or_else(|| data_root.map(|p| p.join("orochi")));
        match (config, data) {
            (Some(config), Some(data)) => Ok(Self { config, data }),
            _ => bail!("set --config and --data-dir when HOME is unavailable"),
        }
    }
}
