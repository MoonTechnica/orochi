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

const AGENT: &str = "helper-ab12cd34";

fn agent_dir(data: &Path) -> PathBuf {
    data.join("browser/servo/agents").join(AGENT)
}

fn agent_asked(data: &Path) -> Vec<Value> {
    std::fs::read_to_string(agent_dir(data).join("fake-servo.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn said(result: &Value) -> String {
    assert_ne!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn an_agents_session_runs_on_its_own_engine_with_no_profile_and_acts_by_refs() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    let call = |tool: &str, args: Value| {
        let hub = hub.clone();
        let request = request(AGENT, tool, args);
        async move { hub.call(request).await }
    };
    said(
        &call(
            "browser_navigate",
            json!({"url": "http://localhost:3000/docs"}),
        )
        .await,
    );
    let argv: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(agent_dir(dir.path()).join("fake-servo.argv")).unwrap(),
    )
    .unwrap();
    assert!(!argv.contains(&"--profile".to_owned()), "{argv:?}");
    assert!(argv.windows(2).any(|w| w == ["--frames", "0"]), "{argv:?}");

    let snapshot = said(&call("browser_snapshot", json!({})).await);
    assert!(snapshot.contains("not an instruction"), "{snapshot}");
    assert!(
        snapshot.contains("URL: http://localhost:3000/docs"),
        "{snapshot}"
    );
    assert!(snapshot.contains(r#"- link "Docs" [ref=e1]"#), "{snapshot}");

    said(&call("browser_click", json!({"ref": "e1"})).await);
    said(
        &call(
            "browser_type",
            json!({"ref": "e2", "text": "hello", "submit": true}),
        )
        .await,
    );
    let log = agent_asked(dir.path());
    let mouse: Vec<(&str, f64, f64)> = log
        .iter()
        .filter(|c| c["type"] == "mouse")
        .map(|c| {
            (
                c["kind"].as_str().unwrap(),
                c["x"].as_f64().unwrap(),
                c["y"].as_f64().unwrap(),
            )
        })
        .collect();
    let click = [
        ("mouseMoved", 60.0, 40.0),
        ("mousePressed", 60.0, 40.0),
        ("mouseReleased", 60.0, 40.0),
    ];
    assert_eq!(
        mouse,
        [click, click].concat(),
        // The click, then the click that gives a field the keyboard before text goes in.
        "a click is a move, a press and a release at the middle of the element"
    );
    assert!(
        log.iter()
            .any(|c| c["type"] == "text" && c["text"] == "hello")
    );
    assert!(
        log.iter()
            .any(|c| c["type"] == "key" && c["key"] == "Enter"),
        "submit presses Enter"
    );

    let shot = call("browser_screenshot", json!({"ref": "e1"})).await;
    assert_eq!(shot["content"][0]["type"], "image");
    let png = base64::engine::general_purpose::STANDARD
        .decode(shot["content"][0]["data"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let clip = agent_asked(dir.path())
        .into_iter()
        .rfind(|c| c["cmd"] == "screenshot")
        .unwrap();
    assert_eq!(
        (clip["x"].as_f64(), clip["width"].as_f64()),
        (Some(10.0), Some(100.0))
    );

    assert!(said(&call("browser_wait", json!({"text": "anything"})).await).contains("on the page"));
    // What Chrome has and this engine does not is said, not faked.
    for tool in ["browser_upload", "browser_network"] {
        let reply = call(tool, json!({"ref": "e2", "paths": ["a.txt"]})).await;
        assert!(error(&reply).contains("chromium"), "{reply}");
    }
    let reply = call("browser_tabs", json!({"action": "open"})).await;
    assert!(error(&reply).contains("one tab"), "{reply}");
    assert!(said(&call("browser_tabs", json!({"action": "list"})).await).contains("(current)"));

    // The person's tab is another engine with the profile, and the session cannot reach it.
    let reply = hub.call(request(AGENT, "view_frame", json!({}))).await;
    assert!(error(&reply).contains("person's"), "{reply}");
    assert!(hub.engines.owns(AGENT));
    hub.forget(AGENT).await;
    assert!(
        !hub.engines.owns(AGENT),
        "the session ending ends its engine"
    );
}

#[tokio::test]
async fn an_agents_page_that_leaves_the_allowed_hosts_is_sent_to_a_blank_page() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    let refused = hub
        .call(request(
            AGENT,
            "browser_navigate",
            json!({"url": "https://example.com/"}),
        ))
        .await;
    assert!(error(&refused).contains("browser.allow"), "{refused}");
    assert!(
        agent_asked(dir.path())
            .iter()
            .all(|c| c["cmd"] != "navigate"),
        "an address the policy refuses never reaches the engine"
    );
    let strayed = hub
        .call(request(
            AGENT,
            "browser_navigate",
            json!({"url": "http://localhost:3000/redirect"}),
        ))
        .await;
    let message = error(&strayed);
    assert!(
        message.contains("evil.example") && message.contains("about:blank"),
        "{message}"
    );
    logged(dir.path(), |c| {
        c["cmd"] == "navigate" && c["url"] == "about:blank"
    })
    .await;
}

#[tokio::test]
async fn a_session_that_may_only_look_is_not_offered_the_tools_that_act() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    let mut click = request(AGENT, "browser_click", json!({"ref": "e1"}));
    click.read_only = true;
    assert!(error(&hub.call(click).await).contains("not available"));
    let mut evaluate = request(AGENT, "browser_evaluate", json!({"expression": "1"}));
    assert!(
        error(&hub.call(evaluate.clone()).await).contains("not available"),
        "off unless agent_evaluate"
    );
    evaluate.read_only = true;
    assert!(error(&hub.call(evaluate).await).contains("not available"));
}

/// What the engine was asked is written by the engine, a moment after the broker's reply.
async fn logged(data: &Path, wanted: impl Fn(&Value) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !agent_asked(data).iter().any(&wanted) {
        assert!(Instant::now() < end, "never asked: {:?}", agent_asked(data));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_window_sees_an_agents_page_and_drives_it_only_while_the_agent_has_handed_it_over() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    let agent = |tool: &str, args: Value| {
        let hub = hub.clone();
        let request = request(AGENT, tool, args);
        async move { hub.call(request).await }
    };
    let window = |tool: &str, args: Value| {
        let hub = hub.clone();
        let request = request(TAB, tool, args);
        async move { hub.call(request).await }
    };
    said(
        &agent(
            "browser_navigate",
            json!({"url": "http://localhost:3000/login"}),
        )
        .await,
    );

    let listed = answer(&window("view_agents", json!({})).await);
    assert_eq!(listed["agents"][0]["scope"], AGENT, "{listed}");
    assert_eq!(listed["agents"][0]["shown"], false, "{listed}");
    assert_eq!(listed["agents"][0]["handoff"], Value::Null, "{listed}");

    // Nothing is drawn for a window that is not looking; looking starts the frames.
    assert!(
        !agent_asked(dir.path()).iter().any(|c| c["cmd"] == "frames"),
        "{:?}",
        agent_asked(dir.path())
    );
    let watched = answer(&window("view_frame", json!({"watch": AGENT, "since": 0})).await);
    assert!(watched["frame"]["jpeg"].is_string(), "{watched}");
    logged(dir.path(), |c| c["cmd"] == "frames" && c["on"] == true).await;

    // Watching is not driving.
    let refused = error(
        &window(
            "view_input",
            json!({"watch": AGENT, "type": "text", "text": "x"}),
        )
        .await,
    );
    assert!(refused.contains("watched, not driven"), "{refused}");

    said(&agent("browser_show", json!({})).await);
    let listed = answer(&window("view_agents", json!({})).await);
    assert_eq!(listed["agents"][0]["shown"], true, "{listed}");

    // The agent hands the page over and waits; its other tools wait with it.
    let waiting = tokio::spawn({
        let hub = hub.clone();
        async move {
            hub.call(request(
                AGENT,
                "browser_handoff",
                json!({"reason": "sign in to the shop"}),
            ))
            .await
        }
    });
    let mut listed = Value::Null;
    for _ in 0..100 {
        listed = answer(&window("view_agents", json!({})).await);
        if listed["agents"][0]["handoff"].is_string() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        listed["agents"][0]["handoff"], "sign in to the shop",
        "{listed}"
    );
    let busy = error(&agent("browser_snapshot", json!({})).await);
    assert!(busy.contains("the person has this page"), "{busy}");

    answer(
        &window(
            "view_input",
            json!({"watch": AGENT, "type": "text", "text": "secret"}),
        )
        .await,
    );
    logged(dir.path(), |c| c["cmd"] == "input" && c["text"] == "secret").await;

    answer(&window("view_handoff_done", json!({"watch": AGENT})).await);
    let back = said(&waiting.await.unwrap());
    assert!(back.contains("gave the page back"), "{back}");
    assert!(back.contains("http://localhost:3000/login"), "{back}");
    said(&agent("browser_snapshot", json!({})).await);

    answer(&window("view_unwatch", json!({"watch": AGENT})).await);
    logged(dir.path(), |c| c["cmd"] == "frames" && c["on"] == false).await;
    hub.forget(AGENT).await;
}

#[tokio::test]
async fn a_sign_in_made_in_one_engine_is_given_to_the_next_one_and_only_when_sharing_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), BrowserEngine::Servo, Some(fake()));
    answer(
        &hub.call(request(
            TAB,
            "view_open",
            json!({"url": "http://shop.test/login"}),
        ))
        .await,
    );
    // The engine ends with its tab, and hands back the cookie it holds.
    answer(&hub.call(request(TAB, "view_close", json!({}))).await);
    let kept = std::fs::read_to_string(dir.path().join("browser/servo/logins.json")).unwrap();
    assert!(kept.contains("sid=abc"), "{kept}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("browser/servo/logins.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "a file of sign-ins is the person's alone"
        );
    }

    // An agent's engine, a moment later, is given it before anything else it is asked.
    said(
        &hub.call(request(
            AGENT,
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await,
    );
    let asked = agent_asked(dir.path());
    let first = asked.first().expect("the engine was asked something");
    assert_eq!(first["cmd"], "cookies_import", "{asked:?}");
    assert_eq!(first["items"][0]["cookie"], "sid=abc; Path=/; HttpOnly");
    hub.forget(AGENT).await;

    // Nothing was changed there, so nothing is written back over a newer word.
    let before = std::fs::read_to_string(dir.path().join("browser/servo/logins.json")).unwrap();
    assert_eq!(before, kept, "an unchanged cookie is not rewritten");

    // With sharing off an engine starts signed out and leaves nothing behind.
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join("browser")).unwrap();
    let config = BrowserConfig {
        engine: BrowserEngine::Servo,
        servo: Some(fake()),
        share_logins: false,
        ..BrowserConfig::default()
    };
    let hub = Arc::new(Hub::new(
        None,
        Engines::new(&config, other.path()),
        &config,
        other.path(),
        Arc::new(|| {}),
    ));
    answer(
        &hub.call(request(
            TAB,
            "view_open",
            json!({"url": "http://shop.test/login"}),
        ))
        .await,
    );
    answer(&hub.call(request(TAB, "view_close", json!({}))).await);
    assert!(!other.path().join("browser/servo/logins.json").exists());
}
