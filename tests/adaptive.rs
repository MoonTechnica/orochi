use orochi::{
    config::{AgentConfig, Config, LearningConfig, LearningStrategy},
    learning,
    quota_sources::{self, Snapshot, Window},
    router::{classifier, profiler},
    storage::Store,
    types::*,
};
use serde_json::json;
fn candidate(id: &str, cost: f64) -> ExecutionCandidate {
    ExecutionCandidate {
        id: id.into(),
        agent: id.into(),
        model: "model".into(),
        provider: Provider::Openai,
        reasoning_level: None,
        mode: None,
        session_strategy: "fresh".into(),
        context_strategy: "filesystem".into(),
        success_probability: 0.9,
        expected_tokens: 1000.0,
        expected_cost: cost,
        confidence: 0.9,
        reasons: vec![],
        prediction: Some(Prediction {
            candidate_id: id.into(),
            model: "model".into(),
            reasoning: None,
            mode: None,
            prior_success: 0.9,
            prior_tokens: 1000.0,
            success: 0.9,
            tokens: 1000.0,
            strategy: "ewma".into(),
            selection_probability: Some(1.0),
            prior_basis: Some(orochi::learning::PRIOR_BASIS.into()),
            cost_features: None,
        }),
    }
}
fn run(n: usize, success: bool) -> RunRecord {
    let c = candidate("a", 1000.0);
    RunRecord {
        id: format!("run-{n}"),
        task_id: format!("task-{n}"),
        repository_id: "repo".into(),
        task_type: "implementation".into(),
        language: "unknown".into(),
        framework: None,
        scope: 1,
        context_size: 500,
        prediction: c.prediction.clone(),
        candidate: c,
        usage: Usage {
            total_tokens: Some(2000),
            ..Default::default()
        },
        duration_ms: 100,
        attempt: 0,
        outcome: if success {
            Outcome::Success
        } else {
            Outcome::Failure
        },
        checks: vec![],
        error_kind: None,
        started_at: n as i64,
        purpose: "execution".into(),
        complexity: Some(Complexity::Normal),
        feedback: None,
    }
}
#[test]
fn ewma_reacts_to_recent_drift_and_scales_token_cost_to_task_size() {
    let mut runs: Vec<_> = (0..100).map(|n| run(n, true)).collect();
    let config = LearningConfig::default();
    let old = learning::estimate(&config, 0.9, 1000.0, &runs);
    assert!(old.success > 0.99);
    assert!((old.tokens - 2000.0).abs() < 1.0);
    runs.extend((100..140).map(|n| run(n, false)));
    let changed = learning::estimate(&config, 0.9, 10000.0, &runs);
    assert!(changed.success < 0.03);
    assert!((changed.tokens - 20000.0).abs() < 1.0);
    let mut static_config = config;
    static_config.strategy = LearningStrategy::Static;
    assert_eq!(
        learning::estimate(&static_config, 0.9, 1000.0, &runs).success,
        0.9
    );
}
#[test]
fn environment_errors_unverified_cancelled_and_switched_models_do_not_train() {
    let mut runs = vec![];
    for error in [
        "rate_limit",
        "authentication",
        "configuration",
        "unavailable",
    ] {
        let mut r = run(runs.len(), false);
        r.error_kind = Some(error.into());
        runs.push(r);
    }
    for outcome in [Outcome::Cancelled, Outcome::PartialSuccess] {
        let mut r = run(runs.len(), false);
        r.outcome = outcome;
        runs.push(r);
    }
    let mut r = run(7, true);
    r.candidate.model = "switched".into();
    runs.push(r);
    let mut r = run(8, true);
    r.purpose = "routing".into();
    runs.push(r);
    let result = learning::estimate(&LearningConfig::default(), 0.9, 1000.0, &runs);
    assert_eq!(result.samples, 0);
    assert_eq!(result.tokens, 1000.0);
}
#[test]
fn history_filters_context_before_limit_and_preserves_chronological_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.record(&run(0, true)).unwrap();
    for n in 1..550 {
        let mut r = run(n, false);
        r.candidate.mode = Some("other".into());
        store.record(&r).unwrap();
    }
    let mut descriptor = profiler::profile("Implement a small endpoint", dir.path());
    descriptor.language = "unknown".into();
    descriptor.framework = None;
    descriptor.complexity = Complexity::Normal;
    let history = store
        .learning_runs(&candidate("a", 1000.0), &descriptor)
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, "run-0");
    descriptor.framework = Some("different".into());
    assert!(
        store
            .learning_runs(&candidate("a", 1000.0), &descriptor)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn bandit_probabilities_sum_to_one_and_exploration_is_cost_bounded() {
    let config = LearningConfig {
        strategy: LearningStrategy::Bandit,
        exploration: 0.2,
        ..Default::default()
    };
    let mut counts = [0usize; 3];
    for n in 0..10000 {
        let mut candidates = vec![
            candidate("a", 1000.0),
            candidate("b", 1200.0),
            candidate("c", 1300.0),
        ];
        learning::select(&mut candidates, &config, (n as f64 + 0.5) / 10000.0);
        let selected = &candidates[0];
        let index = match selected.id.as_str() {
            "a" => 0,
            "b" => 1,
            _ => 2,
        };
        counts[index] += 1;
        assert!(
            (selected
                .prediction
                .as_ref()
                .unwrap()
                .selection_probability
                .unwrap()
                - if index == 0 { 0.9 } else { 0.1 })
            .abs()
                < 1e-10
        );
    }
    assert_eq!(counts, [9000, 1000, 0]);
}
#[test]
fn calibration_uses_frozen_predictions_and_keeps_unknown_usage_null() {
    let mut a = run(0, true);
    a.usage = Usage::default();
    let mut b = run(1, false);
    b.prediction.as_mut().unwrap().success = 0.2;
    let report = learning::calibrate(&[a, b]);
    assert_eq!(report.evaluated, 2);
    assert_eq!(report.token_samples, 1);
    assert!((report.brier.unwrap() - 0.025).abs() < 1e-10);
    assert!(report.brier.unwrap() < report.prior_brier.unwrap());
    let empty = learning::calibrate(&[]);
    assert!(empty.brier.is_none());
    assert!(empty.token_wape.is_none());
}
#[test]
fn quota_parser_keeps_all_windows_and_unknown_bucket_mappings_separate() {
    let windows=quota_sources::parse_codex(&json!({"rateLimitsByLimitId":{
        "codex":{"primary":{"usedPercent":100,"resetsAt":150},"secondary":{"usedPercent":100,"resetsAt":400}},
        "special":{"primary":{"usedPercent":98,"resetsAt":500}}},
        "rateLimits":{"primary":{"usedPercent":5}}}),100).unwrap();
    assert_eq!(windows.len(), 3);
    assert!(!windows[2].affects_routing);
    let snapshot = Snapshot {
        agent: "a".into(),
        source: "test".into(),
        observed_at: 100,
        valid_until: 600,
        windows,
    };
    let mut runtime = RuntimeMap::new();
    quota_sources::apply(&mut runtime, &snapshot, 101);
    assert_eq!(runtime[&("a".into(), "*".into())].cooldown_until, Some(400));
    let mut runtime = RuntimeMap::new();
    quota_sources::apply(&mut runtime, &snapshot, 200);
    assert!(!runtime[&("a".into(), "*".into())].available(200));
    let mut runtime = RuntimeMap::new();
    quota_sources::apply(&mut runtime, &snapshot, 450);
    assert!(runtime.is_empty());
    assert!(
        quota_sources::parse_codex(&json!({}), 100)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn stale_direct_quota_never_applies_and_live_error_cooldown_survives_refresh() {
    let snapshot = Snapshot {
        agent: "a".into(),
        source: "test".into(),
        observed_at: 100,
        valid_until: 200,
        windows: vec![Window {
            bucket: "primary".into(),
            remaining: 0.8,
            reset_at: Some(1000),
            model: None,
            affects_routing: true,
        }],
    };
    let mut map = RuntimeMap::new();
    quota_sources::apply(&mut map, &snapshot, 200);
    assert!(map.is_empty());
    let mut state = RuntimeState::new("a", "*");
    state.status = RuntimeStatus::Cooldown;
    state.cooldown_until = Some(900);
    map.insert(("a".into(), "*".into()), state);
    quota_sources::apply(&mut map, &snapshot, 150);
    assert_eq!(map[&("a".into(), "*".into())].cooldown_until, Some(900));
}
#[test]
fn paired_replay_trains_only_chosen_arm_and_is_reproducible() {
    let root = tempfile::tempdir().unwrap();
    let descriptor = profiler::profile("Implement small endpoint", root.path());
    let cases: Vec<_> = (0..60)
        .map(|n| {
            json!({"id":format!("case-{n}"),"descriptor":descriptor,"arms":[
        {"candidate":candidate("a",1000.0),"success":false,"tokens":2000,"duration_ms":100},
        {"candidate":candidate("b",1100.0),"success":true,"tokens":1000,"duration_ms":100}]})
        })
        .collect();
    let data = json!({"schema_version":1,"provenance":"unit test","synthetic":true,"cases":cases});
    let replay = || {
        orochi::benchmark::replay(
            serde_json::from_value(data.clone()).unwrap(),
            &LearningConfig::default(),
            0.7,
            42,
            100000.0,
        )
        .unwrap()
    };
    let a = replay();
    let b = replay();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(b).unwrap()
    );
    assert_eq!(a.policies[0].successes, 0);
    assert!(a.policies[1].successes > 50);
    assert!(a.policies[1].regret < a.policies[0].regret);
    assert!(a.synthetic);
}
#[test]
fn invalid_adaptive_config_rejected() {
    let mut config = Config::default();
    config.learning.alpha = f64::NAN;
    assert!(config.validate().is_err());
    config.learning.alpha = 0.1;
    config.learning.pooling = 1.5;
    assert!(config.validate().is_err());
    config.learning.pooling = 0.5;
    config.validate().unwrap();
    config.council.enabled = true;
    assert!(config.validate().is_err());
}

#[test]
fn claude_statusline_quota_is_not_context_usage_and_spend_can_exceed_100_percent() {
    let windows=quota_sources::parse_claude_statusline(&json!({
        "context_window":{"remaining_percentage":99},
        "rate_limits":{"five_hour":{"used_percentage":40,"resets_at":300},"seven_day":null,"spend_limit":{"used_percentage":110,"resets_at":500}},
        "transcript_path":"PRIVATE"}),100).unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].remaining, 0.6);
    assert_eq!(windows[1].remaining, 0.0);
    assert!(
        quota_sources::parse_claude_statusline(
            &json!({"context_window":{"remaining_percentage":90}}),
            100
        )
        .unwrap()
        .is_empty()
    );
    assert!(!serde_json::to_string(&windows).unwrap().contains("PRIVATE"));
}

