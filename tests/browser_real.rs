//! The browser tools against a real Chrome, which `tests/browser.rs` never meets: run by hand
//! with `cargo test --test browser_real -- --ignored` on a machine that has Chrome or Chromium.
#![cfg(unix)]
use orochi::{
    browser::{
        broker::{Browser, Request},
        find_chrome,
    },
    config::BrowserConfig,
};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::{io::AsyncWriteExt, net::TcpListener};

const PAGE: &str = "<!doctype html><title>Real</title><h1>Welcome</h1>\
<button style=\"position:absolute;left:20px;top:60px;width:100px;height:40px\" onclick=\"document.getElementById('out').textContent='clicked';document.title='clicked'\">Go</button>\
<input aria-label=\"Search\"><p id=out>waiting</p>";

async fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    port
}

fn request(tool: &str, args: Value) -> Request {
    Request {
        scope: "real".into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    }
}

fn said(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "needs a Chrome or Chromium on this machine"]
async fn a_real_chrome_opens_a_page_reads_it_clicks_types_and_takes_a_picture() {
    let config = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let chrome = find_chrome(&config).expect("a Chrome on this machine");
    let dir = tempfile::tempdir().unwrap();
    let args: Vec<String> = [
        "--headless=new",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
        "--window-size=1280,800",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        format!("--user-data-dir={}", dir.path().display()),
        "about:blank".into(),
    ])
    .collect();
    let browser = Browser::start(&chrome, &args, &config, Stdio::null())
        .await
        .unwrap();
    let port = serve().await;
    let url = format!("http://127.0.0.1:{port}/");

    let opened = browser
        .call(request("browser_navigate", json!({ "url": url })))
        .await;
    assert!(opened["isError"] != true, "{opened}");
    let snapshot = said(&browser.call(request("browser_snapshot", json!({}))).await);
    println!("{snapshot}");
    assert!(snapshot.contains("Welcome"), "{snapshot}");
    let button = snapshot
        .lines()
        .find(|l| l.contains("button") && l.contains("Go"))
        .and_then(|l| l.split("[ref=").nth(1))
        .and_then(|r| r.split(']').next())
        .expect("a button with a ref")
        .to_owned();
    let field = snapshot
        .lines()
        .find(|l| l.contains("textbox"))
        .and_then(|l| l.split("[ref=").nth(1))
        .and_then(|r| r.split(']').next())
        .expect("a textbox with a ref")
        .to_owned();
    let clicked = browser
        .call(request("browser_click", json!({ "ref": button })))
        .await;
    assert!(clicked["isError"] != true, "{clicked}");
    let typed = browser
        .call(request(
            "browser_type",
            json!({ "ref": field, "text": "hello" }),
        ))
        .await;
    assert!(typed["isError"] != true, "{typed}");
    let after = said(&browser.call(request("browser_snapshot", json!({}))).await);
    assert!(after.contains("clicked"), "{after}");
    assert!(after.contains("hello"), "{after}");

    let shot = browser.call(request("browser_screenshot", json!({}))).await;
    let image = shot["content"]
        .as_array()
        .and_then(|c| c.iter().find(|p| p["type"] == "image"))
        .expect("an image part");
    assert_eq!(image["mimeType"], "image/png");
    assert!(image["data"].as_str().unwrap().len() > 1000);

    // The policy holds against the real thing as well.
    let refused = browser
        .call(request(
            "browser_navigate",
            json!({ "url": "https://example.com/" }),
        ))
        .await;
    assert_eq!(refused["isError"], true);
    browser.forget("real").await;
}

