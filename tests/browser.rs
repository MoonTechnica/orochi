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
    assert!(
        asked(dir.path())
            .iter()
            .all(|c| c["method"] == "Target.setDiscoverTargets"),
        "nothing was asked of Chrome but to report targets"
    );
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

fn viewed(result: &Value) -> Value {
    assert!(!failed(result), "{result}");
    serde_json::from_str(&said(result)).unwrap()
}

#[tokio::test]
async fn the_persons_tab_goes_where_they_choose_streams_frames_and_takes_input() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let tab = |tool: &str, args: Value| request("user:thread-1", tool, args);

    // Not held to the agents' host list, but still only web addresses.
    let opened = viewed(
        &browser
            .call(tab(
                "view_open",
                json!({"url": "https://example.com/somewhere"}),
            ))
            .await,
    );
    assert_eq!(opened["url"], "https://example.com/somewhere");
    let refused = browser
        .call(tab("view_navigate", json!({"url": "file:///etc/passwd"})))
        .await;
    assert!(failed(&refused), "{refused}");

    // The size asks for a picture of that size, and the newest frame is handed over once.
    let first = viewed(
        &browser
            .call(tab(
                "view_resize",
                json!({"width": 800, "height": 600, "ratio": 2}),
            ))
            .await,
    );
    let log = asked(dir.path());
    let metrics = of(&log, "Emulation.setDeviceMetricsOverride")[0];
    assert_eq!(metrics["params"]["width"], 800.0);
    assert_eq!(metrics["params"]["deviceScaleFactor"], 2.0);
    let cast = of(&log, "Page.startScreencast")[0];
    assert_eq!(cast["params"]["maxWidth"], 1600.0, "pixels, not points");
    assert_eq!(cast["params"]["format"], "jpeg");
    let _ = first;
    let mut frame = Value::Null;
    for _ in 0..50 {
        let polled = viewed(&browser.call(tab("view_frame", json!({"since": 0}))).await);
        if polled["frame"].is_object() {
            frame = polled["frame"].clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        frame["jpeg"].as_str().is_some_and(|j| !j.is_empty()),
        "{frame}"
    );
    let seq = frame["seq"].as_u64().unwrap();
    let again = viewed(&browser.call(tab("view_frame", json!({"since": seq}))).await);
    assert!(again["frame"].is_null(), "nothing new, nothing sent");
    assert_eq!(
        of(&asked(dir.path()), "Page.screencastFrameAck").len(),
        1,
        "Chrome is told each frame arrived, or it sends no more"
    );

    // Input goes to the page as the window gave it; a bad kind goes nowhere.
    viewed(
        &browser
            .call(tab(
                "view_input",
                json!({"type": "mouse", "kind": "mousePressed", "x": 10.5, "y": 20.0,
                       "button": "left", "buttons": 1, "clickCount": 1}),
            ))
            .await,
    );
    viewed(
        &browser
            .call(tab(
                "view_input",
                json!({"type": "key", "kind": "keyDown", "key": "a", "code": "KeyA", "vk": 65, "text": "a"}),
            ))
            .await,
    );
    viewed(
        &browser
            .call(tab(
                "view_input",
                json!({"type": "ime", "text": "にほん", "commit": false}),
            ))
            .await,
    );
    viewed(
        &browser
            .call(tab(
                "view_input",
                json!({"type": "ime", "text": "日本", "commit": true}),
            ))
            .await,
    );
    let log = asked(dir.path());
    let mouse = of(&log, "Input.dispatchMouseEvent");
    assert_eq!(mouse[0]["params"]["x"], 10.5);
    assert_eq!(mouse[0]["params"]["button"], "left");
    assert_eq!(of(&log, "Input.dispatchKeyEvent")[0]["params"]["text"], "a");
    assert_eq!(
        of(&log, "Input.imeSetComposition")[0]["params"]["text"],
        "にほん"
    );
    assert_eq!(of(&log, "Input.insertText")[0]["params"]["text"], "日本");
    assert!(failed(
        &browser
            .call(tab(
                "view_input",
                json!({"type": "mouse", "kind": "teleport"})
            ))
            .await
    ));

    // The tab is the person's: an agent's session cannot reach it, nor use these tools.
    let theirs = browser
        .call(request("agent-1", "view_frame", json!({})))
        .await;
    assert!(failed(&theirs));
    assert!(
        said(
            &browser
                .call(request(
                    "agent-1",
                    "browser_tabs",
                    json!({"action": "list"})
                ))
                .await
        )
        .matches("example.com")
        .count()
            == 0
    );
    viewed(&browser.call(tab("view_close", json!({}))).await);
    assert_eq!(
        of(&asked(dir.path()), "Target.disposeBrowserContext").len(),
        1
    );
}

