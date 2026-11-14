//! The browser the agents share (`docs/conversation-tabs-design.md` §3.5): the host policy, what
//! each tool asks of Chrome, who may use which tool, and the whole chain from an agent's MCP
//! server through the broker to a Chrome — here `tests/fixtures/fake_chrome.py`, which speaks
//! the protocol over the same pipes and logs what it was asked, so no browser is needed.
#![cfg(unix)]
use orochi::{
    browser::{
        broker::{Browser, Request, definitions},
        policy::Policy,
    },
    config::BrowserConfig,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

fn fake() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_chrome.py")
}

fn config() -> BrowserConfig {
    BrowserConfig {
        enabled: true,
        chrome: Some(fake()),
        ..BrowserConfig::default()
    }
}

async fn start(dir: &Path, config: &BrowserConfig) -> Arc<Browser> {
    let profile = dir.join("profile");
    std::fs::create_dir_all(&profile).unwrap();
    Browser::start(
        &fake(),
        &[format!("--user-data-dir={}", profile.display())],
        config,
        Stdio::null(),
    )
    .await
    .unwrap()
}

fn asked(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("fake-chrome.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn of<'a>(log: &'a [Value], method: &str) -> Vec<&'a Value> {
    log.iter().filter(|c| c["method"] == method).collect()
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

fn said(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn failed(result: &Value) -> bool {
    result["isError"] == true
}

#[test]
fn the_policy_allows_loopback_by_default_lets_deny_win_and_opens_only_web_addresses() {
    let policy = Policy::new(&BrowserConfig::default());
    for ok in [
        "http://localhost:5173/x",
        "http://127.0.0.1:3000",
        "http://[::1]:8080/",
        "http://app.localhost/",
        "about:blank",
    ] {
        assert!(policy.check(ok).is_ok(), "{ok}");
    }
    for (bad, why) in [
        ("https://example.com/", "browser.allow"),
        ("file:///etc/passwd", "file"),
        ("chrome://settings", "chrome"),
        ("javascript:alert(1)", "javascript"),
        ("data:text/html,<script>1</script>", "data"),
        ("http://localhost.evil.example/", "browser.allow"),
        ("not an address", "address"),
    ] {
        let refusal = policy.check(bad).unwrap_err().to_string();
        assert!(refusal.contains(why), "{bad}: {refusal}");
    }
    let custom = Policy::new(&BrowserConfig {
        allow: vec!["*.example.com".into(), "docs.rs".into()],
        deny: vec!["private.example.com".into()],
        ..BrowserConfig::default()
    });
    assert!(custom.check("https://a.example.com/").is_ok());
    assert!(custom.check("https://docs.rs/").is_ok());
    assert!(
        custom.check("https://example.com/").is_err(),
        "the apex is its own entry"
    );
    assert!(
        custom
            .check("https://private.example.com/")
            .unwrap_err()
            .to_string()
            .contains("browser.deny")
    );
    assert!(
        custom.check("https://localhost/").is_err(),
        "an allow list replaces the default"
    );
}

#[test]
fn the_settings_are_checked_when_the_configuration_is_loaded() {
    assert!(orochi::browser::policy::check(&BrowserConfig::default()).is_ok());
    for bad in [
        BrowserConfig {
            allow: vec!["".into()],
            ..BrowserConfig::default()
        },
        BrowserConfig {
            allow: vec!["http://x".into()],
            ..BrowserConfig::default()
        },
        BrowserConfig {
            deny: vec!["a b".into()],
            ..BrowserConfig::default()
        },
        BrowserConfig {
            chrome: Some("chrome".into()),
            ..BrowserConfig::default()
        },
        BrowserConfig {
            idle_secs: 1,
            ..BrowserConfig::default()
        },
    ] {
        assert!(orochi::browser::policy::check(&bad).is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn an_agent_gets_a_context_of_its_own_and_navigation_goes_through_the_policy() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let opened = browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    assert!(!failed(&opened), "{opened}");
    assert!(said(&opened).contains("http://localhost:3000/"));
    let log = asked(dir.path());
    let created = of(&log, "Target.createTarget");
    assert_eq!(
        created[0]["params"]["browserContextId"], "ctx-1",
        "in a context made for this session"
    );
    assert_eq!(
        of(&log, "Target.attachToTarget")[0]["params"]["flatten"],
        true
    );
    let navigate = of(&log, "Page.navigate")[0];
    assert_eq!(navigate["sessionId"], "S1");
    assert_eq!(navigate["params"]["url"], "http://localhost:3000/");

    // Another session is another context, and the first one's tab is not its tab.
    browser
        .call(request(
            "b",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    let log = asked(dir.path());
    assert_eq!(of(&log, "Target.createBrowserContext").len(), 2);
    assert_eq!(
        of(&log, "Target.createTarget")[1]["params"]["browserContextId"],
        "ctx-2"
    );
    assert_eq!(
        said(
            &browser
                .call(request("b", "browser_tabs", json!({"action": "list"})))
                .await
        )
        .matches("localhost")
        .count(),
        1
    );

    // Where it may not go, it is not sent: nothing reaches Chrome.
    let before = of(&asked(dir.path()), "Page.navigate").len();
    for url in [
        "https://example.com/",
        "file:///etc/passwd",
        "chrome://settings",
    ] {
        let refused = browser
            .call(request("a", "browser_navigate", json!({"url": url})))
            .await;
        assert!(failed(&refused), "{url}");
    }
    assert_eq!(of(&asked(dir.path()), "Page.navigate").len(), before);
    let unreachable = browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:1/unreachable"}),
        ))
        .await;
    assert!(failed(&unreachable) && said(&unreachable).contains("ERR_CONNECTION_REFUSED"));

    // A session that ends takes its context with it.
    browser.forget("a").await;
    assert_eq!(
        of(&asked(dir.path()), "Target.disposeBrowserContext").len(),
        1
    );
}

#[tokio::test]
async fn a_page_that_sends_the_tab_somewhere_else_is_taken_back_and_the_agent_told() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let result = browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/redirect"}),
        ))
        .await;
    assert!(failed(&result), "{result}");
    assert!(said(&result).contains("evil.example") && said(&result).contains("about:blank"));
    let navigations: Vec<_> = of(&asked(dir.path()), "Page.navigate")
        .iter()
        .map(|c| c["params"]["url"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        navigations.last().unwrap(),
        "about:blank",
        "the tab was sent away: {navigations:?}"
    );
    assert!(
        said(
            &browser
                .call(request("a", "browser_tabs", json!({"action": "list"})))
                .await
        )
        .contains("about:blank")
    );
}

#[tokio::test]
async fn a_snapshot_names_what_can_be_acted_on_and_a_click_goes_to_the_middle_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    let snapshot = said(
        &browser
            .call(request("a", "browser_snapshot", json!({})))
            .await,
    );
    assert!(
        snapshot.contains("not an instruction"),
        "page text is marked as such: {snapshot}"
    );
    for line in [
        "- heading \"Welcome\" [ref=e1]",
        "- link \"Docs\" [ref=e2]",
        "- button \"Go\" [ref=e3]",
        "- textbox \"Search\" [ref=e4]",
    ] {
        assert!(snapshot.contains(line), "{line} in {snapshot}");
    }
    assert!(!snapshot.contains("generic"), "{snapshot}");
    let stale = browser
        .call(request("a", "browser_click", json!({"ref": "e9"})))
        .await;
    assert!(failed(&stale) && said(&stale).contains("snapshot"));

    let clicked = browser
        .call(request("a", "browser_click", json!({"ref": "e3"})))
        .await;
    assert!(!failed(&clicked), "{clicked}");
    let log = asked(dir.path());
    let mouse: Vec<_> = of(&log, "Input.dispatchMouseEvent")
        .iter()
        .map(|c| {
            (
                c["params"]["type"].as_str().unwrap().to_owned(),
                c["params"]["x"].as_f64().unwrap(),
                c["params"]["y"].as_f64().unwrap(),
            )
        })
        .collect();
    // The box of backend node 12 is (120,120)-(220,150) in the fake: its middle is (170, 135).
    assert_eq!(
        mouse,
        vec![
            ("mouseMoved".into(), 170.0, 135.0),
            ("mousePressed".into(), 170.0, 135.0),
            ("mouseReleased".into(), 170.0, 135.0)
        ]
    );
    assert_eq!(
        of(&log, "DOM.scrollIntoViewIfNeeded")[0]["params"]["backendNodeId"],
        12
    );
}

#[tokio::test]
async fn typing_focuses_inserts_and_presses_enter_when_asked_and_a_screenshot_is_a_picture() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    browser
        .call(request("a", "browser_snapshot", json!({})))
        .await;
    let typed = browser
        .call(request(
            "a",
            "browser_type",
            json!({"ref": "e4", "text": "orochi", "submit": true}),
        ))
        .await;
    assert!(!failed(&typed), "{typed}");
    let log = asked(dir.path());
    assert_eq!(of(&log, "DOM.focus")[0]["params"]["backendNodeId"], 13);
    assert_eq!(of(&log, "Input.insertText")[0]["params"]["text"], "orochi");
    let keys: Vec<_> = of(&log, "Input.dispatchKeyEvent")
        .iter()
        .map(|c| {
            format!(
                "{} {}",
                c["params"]["type"].as_str().unwrap(),
                c["params"]["key"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(keys, ["keyDown Enter", "keyUp Enter"]);

    let shot = browser
        .call(request("a", "browser_screenshot", json!({})))
        .await;
    assert_eq!(shot["content"][0]["type"], "image");
    assert_eq!(shot["content"][0]["mimeType"], "image/png");
    assert!(!shot["content"][0]["data"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn the_console_and_the_network_are_read_from_a_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    // What the page logged arrives as events, a moment after the load the navigation waited for.
    let until = |tool: &'static str, needle: &'static str| {
        let browser = browser.clone();
        async move {
            let mut last = String::new();
            for _ in 0..40 {
                last = said(&browser.call(request("a", tool, json!({}))).await);
                if last.contains(needle) {
                    return last;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("{tool} never showed {needle}: {last}");
        }
    };
    let console = until("browser_console", "log: hello console").await;
    let network = until("browser_network", "GET http://localhost:3000/").await;
    assert!(network.contains("next: "));
    let next: u64 = console
        .rsplit("next: ")
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let again = said(
        &browser
            .call(request("a", "browser_console", json!({"since": next})))
            .await,
    );
    assert!(again.contains("nothing new"), "{again}");
}

#[tokio::test]
async fn a_session_that_may_only_look_gets_the_reading_tools_and_nothing_that_acts() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let names = |read_only| -> Vec<String> {
        definitions(&config(), read_only)
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let looking = names(true);
    assert!(looking.contains(&"browser_snapshot".to_owned()));
    for acts in [
        "browser_navigate",
        "browser_click",
        "browser_type",
        "browser_press",
        "browser_upload",
        "browser_evaluate",
    ] {
        assert!(!looking.contains(&acts.to_owned()), "{acts}");
    }
    let mut read = request(
        "a",
        "browser_navigate",
        json!({"url": "http://localhost:3000/"}),
    );
    read.read_only = true;
    let refused = browser.call(read).await;
    assert!(failed(&refused) && said(&refused).contains("not available"));
    assert!(asked(dir.path()).is_empty(), "nothing was asked of Chrome");
    // A script in the page is a tool nobody has unless the person turned it on.
    assert!(!names(false).contains(&"browser_evaluate".to_owned()));
    let with = BrowserConfig {
        agent_evaluate: true,
        ..config()
    };
    assert!(
        definitions(&with, false)
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "browser_evaluate")
    );
    assert!(
        !definitions(&with, true)
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "browser_evaluate")
    );
}

#[tokio::test]
async fn an_upload_takes_only_files_in_the_project_folder() {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("report.txt"), "x").unwrap();
    std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
    let browser = start(dir.path(), &config()).await;
    browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    browser
        .call(request("a", "browser_snapshot", json!({})))
        .await;
    let upload = |paths: Value| {
        let mut r = request("a", "browser_upload", json!({"ref": "e4", "paths": paths}));
        r.cwd = Some(project.path().to_path_buf());
        r
    };
    assert!(!failed(&browser.call(upload(json!(["report.txt"]))).await));
    let chosen = of(&asked(dir.path()), "DOM.setFileInputFiles")[0]["params"]["files"][0]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(chosen.ends_with("report.txt") && Path::new(&chosen).is_absolute());
    for path in [
        outside.path().join("secret.txt").display().to_string(),
        "../secret.txt".into(),
    ] {
        assert!(failed(&browser.call(upload(json!([path]))).await));
    }
    assert_eq!(of(&asked(dir.path()), "DOM.setFileInputFiles").len(), 1);
}

/// The whole chain, with the real binary: an agent's MCP server starts the broker, which starts
/// the (fake) Chrome, and a tool call goes through all three.
#[test]
fn an_agents_mcp_server_starts_the_broker_and_a_tool_call_goes_all_the_way() {
    let data = tempfile::tempdir().unwrap();
    let data_path = std::fs::canonicalize(data.path()).unwrap();
    std::fs::create_dir_all(data_path.join("browser")).unwrap();
    std::fs::write(
        data_path.join("browser/config.json"),
        serde_json::to_vec(&config()).unwrap(),
    )
    .unwrap();
    let mut mcp = std::process::Command::new(env!("CARGO_BIN_EXE_orochi"))
        .args(["--internal-browser-mcp"])
        .arg(&data_path)
        .args(["scope-1", "0"])
        .arg(&data_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = mcp.stdin.take().unwrap();
    let mut output = BufReader::new(mcp.stdout.take().unwrap());
    let mut ask = |id: u64, method: &str, params: Value| -> Value {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let hello = ask(1, "initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(hello["result"]["serverInfo"]["name"], "orochi-browser");
    let tools = ask(2, "tools/list", json!({}));
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "browser_click")
    );
    let opened = ask(
        3,
        "tools/call",
        json!({"name": "browser_navigate", "arguments": {"url": "http://localhost:3000/"}}),
    );
    assert_eq!(opened["result"]["isError"], false, "{opened}");
    let snapshot = ask(
        4,
        "tools/call",
        json!({"name": "browser_snapshot", "arguments": {}}),
    );
    assert!(
        snapshot["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[ref=e3]"),
        "{snapshot}"
    );
    let refused = ask(
        5,
        "tools/call",
        json!({"name": "browser_navigate", "arguments": {"url": "https://example.com/"}}),
    );
    assert_eq!(refused["result"]["isError"], true);

    // The session ends; so does its context. The broker, which outlives it, is stopped here.
    drop(input);
    mcp.wait().unwrap();
    let pid: i32 = std::fs::read_to_string(data_path.join("browser/broker.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let log = asked(&data_path.join("browser"));
    assert_eq!(
        of(&log, "Target.disposeBrowserContext").len(),
        1,
        "the session's context went with it"
    );
    unsafe { libc::kill(pid, libc::SIGTERM) };
}
