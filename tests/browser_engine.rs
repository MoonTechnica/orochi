//! Servo behind the person's tab (`docs/servo-browser-design.md` §3): which engine answers a
//! call, what the broker asks of the engine and keeps of what it says, and what an engine's
//! death or a refused address does to the tab. The engine is `tests/fixtures/fake_servo.py`,
//! which speaks the same lines of JSON as `orochi-servo` and logs what it was asked.
#![cfg(unix)]
use base64::Engine as _;
use orochi::{
    browser::{broker::Request, engine::Engines, hub::Hub},
    config::{BrowserConfig, BrowserEngine},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn fake() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_servo.py")
}

fn hub(data: &Path, engine: BrowserEngine, servo: Option<PathBuf>) -> Arc<Hub> {
    std::fs::create_dir_all(data.join("browser")).unwrap();
    let config = BrowserConfig {
        engine,
        servo,
        ..BrowserConfig::default()
    };
    Arc::new(Hub::new(
        None,
        Engines::new(&config, data),
        &config,
        data,
        Arc::new(|| {}),
    ))
}

fn request(scope: &str, tool: &str, args: Value) -> Request {
    Request {
        scope: scope.into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    }
}

fn answer(result: &Value) -> Value {
    assert_ne!(result["isError"], true, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn error(result: &Value) -> String {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

fn asked(data: &Path) -> Vec<Value> {
    std::fs::read_to_string(data.join("browser/servo/fake-servo.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

const TAB: &str = "user:thread-1:b1";

/// `view_frame` until `done` accepts what it says, as the window polls.
async fn poll(hub: &Arc<Hub>, since: u64, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let reply = answer(
            &hub.call(request(TAB, "view_frame", json!({"since": since})))
                .await,
        );
        if done(&reply) {
            return reply;
        }
        assert!(Instant::now() < end, "never arrived: {reply}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_tab_opened_on_servo_answers_with_its_state_and_newest_frame() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    answer(
        &hub.call(request(
            TAB,
            "view_open",
            json!({"url": "http://localhost:3000/a"}),
        ))
        .await,
    );
    let reply = poll(&hub, 0, |r| r["frame"].is_object()).await;
    assert_eq!(reply["engine"], "servo");
    assert_eq!(reply["url"], "http://localhost:3000/a");
    assert_eq!(reply["title"], "fake: http://localhost:3000/a");
    assert_eq!(reply["loading"], false);
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(reply["frame"]["jpeg"].as_str().unwrap())
        .unwrap();
    assert!(
        jpeg.starts_with(b"\xff\xd8"),
        "the engine's bytes, untouched"
    );
    let seq = reply["frame"]["seq"].as_u64().unwrap();
    // The window asks for what is newer than the last it drew, and nothing is.
    let quiet = answer(
        &hub.call(request(TAB, "view_frame", json!({"since": seq})))
            .await,
    );
    assert!(quiet.get("frame").is_none(), "{quiet}");
    assert_eq!(asked(dir.path())[0]["cmd"], "navigate");
}

#[tokio::test]
async fn history_and_resizing_reach_the_engine_and_a_resized_tab_draws_again() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    for url in ["http://localhost/1", "http://localhost/2"] {
        hub.call(request(TAB, "view_navigate", json!({"url": url})))
            .await;
    }
    let reply = poll(&hub, 0, |r| {
        r["url"] == "http://localhost/2" && r["back"] == true
    })
    .await;
    assert_eq!(reply["forward"], false);
    hub.call(request(TAB, "view_back", json!({}))).await;
    let reply = poll(&hub, 0, |r| {
        r["url"] == "http://localhost/1" && r["forward"] == true
    })
    .await;
    let before = reply["frame"]["seq"].as_u64().unwrap();
    hub.call(request(
        TAB,
        "view_resize",
        json!({"width": 390, "height": 844, "ratio": 9}),
    ))
    .await;
    let reply = poll(&hub, before, |r| r["frame"].is_object()).await;
    assert_eq!(
        reply["frame"]["width"],
        390 * 3,
        "the ratio is clamped to 3"
    );
    assert_eq!(reply["frame"]["height"], 844 * 3);
    assert!(reply["frame"]["seq"].as_u64().unwrap() > before);
}

#[tokio::test]
async fn only_web_addresses_are_opened_and_only_known_inputs_are_passed_on() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    for bad in [
        "file:///etc/hosts",
        "javascript:alert(1)",
        "chrome://settings",
    ] {
        let said = error(
            &hub.call(request(TAB, "view_navigate", json!({"url": bad})))
                .await,
        );
        assert!(
            said.contains("not opened") || said.contains("address"),
            "{said}"
        );
    }
    assert!(
        asked(dir.path()).iter().all(|c| c["cmd"] != "navigate"),
        "nothing refused was sent on"
    );
    let said = error(
        &hub.call(request(
            TAB,
            "view_input",
            json!({"type": "script", "text": "1"}),
        ))
        .await,
    );
    assert!(said.contains("not an input"), "{said}");
    answer(
        &hub.call(request(
            TAB,
            "view_input",
            json!({"type": "text", "text": "こんにちは"}),
        ))
        .await,
    );
    let reply = poll(&hub, 0, |r| {
        r["title"].as_str().is_some_and(|t| t.starts_with("typed"))
    })
    .await;
    assert_eq!(reply["title"], "typed: こんにちは");
}

#[tokio::test]
async fn an_agents_scope_never_reaches_a_tab_and_the_list_shows_only_the_persons() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    for tool in ["view_open", "view_frame", "view_input", "view_list"] {
        let said = error(&hub.call(request("claude-1a2b3c4d", tool, json!({}))).await);
        assert!(said.contains("person's browser tab"), "{tool}: {said}");
    }
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/x"}),
    ))
    .await;
    let tabs = answer(&hub.call(request(TAB, "view_list", json!({}))).await);
    assert_eq!(tabs["tabs"][0]["scope"], TAB);
    assert_eq!(tabs["tabs"][0]["engine"], "servo");
    // Chrome's alone: an agent's page is watched through Chrome, never through this engine.
    let said = error(
        &hub.call(request(
            TAB,
            "view_frame",
            json!({"watch": "claude-1a2b3c4d"}),
        ))
        .await,
    );
    assert!(
        said.contains("Chrome") || said.contains("Chromium"),
        "{said}"
    );
}

#[tokio::test]
async fn a_dialog_a_crash_a_refusal_and_a_popup_are_told_to_the_viewer() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://dialog.test/"}),
    ))
    .await;
    let reply = poll(&hub, 0, |r| r["dialog"].is_object()).await;
    assert_eq!(
        reply["dialog"],
        json!({"type": "alert", "message": "hello"})
    );
    hub.call(request(TAB, "view_dialog", json!({"accept": true})))
        .await;
    let reply = answer(&hub.call(request(TAB, "view_frame", json!({}))).await);
    assert!(
        reply.get("dialog").is_none(),
        "answering closes it: {reply}"
    );
    hub.call(request(
        TAB,
        "view_navigate",
        json!({"url": "http://popup.test/"}),
    ))
    .await;
    let reply = poll(&hub, 0, |r| r["popup"].is_string()).await;
    assert_eq!(reply["popup"], "http://popup.test/child");
    hub.call(request(
        TAB,
        "view_navigate",
        json!({"url": "http://crash.test/"}),
    ))
    .await;
    let reply = poll(&hub, 0, |r| r["crashed"] == true).await;
    assert_eq!(reply["url"], "http://crash.test/");
}

