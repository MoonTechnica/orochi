//! The decision model: typed questions about a request over the System One API, answered by a
//! local server Orochi puts in and keeps, or by the cloud. Driven against
//! `tests/fixtures/fake_systemone.py`, with no network, no account and no keychain.
use orochi::{
    config::{Config, DecisionBackend},
    decision::{self, Gguf, Model, key, local},
    policy::Registry,
    router::profiler,
    storage::Store,
    types::Complexity,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_systemone.py")
}

struct Server {
    child: Child,
    url: String,
    log: PathBuf,
    _dir: tempfile::TempDir,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn start(answers: Value, env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log.jsonl");
        let file = dir.path().join("answers.json");
        std::fs::write(&file, answers.to_string()).unwrap();
        let mut child = Command::new("python3")
            .arg(fixture())
            .env("MOCK_SYSTEMONE_LOG", &log)
            .env("MOCK_SYSTEMONE_ANSWERS", &file)
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut url = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut url)
            .unwrap();
        Self {
            child,
            url: url.trim().to_owned(),
            log,
            _dir: dir,
        }
    }
    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// A config that reaches the fixture as the chosen backend and can never read a real key.
fn config(backend: DecisionBackend, endpoint: &str) -> Config {
    let mut config = Config::default();
    config.decision.backend = Some(backend);
    config.decision.endpoint = Some(format!("{endpoint}/v1/systemone"));
    config.decision.keychain = false;
    config.decision.api_key_env = Some("OROCHI_TEST_DECISION_KEY_NEVER_SET".into());
    config
}

async fn classify(
    config: &Config,
    data: &Path,
    root: &Path,
    task: &str,
) -> (orochi::types::TaskDescriptor, bool, Vec<String>) {
    let store = Store::open(data).unwrap();
    let policies = Registry::bundled().unwrap();
    let mut descriptor = profiler::profile(task, root);
    let mut notes = vec![];
    let decided = decision::refine(
        config,
        &policies,
        &store,
        root,
        task,
        &mut descriptor,
        |p| {
            if let orochi::acp::Progress::Note(n) = p {
                notes.push(n)
            }
        },
    )
    .await;
    (descriptor, decided, notes)
}

#[test]
fn nothing_is_asked_until_a_decision_model_is_chosen_and_the_agent_classifier_is_off_by_default() {
    let config = Config::default();
    assert_eq!(config.decision.backend, None);
    assert!(!decision::chosen(&config.decision));
    assert!(!config.classifier.enabled);
    // lev is recommended, and only ever on this machine; the cloud has no default.
    assert_eq!(config.decision.model, "lev");
    assert_eq!(decision::RECOMMENDED, "lev");
    let lev = decision::model("lev").unwrap();
    assert!(lev.gguf.is_some() && lev.cloud.is_none());
    assert!(
        decision::recommend(tempfile::tempdir().unwrap().path())
            .backend
            .is_none_or(|b| b == DecisionBackend::Local)
    );
    // A cloud model goes to its maker, never a reseller.
    assert_eq!(
        decision::model("jev").unwrap().cloud.unwrap().provider,
        "typesafe"
    );
    assert_eq!(
        decision::model("gpt-6-luna")
            .unwrap()
            .cloud
            .unwrap()
            .provider,
        "openai"
    );
    assert!(decision::model("kev-4b").unwrap().cloud.is_none());
    // Every cloud model names a provider Orochi knows, so it has a key to be reached with.
    for m in decision::MODELS {
        if let Some(cloud) = m.cloud {
            assert!(decision::provider(cloud.provider).is_some(), "{}", m.name);
        }
        assert!(
            m.gguf.is_some() || m.cloud.is_some(),
            "{} runs nowhere",
            m.name
        );
    }
}

#[test]
fn an_endpoint_that_would_carry_the_task_in_plaintext_off_this_machine_is_refused() {
    let mut config = Config::default();
    config.decision.endpoint = Some("http://example.com/v1/systemone".into());
    assert!(config.validate().is_err());
    config.decision.endpoint = Some("http://127.0.0.1:9/v1/systemone".into());
    assert!(config.validate().is_ok());
    config.decision.endpoint = Some("https://example.com/v1/systemone".into());
    assert!(config.validate().is_ok());
}

