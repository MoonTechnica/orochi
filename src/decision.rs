//! Typed decisions about a request, answered by a decision model.
//!
//! A decision model reads the request and answers named questions with probabilities and no
//! text (`choice`, `score`, `noul`), so there is nothing to parse and nothing to stream. It
//! runs on this machine or in the cloud, and either way a model is chosen from those offered
//! there (`MODELS`). Here it is a `llama-server` Orochi puts in and keeps running
//! (`decision/local.rs`, llama.cpp since ggml-org/llama.cpp#29818), asked over the System One
//! API (`POST /v1/systemone`); in the cloud each provider is reached with its own key
//! (`PROVIDERS`): TypeSafe speaks System One, OpenAI its Decisions API, the same three
//! question types under other names. Only a model's own maker is a cloud provider: TypeSafe
//! for Jev, OpenAI for gpt-6-luna. lev, the one recommended, runs only here.
//!
//! This replaces asking a coding agent in an ACP session of its own before every turn
//! (`router::classifier`), which cost a whole session start — measured 2026-10-15 at 31 s
//! and 815 tokens on the leanest path — against a few hundred milliseconds here. A decision
//! model writes nothing, so what needs words (a new memory line, a roster proposal) stays
//! with an agent, at the end of a session (`classifier::distill`) or when asked for.
//!
//! The task text goes to the decision model and nowhere else; the cache holds labels keyed by
//! the same salted hash the classifier used, and the cloud backend is chosen, never assumed.
pub mod choose;
pub mod key;
pub mod local;

use crate::{
    acp::Progress,
    config::{Config, DecisionBackend, DecisionConfig},
    memory::Memory,
    policy::Registry,
    router::classifier::{self, Classification, TASK_TYPES},
    storage::Store,
    types::{Complexity, TaskDescriptor, now},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

/// A decision model Orochi knows how to put in or reach.
#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub name: &'static str,
    pub summary: &'static str,
    pub license: &'static str,
    /// The GGUF a local server loads, pinned to a revision and its SHA-256.
    pub gguf: Option<Gguf>,
    /// Where the same model is offered in the cloud, and what that provider calls it.
    pub cloud: Option<Cloud>,
    /// Where a `complexity` score between 0 and 3 is cut into the four levels. A model's
    /// scores have its own spread, so the cuts are the model's, read from measurements.
    pub cuts: [f64; 3],
    /// What a model needs in memory to run locally, roughly, for the recommendation.
    pub memory_gb: u64,
    /// What was measured here on the 30 requests of `docs/real-validation-20261108.md`, as
    /// (kind of work right, of), where it was.
    pub measured: Option<(u32, u32)>,
}
#[derive(Debug, Clone, Copy)]
pub struct Gguf {
    pub url: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}
#[derive(Debug, Clone, Copy)]
pub struct Cloud {
    pub provider: &'static str,
    pub id: &'static str,
}

/// How a provider is asked: the System One API (llama.cpp, TypeSafe) or OpenAI's
/// Decisions API, which has the same three question types under other names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Api {
    SystemOne,
    OpenAiDecisions,
}

/// A cloud provider of decision models, each with its own key. Only a model's own maker is
/// listed, never a reseller (the user's rule, 2026-11-09): kev, whose only host is OpenRouter,
/// runs on this machine and nowhere else.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Provider {
    pub name: &'static str,
    pub label: &'static str,
    pub endpoint: &'static str,
    pub api: Api,
    /// Read before the stored key.
    pub key_env: &'static str,
    /// Where a key is made.
    pub key_url: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        name: "typesafe",
        label: "TypeSafe",
        endpoint: "https://api.typesafe.ai/v1/systemone",
        api: Api::SystemOne,
        key_env: "TYPESAFE_API_KEY",
        key_url: "https://console.typesafe.ai",
    },
    Provider {
        name: "openai",
        label: "OpenAI",
        endpoint: "https://api.openai.com/v1/decisions",
        api: Api::OpenAiDecisions,
        key_env: "OPENAI_API_KEY",
        key_url: "https://platform.openai.com/api-keys",
    },
];

pub fn provider(name: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.name == name)
}

