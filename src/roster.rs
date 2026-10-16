//! The roster: which of each agent's models the scheduler may use. Chosen by hand
//! (`orochi roster set`, the window's Agents screen) or proposed by an agent from what each
//! model is for and what it has measured here (`orochi roster suggest`), written down as the
//! decision and adjusted by the user afterwards. It lives in the configuration, as
//! `[[agents]] models`, and is a hard gate before scoring: what is off the roster is never
//! brought back by a preference, an adviser or the bandit.
use crate::{
    config::Config,
    discovery,
    memory::{Memory, Scope, Update},
    policy::Registry,
    storage::Store,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

/// One model an agent advertised, as the roster sees it.
#[derive(Debug, Clone, Serialize)]
pub struct KnownModel {
    pub id: String,
    pub chosen: bool,
    pub tier: String,
    pub about: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub provider: String,
    pub enabled: bool,
    pub routing_only: bool,
    /// The patterns as written; empty is every model.
    pub models: Vec<String>,
    pub known: Vec<KnownModel>,
    pub discovered_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub agents: Vec<Entry>,
}

pub fn listing(config: &Config, policies: &Registry, data: &Path) -> Listing {
    let known = discovery::known(data);
    let agents = config
        .agents
        .iter()
        .map(|agent| {
            let policy = policies.get(agent.provider);
            let seen = known.get(&agent.id);
            Entry {
                id: agent.id.clone(),
                provider: agent.provider.to_string(),
                enabled: agent.enabled,
                routing_only: agent.routing_only,
                models: agent.models.clone(),
                known: seen
                    .map(|k| {
                        k.models
                            .iter()
                            .map(|id| {
                                let rule = policy.model_rule(id);
                                KnownModel {
                                    id: id.clone(),
                                    chosen: agent.admits(id),
                                    tier: rule.tier.clone(),
                                    about: rule.describe(),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                discovered_at: seen.map(|k| k.at),
            }
        })
        .collect();
    Listing { agents }
}

/// Sets one agent's roster by hand. An empty list is every model; `off` takes the agent out
/// of the field altogether, and setting a roster at all, even the empty one, puts it back in:
/// choosing what an agent may run is wanting it in the field.
pub fn set(config: &mut Config, agent: &str, models: &[String], off: bool) -> Result<()> {
    let entry = config
        .agents
        .iter_mut()
        .find(|a| a.id == agent)
        .with_context(|| format!("no agent {agent}"))?;
    let mut patterns: Vec<String> = models
        .iter()
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .collect();
    patterns.dedup();
    entry.models = patterns;
    entry.enabled = !off;
    config.validate()
}

/// What a proposal is asked for. The list it is given is content-free but for what is
/// remembered, which the classifier is already shown: agents, their models with what each is
/// for and what it costs, and what each route has measured here.
pub const SUGGEST: &str = "You are choosing which models a routing scheduler may use on this machine. Do not read files, run commands or use any tool. \"agents\" lists each agent, the models it advertised, what each model is for and its price relative to the others (under 0.3 is nearly free: a model running on this machine, or a free tier), and what each has measured here (verified runs and their success rate, by kind of task). \"remembered\" is what the user has said they want. \"current\" is the roster as it stands. Reply ONLY with a JSON object:\n\
{\"roster\":{\"<agent>\":[\"<model id>\"]},\"preferences\":[{\"text\":\"...\",\"scope\":\"user\"}],\"why\":\"...\"}\n\n\
roster: for every agent, the models worth keeping, copied exactly from its own list and never a name that is not there, at least one per agent. Keep one cheap model and one capable model, keep a model that runs on this machine, keep what has measured well here, and drop the rest: older versions, duplicates of one tier, and variants nothing distinguishes.\n\
preferences: at most three short sentences, each saying what kind of work should go to which agent or model and why, drawn from the measurements and from what is remembered; in the user's language where they have written any. None where nothing supports one, and never one that repeats what is already remembered.\n\
why: one sentence on what decided it.";

pub fn request(config: &Config, policies: &Registry, store: &Store, remembered: &Value) -> String {
    let data = store.data_dir();
    let known = discovery::known(data);
    let measured = measured(store).unwrap_or_default();
    let agents: Vec<Value> = config
        .agents
        .iter()
        .filter(|a| a.enabled && !a.routing_only)
        .map(|agent| {
            let policy = policies.get(agent.provider);
            let models: Vec<Value> = known
                .get(&agent.id)
                .map(|k| k.models.clone())
                .unwrap_or_default()
                .iter()
                .map(|id| {
                    let rule = policy.model_rule(id);
                    let mut model = json!({
                        "id": id,
                        "tier": rule.tier,
                        "about": rule.describe(),
                        "price": rule.relative_tokens * rule.price(),
                    });
                    let runs: Vec<&Value> = measured
                        .iter()
                        .filter(|m| m["agent"] == agent.id && m["model"] == *id)
                        .collect();
                    if !runs.is_empty() {
                        model["measured"] = json!(runs);
                    }
                    model
                })
                .collect();
            json!({
                "id": agent.id,
                "provider": agent.provider.to_string(),
                "web": agent.web, "browser": agent.browser, "image": agent.image,
                "models": models,
            })
        })
        .collect();
    let current: BTreeMap<&str, &Vec<String>> = config
        .agents
        .iter()
        .filter(|a| !a.models.is_empty())
        .map(|a| (a.id.as_str(), &a.models))
        .collect();
    format!(
        "{SUGGEST}\n\n{}",
        json!({"agents": agents, "remembered": remembered, "current": current})
    )
}

/// What each route has done here, per kind of task: verified runs, their success rate and
/// what they cost. Nothing in a row is content.
fn measured(store: &Store) -> Result<Vec<Value>> {
    let connection = store.connection();
    let mut statement = connection.prepare(
        "SELECT agent, model, task_type, verified, success_rate, mean_tokens
         FROM v_route_stats WHERE verified > 0 ORDER BY verified DESC LIMIT 80",
    )?;
    let rows = statement.query_map([], |r| {
        Ok(json!({
            "agent": r.get::<_, String>(0)?,
            "model": r.get::<_, String>(1)?,
            "task": r.get::<_, String>(2)?,
            "verified": r.get::<_, i64>(3)?,
            "success_rate": r.get::<_, f64>(4)?,
            "mean_tokens": r.get::<_, f64>(5)?,
        }))
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Suggestion {
    /// Per agent, model IDs copied from what it advertised; an agent left out keeps its roster.
    #[serde(default)]
    pub roster: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub preferences: Vec<Preference>,
    #[serde(default)]
    pub why: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub text: String,
    #[serde(default)]
    pub scope: Option<String>,
}
const MAX_PREFERENCES: usize = 3;

/// Reads a reply, keeping only model IDs the agent actually advertised: a name the reply made
/// up is dropped, and an agent left with nothing keeps the roster it had.
pub fn parse(reply: &str, known: &BTreeMap<String, discovery::Known>) -> Option<Suggestion> {
    let mut suggestion: Suggestion = crate::context::trailing_json(reply)?;
    suggestion.roster = suggestion
        .roster
        .into_iter()
        .filter_map(|(agent, models)| {
            let advertised = &known.get(&agent)?.models;
            let mut kept: Vec<String> = models
                .into_iter()
                .filter(|m| advertised.contains(m))
                .collect();
            kept.dedup();
            (!kept.is_empty()).then_some((agent, kept))
        })
        .collect();
    suggestion.preferences = suggestion
        .preferences
        .into_iter()
        .map(|p| Preference {
            text: crate::memory::clean(&p.text, 200),
            scope: p.scope,
        })
        .filter(|p| !p.text.is_empty())
        .take(MAX_PREFERENCES)
        .collect();
    suggestion.why = suggestion
        .why
        .map(|w| crate::memory::clean(&w, 200))
        .filter(|w| !w.is_empty());
    Some(suggestion)
}

/// Asks for a proposal. Every agent must have been enumerated here once (`discovered.json`),
/// or there is nothing to choose from.
pub async fn suggest(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
) -> Result<Suggestion> {
    let known = discovery::known(store.data_dir());
    ensure!(
        config
            .agents
            .iter()
            .any(|a| a.enabled && !a.routing_only && known.contains_key(&a.id)),
        "no agent has been discovered here yet; run `orochi agents --discover` first"
    );
    let remembered = Memory::open(store.data_dir(), &config.memory)
        .zip(store.repository_id(root).ok())
        .map(|(memory, repository)| memory.listing(&repository, crate::types::now()))
        .unwrap_or_else(|| json!({}));
    let text = request(config, policies, store, &remembered);
    let reply = crate::router::classifier::consult_as(
        config, policies, store, root, "roster", SUGGEST, &text,
    )
    .await?;
    parse(&reply, &known).context("the reply was not a proposal")
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Applied {
    /// Agents whose roster changed, with what it is now.
    pub rosters: BTreeMap<String, Vec<String>>,
    pub remembered: Vec<String>,
}

/// Writes a proposal down as the decision: the rosters into the configuration, exact IDs as
/// proposed, and the preferences into memory as lines Orochi wrote, so an unheeded one fades
/// as any of Orochi's own do and one the user keeps becomes theirs. The agent is never
/// switched off by a proposal.
pub fn apply(
    config: &mut Config,
    suggestion: &Suggestion,
    memory: Option<(&Memory, &str)>,
    now: i64,
) -> Result<Applied> {
    let mut applied = Applied::default();
    for (agent, models) in &suggestion.roster {
        if let Some(entry) = config.agents.iter_mut().find(|a| &a.id == agent)
            && entry.models != *models
        {
            entry.models = models.clone();
            applied.rosters.insert(agent.clone(), models.clone());
        }
    }
    config.validate()?;
    if let Some((memory, repository)) = memory {
        for preference in &suggestion.preferences {
            let scope = match preference.scope.as_deref() {
                Some("repo") => Scope::Repo,
                _ => Scope::User,
            };
            let update = Update {
                remember: vec![(scope, preference.text.clone())],
                ..Update::default()
            };
            applied
                .remembered
                .extend(memory.apply(repository, &update, now)?);
        }
    }
    Ok(applied)
}