#[test]
fn an_unsure_general_never_replaces_a_specific_heuristic_label_and_flags_need_conviction() {
    let root = tempfile::tempdir().unwrap();
    let heuristic = profiler::profile("Add a --json flag to the runs command", root.path());
    assert_ne!(heuristic.task_type, "general");
    let cuts = decision::model("kev-4b").unwrap().cuts;
    let answers = json!({
        "task_type": {"choice": "general", "probabilities": {"general": 0.2}},
        "complexity": {"score": 2.45},
        "requires_web": {"noul": 0.65},
        "requires_browser": {"noul": 0.95},
        "suited": {"choice": "none", "probabilities": {"none": 0.9}},
        "prefer": {"choice": "codex", "probabilities": {"codex": 0.3}},
    });
    let c = decision::interpret(&answers, &cuts, &heuristic).unwrap();
    assert_eq!(c.task_type, heuristic.task_type);
    assert_eq!(c.complexity, Complexity::Extreme);
    assert!(!c.requires_web, "0.65 is not enough to narrow the field");
    assert!(c.requires_browser);
    assert_eq!(c.suited, None);
    assert_eq!(c.prefer, None, "a name it is unsure of is not a preference");
    let invented = json!({"task_type": {"choice": "astrology"}});
    assert!(decision::interpret(&invented, &cuts, &heuristic).is_none());
}

#[tokio::test]
async fn a_chosen_decision_model_labels_the_request_and_a_cached_answer_asks_nothing() {
    let server = Server::start(json!({"task_type": "bug_fix", "complexity": 1.7}), &[]);
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let config = config(DecisionBackend::Local, &server.url);
    let task = "ログイン画面でパスワードを入力するとクラッシュする。直して";
    let (descriptor, decided, notes) = classify(&config, data.path(), root.path(), task).await;
    assert!(decided, "{notes:?}");
    assert_eq!(descriptor.task_type, "bug_fix");
    assert_eq!(descriptor.complexity, Complexity::Complex);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["request"]["model"], "lev");
    assert_eq!(
        requests[0]["authorization"],
        Value::Null,
        "a local server gets no key"
    );
    assert!(
        requests[0]["request"]["state"][0]
            .as_str()
            .unwrap()
            .contains(task)
    );
    let (again, decided, notes) = classify(&config, data.path(), root.path(), task).await;
    assert!(
        decided && notes.iter().any(|n| n.contains("cached")),
        "{notes:?}"
    );
    assert_eq!(again.task_type, "bug_fix");
    assert_eq!(server.requests().len(), 1);
    // The cache is labels under a salted hash; the request's words stay out of telemetry.
    let telemetry = std::fs::read(data.path().join("telemetry.sqlite3")).unwrap();
    assert!(
        !telemetry
            .windows("クラッシュ".len())
            .any(|w| w == "クラッシュ".as_bytes())
    );
}

#[tokio::test]
async fn complexity_is_only_ever_raised_by_the_decision_model() {
    let server = Server::start(json!({"task_type": "migration", "complexity": 0.1}), &[]);
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let task = "Rewrite the whole backend from Python to Rust, every service";
    let heuristic = profiler::profile(task, root.path());
    let (descriptor, decided, _) = classify(
        &config(DecisionBackend::Local, &server.url),
        data.path(),
        root.path(),
        task,
    )
    .await;
    assert!(decided);
    assert_eq!(
        descriptor.complexity,
        heuristic.complexity.max(Complexity::Simple)
    );
    assert!(descriptor.complexity >= heuristic.complexity);
}

#[tokio::test]
async fn the_cloud_backend_sends_the_stored_key_and_without_one_keeps_the_heuristic() {
    let server = Server::start(json!({"task_type": "review"}), &[]);
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut config = config(DecisionBackend::Cloud, &server.url);
    config.decision.model = "jev".into();
    let task = "Review my changes to the storage layer";
    let (descriptor, decided, notes) = classify(&config, data.path(), root.path(), task).await;
    assert!(!decided);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("keeping the local profile")),
        "{notes:?}"
    );
    assert_eq!(
        descriptor.task_type,
        profiler::profile(task, root.path()).task_type
    );
    assert!(server.requests().is_empty());

    key::put(data.path(), &config.decision, "typesafe", "ts-test-123").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.path().join("decision/keys/typesafe"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let (descriptor, decided, _) = classify(&config, data.path(), root.path(), task).await;
    assert!(decided);
    assert_eq!(descriptor.task_type, "review");
    let requests = server.requests();
    assert_eq!(requests[0]["authorization"], "Bearer ts-test-123");
    assert_eq!(requests[0]["request"]["model"], "jev-1.14.0");
    assert!(key::forget(data.path(), &config.decision, "typesafe").unwrap());
    assert!(key::get(data.path(), &config.decision, "typesafe").is_none());
}

#[tokio::test]
async fn a_failing_decision_model_never_stops_the_run_or_echoes_what_it_was_sent() {
    let server = Server::start(json!({}), &[("MOCK_SYSTEMONE_STATUS", "500")]);
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let task = "secret-project-name: fix the flaky test";
    let (descriptor, decided, notes) = classify(
        &config(DecisionBackend::Local, &server.url),
        data.path(),
        root.path(),
        task,
    )
    .await;
    assert!(!decided);
    assert_eq!(
        descriptor.task_type,
        profiler::profile(task, root.path()).task_type
    );
    assert!(
        notes.iter().all(|n| !n.contains("secret-project-name")),
        "{notes:?}"
    );
}