/// Every model on offer, each with what was measured of it here on 30 requests in Japanese and
/// English (`docs/real-validation-20261108.md`), or nothing where nothing was. The keyword
/// profile labelled the kind of work 11 times; with no fine-tuning lev managed 26 and kev-4b
/// 25, which is why lev is what a machine that can run it is recommended. Nothing is
/// recommended in the cloud: that is the user's choice of provider (2026-11-09).
pub const RECOMMENDED: &str = "lev";

pub const MODELS: &[Model] = &[
    Model {
        name: "lev",
        summary: "lev, Interfaze's decision model",
        license: "Apache-2.0",
        gguf: Some(Gguf {
            url: "https://huggingface.co/ggml-org/lev-GGUF/resolve/2ef3b3c3ab3defcab1058ab34cac52c467fa9304/lev-Q4_K_M.gguf",
            file: "lev-Q4_K_M.gguf",
            sha256: "bff94af408cc9ea616dd2543abf77afacefeec27d32d635ebe0ed0eb7ccd7efe",
            bytes: 3_032_093_632,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 8,
        measured: Some((26, 30)),
    },
    Model {
        name: "kev-4b",
        summary: "kev 4B, built on Qwen3.5; reads Japanese and English",
        license: "Apache-2.0",
        gguf: Some(Gguf {
            url: "https://huggingface.co/ggml-org/Kev-4B-GGUF/resolve/f4624b6bb7072f1bab726a622c1cad3f96030b9e/Kev-4B-Q4_K_M.gguf",
            file: "Kev-4B-Q4_K_M.gguf",
            sha256: "33ae6b18926502b2209a1bf7d3b61a350d65938441515c686bb87d226a19eff9",
            bytes: 3_033_489_824,
        }),
        cloud: None,
        cuts: [0.6, 1.4, 2.2],
        memory_gb: 8,
        measured: Some((25, 30)),
    },
    Model {
        name: "laya-multilingual",
        summary: "Laya multilingual, an encoder that reads 100+ languages",
        license: "Apache-2.0",
        gguf: Some(Gguf {
            url: "https://huggingface.co/andreilungeanu/laya-multilingual-GGUF/resolve/d481f236dfe356b8f9049653c45f9cfd1bc6c96a/Laya-Multilingual-Q8_0.gguf",
            file: "Laya-Multilingual-Q8_0.gguf",
            sha256: "bf1b31125292b8b159efd749271e8a797352d1ba629be1b77d58440942436f5a",
            bytes: 356_866_496,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 2,
        measured: Some((11, 30)),
    },
    Model {
        name: "laya",
        summary: "Laya, an English-only encoder",
        license: "Apache-2.0",
        gguf: Some(Gguf {
            url: "https://huggingface.co/ggml-org/Laya-GGUF/resolve/845f314dd047fc2444a5289a4d50c1e1d59e27e1/Laya-Q8_0.gguf",
            file: "Laya-Q8_0.gguf",
            sha256: "c06528c5746d3bb8baa72a27938be95abbfd0b226f8471e8a9e365ed0bb066d2",
            bytes: 449_397_600,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 2,
        measured: Some((18, 30)),
    },
    Model {
        name: "julia-1",
        summary: "Julia-1, Supersonic Labs' small decision model",
        license: "Apache-2.0",
        gguf: Some(Gguf {
            url: "https://huggingface.co/ggml-org/Julia-1-GGUF/resolve/1e6b48a6983d7467f8eedc2cb0d21267b5fc052d/Julia-1-Q8_0.gguf",
            file: "Julia-1-Q8_0.gguf",
            sha256: "1ea6a7e87156eeeda88cb7a36a61265b37ba7b993897b7289b99aea5b5e47069",
            bytes: 168_166_496,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 1,
        measured: Some((12, 30)),
    },
    Model {
        name: "jev",
        summary: "TypeSafe Jev; closed weights, hosted only",
        license: "proprietary",
        gguf: None,
        cloud: Some(Cloud {
            provider: "typesafe",
            id: "jev-1.14.0",
        }),
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 0,
        measured: None,
    },
    Model {
        name: "gpt-6-luna",
        summary: "OpenAI gpt-6-luna on the Decisions API",
        license: "proprietary",
        gguf: None,
        cloud: Some(Cloud {
            provider: "openai",
            id: "gpt-6-luna",
        }),
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 0,
        measured: None,
    },
];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

/// Whether a backend has been chosen and is not `off`.
pub fn chosen(config: &DecisionConfig) -> bool {
    matches!(
        config.backend,
        Some(DecisionBackend::Local | DecisionBackend::Cloud)
    )
}

/// Bumped whenever the questions change, so a cached answer to other questions is not reused.
const QUESTIONS_VERSION: u32 = 1;
const MAX_TASK_CHARS: usize = 4000;
/// A `noul` raises a capability need or a flag only above this: raising one narrows the field
/// or seats another agent, and an uncalibrated threshold should err towards the heuristic.
const RAISE: f64 = 0.8;
/// A name in `suited`, `prefer` or `avoid` is taken only where the model is this sure of it.
const NAME: f64 = 0.6;
const NONE: &str = "none";

fn task_types() -> Value {
    let describe = |t: &str| match t {
        "discussion" => "talk it through or think it over together, with no deliverable",
        "migration" => "move software to another framework, language, version or platform",
        "architecture" => "design the structure of software",
        "refactor" => "restructure code without changing what it does",
        "review" => "assess work someone else did",
        "bug_fix" => "broken software should work",
        "documentation" => "produce an explanatory document, guide or report",
        "test" => "write or run tests, or check that software behaves",
        "investigation" => "research facts, sources or causes",
        "analysis" => "interpret evidence, compare options or evaluate conclusions",
        "planning" => "devise a plan or strategy",
        "creative" => "creative writing or other creative deliverables",
        "general" => "a task that is none of the others and is not software work",
        "small_edit" => "a rename, a typo, a one-line change",
        "implementation" => "build software, or add a feature, option or command to it",
        _ => "",
    };
    Value::Object(
        TASK_TYPES
            .iter()
            .map(|t| (t.to_string(), json!(describe(t))))
            .collect(),
    )
}

/// The questions put about every request. `names` is what `suited`, `prefer` and `avoid`
/// may pick from: the catalog's model names and the agents' IDs.
pub fn questions(models: &Value, agents: &[String], remembered: bool) -> Value {
    let mut q = json!({
        "task_type": {"type": "choice", "instructions": "What kind of work is being asked for? Judge what is asked for, not the words used to ask.", "criteria": task_types()},
        "complexity": {"type": "score", "instructions": "How difficult and large is this task?", "criteria": [
            "simple: a brief obvious task",
            "normal: a contained task",
            "complex: multiple dependent questions or deliverables, substantial research, cross-cutting work or a significant decision",
            "extreme: extensive dependent work, synthesis across many sources, or a whole-system rewrite or migration"]},
        "ambiguity": {"type": "score", "instructions": "How much would whoever does this have to guess what is wanted?", "criteria": ["fully specified", "mostly clear", "vague"]},
        "requires_architecture_change": {"type": "noul", "instructions": "Does doing this change the structure of the software?"},
        "long_horizon": {"type": "noul", "instructions": "Is this too much to plausibly finish in one sitting?"},
        "requires_browser": {"type": "noul", "instructions": "Does the task itself say a real web browser must be driven?"},
        "requires_web": {"type": "noul", "instructions": "Does the task itself say the open internet must be searched?"},
        "requires_image": {"type": "noul", "instructions": "Must an image be generated, not just read?"},
    });
    if let Some(described) = models.as_object().filter(|m| !m.is_empty()) {
        let mut criteria: serde_json::Map<String, Value> = described.clone();
        criteria.insert(
            NONE.into(),
            json!("nothing in the list speaks to this task"),
        );
        q["suited"] = json!({"type": "choice", "instructions": "Which of these models is this task best suited to, by what each is for? Where a line says \"you said:\", that is what the user wants the model for.", "criteria": criteria});
    }
    if remembered {
        let mut names: serde_json::Map<String, Value> = agents
            .iter()
            .map(|a| (a.clone(), json!(format!("the agent {a}"))))
            .collect();
        if let Some(described) = models.as_object() {
            for name in described.keys() {
                names.insert(name.clone(), json!(format!("the model {name}")));
            }
        }
        names.insert(
            NONE.into(),
            json!("nothing remembered or requested names one"),
        );
        q["prefer"] = json!({"type": "choice", "instructions": "Does what is remembered about the user, or the request itself, say which agent or model they want for this kind of work? Which?", "criteria": names.clone()});
        q["avoid"] = json!({"type": "choice", "instructions": "Does what is remembered about the user, or the request itself, say which agent or model they do not want for this kind of work? Which?", "criteria": names});
    }
    q
}

/// The state the questions are asked about: the request, and what is remembered when the
/// preference questions are asked.
pub fn state(task: &str, remembered: Option<&Value>) -> Value {
    let text: String = task.chars().take(MAX_TASK_CHARS).collect();
    let mut state = vec![json!(format!("Request: {text}"))];
    if let Some(remembered) = remembered {
        state.push(json!(format!("Remembered about this user: {remembered}")));
    }
    Value::Array(state)
}

fn level(score: f64, cuts: &[f64; 3]) -> Complexity {
    if score < cuts[0] {
        Complexity::Simple
    } else if score < cuts[1] {
        Complexity::Normal
    } else if score < cuts[2] {
        Complexity::Complex
    } else {
        Complexity::Extreme
    }
}

/// What the answers say, in the classifier's terms, so `classifier::apply` merges them the
/// same one-directional way. `general` never replaces a more specific heuristic label: it is
/// where an unsure decision model goes, and measured it was wrong every time it went there.
pub fn interpret(
    answers: &Value,
    cuts: &[f64; 3],
    heuristic: &TaskDescriptor,
) -> Option<Classification> {
    let choice = |name: &str| -> Option<(String, f64)> {
        let answer = &answers[name];
        let picked = answer["choice"].as_str()?.to_owned();
        let p = answer["probabilities"][&picked]
            .as_f64()
            .or_else(|| answer["confidence"].as_f64())
            .unwrap_or(0.0);
        Some((picked, p))
    };
    let noul = |name: &str| answers[name]["noul"].as_f64().is_some_and(|p| p >= RAISE);
    let named = |name: &str| {
        choice(name)
            .filter(|(n, p)| n != NONE && *p >= NAME)
            .map(|(n, _)| n)
    };
    let (mut task_type, _) = choice("task_type")?;
    if !TASK_TYPES.contains(&task_type.as_str()) {
        return None;
    }
    if task_type == "general" && heuristic.task_type != "general" {
        task_type = heuristic.task_type.clone();
    }
    let complexity = answers["complexity"]["score"]
        .as_f64()
        .filter(|s| s.is_finite())
        .map(|s| level(s, cuts))
        .unwrap_or(Complexity::Simple);
    let ambiguity = answers["ambiguity"]["score"]
        .as_f64()
        .filter(|s| s.is_finite())
        .map(|s| (s / 2.0).clamp(0.0, 1.0));
    let mut c: Classification = serde_json::from_value(json!({
        "task_type": task_type,
        "complexity": complexity,
    }))
    .ok()?;
    c.ambiguity = ambiguity;
    c.requires_architecture_change = noul("requires_architecture_change");
    c.long_horizon = noul("long_horizon");
    c.requires_browser = noul("requires_browser");
    c.requires_web = noul("requires_web");
    c.requires_image = noul("requires_image");
    c.suited = named("suited");
    c.prefer = named("prefer");
    c.avoid = named("avoid");
    Some(c)
}

/// Where the questions go and with what key, for the backend that is chosen.
pub struct Target {
    pub endpoint: String,
    pub api: Api,
    pub model: String,
    pub key: Option<String>,
    pub cuts: [f64; 3],
}

/// The provider a cloud choice reaches: the model's own, or with a hand-set endpoint, whatever
/// speaks System One there, its key stored as `custom`.
pub fn cloud_of(config: &DecisionConfig) -> Result<(Option<&'static Provider>, String)> {
    match model(&config.model) {
        Some(m) => {
            let cloud = m
                .cloud
                .with_context(|| format!("{} is not offered in the cloud", m.name))?;
            Ok((provider(cloud.provider), cloud.id.to_owned()))
        }
        None if config.endpoint.is_some() => Ok((None, config.model.clone())),
        None => bail!(
            "{} is not a model Orochi knows; set decision.endpoint",
            config.model
        ),
    }
}

pub async fn target(config: &DecisionConfig, data: &Path) -> Result<Target> {
    let known = model(&config.model);
    let cuts = known.map(|m| m.cuts).unwrap_or([0.5, 1.5, 2.5]);
    match config.backend {
        Some(DecisionBackend::Local) => {
            let endpoint = match &config.endpoint {
                Some(endpoint) => endpoint.clone(),
                None => {
                    let known = known.filter(|m| m.gguf.is_some()).with_context(|| {
                        format!(
                            "{} is not a model Orochi puts on this machine; set decision.endpoint",
                            config.model
                        )
                    })?;
                    local::ensure(&local::home(data, config), config, known).await?
                }
            };
            Ok(Target {
                endpoint,
                api: Api::SystemOne,
                model: config.model.clone(),
                key: None,
                cuts,
            })
        }
        Some(DecisionBackend::Cloud) => {
            let (provider, id) = cloud_of(config)?;
            let name = provider.map(|p| p.name).unwrap_or("custom");
            let label = provider.map(|p| p.label).unwrap_or("the decision endpoint");
            Ok(Target {
                endpoint: config
                    .endpoint
                    .clone()
                    .or_else(|| provider.map(|p| p.endpoint.to_owned()))
                    .context("no endpoint")?,
                api: provider.map(|p| p.api).unwrap_or(Api::SystemOne),
                model: id,
                key: Some(key::get(data, config, name).with_context(|| {
                    format!("no {label} API key; run `orochi decision key {name}`")
                })?),
                cuts,
            })
        }
        Some(DecisionBackend::Off) | None => bail!("no decision model is chosen"),
    }
}

/// The System One questions in OpenAI's Decisions shape: `noul` is `predicate`, a choice's
/// criteria are `choices`, a score's are `levels`, and the request is one `input`.
fn openai_request(model: &str, state: &Value, questions: &Value) -> Value {
    let input = state
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    let mut list = vec![];
    for (name, q) in questions.as_object().into_iter().flatten() {
        let instructions = q["instructions"].clone();
        match q["type"].as_str() {
            Some("noul") => {
                list.push(json!({"type": "predicate", "name": name, "instructions": instructions}))
            }
            Some("choice") => {
                let choices: Vec<Value> = q["criteria"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(value, about)| match about.as_str() {
                        Some(about) => json!({"value": value, "description": about}),
                        None => json!({"value": value}),
                    })
                    .collect();
                list.push(json!({"type": "choice", "name": name, "instructions": instructions, "choices": choices}));
            }
            Some("score") => {
                let levels: Vec<Value> = q["criteria"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|level| json!({"label": level}))
                    .collect();
                list.push(json!({"type": "score", "name": name, "instructions": instructions, "levels": levels}));
            }
            _ => {}
        }
    }
    json!({"model": model, "input": input, "questions": list})
}

/// OpenAI's answers, a list, as the System One map `interpret` reads. A refusal is left out,
/// so the question it refused reads as unanswered.
fn openai_answers(reply: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for answer in reply["answers"].as_array().into_iter().flatten() {
        let Some(name) = answer["name"].as_str() else {
            continue;
        };
        let read = match answer["type"].as_str() {
            Some("predicate") => json!({"noul": answer["probability"]}),
            Some("choice") => {
                let probabilities: serde_json::Map<String, Value> = answer["probabilities"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| {
                        Some((p["value"].as_str()?.to_owned(), p["probability"].clone()))
                    })
                    .collect();
                json!({"choice": answer["choice"], "probabilities": probabilities, "confidence": answer["confidence"]})
            }
            Some("score") => json!({"score": answer["score"], "confidence": answer["confidence"]}),
            _ => continue,
        };
        out.insert(name.to_owned(), read);
    }
    Value::Object(out)
}

/// One request, in whichever shape the target speaks, read back as System One answers.
pub async fn ask(
    target: &Target,
    state: &Value,
    questions: &Value,
    timeout: Duration,
) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("orochi/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let body = match target.api {
        Api::SystemOne => json!({
            "model": target.model,
            "state": state,
            "questions": questions,
        }),
        Api::OpenAiDecisions => openai_request(&target.model, state, questions),
    };
    let mut request = client
        .post(&target.endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string());
    if let Some(key) = &target.key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    // The body of a failure can echo the request, which holds the task text.
    ensure!(status.is_success(), "the decision model answered {status}");
    let reply: Value =
        serde_json::from_str(&body).context("the decision model's reply was not JSON")?;
    let answers = match target.api {
        Api::SystemOne => reply.get("answers").cloned().unwrap_or(reply),
        Api::OpenAiDecisions => openai_answers(&reply),
    };
    ensure!(
        answers.as_object().is_some_and(|a| !a.is_empty()),
        "the decision model's reply had no answers"
    );
    Ok(answers)
}

/// Labels the request when a decision model is chosen. `true` when it did; every failure
/// leaves the heuristic profile in place and says why, since a decision model is never why a
/// run cannot start.
pub async fn refine(
    config: &Config,
    policies: &Registry,
    store: &Store,
    root: &Path,
    task: &str,
    descriptor: &mut TaskDescriptor,
    mut report: impl FnMut(Progress),
) -> bool {
    if !chosen(&config.decision) {
        return false;
    }
    let memory = Memory::open(store.data_dir(), &config.memory).zip(store.repository_id(root).ok());
    let remembered = memory
        .as_ref()
        .map(|(memory, repository)| memory.listing(repository, now()))
        .filter(|listing| {
            ["user", "repo"]
                .iter()
                .any(|scope| listing[scope].as_array().is_some_and(|l| !l.is_empty()))
        });
    let models = classifier::catalog(config, policies);
    let agents: Vec<String> = config
        .agents
        .iter()
        .filter(|a| a.enabled && !a.routing_only)
        .map(|a| a.id.clone())
        .collect();
    let questions = questions(&models, &agents, remembered.is_some());
    let state = state(task, remembered.as_ref());
    let backend = match config.decision.backend {
        Some(DecisionBackend::Cloud) => "cloud",
        _ => "local",
    };
    let Ok(key) = store.classification_key(&format!(
        "decision:{QUESTIONS_VERSION}:{backend}:{}:{}\n{state}\n{questions}",
        config.decision.model,
        config.decision.endpoint.as_deref().unwrap_or("")
    )) else {
        return false;
    };
    if let Ok(Some(hit)) = store
        .classification(&key)
        .map(|r| r.and_then(|json| serde_json::from_str::<Classification>(&json).ok()))
    {
        classifier::apply(descriptor, &hit);
        report(note(descriptor, "cached"));
        return true;
    }
    let started = std::time::Instant::now();
    let answered = async {
        let target = target(&config.decision, store.data_dir()).await?;
        let answers = ask(
            &target,
            &state,
            &questions,
            Duration::from_millis(config.decision.timeout_ms),
        )
        .await?;
        interpret(&answers, &target.cuts, descriptor)
            .context("the decision model's answers did not classify the request")
    }
    .await;
    match answered {
        Ok(classification) => {
            let _ = store.save_classification(
                &key,
                &serde_json::to_string(&classification).unwrap_or_default(),
            );
            classifier::apply(descriptor, &classification);
            report(note(
                descriptor,
                &format!("{} ms", started.elapsed().as_millis()),
            ));
            true
        }
        Err(error) => {
            report(Progress::Note(format!(
                "decision model {} unavailable: {error:#}; keeping the local profile",
                config.decision.model
            )));
            false
        }
    }
}

fn note(descriptor: &TaskDescriptor, how: &str) -> Progress {
    Progress::Note(format!(
        "classified as {} / {}{} ({how})",
        descriptor.task_type,
        descriptor.complexity.key(),
        descriptor
            .preferred
            .as_deref()
            .map(|name| format!(" · you prefer {name}"))
            .unwrap_or_default(),
    ))
}

/// What a first run recommends: the recommended model on this machine when it can hold it,
/// and nothing otherwise — the cloud is never recommended, since which provider sees the
/// request is the user's to choose. `reason` is for a terminal; a window says it in its own
/// language from the numbers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Recommendation {
    pub model: &'static str,
    pub backend: Option<DecisionBackend>,
    pub memory_gb: Option<u64>,
    pub free_disk_gb: Option<u64>,
    pub local_supported: bool,
    pub fits: bool,
    pub reason: String,
}

/// `home` is where a local model would be downloaded (`local::home`), whose disk is the one
/// that has to have room.
/// Which of the agents that may carry on the work the decision model names as the one it suits,
/// as it reads the request. `None` when no decision model is chosen, it does not answer, or it
/// names none of them: the scheduler then chooses as it does without one.
pub async fn handover(
    config: &DecisionConfig,
    data: &Path,
    task: &str,
    agents: &[String],
) -> Option<String> {
    if !chosen(config) || agents.is_empty() {
        return None;
    }
    let target = target(config, data).await.ok()?;
    let mut criteria: serde_json::Map<String, Value> = agents
        .iter()
        .map(|agent| (agent.clone(), json!(format!("the agent {agent}"))))
        .collect();
    criteria.insert(NONE.into(), json!("none of them suits this work"));
    let questions = json!({"handover": {
        "type": "choice",
        "instructions": "Which of these agents should carry on this work? Each one has an account with room now; pick the one whose models suit what is asked for.",
        "criteria": criteria,
    }});
    let answers = ask(
        &target,
        &state(task, None),
        &questions,
        Duration::from_millis(config.timeout_ms),
    )
    .await
    .ok()?;
    let picked = answers["handover"]["choice"].as_str()?;
    agents
        .iter()
        .any(|agent| agent == picked)
        .then(|| picked.to_owned())
}

pub fn recommend(home: &Path) -> Recommendation {
    let default = model(RECOMMENDED).expect("the recommended model is listed");
    let memory_gb = local::memory_gb();
    let free_disk_gb = local::free_disk_gb(home);
    let local_supported = local::platform().is_some();
    let disk = default
        .gguf
        .map(|g| g.bytes / 1_000_000_000 + 2)
        .unwrap_or(0);
    let fits = local_supported
        && memory_gb.is_some_and(|m| m >= default.memory_gb * 2)
        && free_disk_gb.is_none_or(|d| d >= disk);
    let reason = if fits {
        format!(
            "this machine has {} GB of memory and room for the {:.1} GB download",
            memory_gb.unwrap_or(0),
            default.gguf.map(|g| g.bytes as f64 / 1e9).unwrap_or(0.0)
        )
    } else if !local_supported {
        "llama.cpp publishes no build for this machine".into()
    } else {
        "this machine is short of memory or disk for it".into()
    };
    Recommendation {
        model: default.name,
        backend: fits.then_some(DecisionBackend::Local),
        memory_gb,
        free_disk_gb,
        local_supported,
        fits,
        reason,
    }
}

/// What `orochi decision status` and the window read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Status {
    pub backend: Option<DecisionBackend>,
    /// Where a local model is downloaded to.
    pub home: std::path::PathBuf,
    pub model: String,
    pub endpoint: Option<String>,
    pub installed: bool,
    pub running: Option<u16>,
    /// Which providers have a key here, from the environment or stored.
    pub keys: std::collections::BTreeMap<String, bool>,
    pub recommendation: Recommendation,
    pub providers: Vec<Provider>,
    pub models: Vec<Value>,
}

pub fn status(config: &DecisionConfig, data: &Path) -> Status {
    let known = model(&config.model);
    let home = local::home(data, config);
    Status {
        home: home.clone(),
        backend: config.backend,
        model: config.model.clone(),
        endpoint: config.endpoint.clone(),
        installed: known.is_some_and(|m| local::installed(&home, m)),
        running: local::running(&home, &config.model),
        keys: PROVIDERS
            .iter()
            .map(|p| (p.name.to_owned(), key::get(data, config, p.name).is_some()))
            .collect(),
        recommendation: recommend(&home),
        providers: PROVIDERS.to_vec(),
        models: MODELS
            .iter()
            .map(|m| {
                json!({
                    "name": m.name,
                    "summary": m.summary,
                    "license": m.license,
                    "local": m.gguf.is_some(),
                    "bytes": m.gguf.map(|g| g.bytes),
                    "installed": local::installed(&home, m),
                    "cloud": m.cloud.map(|c| json!({"provider": c.provider, "id": c.id})),
                    "measured": m.measured.map(|(right, of)| json!({"right": right, "of": of})),
                })
            })
            .collect(),
    }
}

/// Puts the question to the chosen backend once with a request of no consequence, so a
/// choice is known to work before it is saved.
pub async fn check(config: &DecisionConfig, data: &Path) -> Result<Duration> {
    let started = std::time::Instant::now();
    let target = target(config, data).await?;
    let answers = ask(
        &target,
        &state("Fix the typo in the README.", None),
        &json!({"task_type": {"type": "choice", "instructions": "What kind of work is being asked for?", "criteria": task_types()}}),
        Duration::from_millis(config.timeout_ms.max(60_000)),
    )
    .await?;
    ensure!(
        answers["task_type"]["choice"].is_string(),
        "the decision model answered without a choice"
    );
    Ok(started.elapsed())
}
