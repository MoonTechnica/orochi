use crate::types::{Complexity, Provider};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderPolicy {
    pub provider: Provider,
    pub version: u32,
    pub source: Vec<String>,
    pub updated_at: String,
    pub models: Vec<ModelRule>,
    /// Retired: it listed task types per complexity, and nothing ever read it. Still accepted, so
    /// an installed or published registry that carries it keeps loading.
    #[serde(default, skip_serializing, rename = "task_profiles")]
    _task_profiles: Option<serde::de::IgnoredAny>,
    pub reasoning_rules: BTreeMap<String, Vec<String>>,
    pub context_rules: ContextRules,
    pub cache_rules: CacheRules,
    pub hard_constraints: Vec<HardConstraint>,
    pub fallback_rules: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRule {
    /// Case-insensitive substring; never used to invent undiscovered model IDs.
    pub pattern: String,
    pub tier: String,
    pub relative_tokens: f64,
    pub success_prior: [f64; 4],
    /// How much more a token here costs than `relative_tokens` already assumes. Two models of
    /// one tier can be a factor apart in price and identical in everything else, and no token
    /// count says so (Anthropic 2026-09-24: Claude Fable 5.1 at $10/MTok against Claude Opus
    /// 5.5 at $4, both frontier). It is a correction and not a price list: 1.0, the default, leaves a
    /// model priced exactly as it was before this field existed, so only a model whose price
    /// departs from its tier needs one. It multiplies the cost and never the token
    /// prediction, which the EWMA learns from and calibration compares against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_premium: Option<f64>,
    /// What this model is for, in a sentence, for the one adviser that reads the task. It
    /// carries what the numbers cannot: two frontier models with the same prior and the same
    /// price still differ in what they are good at. It is the provider's own account of what
    /// the model is for, not Orochi's opinion of it. Absent, `describe` generates one from the
    /// tier and the price, so a model nobody has written about is still described.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_for: Option<String>,
}
impl ModelRule {
    /// The cost correction to apply on top of the token prior; neutral unless stated.
    pub fn price(&self) -> f64 {
        self.price_premium.unwrap_or(1.0)
    }
    /// The line the classifier is shown for this rule. A rule that says nothing is described
    /// from what it does say, so adding a model to a provider never means writing prose.
    pub fn describe(&self) -> String {
        if let Some(use_for) = &self.use_for {
            return use_for.clone();
        }
        let price = match self.relative_tokens * self.price() {
            c if c >= 1.5 => "costly",
            c if c <= 0.7 => "cheap",
            _ => "mid-priced",
        };
        match self.tier.as_str() {
            "frontier" => format!("{price}; the provider's most capable tier"),
            "standard" => format!("{price}; capable enough for most work"),
            "small" => format!("{price}; for small, well-specified work"),
            _ => format!("{price}; not characterised here, treat as standard"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardConstraint {
    pub model_pattern: String,
    pub forbidden_reasoning: Vec<String>,
    #[serde(default)]
    pub disabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRules {
    pub prefer_envelope: bool,
    pub thinking: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRules {
    pub affinity_discount: f64,
    pub stable_prefix: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    pub policies: Vec<ProviderPolicy>,
}

impl Registry {
    pub fn bundled() -> Result<Self> {
        let policies = [
            include_str!("../policies/openai.json"),
            include_str!("../policies/anthropic.json"),
            include_str!("../policies/google.json"),
        ]
        .iter()
        .map(|p| serde_json::from_str(p))
        .collect::<Result<Vec<_>, _>>()?;
        let registry = Self {
            schema_version: 1,
            policies,
        };
        registry.validate()?;
        Ok(registry)
    }
    pub fn load(data: &Path) -> Result<Self> {
        let path = data.join("policies.json");
        let registry = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)
                .context("invalid installed policy registry")?
        } else {
            Self::bundled()?
        };
        Self::validate(&registry)?;
        Ok(registry)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported policy schema version"
        );
        // One entry per provider, and the set of providers is open: a registry may carry a
        // vendor Orochi ships nothing for, and a provider with no entry is priced by
        // `ProviderPolicy::generic` rather than being unroutable.
        ensure!(!self.policies.is_empty(), "policy registry is empty");
        ensure!(self.policies.len() <= 64, "unexpected policy count");
        let mut providers = std::collections::BTreeSet::new();
        for p in &self.policies {
            ensure!(
                providers.insert(p.provider),
                "policy must contain exactly one entry for {}",
                p.provider
            );
        }
        for p in &self.policies {
            ensure!(
                p.version > 0 && !p.source.is_empty() && !p.updated_at.is_empty(),
                "missing policy provenance"
            );
            for source in &p.source {
                ensure!(
                    source.starts_with("https://"),
                    "policy sources must be HTTPS"
                );
            }
            for complexity in [
                Complexity::Simple,
                Complexity::Normal,
                Complexity::Complex,
                Complexity::Extreme,
            ] {
                ensure!(
                    p.reasoning_rules
                        .get(complexity.key())
                        .is_some_and(|v| !v.is_empty()),
                    "missing reasoning rule"
                );
            }
            ensure!(
                (0.0..=0.5).contains(&p.cache_rules.affinity_discount),
                "invalid cache discount"
            );
            ensure!(
                p.models.iter().any(|m| m.pattern == "*"),
                "policy must have a fallback model prior"
            );
            for m in &p.models {
                ensure!(
                    !m.pattern.is_empty()
                        && m.relative_tokens.is_finite()
                        && m.relative_tokens > 0.0
                        && m.relative_tokens <= 100.0,
                    "invalid model prior"
                );
                ensure!(
                    m.price_premium
                        .is_none_or(|c| c.is_finite() && c > 0.0 && c <= 100.0),
                    "invalid price premium"
                );
                ensure!(
                    m.use_for.as_ref().is_none_or(|u| {
                        let u = u.trim();
                        !u.is_empty() && u.chars().count() <= 200
                    }),
                    "invalid model description"
                );
                ensure!(
                    m.success_prior
                        .iter()
                        .all(|s| s.is_finite() && *s > 0.0 && *s <= 1.0),
                    "invalid success prior"
                );
            }
        }
        Ok(())
    }
    /// The policy for a provider, or the neutral one where nobody has written it a policy. An
    /// ACP agent from a vendor Orochi ships nothing for is routable, priced by what is known
    /// about it -- which is nothing, so it is priced as an unknown model of a known provider is.
    pub fn get(&self, provider: Provider) -> &ProviderPolicy {
        self.policies
            .iter()
            .find(|p| p.provider == provider)
            .unwrap_or_else(|| ProviderPolicy::generic())
    }
    pub fn install(&self, data: &Path) -> Result<()> {
        self.validate()?;
        std::fs::create_dir_all(data)?;
        let mut file = tempfile::NamedTempFile::new_in(data)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(data.join("policies.json"))?;
        Ok(())
    }
}

impl ProviderPolicy {
    /// What is assumed about a provider nobody has published anything about: the same
    /// ignorance every policy already spells out for a model it does not recognise (its `*`
    /// row, identical in all three bundled policies), the canonical effort rungs -- which
    /// `effort::Ladder` translates into whatever words the agent uses -- and no cache discount,
    /// because a discount nobody has measured is a cost that has been made up. Version 0 says
    /// as much wherever a route is explained. It is never part of a `Registry`, so it is not
    /// what `validate` requires provenance of; there is no provider here to cite.
    pub fn generic() -> &'static Self {
        static GENERIC: std::sync::LazyLock<ProviderPolicy> =
            std::sync::LazyLock::new(|| ProviderPolicy {
                provider: Provider::new("unknown").expect("valid name"),
                version: 0,
                source: vec![],
                updated_at: String::new(),
                models: vec![ModelRule {
                    pattern: "*".into(),
                    tier: "unknown".into(),
                    relative_tokens: 1.2,
                    success_prior: [0.86, 0.78, 0.7, 0.5],
                    price_premium: None,
                    use_for: None,
                }],
                _task_profiles: None,
                reasoning_rules: [
                    ("simple", ["low", "minimal", "medium"]),
                    ("normal", ["medium", "low", "high"]),
                    ("complex", ["high", "medium", "xhigh"]),
                    ("extreme", ["xhigh", "high", "medium"]),
                ]
                .into_iter()
                .map(|(complexity, levels)| {
                    (
                        complexity.to_owned(),
                        levels.iter().map(|l| (*l).to_owned()).collect(),
                    )
                })
                .collect(),
                context_rules: ContextRules {
                    prefer_envelope: true,
                    thinking: "agent_default".into(),
                },
                cache_rules: CacheRules {
                    affinity_discount: 0.0,
                    stable_prefix: false,
                },
                hard_constraints: vec![],
                fallback_rules: vec![
                    "exclude_cooldown".into(),
                    "preserve_filesystem".into(),
                    "require_success_floor".into(),
                ],
            });
        &GENERIC
    }
}

fn matches(pattern: &str, model: &str) -> bool {
    pattern == "*" || model.to_lowercase().contains(&pattern.to_lowercase())
}
impl ProviderPolicy {
    pub fn model_rule(&self, model: &str) -> &ModelRule {
        self.models
            .iter()
            .filter(|r| r.pattern != "*" && matches(&r.pattern, model))
            .max_by_key(|r| r.pattern.len())
            .or_else(|| self.models.iter().find(|r| r.pattern == "*"))
            .expect("validated fallback")
    }
    pub fn permits(&self, model: &str, reasoning: Option<&str>) -> bool {
        !self.hard_constraints.iter().any(|rule| {
            matches(&rule.model_pattern, model)
                && (rule.disabled
                    || reasoning.is_some_and(|r| rule.forbidden_reasoning.iter().any(|v| v == r)))
        })
    }
    /// Which effort level to ask this model for. The policy's list for a complexity is an
    /// ordered set of rungs, not a set of words: where the agent speaks the provider's
    /// vocabulary it is matched literally, and where it does not, the same rungs are looked for
    /// in the agent's own words (`effort::Ladder`). Without that translation a vocabulary the
    /// policy never named left every complexity on whatever level the agent happened to open
    /// with, which is the effort axis doing nothing at all.
    pub fn reasoning(
        &self,
        complexity: Complexity,
        available: &[String],
        current: Option<&str>,
        model: &str,
    ) -> Option<String> {
        let rules = &self.reasoning_rules[complexity.key()];
        let permitted = |r: &str| self.permits(model, Some(r));
        if let Some(named) = rules
            .iter()
            .find(|r| available.contains(r) && permitted(r))
            .cloned()
        {
            return Some(named);
        }
        let ladder = crate::effort::Ladder::new(available);
        for rung in rules.iter().filter_map(|r| crate::effort::rank(r)) {
            if let Some(found) = ladder.at(rung).find(|v| permitted(v)) {
                return Some(found.to_owned());
            }
        }
        // The rung the policy reaches for first is absent here; the nearest one stands in.
        if let Some(target) = rules.iter().find_map(|r| crate::effort::rank(r))
            && let Some(found) = ladder.nearest(target).into_iter().find(|v| permitted(v))
        {
            return Some(found.to_owned());
        }
        current
            .filter(|r| permitted(r))
            .map(String::from)
            .or_else(|| available.iter().find(|r| permitted(r)).cloned())
    }
}
