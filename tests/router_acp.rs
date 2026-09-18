use orochi::{
    config::{AgentConfig, Config, RouterConfig},
    router::{advisory, profiler},
    storage::Store,
    types::*,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn candidate() -> ExecutionCandidate {
    ExecutionCandidate {
        prediction: None,
        id: "permitted".into(),
        agent: "codex".into(),
        provider: Provider::Openai,
        model: "test-small".into(),
        reasoning_level: None,
        mode: None,
        session_strategy: "fresh".into(),
        context_strategy: "task_envelope".into(),
        success_probability: 0.9,
        expected_tokens: 1_000_000.0,
        expected_cost: 1_100_000.0,
        confidence: 0.5,
        reasons: vec![],
    }
}
fn other(id: &str, cost: f64) -> ExecutionCandidate {
    let mut c = candidate();
    c.id = id.into();
    c.expected_cost *= cost;
    c
}

struct Bench {
    dir: tempfile::TempDir,
    config: Config,
}
impl Bench {
    fn new() -> Self {
        let mut config = Config::default();
        config.discovery.auto_add = false;
        config.agents.clear();
        Self {
            dir: tempfile::tempdir().unwrap(),
            config,
        }
    }
    /// A routing-only fixture adviser. `votes` are consumed per turn across its sessions.
    fn adviser(&mut self, id: &str, votes: &str) -> RouterConfig {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_acp.py");
        let mut agent = AgentConfig::preset(id, Provider::Anthropic, "python3", &[]);
        agent.args = vec![script.display().to_string()];
        agent.routing_only = true;
        for (key, value) in [
            ("MOCK_BEHAVIOR", "adviser".to_owned()),
            ("MOCK_VOTES", votes.to_owned()),
            ("MOCK_MODELS", "adviser-model".to_owned()),
            ("MOCK_COUNTER", self.path(&format!("{id}.count"))),
            ("MOCK_LOG", self.path(&format!("{id}.jsonl"))),
        ] {
            agent.env.insert(key.into(), value);
        }
        self.config.agents.push(agent);
        RouterConfig {
            agent: id.into(),
            model: Some("adviser-model".into()),
            reasoning: None,
            timeout_secs: 10,
            max_resource_fraction: 1.0,
            max_output_tokens: 64,
            session_overhead_tokens: Some(1000),
            fallbacks: vec![],
        }
    }
    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }
    fn store(&self) -> Store {
        Store::open(&self.dir.path().join("data")).unwrap()
    }
    fn requests(&self, id: &str) -> Vec<Value> {
        std::fs::read_to_string(PathBuf::from(self.path(&format!("{id}.jsonl"))))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    fn prompts(&self, id: &str) -> Vec<(String, String)> {
        self.requests(id)
            .into_iter()
            .filter(|r| r["method"] == "session/prompt")
            .map(|r| {
                (
                    r["params"]["sessionId"].as_str().unwrap().to_owned(),
                    r["params"]["prompt"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect()
    }
    async fn advise(
        &self,
        task: &TaskDescriptor,
        candidates: &[ExecutionCandidate],
    ) -> advisory::Decision {
        let policies = orochi::policy::Registry::bundled().unwrap();
        advisory::advise(&self.config, &policies, task, candidates, &self.store()).await
    }
}
fn difficult(text: &str) -> TaskDescriptor {
    let root = tempfile::tempdir().unwrap();
    let mut task = profiler::profile(text, root.path());
    task.complexity = Complexity::Extreme;
    task
}

#[tokio::test]
async fn valid_advice_records_usage_and_sends_only_descriptors_to_an_isolated_session() {
    let mut b = Bench::new();
    b.config.frontier = Some(b.adviser("judge", "permitted"));
    let mut task = difficult("PRIVATE RAW TASK");
    task.candidate_files = vec!["private/customer-name.rs".into()];
    let decision = b
        .advise(&task, &[candidate(), other("alternative", 1.1)])
        .await;
    assert_eq!(decision.candidate_id.as_deref(), Some("permitted"));
    assert_eq!(decision.purpose, "frontier_routing");
    let call = &decision.consultations[0];
    assert_eq!(
        (call.agent.as_str(), call.model.as_str()),
        ("judge", "adviser-model")
    );
    assert_eq!(call.usage.total(), Some(900));
    assert!(call.error.is_none());
    let (_, prompt) = &b.prompts("judge")[0];
    assert!(!prompt.contains("PRIVATE RAW TASK"));
    assert!(!prompt.contains("customer-name"));
    assert!(prompt.contains("Do not read files"));
    let cwd = b
        .requests("judge")
        .into_iter()
        .find(|r| r["method"] == "session/new")
        .unwrap()["params"]["cwd"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        !Path::new(&cwd).exists(),
        "adviser workspace must be temporary"
    );
}

#[tokio::test]
async fn invalid_advice_has_no_authority_but_preserves_usage() {
    let mut b = Bench::new();
    b.config.frontier = Some(b.adviser("judge", "forbidden"));
    let decision = b
        .advise(
            &difficult("task"),
            &[candidate(), other("alternative", 1.1)],
        )
        .await;
    assert!(decision.candidate_id.is_none());
    assert_eq!(decision.consultations[0].usage.total(), Some(900));
    assert_eq!(
        decision.consultations[0].error.as_deref(),
        Some("invalid_advice")
    );
}

#[tokio::test]
async fn adviser_limits_cool_down_the_shared_agent_and_skip_later_launches() {
    let mut b = Bench::new();
    b.config.frontier = Some(b.adviser("judge", "RATE_LIMIT"));
    let task = difficult("task");
    let candidates = [candidate(), other("alternative", 1.1)];
    let decision = b.advise(&task, &candidates).await;
    assert!(decision.candidate_id.is_none());
    assert_eq!(
        decision.consultations[0].error.as_deref(),
        Some("rate_limit")
    );
    let runtime = b.store().runtime().unwrap();
    assert!(!runtime[&("judge".into(), "*".into())].available(now()));
    let launches = b.requests("judge").len();
    let again = b.advise(&task, &candidates).await;
    assert_eq!(again.consultations[0].error.as_deref(), Some("cooldown"));
    assert_eq!(
        b.requests("judge").len(),
        launches,
        "cooled-down adviser was started"
    );
}

#[tokio::test]
async fn budget_gate_never_starts_an_adviser() {
    let mut b = Bench::new();
    b.config.frontier = Some(b.adviser("judge", "permitted"));
    let mut cheap = candidate();
    cheap.expected_tokens = 1000.0;
    let decision = b
        .advise(&difficult("task"), &[cheap, other("alternative", 1.1)])
        .await;
    assert!(decision.consultations.is_empty());
    assert!(b.requests("judge").is_empty());
}

#[tokio::test]
async fn council_reviews_peer_votes_in_the_same_session_and_requires_strict_majority() {
    let mut b = Bench::new();
    let first = b.adviser("first", "alternative,permitted");
    let second = b.adviser("second", "permitted,permitted");
    b.config.council.enabled = true;
    b.config.council.members = vec![first, second];
    let decision = b
        .advise(
            &difficult("PRIVATE TASK"),
            &[candidate(), other("alternative", 1.1)],
        )
        .await;
    assert_eq!(decision.candidate_id.as_deref(), Some("permitted"));
    assert_eq!(decision.consultations.len(), 4);
    assert_eq!(
        decision
            .consultations
            .iter()
            .filter_map(|c| c.usage.total())
            .sum::<u64>(),
        3600
    );
    for seat in ["first", "second"] {
        let starts = b
            .requests(seat)
            .iter()
            .filter(|r| r["method"] == "session/new")
            .count();
        assert_eq!(starts, 1, "{seat} should keep its session for the review");
        let prompts = b.prompts(seat);
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].0, prompts[1].0);
        assert!(prompts[1].1.contains("peer_candidate_votes"));
        assert!(!prompts[1].1.contains("\"candidates\""));
        assert!(!prompts[1].1.contains("PRIVATE TASK"));
    }
}

#[tokio::test]
async fn council_invalid_members_do_not_reduce_majority_denominator() {
    let mut b = Bench::new();
    let first = b.adviser("first", "permitted");
    let second = b.adviser("second", "not-a-candidate");
    b.config.council.enabled = true;
    b.config.council.members = vec![first, second];
    let decision = b
        .advise(
            &difficult("task"),
            &[candidate(), other("alternative", 2.0)],
        )
        .await;
    assert!(decision.candidate_id.is_none());
}

#[tokio::test]
async fn frontier_only_runs_for_difficult_tasks_and_rejects_expensive_advice() {
    let mut b = Bench::new();
    b.config.frontier = Some(b.adviser("judge", "expensive"));
    let mut task = difficult("task");
    task.complexity = Complexity::Simple;
    task.ambiguity = 0.1;
    let candidates = [candidate(), other("expensive", 2.0)];
    assert!(b.advise(&task, &candidates).await.consultations.is_empty());
    task.complexity = Complexity::Extreme;
    let decision = b.advise(&task, &candidates).await;
    assert_eq!(decision.consultations.len(), 1);
    assert!(decision.candidate_id.is_none());
}

#[tokio::test]
async fn council_aggregate_budget_prevents_every_launch() {
    let mut b = Bench::new();
    b.config.council.enabled = true;
    b.config.council.members = ["a", "b", "c", "d"]
        .into_iter()
        .map(|id| b.adviser(id, "permitted"))
        .collect();
    let mut first = candidate();
    first.expected_tokens = 5000.0;
    let decision = b
        .advise(&difficult("task"), &[first, other("other", 1.0)])
        .await;
    assert!(decision.consultations.is_empty());
    assert!(
        ["a", "b", "c", "d"]
            .iter()
            .all(|id| b.requests(id).is_empty())
    );
}

#[tokio::test]
async fn router_and_frontier_replace_exhausted_advisers_with_the_same_request() {
    for frontier in [false, true] {
        let mut b = Bench::new();
        let mut route = b.adviser("primary", "CREDIT");
        route.fallbacks.push(b.adviser("backup", "permitted"));
        let mut task = difficult("PRIVATE TASK");
        if frontier {
            b.config.frontier = Some(route);
        } else {
            task.complexity = Complexity::Simple;
            task.ambiguity = 0.0;
            b.config.router = Some(route);
        }
        let decision = b.advise(&task, &[candidate(), other("other", 1.0)]).await;
        assert_eq!(decision.candidate_id.as_deref(), Some("permitted"));
        assert_eq!(decision.consultations.len(), 2);
        assert_eq!(
            decision.consultations[0].error.as_deref(),
            Some("rate_limit")
        );
        assert_eq!(decision.consultations[1].agent, "backup");
        assert_eq!(b.prompts("primary")[0].1, b.prompts("backup")[0].1);
    }
}

#[tokio::test]
async fn council_replacement_keeps_one_seat_and_receives_peer_votes() {
    for failed_round in [0, 1] {
        let mut b = Bench::new();
        let mut primary = b.adviser(
            "primary",
            if failed_round == 0 {
                "RATE_LIMIT"
            } else {
                "permitted,RATE_LIMIT"
            },
        );
        primary.fallbacks.push(b.adviser("backup", "permitted"));
        let peer = b.adviser("peer", "permitted");
        b.config.council.enabled = true;
        b.config.council.members = vec![primary, peer];
        let decision = b
            .advise(&difficult("task"), &[candidate(), other("other", 1.0)])
            .await;
        assert_eq!(decision.candidate_id.as_deref(), Some("permitted"));
        assert_eq!(decision.consultations.len(), 5);
        let prompts = b.prompts("backup");
        let review = &prompts.last().unwrap().1;
        assert!(review.contains("peer_candidate_votes"));
        if failed_round == 0 {
            // The replacement answered round one and reviews in that same session.
            assert_eq!(prompts.len(), 2);
            assert_eq!(prompts[0].0, prompts[1].0);
        } else {
            // A fresh replacement in round two receives the full state plus the votes.
            assert_eq!(prompts.len(), 1);
            assert!(review.contains("\"candidates\""));
        }
        // The exhausted primary is not started again for the review round.
        let primary_starts = b
            .requests("primary")
            .iter()
            .filter(|r| r["method"] == "session/new")
            .count();
        assert_eq!(primary_starts, 1);
    }
}

#[test]
fn advisers_must_name_enabled_agents_and_routing_only_agents_are_valid() {
    let mut b = Bench::new();
    let mut route = b.adviser("judge", "permitted");
    route.fallbacks.push(b.adviser("backup", "permitted"));
    b.config.router = Some(route);
    b.config.validate().unwrap();
    b.config.router.as_mut().unwrap().fallbacks[0].agent = "missing".into();
    assert!(b.config.validate().is_err());
    b.config.router.as_mut().unwrap().fallbacks[0].agent = "backup".into();
    b.config.agents[1].enabled = false;
    assert!(b.config.validate().is_err());
}

#[tokio::test]
async fn advisers_without_a_model_get_a_policy_appropriate_model_for_their_role() {
    let mut b = Bench::new();
    let mut judge = b.adviser("judge", "permitted");
    judge.model = None;
    // The agent's own default (first listed) is its most expensive model.
    b.config.agents[0].provider = Provider::Openai;
    b.config.agents[0].env.insert(
        "MOCK_MODELS".into(),
        "gpt-test-astra,gpt-test-luna,gpt-test-sol".into(),
    );
    b.config.frontier = Some(judge.clone());
    b.config.router = Some(judge);
    b.config.scheduler.routing_confidence = 1.0;
    b.config.validate().unwrap();
    let candidates = [candidate(), other("alternative", 1.1)];
    // A frontier judge decides difficult routing: the small model misses the success floor.
    let hard = b.advise(&difficult("task"), &candidates).await;
    assert_eq!(hard.purpose, "frontier_routing");
    assert_eq!(hard.consultations[0].model, "gpt-test-sol");
    assert_eq!(hard.consultations[0].reasoning.as_deref(), Some("medium"));
    let mut easy = difficult("task");
    easy.complexity = Complexity::Simple;
    easy.ambiguity = 0.0;
    // A router's decision is simple: the cheapest eligible model with low reasoning.
    let routed = b.advise(&easy, &candidates).await;
    assert_eq!(routed.purpose, "routing");
    assert_eq!(routed.consultations[0].model, "gpt-test-luna");
    assert_eq!(routed.consultations[0].reasoning.as_deref(), Some("low"));
    // Cooling-down models are skipped when choosing.
    b.store()
        .update_runtime("judge", "gpt-test-luna", |state| {
            state.status = RuntimeStatus::Cooldown;
            state.cooldown_until = Some(now() + 3600);
        })
        .unwrap();
    let rerouted = b.advise(&easy, &candidates).await;
    assert_eq!(rerouted.consultations[0].model, "gpt-test-sol");
    assert_eq!(rerouted.candidate_id.as_deref(), Some("permitted"));
}