#[tokio::test]
#[ignore = "needs a Chrome or Chromium on this machine"]
async fn a_real_chrome_streams_the_persons_tab_and_a_click_through_the_window_reaches_the_page() {
    let config = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let chrome = find_chrome(&config).expect("a Chrome on this machine");
    let dir = tempfile::tempdir().unwrap();
    let args: Vec<String> = [
        "--headless=new",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        format!("--user-data-dir={}", dir.path().display()),
        "about:blank".into(),
    ])
    .collect();
    let browser = Browser::start(&chrome, &args, &config, Stdio::null())
        .await
        .unwrap();
    let port = serve().await;
    let tab = |tool: &str, args: Value| Request {
        scope: "user:t".into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    };
    let view = |value: Value| -> Value {
        assert!(value["isError"] != true, "{value}");
        serde_json::from_str(&said(&value)).unwrap()
    };
    view(
        browser
            .call(tab(
                "view_resize",
                json!({"width": 640, "height": 400, "ratio": 1}),
            ))
            .await,
    );
    view(
        browser
            .call(tab(
                "view_open",
                json!({"url": format!("http://127.0.0.1:{port}/")}),
            ))
            .await,
    );
    let mut since = 0;
    let mut frame = Value::Null;
    let mut title = Value::Null;
    for _ in 0..100 {
        let polled = view(
            browser
                .call(tab("view_frame", json!({"since": since})))
                .await,
        );
        title = polled["title"].clone();
        // A page that is not changing sends one frame and then nothing, which is the point.
        if polled["frame"].is_object() {
            since = polled["frame"]["seq"].as_u64().unwrap();
            frame = polled["frame"].clone();
        }
        if frame.is_object() && title == "Real" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(title, "Real");
    let jpeg = frame["jpeg"].as_str().expect("a frame of the loaded page");
    assert!(jpeg.starts_with("/9j/"), "a JPEG: {}", &jpeg[..8]);
    println!(
        "frame {}x{}, {} base64 bytes",
        frame["width"],
        frame["height"],
        jpeg.len()
    );

    // The button is drawn at 20,60 and 100 by 40: click its middle, as the window would.
    for (kind, buttons) in [("mouseMoved", 0), ("mousePressed", 1), ("mouseReleased", 0)] {
        view(
            browser
                .call(tab(
                    "view_input",
                    json!({"type": "mouse", "kind": kind, "x": 60.0, "y": 80.0,
                           "button": if buttons == 0 && kind == "mouseMoved" { "none" } else { "left" },
                           "buttons": buttons, "clickCount": 1}),
                ))
                .await,
        );
    }
    for _ in 0..50 {
        let polled = view(browser.call(tab("view_frame", json!({"since": 0}))).await);
        title = polled["title"].clone();
        if title == "clicked" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(title, "clicked", "the click reached the page");
    view(browser.call(tab("view_close", json!({}))).await);
}

#[tokio::test]
#[ignore = "needs a Chrome or Chromium on this machine"]
async fn a_real_chrome_previews_a_page_from_a_folder_and_it_cannot_reach_anything_else() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let config = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let chrome = find_chrome(&config).expect("a Chrome on this machine");
    let dir = tempfile::tempdir().unwrap();
    let args: Vec<String> = [
        "--headless=new",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        format!("--user-data-dir={}", dir.path().join("profile").display()),
        "about:blank".into(),
    ])
    .collect();
    let browser = Browser::start(&chrome, &args, &config, Stdio::null())
        .await
        .unwrap();

    // Somewhere else on this machine, which the page below tries to call.
    let hits = Arc::new(AtomicUsize::new(0));
    let outside = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let outside_port = outside.local_addr().unwrap().port();
    let counted = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = outside.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\naccess-control-allow-origin: *\r\ncontent-length: 2\r\nconnection: close\r\n\r\nhi")
                .await;
        }
    });

    let site = dir.path().join("site");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(
        site.join("index.html"),
        format!(
            "<!doctype html><title>start</title><link rel=stylesheet href=style.css>\
             <script src=app.js></script><script>\
             fetch('http://127.0.0.1:{outside_port}/').then(()=>window.leak='leaked').catch(()=>window.leak='blocked')\
             .finally(()=>document.title=(window.own||'no-own')+'/'+window.leak)</script>"
        ),
    )
    .unwrap();
    std::fs::write(site.join("style.css"), "body{background:#123}").unwrap();
    std::fs::write(site.join("app.js"), "window.own='own-script-ran'").unwrap();

    let tab = |tool: &str, args: Value| Request {
        scope: "user:t:p".into(),
        tool: tool.into(),
        args,
        read_only: false,
        cwd: None,
    };
    let view = |value: Value| -> Value {
        assert!(value["isError"] != true, "{value}");
        serde_json::from_str(&said(&value)).unwrap()
    };
    view(
        browser
            .call(tab(
                "view_preview",
                json!({"root": site, "path": "index.html"}),
            ))
            .await,
    );
    let mut title = Value::Null;
    for _ in 0..80 {
        let polled = view(browser.call(tab("view_frame", json!({"since": 0}))).await);
        title = polled["title"].clone();
        if title.as_str().is_some_and(|t| t.contains('/')) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        title, "own-script-ran/blocked",
        "its own files ran; the call out did not"
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "nothing reached the other server"
    );
    view(browser.call(tab("view_close", json!({}))).await);
}

fn chrome_of(profile: &std::path::Path) -> Vec<i32> {
    let ps = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter(|l| l.contains(&format!("--user-data-dir={}", profile.display())))
        .filter(|l| !l.contains("--type="))
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect()
}

