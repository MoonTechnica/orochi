//! The real `orochi-servo`, run by hand (`cargo test --test browser_servo_real -- --ignored`
//! after `cargo build --release --manifest-path browser-servo/Cargo.toml`), in the way
//! `tests/browser_real.rs` runs a real Chrome: a page served from this machine, opened through
//! the hub, seen as frames and driven with input. Nothing here needs an account or a network.
#![cfg(unix)]
use base64::Engine as _;
use orochi::{
    browser::{broker::Request, engine::Engines, hub::Hub},
    config::{BrowserConfig, BrowserEngine},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>start</title>
<style>body{font:20px sans-serif} #b{width:200px;height:60px;background:#39f}</style>
<input id=i autofocus><div id=b onclick="document.title='clicked '+i.value">go</div>
<script>i.oninput=()=>document.title='typed '+i.value</script>"#;

async fn call(hub: &Arc<Hub>, tool: &str, args: Value) -> Value {
    let reply = hub
        .call(Request {
            scope: "user:real:1".into(),
            tool: tool.into(),
            args,
            read_only: false,
            cwd: None,
        })
        .await;
    assert_ne!(reply["isError"], true, "{reply}");
    serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn until(hub: &Arc<Hub>, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let reply = call(hub, "view_frame", json!({"since": 0})).await;
        if done(&reply) {
            return reply;
        }
        assert!(Instant::now() < end, "{what}: {reply}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[ignore = "needs a built browser-servo/target/release/orochi-servo"]
async fn the_real_engine_draws_a_local_page_and_takes_a_click_and_typing() {
    let binary =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("browser-servo/target/release/orochi-servo");
    assert!(binary.is_file(), "build it first: {}", binary.display());
    let site = tempfile::tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), PAGE).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(site.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));

    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("browser")).unwrap();
    let config = BrowserConfig {
        engine: BrowserEngine::Servo,
        servo: Some(binary),
        ..BrowserConfig::default()
    };
    let hub = Arc::new(Hub::new(
        None,
        Engines::new(&config, data.path()),
        &config,
        data.path(),
        Arc::new(|| {}),
    ));
    call(
        &hub,
        "view_open",
        json!({"url": format!("http://127.0.0.1:{port}/")}),
    )
    .await;
    let reply = until(&hub, "the page", |r| {
        r["frame"].is_object() && r["title"] == "start"
    })
    .await;
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(reply["frame"]["jpeg"].as_str().unwrap())
        .unwrap();
    assert!(jpeg.starts_with(b"\xff\xd8"), "a JPEG");
    std::fs::write(std::env::temp_dir().join("orochi-servo-real.jpg"), &jpeg).unwrap();

    // A click gives the input the keyboard (autofocus does not, in this engine), then typing
    // reaches the page.
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        call(
            &hub,
            "view_input",
            json!({"type": "mouse", "kind": kind, "x": 60, "y": 20,
            "button": "left", "buttons": 1, "clickCount": 1}),
        )
        .await;
    }
    for ch in "hello".chars() {
        call(
            &hub,
            "view_input",
            json!({"type": "key", "kind": "keyDown", "key": ch.to_string(),
            "code": format!("Key{}", ch.to_ascii_uppercase()), "text": ch.to_string()}),
        )
        .await;
        call(
            &hub,
            "view_input",
            json!({"type": "key", "kind": "keyUp", "key": ch.to_string(),
            "code": format!("Key{}", ch.to_ascii_uppercase())}),
        )
        .await;
    }
    until(&hub, "typing", |r| r["title"] == "typed hello").await;
    // A click on the blue box (page coordinates: it is below the input).
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        call(
            &hub,
            "view_input",
            json!({"type": "mouse", "kind": kind, "x": 50, "y": 80,
            "button": "left", "buttons": 1, "clickCount": 1}),
        )
        .await;
    }
    until(&hub, "the click", |r| r["title"] == "clicked hello").await;
    call(&hub, "view_close", json!({})).await;
    let _ = server.kill();
    let _ = server.wait();
}

