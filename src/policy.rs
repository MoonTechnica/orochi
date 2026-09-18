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
    pub task_profiles: BTreeMap<String, String>,
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
        for provider in [Provider::Openai, Provider::Anthropic, Provider::Google] {
            ensure!(
                self.policies
                    .iter()
                    .filter(|p| p.provider == provider)
                    .count()
                    == 1,
                "policy must contain exactly one entry for {provider}"
            );
        }
        ensure!(self.policies.len() == 3, "unexpected policy count");
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
                    m.success_prior
                        .iter()
                        .all(|s| s.is_finite() && *s > 0.0 && *s <= 1.0),
                    "invalid success prior"
                );
            }
        }
        Ok(())
    }
    pub fn get(&self, provider: Provider) -> &ProviderPolicy {
        self.policies
            .iter()
            .find(|p| p.provider == provider)
            .expect("validated registry")
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
    pub fn reasoning(
        &self,
        complexity: Complexity,
        available: &[String],
        current: Option<&str>,
        model: &str,
    ) -> Option<String> {
        self.reasoning_rules[complexity.key()]
            .iter()
            .find(|r| available.contains(r) && self.permits(model, Some(r)))
            .cloned()
            .or_else(|| {
                current
                    .filter(|r| self.permits(model, Some(r)))
                    .map(String::from)
            })
            .or_else(|| {
                available
                    .iter()
                    .find(|r| self.permits(model, Some(r)))
                    .cloned()
            })
    }
}