#[tokio::test]
async fn closing_a_tab_ends_its_engine_and_a_reopened_tab_goes_on_counting_frames() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/a"}),
    ))
    .await;
    let first = poll(&hub, 0, |r| r["frame"].is_object()).await["frame"]["seq"]
        .as_u64()
        .unwrap();
    let pid: i32 = std::fs::read_to_string(dir.path().join("browser/servo/fake-servo.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    hub.call(request(TAB, "view_close", json!({}))).await;
    let end = Instant::now() + Duration::from_secs(10);
    // SAFETY: signal 0 only asks whether the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < end, "the engine outlived its tab");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/b"}),
    ))
    .await;
    let again = poll(&hub, first, |r| r["frame"].is_object()).await["frame"]["seq"]
        .as_u64()
        .unwrap();
    assert!(again > first, "{again} should come after {first}");
}

#[tokio::test]
async fn the_engine_ends_when_the_broker_does() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/a"}),
    ))
    .await;
    poll(&hub, 0, |r| r["frame"].is_object()).await;
    let pid: i32 = std::fs::read_to_string(dir.path().join("browser/servo/fake-servo.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(hub);
    let end = Instant::now() + Duration::from_secs(10);
    // SAFETY: signal 0 only asks whether the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < end, "the engine outlived its broker");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn the_setting_decides_the_engine_and_a_missing_one_is_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    // Servo asked for and not there: an error that names the way out, not a silent Chrome.
    let hub_servo = hub(dir.path(), BrowserEngine::Servo, None);
    let said = error(&hub_servo.call(request(TAB, "view_open", json!({}))).await);
    assert!(
        said.contains("mise run servo") && said.contains("browser.servo"),
        "{said}"
    );
    // Chrome asked for and not there.
    let hub_chrome = hub(dir.path(), BrowserEngine::Chromium, Some(fake()));
    let said = error(&hub_chrome.call(request(TAB, "view_open", json!({}))).await);
    assert!(said.contains("Chrome"), "{said}");
    // Auto with neither.
    let hub_none = hub(dir.path(), BrowserEngine::Auto, None);
    let said = error(&hub_none.call(request(TAB, "view_open", json!({}))).await);
    assert!(said.contains("orochi-servo"), "{said}");
    // Auto with a binary draws on Servo.
    let hub_auto = hub(dir.path(), BrowserEngine::Auto, Some(fake()));
    answer(
        &hub_auto
            .call(request(
                TAB,
                "view_open",
                json!({"url": "http://localhost/"}),
            ))
            .await,
    );
    let reply = poll(&hub_auto, 0, |r| r["frame"].is_object()).await;
    assert_eq!(reply["engine"], "servo");
}

#[test]
fn the_settings_for_the_engine_and_the_phone_are_checked() {
    use orochi::{browser::policy, config::RemoteConfig};
    let remote = |bind: &str, insecure_lan: bool| BrowserConfig {
        remote: RemoteConfig {
            bind: bind.into(),
            insecure_lan,
            ..RemoteConfig::default()
        },
        ..BrowserConfig::default()
    };
    assert!(policy::check(&BrowserConfig::default()).is_ok());
    assert!(policy::check(&remote("127.0.0.1", false)).is_ok());
    assert!(policy::check(&remote("::1", false)).is_ok());
    assert!(
        policy::check(&remote("100.101.102.103", false)).is_ok(),
        "a tailnet address"
    );
    for lan in [
        "192.168.1.20",
        "10.0.0.5",
        "0.0.0.0",
        "100.63.0.1",
        "100.128.0.1",
    ] {
        let said = policy::check(&remote(lan, false)).unwrap_err().to_string();
        assert!(said.contains("insecure_lan"), "{lan}: {said}");
        assert!(
            policy::check(&remote(lan, true)).is_ok(),
            "{lan} with the person's word"
        );
    }
    assert!(
        policy::check(&remote("my-mac.local", true)).is_err(),
        "an address, not a name"
    );
    let relative = BrowserConfig {
        servo: Some("orochi-servo".into()),
        ..BrowserConfig::default()
    };
    assert!(policy::check(&relative).is_err());
    let toml: BrowserConfig =
        toml::from_str("engine = \"servo\"\n[remote]\nenabled = true").unwrap();
    assert_eq!(toml.engine, BrowserEngine::Servo);
    assert!(toml.remote.enabled && toml.remote.bind == "127.0.0.1");
    assert!(toml::from_str::<BrowserConfig>("engine = \"firefox\"").is_err());
}

#[tokio::test]
async fn the_windows_switch_starts_the_phone_listener_pairs_and_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    let status = answer(&hub.call(request("admin", "remote_status", json!({}))).await);
    assert_eq!(status["running"], false);
    let pairing = answer(&hub.call(request("admin", "remote_pair", json!({}))).await);
    let url = pairing["url"].as_str().unwrap();
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "loopback unless said otherwise: {url}"
    );
    assert!(url.ends_with(&format!("/pair#{}", pairing["code"].as_str().unwrap())));
    let status = answer(&hub.call(request("admin", "remote_status", json!({}))).await);
    assert_eq!(status["running"], true);
    assert_eq!(status["devices"], json!([]));
    let said = answer(
        &hub.call(request("admin", "remote_revoke", json!({"id": "nobody"})))
            .await,
    );
    assert_eq!(said["revoked"], false);
    answer(&hub.call(request("admin", "remote_stop", json!({}))).await);
    let status = answer(&hub.call(request("admin", "remote_status", json!({}))).await);
    assert_eq!(status["running"], false);
    let said = error(&hub.call(request("admin", "remote_bogus", json!({}))).await);
    assert!(said.contains("not something"), "{said}");
}