fn fetch(url: &str, method: &str) -> (u16, String) {
    use std::io::Read;
    let without = url.strip_prefix("http://").unwrap();
    let (host, path) = without
        .split_once('/')
        .map_or((without, ""), |(h, p)| (h, p));
    let mut socket = std::net::TcpStream::connect(host).unwrap();
    write!(
        socket,
        "{method} /{path} HTTP/1.1\r\nhost: {host}\r\nconnection: close\r\n\r\n"
    )
    .unwrap();
    let mut reply = String::new();
    socket.read_to_string(&mut reply).unwrap();
    let status = reply.split(' ').nth(1).unwrap().parse().unwrap();
    (
        status,
        reply.split("\r\n\r\n").nth(1).unwrap_or("").to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_preview_serves_one_folder_to_a_tab_that_may_load_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let site = dir.path().join("site");
    std::fs::create_dir_all(site.join(".git")).unwrap();
    std::fs::create_dir_all(site.join("assets")).unwrap();
    std::fs::write(site.join("index.html"), "<h1>hi</h1>").unwrap();
    std::fs::write(site.join("paused.html"), "<p>x</p>").unwrap();
    std::fs::write(site.join("assets/app.css"), "p{}").unwrap();
    std::fs::write(site.join(".git/config"), "secret").unwrap();
    std::fs::write(dir.path().join("outside.txt"), "outside").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside.txt"), site.join("link.txt")).unwrap();
    let tab = |tool: &str, args: Value| request("user:t:p1", tool, args);

    // A file that is not in the folder starts nothing.
    let missing = browser
        .call(tab(
            "view_preview",
            json!({"root": site, "path": "../outside.txt"}),
        ))
        .await;
    assert!(failed(&missing), "{missing}");
    assert!(of(&asked(dir.path()), "Page.navigate").is_empty());

    viewed(
        &browser
            .call(tab(
                "view_preview",
                json!({"root": site, "path": "paused.html"}),
            ))
            .await,
    );
    let log = asked(dir.path());
    // Confined before it went anywhere.
    let order: Vec<_> = log.iter().map(|c| c["method"].as_str().unwrap()).collect();
    assert!(
        order.iter().position(|m| *m == "Fetch.enable")
            < order.iter().position(|m| *m == "Page.navigate"),
        "{order:?}"
    );
    let url = of(&log, "Page.navigate")[0]["params"]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with("/paused.html"),
        "{url}"
    );

    // The folder's own origin is let through, bytes the page carries too; everywhere else is not.
    let mut log = asked(dir.path());
    for _ in 0..50 {
        if of(&log, "Fetch.continueRequest").len() + of(&log, "Fetch.failRequest").len() == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        log = asked(dir.path());
    }
    let continued: Vec<_> = of(&log, "Fetch.continueRequest")
        .iter()
        .map(|c| c["params"]["requestId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(continued, ["r0", "r2"]);
    let failed_ids: Vec<_> = of(&log, "Fetch.failRequest")
        .iter()
        .map(|c| c["params"]["requestId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(failed_ids, ["r1"]);

    // What the server hands out: the folder's files, and nothing past it.
    let base = url.trim_end_matches("paused.html").to_owned();
    assert_eq!(
        fetch(&format!("{base}index.html"), "GET"),
        (200, "<h1>hi</h1>".into())
    );
    assert_eq!(
        fetch(&base, "GET").1,
        "<h1>hi</h1>",
        "a folder is its index"
    );
    assert_eq!(fetch(&format!("{base}assets/app.css"), "GET").0, 200);
    for refused in [
        "../outside.txt",
        "%2e%2e/outside.txt",
        ".git/config",
        "link.txt",
        "missing.html",
    ] {
        assert_eq!(
            fetch(&format!("{base}{refused}"), "GET").0,
            404,
            "{refused}"
        );
    }
    let origin = base.split('/').take(3).collect::<Vec<_>>().join("/");
    assert_eq!(
        fetch(&format!("{origin}/not-the-token/index.html"), "GET").0,
        404
    );
    assert_eq!(fetch(&format!("{base}index.html"), "POST").0, 404);
    assert_eq!(fetch(&format!("{base}index.html"), "HEAD").1, "");
}

#[tokio::test]
async fn a_screenshot_can_be_kept_as_a_png_in_the_project_folder_and_nowhere_else() {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    browser
        .call(request(
            "a",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    let with = |path: &str, read_only: bool| {
        let mut r = request("a", "browser_screenshot", json!({"path": path}));
        r.cwd = Some(project.path().to_path_buf());
        r.read_only = read_only;
        r
    };
    let kept = browser.call(with("shots/login.png", false)).await;
    assert!(!failed(&kept), "{kept}");
    let text: String = kept["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect();
    assert!(
        text.contains("share_file"),
        "it says what to do next: {text}"
    );
    assert!(project.path().join("shots/login.png").is_file());
    assert!(
        kept["content"][0]["type"] == "image",
        "the picture is still given back to the agent"
    );
    for refused in [
        "../outside.png",
        "/tmp/abs.png",
        "notes.txt",
        "a/../../b.png",
    ] {
        let result = browser.call(with(refused, false)).await;
        assert!(failed(&result), "{refused}: {result}");
    }
    assert!(
        !project
            .path()
            .parent()
            .unwrap()
            .join("outside.png")
            .exists()
    );
    let looking = browser.call(with("shots/looked.png", true)).await;
    assert!(failed(&looking), "a seat that may only look writes nothing");
    assert!(!project.path().join("shots/looked.png").exists());
}

#[tokio::test]
async fn the_person_can_watch_an_agents_page_but_not_drive_it_and_watching_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let browser = start(dir.path(), &config()).await;
    let me = |tool: &str, args: Value| request("user:thread-1:b1", tool, args);

    // Nobody has a page yet: the list is empty, and asking made nothing for the person.
    let none = viewed(&browser.call(me("view_agents", json!({}))).await);
    assert_eq!(none["agents"], json!([]));
    assert!(of(&asked(dir.path()), "Target.createTarget").is_empty());

    browser
        .call(request(
            "implementer-1a2b3c4d",
            "browser_navigate",
            json!({"url": "http://localhost:3000/"}),
        ))
        .await;
    browser
        .call(request(
            "reviewer-9f8e7d6c",
            "browser_tabs",
            json!({"action": "list"}),
        ))
        .await;
    let listed = viewed(&browser.call(me("view_agents", json!({}))).await);
    let agents = listed["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1, "only the agent with a page open: {listed}");
    assert_eq!(agents[0]["scope"], "implementer-1a2b3c4d");
    assert_eq!(agents[0]["label"], "implementer");
    assert_eq!(agents[0]["url"], "http://localhost:3000/");

    // Frames of that page, and the person's own tab is still not made.
    let mut frame = Value::Null;
    for _ in 0..50 {
        let polled = viewed(
            &browser
                .call(me(
                    "view_frame",
                    json!({"watch": "implementer-1a2b3c4d", "since": 0, "width": 640, "height": 400}),
                ))
                .await,
        );
        if polled["frame"].is_object() {
            frame = polled["frame"].clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        frame["jpeg"].as_str().is_some_and(|j| !j.is_empty()),
        "{frame}"
    );
    assert_eq!(
        of(&asked(dir.path()), "Target.createTarget").len(),
        1,
        "the agent's tab only"
    );
    assert!(
        of(&asked(dir.path()), "Runtime.evaluate").is_empty(),
        "nothing is run in a page that is watched"
    );

    // Watched, not driven: no input, no navigation, no dialog answers, and not the person's own.
    for tool in [
        "view_input",
        "view_navigate",
        "view_dialog",
        "view_resize",
        "view_close",
    ] {
        let refused = browser
            .call(me(
                tool,
                json!({"watch": "implementer-1a2b3c4d", "type": "mouse", "kind": "mousePressed", "url": "http://localhost:3000/x"}),
            ))
            .await;
        assert!(failed(&refused), "{tool}: {refused}");
    }
    assert!(of(&asked(dir.path()), "Input.dispatchMouseEvent").is_empty());
    assert!(failed(
        &browser
            .call(me("view_frame", json!({"watch": "user:thread-2:b1"})))
            .await
    ));
    assert!(failed(
        &browser
            .call(me("view_frame", json!({"watch": "nobody-00000000"})))
            .await
    ));

    // Leaving stops the stream and leaves the agent's page as it was.
    viewed(
        &browser
            .call(me("view_unwatch", json!({"watch": "implementer-1a2b3c4d"})))
            .await,
    );
    assert_eq!(of(&asked(dir.path()), "Page.stopScreencast").len(), 1);
}