#[test]
fn coefficient_tuning_never_uses_validation_labels_for_selection() {
    use orochi::benchmark::{Arm, Case, Dataset};
    let descriptor = profiler::profile("Implement a function", std::path::Path::new("/tmp"));
    let dataset = Dataset {
        schema_version: 1,
        provenance: "test only".into(),
        synthetic: true,
        cases: (0..12)
            .map(|i| Case {
                id: i.to_string(),
                descriptor: descriptor.clone(),
                arms: vec![
                    Arm {
                        candidate: candidate("a", 1000.0),
                        success: i % 3 != 0,
                        tokens: 5000,
                        duration_ms: 100,
                    },
                    Arm {
                        candidate: candidate("b", 1100.0),
                        success: true,
                        tokens: 1000,
                        duration_ms: 100,
                    },
                ],
            })
            .collect(),
    };
    let config = LearningConfig::default();
    let a = orochi::benchmark::tune(dataset.clone(), &config, 0.7, 42, 100000.0, 6).unwrap();
    let mut changed = dataset;
    for case in &mut changed.cases[6..] {
        for arm in &mut case.arms {
            arm.success = !arm.success;
            arm.tokens *= 10;
        }
    }
    let b = orochi::benchmark::tune(changed, &config, 0.7, 42, 100000.0, 6).unwrap();
    assert_eq!(
        serde_json::to_value(a.recommended).unwrap(),
        serde_json::to_value(b.recommended).unwrap()
    );
    assert_eq!(a.validation_cases, 6);
    assert!(
        a.tuned_validation
            .policies
            .iter()
            .all(|p| p.selected + p.abstained == 6)
    );
}