const SHOP: &str = r#"<!doctype html><meta charset=utf-8><title>shop</title>
<style>body{font:20px sans-serif;margin:0} .gap{height:1500px} #end{display:block}</style>
<h1>Shop</h1><label for=q>Search</label><input id=q>
<div id=ed contenteditable style="border:1px solid #888;height:40px">notes: </div>
<a href="/next.html">Next page</a><div class=gap></div>
<button id=buy onclick="document.title='bought '+q.value+'|'+ed.textContent">Buy</button>
<script>document.addEventListener('keydown',e=>{if(e.key==='Enter'&&document.activeElement===q)document.title='enter '+q.value})</script>"#;

async fn tool(hub: &Arc<Hub>, tool: &str, args: Value) -> Value {
    hub.call(Request {
        scope: "helper-real0001".into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    })
    .await
}

fn said(reply: &Value) -> String {
    assert_ne!(reply["isError"], true, "{reply}");
    reply["content"][0]["text"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "needs a built browser-servo/target/release/orochi-servo"]
async fn the_real_engine_lets_an_agent_read_a_page_by_refs_and_act_on_it() {
    let binary =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("browser-servo/target/release/orochi-servo");
    assert!(binary.is_file(), "build it first: {}", binary.display());
    let site = tempfile::tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), SHOP).unwrap();
    std::fs::write(
        site.path().join("next.html"),
        "<title>second</title><p>the second page",
    )
    .unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(site.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("browser")).unwrap();
    let config = BrowserConfig {
        engine: BrowserEngine::Servo,
        servo: Some(binary),
        agent_evaluate: true,
        ..BrowserConfig::default()
    };
    let hub = Arc::new(Hub::new(
        None,
        Engines::new(&config, data.path()),
        &config,
        data.path(),
        Arc::new(|| {}),
    ));
    let base = format!("http://127.0.0.1:{port}");
    said(&tool(&hub, "browser_navigate", json!({"url": format!("{base}/")})).await);
    let snapshot = said(&tool(&hub, "browser_snapshot", json!({})).await);
    println!("{snapshot}");
    let find = |needle: &str| -> String {
        let line = snapshot
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not in\n{snapshot}"));
        line.split("[ref=")
            .nth(1)
            .unwrap()
            .trim_end_matches(']')
            .to_owned()
    };
    let (search, notes, link, buy) = (
        find("textbox \"Search\""),
        find("textbox \"notes:\""),
        find("link \"Next page\""),
        find("button \"Buy\""),
    );
    assert_ne!(
        search, notes,
        "the editable div is another element: {snapshot}"
    );

    said(
        &tool(
            &hub,
            "browser_type",
            json!({"ref": search, "text": "boots"}),
        )
        .await,
    );
    said(&tool(&hub, "browser_type", json!({"ref": notes, "text": "gift"})).await);
    // The button is far down the page: clicking it scrolls it into view first.
    said(&tool(&hub, "browser_click", json!({"ref": buy})).await);
    let title = said(
        &tool(
            &hub,
            "browser_evaluate",
            json!({"expression": "document.title"}),
        )
        .await,
    );
    assert!(
        title.contains("bought boots|") && title.contains("gift"),
        "typing and a click far below: {title}"
    );

    said(
        &tool(
            &hub,
            "browser_type",
            json!({"ref": search, "text": "!", "submit": true}),
        )
        .await,
    );
    let title = said(
        &tool(
            &hub,
            "browser_evaluate",
            json!({"expression": "document.title"}),
        )
        .await,
    );
    assert!(title.contains("enter boots!"), "Enter submits: {title}");

    let shot = tool(&hub, "browser_screenshot", json!({})).await;
    assert_eq!(shot["content"][0]["type"], "image");
    let png = base64::engine::general_purpose::STANDARD
        .decode(shot["content"][0]["data"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    std::fs::write(std::env::temp_dir().join("orochi-servo-agent.png"), &png).unwrap();

    said(&tool(&hub, "browser_click", json!({"ref": link})).await);
    said(&tool(&hub, "browser_wait", json!({"text": "the second page"})).await);
    assert!(said(&tool(&hub, "browser_snapshot", json!({})).await).contains("the second page"));
    said(&tool(&hub, "browser_back", json!({})).await);
    assert!(said(&tool(&hub, "browser_snapshot", json!({})).await).contains("Shop"));

    hub.forget("helper-real0001").await;
    hub.shutdown().await;
    let _ = server.kill();
    let _ = server.wait();
}

const LOGIN: &str = r#"<!doctype html><meta charset=utf-8><title>login</title>
<style>body{font:20px sans-serif}</style><h1>Sign in</h1><input id=u style="width:300px;height:40px">"#;

#[tokio::test]
#[ignore = "needs a built browser-servo/target/release/orochi-servo"]
async fn the_real_engine_shows_an_agents_page_and_takes_the_persons_hands_only_while_handed_over() {
    let binary =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("browser-servo/target/release/orochi-servo");
    assert!(binary.is_file(), "build it first: {}", binary.display());
    let site = tempfile::tempdir().unwrap();
    std::fs::write(site.path().join("index.html"), LOGIN).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(site.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("browser")).unwrap();
    let config = BrowserConfig {
        engine: BrowserEngine::Servo,
        servo: Some(binary),
        agent_evaluate: true,
        ..BrowserConfig::default()
    };
    let hub = Arc::new(Hub::new(
        None,
        Engines::new(&config, data.path()),
        &config,
        data.path(),
        Arc::new(|| {}),
    ));
    let agent = "helper-real0002";
    said(
        &tool_as(
            &hub,
            agent,
            "browser_navigate",
            json!({"url": format!("http://127.0.0.1:{port}/")}),
        )
        .await,
    );
    let window = |tool: &'static str, args: Value| {
        let hub = hub.clone();
        async move {
            let reply = hub
                .call(Request {
                    scope: "user:real:1".into(),
                    tool: tool.into(),
                    args,
                    read_only: false,
                    cwd: None,
                })
                .await;
            (
                reply["isError"] == true,
                reply["content"][0]["text"]
                    .as_str()
                    .map(|t| serde_json::from_str::<Value>(t).unwrap_or(Value::Null))
                    .unwrap_or(Value::Null),
            )
        }
    };
    let end = Instant::now() + Duration::from_secs(30);
    let frame = loop {
        let (failed, reply) = window("view_frame", json!({"watch": agent, "since": 0})).await;
        assert!(!failed, "{reply}");
        if reply["frame"]["jpeg"].is_string() {
            break reply;
        }
        assert!(Instant::now() < end, "no frame for the window: {reply}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(frame["frame"]["jpeg"].as_str().unwrap())
        .unwrap();
    assert!(jpeg.starts_with(b"\xff\xd8"), "a JPEG");

    // Watched, not driven.
    let (failed, _) = window(
        "view_input",
        json!({"watch": agent, "type": "text", "text": "nope"}),
    )
    .await;
    assert!(failed, "input before a handoff is refused");

    let waiting = tokio::spawn({
        let hub = hub.clone();
        async move {
            tool_as(
                &hub,
                agent,
                "browser_handoff",
                json!({"reason": "type your name"}),
            )
            .await
        }
    });
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, listed) = window("view_agents", json!({})).await;
        if listed["agents"][0]["handoff"] == "type your name" {
            break;
        }
        assert!(Instant::now() < end, "never offered: {listed}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        let (failed, reply) = window(
            "view_input",
            json!({"watch": agent, "type": "mouse", "kind": kind, "x": 60, "y": 110,
                   "button": "left", "buttons": 1, "clickCount": 1}),
        )
        .await;
        assert!(!failed, "{reply}");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (failed, reply) = window(
        "view_input",
        json!({"watch": agent, "type": "ime", "text": "いたばし", "commit": true}),
    )
    .await;
    assert!(!failed, "{reply}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (failed, reply) = window("view_handoff_done", json!({"watch": agent})).await;
    assert!(!failed && reply["returned"] == true, "{reply}");
    let back = said(&waiting.await.unwrap());
    assert!(back.contains("gave the page back"), "{back}");

    let typed = said(
        &tool_as(
            &hub,
            agent,
            "browser_evaluate",
            json!({"expression": "document.getElementById('u').value"}),
        )
        .await,
    );
    assert!(typed.contains("いたばし"), "what the person typed: {typed}");

    let (_, _) = window("view_unwatch", json!({"watch": agent})).await;
    hub.forget(agent).await;
    hub.shutdown().await;
    let _ = server.kill();
    let _ = server.wait();
}

async fn tool_as(hub: &Arc<Hub>, scope: &str, tool: &str, args: Value) -> Value {
    hub.call(Request {
        scope: scope.into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    })
    .await
}
