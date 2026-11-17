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