#[test]
fn nested_acp_rate_limit_details_are_classified_without_exposing_them() {
    use orochi::agents::{AgentAdapter, ErrorKind, ProviderAdapter};
    let error = json!({"code":-32603,"message":"Internal error","data":{"details":"API error: 429 rate_limit_error; private provider request id"}});
    let classified = ProviderAdapter(Provider::Anthropic).classify_error(&error, "fallback", 100);
    assert_eq!(classified.kind, ErrorKind::RateLimit);
    assert_eq!(classified.message, "Internal error");
    assert!(!classified.model_scoped);
}

#[test]
fn terminal_quota_parser_handles_incremental_redraws_and_ignores_context_percentages() {
    let status = std::process::Command::new("python3")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "tests",
            "-p",
            "test_quota_terminal.py",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn entirely_does_not_make_a_small_function_a_long_horizon_task() {
    let task = profiler::profile(
        "Implement a slug function. Entirely non-ASCII input returns empty string.",
        std::path::Path::new("/tmp"),
    );
    assert_eq!(task.complexity, Complexity::Normal);
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_model_does_not_probe_a_rate_limited_unrelated_model() {
    use orochi::{
        acp::Client,
        config::{AgentConfig, PermissionMode},
    };
    let dir = tempfile::tempdir().unwrap();
    let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
    let mut config = AgentConfig::preset("mock", Provider::Openai, "python3", &[&fixture]);
    config
        .env
        .insert("MOCK_BEHAVIOR".into(), "blocked_unselected".into());
    let mut client = Client::start_with_model(
        config,
        dir.path(),
        PermissionMode::Deny,
        std::time::Duration::from_secs(5),
        None,
        Some("sol-test"),
    )
    .await
    .unwrap();
    assert_eq!(client.capabilities.models.len(), 1);
    assert_eq!(client.capabilities.models[0].model, "sol-test");
    client.stop().await;
}

#[tokio::test]
async fn model_scoped_discovery_failure_keeps_other_models_available() {
    use orochi::{
        acp::Client,
        config::{AgentConfig, PermissionMode},
    };
    let root = tempfile::tempdir().unwrap();
    let fixture = format!("{}/tests/fixtures/mock_acp.py", env!("CARGO_MANIFEST_DIR"));
    let mut config = AgentConfig::preset("mock", Provider::Openai, "python3", &[&fixture]);
    config
        .env
        .insert("MOCK_BEHAVIOR".into(), "blocked_unselected".into());
    config
        .env
        .insert("MOCK_DISCOVERY_SCOPE".into(), "model".into());
    let mut client = Client::start(
        config,
        root.path(),
        PermissionMode::Deny,
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(client.capabilities.models.len(), 1);
    assert_eq!(client.capabilities.models[0].model, "sol-test");
    client.stop().await;
}

#[test]
fn pooling_carries_an_arms_token_overrun_into_new_task_contexts() {
    use orochi::benchmark::{Arm, Case, Dataset, replay};
    let base = profiler::profile("Implement a function", std::path::Path::new("/tmp"));
    let dataset = Dataset {
        schema_version: 1,
        provenance: "test only".into(),
        synthetic: true,
        cases: ["implementation", "bug_fix", "review", "investigation"]
            .into_iter()
            .map(|task_type| Case {
                id: task_type.into(),
                descriptor: TaskDescriptor {
                    task_type: task_type.into(),
                    ..base.clone()
                },
                arms: vec![
                    Arm {
                        candidate: candidate("a", 1000.0),
                        success: true,
                        tokens: 20_000,
                        duration_ms: 0,
                    },
                    Arm {
                        candidate: candidate("b", 1000.5),
                        success: true,
                        tokens: 2_000,
                        duration_ms: 0,
                    },
                ],
            })
            .collect(),
    };
    let tokens = |pooling: f64, strategy: usize| {
        let config = LearningConfig {
            pooling,
            ..LearningConfig::default()
        };
        replay(dataset.clone(), &config, 0.7, 42, 100000.0)
            .unwrap()
            .policies[strategy]
            .tokens
    };
    // Every case is a new context, so context-only EWMA never learns that "a" overruns.
    assert_eq!(tokens(0.0, 1), 80_000);
    assert_eq!(tokens(1.0, 1), 26_000);
    assert_eq!(tokens(1.0, 0), 80_000, "static routing ignores pooling");
}

/// The case tier pooling exists for: the frontier model is replaced mid-stream by an ID with
/// no history and a prior under the success floor. Without pooling the replacement is never
/// eligible and the work falls to the model that keeps failing; with it, what its predecessor
/// proved on this work carries over. This is the replay D2 asks for before turning it on.
#[test]
fn tier_pooling_carries_evidence_across_a_model_replacement() {
    let root = tempfile::tempdir().unwrap();
    let mut descriptor = profiler::profile("Implement small endpoint", root.path());
    descriptor.complexity = orochi::types::Complexity::Complex;
    let arm = |model: &str, prior: f64, tokens: f64| {
        let mut c = candidate(model, tokens / prior);
        c.agent = "claude".into();
        c.provider = Provider::Anthropic;
        c.model = model.into();
        c.success_probability = prior;
        c.expected_tokens = tokens;
        if let Some(p) = &mut c.prediction {
            p.model = model.into();
            p.prior_success = prior;
            p.prior_tokens = tokens;
        }
        c
    };
    let cases: Vec<_> = (0..60)
        .map(|n| {
            let frontier = if n < 30 { arm("opus-5", 0.8, 1600.0) } else { arm("opus-6", 0.66, 1600.0) };
            json!({"id":format!("case-{n}"),"descriptor":descriptor,"arms":[
                {"candidate":frontier,"success":true,"tokens":1600,"duration_ms":100},
                {"candidate":arm("haiku-5",0.75,500.0),"success":false,"tokens":500,"duration_ms":100}]})
        })
        .collect();
    let data =
        json!({"schema_version":1,"provenance":"model replacement","synthetic":true,"cases":cases});
    let replay = |tier_pooling| {
        orochi::benchmark::replay(
            serde_json::from_value(data.clone()).unwrap(),
            &LearningConfig {
                tier_pooling,
                ..LearningConfig::default()
            },
            0.7,
            42,
            100000.0,
        )
        .unwrap()
        .policies
        .into_iter()
        .find(|p| p.strategy == orochi::config::LearningStrategy::Ewma)
        .unwrap()
    };
    let (off, on) = (replay(0.0), replay(1.0));
    assert!(
        on.successes > off.successes + 20,
        "off {} / on {} successes",
        off.successes,
        on.successes
    );
}

/// Asking what a task is costs a whole extra ACP session, and on a given machine one agent's
/// session is far dearer than another's (2026-09-20: 37,630 tokens against 23,800 for the same
/// question). Which agent is asked follows what asking it has actually cost, not the order
/// agents happen to appear in the configuration — but one that has never answered is asked
/// once first, or its own price would stay unknown for good.
#[test]
fn the_classifier_asks_whichever_agent_has_answered_most_cheaply() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let config = Config {
        agents: vec![
            AgentConfig::preset("claude", Provider::Anthropic, "claude", &[]),
            AgentConfig::preset("codex", Provider::Openai, "codex", &[]),
        ],
        ..Config::default()
    };
    let asked = |tokens: u64, agent: &str, at: i64| {
        let mut record = run(at as usize, true);
        record.purpose = "classification".into();
        record.candidate.agent = agent.into();
        record.usage = Usage {
            total_tokens: Some(tokens),
            ..Default::default()
        };
        record.started_at = at;
        record
    };
    assert_eq!(classifier::agents(&config, &store), ["claude", "codex"]);
    store.record(&asked(37_630, "claude", 1)).unwrap();
    assert_eq!(classifier::agents(&config, &store), ["codex", "claude"]);
    store.record(&asked(23_800, "codex", 2)).unwrap();
    assert_eq!(classifier::agents(&config, &store), ["codex", "claude"]);
    // It follows the evidence when the evidence changes.
    for (at, tokens) in [(3, 900), (4, 900)] {
        store.record(&asked(tokens, "claude", at)).unwrap();
    }
    assert_eq!(classifier::agents(&config, &store), ["claude", "codex"]);
    // Execution is not advice: what the work itself costs never decides who is asked.
    for at in 5..9 {
        let mut record = run(at, true);
        record.candidate.agent = "codex".into();
        record.usage = Usage {
            total_tokens: Some(1),
            ..Default::default()
        };
        store.record(&record).unwrap();
    }
    assert_eq!(classifier::agents(&config, &store), ["claude", "codex"]);
}

/// Asking what a task is costs a session of its own, so the question has to be worth what it
/// costs. Once Orochi has measured both sides it asks only where the answer is cheap beside
/// the work it would change; knowing neither, it asks — that is how it comes to know.
#[test]
fn a_question_that_costs_more_than_the_work_it_decides_is_not_asked() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let config = Config {
        agents: vec![AgentConfig::preset("codex", Provider::Openai, "codex", &[])],
        ..Config::default()
    };
    let small = profiler::profile("rename the header in the readme", dir.path());
    let big = TaskDescriptor {
        complexity: Complexity::Extreme,
        ..small.clone()
    };
    let spent = |at: usize, purpose: &str, tokens: u64, task: &TaskDescriptor| {
        let mut record = run(at, true);
        record.purpose = purpose.into();
        record.candidate.agent = "codex".into();
        record.task_type = task.task_type.clone();
        record.complexity = Some(task.complexity);
        record.usage = Usage {
            total_tokens: Some(tokens),
            ..Default::default()
        };
        record.started_at = at as i64;
        record
    };
    // Nothing measured yet: ask, and the asking is what makes both costs known.
    assert!(classifier::worth_asking(&config, &store, &small));
    for at in 0..3 {
        store
            .record(&spent(at, "classification", 23_800, &small))
            .unwrap();
    }
    // What asking costs is known; what the work costs is not, so it still asks.
    assert!(classifier::worth_asking(&config, &store, &small));
    for at in 3..7 {
        store
            .record(&spent(at, "execution", 24_000, &small))
            .unwrap();
    }
    assert!(!classifier::worth_asking(&config, &store, &small));
    // The same question against work of another size is worth asking.
    for at in 7..11 {
        store
            .record(&spent(at, "execution", 300_000, &big))
            .unwrap();
    }
    assert!(classifier::worth_asking(&config, &store, &big));
    assert!(!classifier::worth_asking(&config, &store, &small));
    // The share is the user's to set: one that never balks keeps asking.
    let eager = Config {
        classifier: orochi::config::ClassifierConfig {
            max_cost_share: 10.0,
            ..config.classifier.clone()
        },
        ..config.clone()
    };
    assert!(classifier::worth_asking(&eager, &store, &small));
}

/// §5: the Insights and Agents screens read telemetry through views, so the numbers a client
/// shows are the ones `orochi calibrate` and `orochi status` show, not a second calculation.
#[test]
fn telemetry_is_readable_through_views_without_reaching_into_the_json() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    // Two verified runs on one route, one failure on another, and one weak signal.
    for n in 0..2 {
        store.record(&run(n, true)).unwrap();
    }
    let mut failed = run(9, false);
    failed.candidate = candidate("b", 1000.0);
    store.record(&failed).unwrap();
    let mut unverified = run(5, true);
    unverified.outcome = Outcome::PartialSuccess;
    store.record(&unverified).unwrap();
    store
        .feedback(&unverified.id, &Feedback::continued())
        .unwrap();

    let connection = rusqlite::Connection::open(dir.path().join("telemetry.sqlite3")).unwrap();
    let (verified, rate, tokens, weak): (i64, f64, f64, i64) = connection
        .query_row(
            "SELECT verified, success_rate, mean_tokens, weak FROM v_route_stats
             WHERE agent='a' AND task_type='implementation'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(verified, 2, "only verified outcomes count as evidence");
    assert_eq!(rate, 1.0);
    assert_eq!(tokens, 2000.0);
    assert_eq!(
        weak, 1,
        "and a weak signal is counted apart, never mixed in"
    );

    let failures: i64 = connection
        .query_row(
            "SELECT failures FROM v_route_stats WHERE agent='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(failures, 1);

    // `v_runs` is the same records with their labels as columns, for a table a client sorts.
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM v_runs WHERE purpose='execution'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 4);

    // Runtime and quota are what the Agents screen shows.
    store
        .save_runtime(&RuntimeState {
            status: RuntimeStatus::Cooldown,
            cooldown_until: Some(now() + 600),
            ..RuntimeState::new("b", "*")
        })
        .unwrap();
    store
        .save_quota_snapshot(&Snapshot {
            agent: "b".into(),
            source: "probe".into(),
            observed_at: now(),
            valid_until: now() + 3600,
            windows: vec![Window {
                bucket: "5h".into(),
                remaining: 0.42,
                reset_at: Some(now() + 1800),
                model: None,
                affects_routing: true,
            }],
        })
        .unwrap();

    let connection = rusqlite::Connection::open(dir.path().join("telemetry.sqlite3")).unwrap();
    let (status, cooling): (String, i64) = connection
        .query_row(
            "SELECT status, cooling FROM v_runtime WHERE agent='b'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "cooldown");
    assert!(cooling > 0, "with the time left on it");

    let (bucket, remaining): (String, f64) = connection
        .query_row(
            "SELECT bucket, remaining FROM v_quota WHERE agent='b'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(bucket, "5h");
    assert!((remaining - 0.42).abs() < 1e-9);
}

/// The EWMA learns `measured / predicted`, which only means anything while the two are made
/// the same way. A prediction from before the prior counted what opening a session costs is
/// low by that whole amount — 2.8× on Codex and 15.5× on Claude in the runs recorded on
/// 2026-09-20 — so mixing its ratio into a corrected prior would scale the new one by the old
/// one's error. Such a run still says whether it succeeded and how long it took.
#[test]
fn a_ratio_learned_against_an_older_prior_is_not_mixed_into_the_new_one() {
    let config = LearningConfig::default();
    let older: Vec<RunRecord> = (0..100)
        .map(|n| {
            let mut record = run(n, true);
            if let Some(prediction) = record.prediction.as_mut() {
                prediction.prior_basis = None;
            }
            record
        })
        .collect();
    let estimate = learning::estimate(&config, 0.9, 1000.0, &older);
    assert!(
        (estimate.tokens - 1000.0).abs() < 1.0,
        "an older prior moved the estimate: {}",
        estimate.tokens
    );
    assert!(estimate.success > 0.9, "but it still counts as a success");

    // The same runs, predicted the way the prior is made now: the ratio is usable again.
    let same: Vec<RunRecord> = (0..100).map(|n| run(n, true)).collect();
    let current = learning::estimate(&config, 0.9, 1000.0, &same);
    assert!((current.tokens - 2000.0).abs() < 1.0, "{}", current.tokens);
}

/// The opening line decides whether a request is for software or about it, and it has to do
/// that from a sentence a person actually wrote. An allowlist of exact pairs ("create a web")
/// read "Create a ToDo web application in this repository, which is empty apart from a
/// README" as a documentation task and routed a whole application build to the cheapest model
/// at the lowest reasoning level (measured 2026-09-22).
#[test]
fn a_request_to_build_an_application_is_not_read_as_a_request_to_write_about_one() {
    let root = tempfile::tempdir().unwrap();
    let kind = |task: &str| profiler::profile(task, root.path()).task_type;

    for builds in [
        "Create a ToDo web application in this repository, which is empty apart from a README.",
        "Create a web app",
        "Build an app that tracks invoices",
        "Implement a feature flag service",
        "Write a small CLI tool for tailing logs",
        "アプリを作成してください",
        "ウェブサイトを実装して",
    ] {
        assert_eq!(kind(builds), "implementation", "{builds}");
    }

    // The body of a build request mentions all sorts of things in passing. None of them is
    // what is being asked for: "renamed inline" in a list of features is not a rename.
    assert_eq!(
        kind(
            "Create a ToDo web application. A task can be added, toggled complete, renamed \
             inline, and deleted. Review the result and investigate anything that fails."
        ),
        "implementation"
    );

    // Asking about software, or for one of those narrower things outright, still is.
    for (task, expected) in [
        ("Write a README for the web app", "documentation"),
        ("Update the documentation for the API", "documentation"),
        ("Write tests for the parser", "test"),
        (
            "Rename is_balanced to balanced across the tree",
            "small_edit",
        ),
        ("Review the authentication module", "review"),
        (
            "Investigate why the parser drops the last token",
            "investigation",
        ),
    ] {
        assert_eq!(kind(task), expected, "{task}");
    }
}

/// The stratum says what work of a kind has cost; it cannot say whether this one is a line or
/// a library. Measured, that is the whole difference: one classification bought a decision
/// over 246,991 tokens of work and another over 5,158, and against the stratum alone the two
/// read as 0.346 and 0.342 — no threshold separates those. The size each task showed before
/// anything ran does separate them.
#[test]
fn what_asking_is_worth_is_weighed_against_this_task_and_not_only_against_its_kind() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let config = Config {
        agents: vec![AgentConfig::preset("codex", Provider::Openai, "codex", &[])],
        ..Config::default()
    };
    let base = profiler::profile("implement the thing", dir.path());
    let sized = |scope: usize| TaskDescriptor {
        estimated_context: 500,
        estimated_scope: scope,
        ..base.clone()
    };
    let spent = |at: usize, purpose: &str, tokens: u64| {
        let mut record = run(at, true);
        record.purpose = purpose.into();
        record.candidate.agent = "codex".into();
        // `run` records scope 1 and context 500, so work here typically looks like 2,000.
        record.usage = Usage {
            total_tokens: Some(tokens),
            ..Default::default()
        };
        record.started_at = at as i64;
        record
    };
    for at in 0..3 {
        store.record(&spent(at, "classification", 24_000)).unwrap();
    }
    for at in 3..7 {
        store.record(&spent(at, "execution", 48_000)).unwrap();
    }

    // A task the size of the work already measured: asking takes half of it, and is refused.
    assert!(!classifier::worth_asking(&config, &store, &sized(1)));
    // A task that looks four times that size: the same question is a quarter of it, and worth
    // asking. Nothing about the stratum changed — only what this task looks like.
    assert!(classifier::worth_asking(&config, &store, &sized(5)));
    // The scaling is bounded, so a task that looks enormous cannot claim an unbounded budget.
    assert_eq!(
        classifier::worth_asking(&config, &store, &sized(100)),
        classifier::worth_asking(&config, &store, &sized(20)),
    );
}