#[test]
#[ignore = "needs a Chrome or Chromium on this machine"]
fn a_broker_killed_outright_leaves_no_chrome_in_the_way_of_the_next_and_ends_its_chrome_when_asked()
{
    let config = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let data = tempfile::tempdir().unwrap();
    let data = std::fs::canonicalize(data.path()).unwrap();
    let executable = std::path::PathBuf::from(env!("CARGO_BIN_EXE_orochi"));
    orochi::browser::register(&data, executable.clone(), &config).unwrap();
    let profile = data.join("browser/profile");
    let frame = |client: &mut orochi::browser::Client| {
        client
            .call(&Request {
                scope: "user:t:p".into(),
                tool: "view_frame".into(),
                args: json!({"width": 400, "height": 300}),
                read_only: false,
                cwd: None,
            })
            .unwrap()
    };
    let pid = |name: &str| -> i32 {
        std::fs::read_to_string(data.join("browser").join(name))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };

    let mut first = orochi::browser::Client::open(&executable, &data).unwrap();
    assert!(frame(&mut first)["isError"] != true);
    let broker = pid("broker.pid");
    assert!(!chrome_of(&profile).is_empty(), "a Chrome is running");

    // The broker dies without a chance to stop anything.
    unsafe { libc::kill(broker, libc::SIGKILL) };
    drop(first);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let orphans = chrome_of(&profile);
    println!("left behind: {orphans:?}");

    // The next one starts all the same: it ends what was left and takes the profile.
    let mut second = orochi::browser::Client::open(&executable, &data).unwrap();
    let again = frame(&mut second);
    assert!(again["isError"] != true, "{again}");
    for orphan in orphans {
        assert!(
            !chrome_of(&profile).contains(&orphan),
            "the old Chrome {orphan} is gone"
        );
    }

    // A broker asked to stop takes its Chrome with it.
    let broker = pid("broker.pid");
    drop(second);
    unsafe { libc::kill(broker, libc::SIGTERM) };
    for _ in 0..50 {
        if chrome_of(&profile).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        chrome_of(&profile).is_empty(),
        "no Chrome outlives its broker"
    );
}

#[tokio::test]
#[ignore = "needs a Chrome or Chromium on this machine"]
async fn a_real_chrome_lets_the_person_watch_the_page_an_agent_opened() {
    let config = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let chrome = find_chrome(&config).expect("a Chrome on this machine");
    let dir = tempfile::tempdir().unwrap();
    let args: Vec<String> = [
        "--headless=new",
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        format!("--user-data-dir={}", dir.path().display()),
        "about:blank".into(),
    ])
    .collect();
    let browser = Browser::start(&chrome, &args, &config, Stdio::null())
        .await
        .unwrap();
    let port = serve().await;
    let call = |scope: &str, tool: &str, args: Value| {
        browser.call(Request {
            scope: scope.into(),
            tool: tool.into(),
            args,
            read_only: false,
            cwd: None,
        })
    };
    let agent = "implementer-1a2b3c4d";
    let opened = call(
        agent,
        "browser_navigate",
        json!({"url": format!("http://127.0.0.1:{port}/")}),
    )
    .await;
    assert!(opened["isError"] != true, "{opened}");

    let parse = |value: Value| -> Value {
        assert!(value["isError"] != true, "{value}");
        serde_json::from_str(&said(&value)).unwrap()
    };
    let listed = parse(call("user:t:w1", "view_agents", json!({})).await);
    assert_eq!(listed["agents"][0]["scope"], agent, "{listed}");
    assert_eq!(listed["agents"][0]["label"], "implementer");

    let mut jpeg = String::new();
    for _ in 0..100 {
        let polled = parse(
            call(
                "user:t:w1",
                "view_frame",
                json!({"watch": agent, "since": 0, "width": 640, "height": 400}),
            )
            .await,
        );
        if let Some(data) = polled["frame"]["jpeg"].as_str() {
            jpeg = data.to_owned();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(jpeg.starts_with("/9j/"), "a JPEG of the agent's page");
    // The agent's own tools still work on the page being watched.
    let snapshot = call(agent, "browser_snapshot", json!({})).await;
    assert!(said(&snapshot).contains("Welcome"), "{snapshot}");
    assert!(
        call(
            "user:t:w1",
            "view_input",
            json!({"watch": agent, "type": "mouse", "kind": "mousePressed"})
        )
        .await["isError"]
            == true
    );
    parse(call("user:t:w1", "view_unwatch", json!({"watch": agent})).await);
}