#[tokio::test]
async fn preferences_are_asked_only_when_something_is_remembered() {
    let server = Server::start(json!({"task_type": "implementation"}), &[]);
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let config = config(DecisionBackend::Local, &server.url);
    classify(&config, data.path(), root.path(), "Add a --json flag").await;
    let questions = &server.requests()[0]["request"]["questions"];
    assert!(questions.get("task_type").is_some() && questions.get("complexity").is_some());
    assert!(questions.get("prefer").is_none() && questions.get("avoid").is_none());
}

/// A llama.cpp release in miniature: an archive whose `llama-server` is the fixture.
fn release(dir: &Path) {
    let top = dir.join("build/llama-b12202");
    std::fs::create_dir_all(&top).unwrap();
    let server = top.join("llama-server");
    std::fs::write(
        &server,
        format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", fixture().display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (archive, _) = local::platform().expect("a platform llama.cpp builds for");
    let files = dir.join("files");
    std::fs::create_dir_all(files.join("b12202")).unwrap();
    let out = files.join("b12202").join(archive);
    assert!(
        Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(dir.join("build"))
            .arg("llama-b12202")
            .status()
            .unwrap()
            .success()
    );
}
fn sha(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

#[tokio::test]
async fn a_model_whose_download_does_not_match_its_pinned_checksum_is_never_kept() {
    let dir = tempfile::tempdir().unwrap();
    let files = dir.path().join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("model.gguf"), b"not the model").unwrap();
    let server = Server::start(
        json!({}),
        &[("MOCK_SYSTEMONE_FILES", files.to_str().unwrap())],
    );
    let model: &'static Model = Box::leak(Box::new(Model {
        name: "tiny",
        summary: "",
        license: "test",
        measured: None,
        gguf: Some(Gguf {
            url: "unused",
            file: "model.gguf",
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            bytes: 13,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 0,
    }));
    let data = tempfile::tempdir().unwrap();
    // llama.cpp already in place, so only the model is fetched.
    let bin = data.path().join("decision/llama/b12202");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(
        bin.join(if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        }),
        b"",
    )
    .unwrap();
    let url = format!("{}/files/model.gguf", server.url);
    let error = local::install(
        &data.path().join("decision"),
        model,
        &server.url,
        Some(&url),
        |_| {},
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("checksum"), "{error:#}");
    let models = data.path().join("decision/models");
    assert!(!models.join("model.gguf").exists());
    assert!(!models.join("model.part").exists());
    assert_eq!(
        local::Download::read(&data.path().join("decision"))
            .unwrap()
            .state,
        "failed"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_local_model_downloads_resumes_after_a_cut_starts_one_shared_server_and_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    release(dir.path());
    let files = dir.path().join("files");
    let gguf: Vec<u8> = (0..200_000u32).flat_map(|i| i.to_le_bytes()).collect();
    std::fs::write(files.join("model.gguf"), &gguf).unwrap();
    let files_env = files.to_str().unwrap().to_owned();
    // The first full response is cut after 100 KB, as a dropped connection would.
    let server = Server::start(
        json!({}),
        &[
            ("MOCK_SYSTEMONE_FILES", &files_env),
            ("MOCK_SYSTEMONE_CUT", "100000"),
        ],
    );
    let model: &'static Model = Box::leak(Box::new(Model {
        name: "tiny",
        summary: "",
        license: "test",
        measured: None,
        gguf: Some(Gguf {
            url: "unused",
            file: "model.gguf",
            sha256: Box::leak(sha(&gguf).into_boxed_str()),
            bytes: gguf.len() as u64,
        }),
        cloud: None,
        cuts: [0.5, 1.5, 2.5],
        memory_gb: 0,
    }));
    let data = tempfile::tempdir().unwrap();
    // llama.cpp's archive is pinned by the build's own checksums, which the miniature cannot
    // match, so it is unpacked in place as `install` would leave it.
    let stage = data.path().join("decision/llama");
    std::fs::create_dir_all(&stage).unwrap();
    assert!(
        Command::new("cp")
            .arg("-R")
            .arg(dir.path().join("build/llama-b12202"))
            .arg(stage.join("b12202"))
            .status()
            .unwrap()
            .success()
    );
    let releases = format!("{}/files", server.url);
    let url = format!("{}/files/model.gguf", server.url);
    let first = local::install(
        &data.path().join("decision"),
        model,
        &releases,
        Some(&url),
        |_| {},
    )
    .await;
    assert!(first.is_err(), "the cut download cannot pass its checksum");
    let part = data.path().join("decision/models/model.part");
    assert!(
        part.metadata().unwrap().len() > 0,
        "what arrived is kept to resume from"
    );
    local::install(
        &data.path().join("decision"),
        model,
        &releases,
        Some(&url),
        |_| {},
    )
    .await
    .unwrap();
    assert!(local::installed(&data.path().join("decision"), model));
    assert_eq!(
        std::fs::read(data.path().join("decision/models/model.gguf")).unwrap(),
        gguf
    );

    local::supervisor(env!("CARGO_BIN_EXE_orochi").into());
    let mut config = Config::default();
    config.decision.model = "tiny".into();
    config.decision.timeout_ms = 30_000;
    let endpoint = local::ensure(&data.path().join("decision"), &config.decision, model)
        .await
        .unwrap();
    let port =
        local::running(&data.path().join("decision"), "tiny").expect("the server is recorded");
    assert!(endpoint.ends_with(&format!(":{port}/v1/systemone")));
    let again = local::ensure(&data.path().join("decision"), &config.decision, model)
        .await
        .unwrap();
    assert_eq!(again, endpoint, "a second run shares the server");
    assert!(local::stop(&data.path().join("decision")));
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert_eq!(local::running(&data.path().join("decision"), "tiny"), None);
}