#[tokio::test]
async fn nothing_is_sent_to_an_engine_before_it_says_it_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    // An engine that is slow to come up and drops what it is sent meanwhile, as the real one does.
    std::fs::create_dir_all(dir.path().join("browser/servo")).unwrap();
    std::fs::write(dir.path().join("browser/servo/fake-servo.slow"), "").unwrap();
    answer(
        &hub.call(request(
            TAB,
            "view_open",
            json!({"url": "http://localhost/slow"}),
        ))
        .await,
    );
    let reply = poll(&hub, 0, |r| r["frame"].is_object()).await;
    assert_eq!(
        reply["url"], "http://localhost/slow",
        "the first address was not lost"
    );
}

#[tokio::test]
async fn an_engine_is_asked_to_exit_and_waited_for_so_the_profile_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/a"}),
    ))
    .await;
    poll(&hub, 0, |r| r["frame"].is_object()).await;
    let pid: i32 = std::fs::read_to_string(dir.path().join("browser/servo/fake-servo.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    hub.shutdown().await;
    // Not killed: it was told to shut down, and `shutdown` returned only after it had gone.
    assert!(asked(dir.path()).iter().any(|c| c["cmd"] == "shutdown"));
    // SAFETY: signal 0 only asks whether the process exists.
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "it has exited");
    // A tab's own close does the same.
    hub.call(request(
        TAB,
        "view_open",
        json!({"url": "http://localhost/b"}),
    ))
    .await;
    poll(&hub, 0, |r| r["frame"].is_object()).await;
    hub.call(request(TAB, "view_close", json!({}))).await;
    let closes = asked(dir.path())
        .iter()
        .filter(|c| c["cmd"] == "shutdown")
        .count();
    assert_eq!(closes, 2);
}