#[tokio::test]
async fn a_cloud_model_is_reached_with_its_own_providers_key_and_openais_decisions_shape_is_read_back()
 {
    let server = Server::start(
        json!({"task_type": "bug_fix", "complexity": 2.5, "requires_web": 0.95}),
        &[],
    );
    let (data, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut config = config(DecisionBackend::Cloud, &server.url);
    config.decision.model = "gpt-6-luna".into();
    config.decision.endpoint = Some(format!("{}/v1/decisions", server.url));
    // TypeSafe's key is not OpenAI's: with only that one stored, nothing is sent.
    key::put(data.path(), &config.decision, "typesafe", "ts-elsewhere").unwrap();
    let task = "The app crashes when the config file is missing; fix it";
    let (_, decided, _) = classify(&config, data.path(), root.path(), task).await;
    assert!(!decided);
    assert!(server.requests().is_empty());

    key::put(data.path(), &config.decision, "openai", "sk-openai-123").unwrap();
    let (descriptor, decided, notes) = classify(&config, data.path(), root.path(), task).await;
    assert!(decided, "{notes:?}");
    assert_eq!(descriptor.task_type, "bug_fix");
    assert_eq!(descriptor.complexity, Complexity::Extreme);
    assert!(descriptor.requires_web);
    let sent = &server.requests()[0];
    assert_eq!(sent["path"], "/v1/decisions");
    assert_eq!(sent["authorization"], "Bearer sk-openai-123");
    let request = &sent["request"];
    assert_eq!(request["model"], "gpt-6-luna");
    assert!(request["input"].as_str().unwrap().contains(task));
    let questions = request["questions"].as_array().unwrap();
    let of = |name: &str| questions.iter().find(|q| q["name"] == name).unwrap();
    assert_eq!(of("requires_web")["type"], "predicate");
    assert!(
        of("task_type")["choices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["value"] == "bug_fix")
    );
    assert_eq!(of("complexity")["levels"].as_array().unwrap().len(), 4);
}

#[test]
fn a_key_names_only_a_provider_orochi_knows() {
    let data = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.decision.keychain = false;
    assert!(key::put(data.path(), &config.decision, "../../etc", "x").is_err());
    assert!(key::put(data.path(), &config.decision, "nobody", "x").is_err());
    assert!(key::put(data.path(), &config.decision, "openai", "sk-1").is_ok());
}

#[tokio::test]
async fn the_decision_model_names_the_agent_that_carries_on_only_among_those_offered() {
    let offered = vec!["claude".to_owned(), "opencode".to_owned()];
    let data = tempfile::tempdir().unwrap();
    let server = Server::start(json!({"handover": "opencode"}), &[]);
    let cfg = config(DecisionBackend::Local, &server.url);
    let picked = decision::handover(&cfg.decision, data.path(), "続きを実装して", &offered).await;
    assert_eq!(picked.as_deref(), Some("opencode"));
    let questions = &server.requests()[0]["request"]["questions"];
    assert_eq!(questions["handover"]["type"], "choice");
    assert!(
        questions["handover"]["criteria"]["claude"].is_string(),
        "the agents offered are the choices"
    );
    // An agent it was not offered is never taken, however it answers.
    let server = Server::start(json!({"handover": "codex"}), &[]);
    let cfg = config(DecisionBackend::Local, &server.url);
    assert_eq!(
        decision::handover(&cfg.decision, data.path(), "続きを実装して", &offered).await,
        None
    );
}
