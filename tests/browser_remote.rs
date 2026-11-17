//! The phone listener (`docs/servo-browser-design.md` §6): who may connect, what a paired phone
//! may ask, one driver per tab, and what is kept on disk. Everything runs on loopback against a
//! fake broker that records what it is asked; no browser and no network are involved.
#![cfg(unix)]
use base64::Engine as _;
use orochi::{
    browser::{
        broker::Request,
        policy,
        remote::{Dispatch, Remote, Reply},
    },
    config::RemoteConfig,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

const JPEG: &[u8] = b"\xff\xd8not really a picture\xff\xd9";

#[derive(Default)]
struct Fake {
    seen: Mutex<Vec<Request>>,
}

impl Fake {
    fn tools(&self) -> Vec<(String, String)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.scope.clone(), r.tool.clone()))
            .collect()
    }
}

fn said(value: Value) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false})
}

impl Dispatch for Fake {
    fn call(&self, request: Request) -> Reply {
        self.seen.lock().unwrap().push(request.clone());
        Box::pin(async move {
            match request.tool.as_str() {
                "view_list" => said(json!({"tabs": [
                    {"scope": "user:t1:1", "title": "On the computer", "url": "http://localhost/", "engine": "chromium"}
                ]})),
                "view_frame" if request.args["slow"] == true => {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    said(json!({"url": "http://localhost/", "title": "t"}))
                }
                "view_frame" if request.args["since"] == 0 => said(json!({
                    "url": "http://localhost/", "title": "t",
                    "frame": {"seq": 7, "jpeg": base64::engine::general_purpose::STANDARD.encode(JPEG),
                              "width": 800, "height": 600}
                })),
                "view_frame" => said(json!({"url": "http://localhost/", "title": "t"})),
                "view_open" if request.args["url"] == "refuse" => {
                    json!({"content": [{"type": "text", "text": "refused by the broker"}], "isError": true})
                }
                _ => said(json!({})),
            }
        })
    }
}

struct Rig {
    remote: Arc<Remote>,
    fake: Arc<Fake>,
    address: SocketAddr,
    data: tempfile::TempDir,
}

async fn rig() -> Rig {
    rig_with(RemoteConfig {
        enabled: true,
        ..RemoteConfig::default()
    })
    .await
}

async fn rig_with(config: RemoteConfig) -> Rig {
    let data = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let remote = Remote::start(data.path(), &config, fake.clone(), Arc::new(|| {}))
        .await
        .unwrap();
    let address = remote.address();
    Rig {
        remote,
        fake,
        address,
        data,
    }
}

struct Answer {
    status: u16,
    headers: String,
    body: String,
}

async fn http(
    address: SocketAddr,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes)).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Answer {
        status: head
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        headers: head.to_owned(),
        body: body.to_owned(),
    }
}

async fn redeem(address: SocketAddr, code: &str) -> Answer {
    let origin = format!("http://{address}");
    http(
        address,
        "POST",
        "/pair",
        &[
            ("Origin", &origin),
            ("Content-Type", "application/json"),
            ("User-Agent", "Mozilla/5.0 (iPhone)"),
        ],
        &json!({"code": code}).to_string(),
    )
    .await
}

fn cookie_of(answer: &Answer) -> String {
    answer
        .headers
        .lines()
        .find_map(|l| l.strip_prefix("Set-Cookie: "))
        .and_then(|c| c.split(';').next())
        .expect("a cookie")
        .to_owned()
}

/// A paired device's cookie header.
async fn pair(rig: &Rig) -> String {
    let code = rig.remote.pair().unwrap().code;
    let answer = redeem(rig.address, &code).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    cookie_of(&answer)
}

enum Message {
    Text(Value),
    Binary(Vec<u8>),
    Closed,
}

struct Ws {
    stream: TcpStream,
    next: u64,
}

async fn upgrade(
    address: SocketAddr,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<Ws, Answer> {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!(
        "GET /ws HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n"
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    if let Some(origin) = origin {
        request.push_str(&format!("Origin: {origin}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if status != 101 {
        let mut rest = String::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(1), stream.read_to_string(&mut rest)).await;
        return Err(Answer {
            status,
            headers: text,
            body: rest,
        });
    }
    // RFC 6455 §1.3's own example.
    assert!(
        text.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
        "{text}"
    );
    Ok(Ws { stream, next: 1 })
}

async fn connect(rig: &Rig, cookie: &str) -> Ws {
    let origin = format!("http://{}", rig.address);
    match upgrade(rig.address, Some(cookie), Some(&origin)).await {
        Ok(ws) => ws,
        Err(answer) => panic!("not upgraded: {} {}", answer.status, answer.body),
    }
}

impl Ws {
    async fn send(&mut self, value: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let mut value = value;
        value["id"] = json!(id);
        self.send_raw(value.to_string().as_bytes()).await;
        id
    }

    async fn send_raw(&mut self, payload: &[u8]) {
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let mut out = vec![0x81u8];
        match payload.len() {
            n if n < 126 => out.push(0x80 | n as u8),
            n => {
                out.push(0x80 | 126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
        }
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&out).await.unwrap();
    }

    async fn recv(&mut self) -> Message {
        loop {
            let mut head = [0u8; 2];
            match tokio::time::timeout(Duration::from_secs(5), self.stream.read_exact(&mut head))
                .await
            {
                Ok(Ok(_)) => {}
                _ => return Message::Closed,
            }
            let opcode = head[0] & 0x0f;
            let length = match head[1] & 0x7f {
                126 => {
                    let mut b = [0u8; 2];
                    self.stream.read_exact(&mut b).await.unwrap();
                    usize::from(u16::from_be_bytes(b))
                }
                127 => {
                    let mut b = [0u8; 8];
                    self.stream.read_exact(&mut b).await.unwrap();
                    u64::from_be_bytes(b) as usize
                }
                n => usize::from(n),
            };
            let mut payload = vec![0u8; length];
            self.stream.read_exact(&mut payload).await.unwrap();
            match opcode {
                0x1 => return Message::Text(serde_json::from_slice(&payload).unwrap()),
                0x2 => return Message::Binary(payload),
                0x8 => return Message::Closed,
                _ => {}
            }
        }
    }

    /// A call and its answer, as the text JSON the phone's page would read.
    async fn call(&mut self, tool: &str, scope: Option<&str>, args: Value) -> Value {
        let mut message = json!({"tool": tool, "args": args});
        if let Some(scope) = scope {
            message["scope"] = json!(scope);
        }
        let id = self.send(message).await;
        match self.recv().await {
            Message::Text(value) => {
                assert_eq!(value["id"], json!(id));
                value
            }
            Message::Binary(bytes) => {
                let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
                let mut value: Value = serde_json::from_slice(&bytes[4..4 + length]).unwrap();
                value["jpeg_len"] = json!(bytes.len() - 4 - length);
                value["jpeg_ok"] = json!(&bytes[4 + length..] == JPEG);
                value
            }
            Message::Closed => panic!("the socket closed"),
        }
    }
}

fn error_of(value: &Value) -> String {
    value["error"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn an_unpaired_visitor_gets_the_pairing_page_and_no_socket() {
    let rig = rig().await;
    let page = http(rig.address, "GET", "/", &[], "").await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Pair this phone"));
    assert!(
        !page.body.contains("Orochi browser"),
        "the viewer must not be served unpaired"
    );
    let origin = format!("http://{}", rig.address);
    let refused = upgrade(rig.address, None, Some(&origin))
        .await
        .err()
        .unwrap();
    assert_eq!(refused.status, 401);
    let forged = upgrade(
        rig.address,
        Some(&format!("orochi_remote={}", "a".repeat(64))),
        Some(&origin),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(forged.status, 401);
    assert!(rig.fake.tools().is_empty());
}

#[tokio::test]
async fn a_code_works_once_and_a_wrong_or_expired_one_never() {
    let rig = rig().await;
    let wrong = redeem(rig.address, &"0".repeat(32)).await;
    assert_eq!(wrong.status, 403);
    assert!(!wrong.headers.contains("Set-Cookie"));

    let code = rig.remote.pair().unwrap().code;
    let good = redeem(rig.address, &code).await;
    assert_eq!(good.status, 200);
    assert!(
        good.headers.contains("HttpOnly") && good.headers.contains("SameSite=Strict"),
        "{}",
        good.headers
    );
    assert!(
        !good.headers.contains("Secure"),
        "plain HTTP must not claim Secure"
    );
    assert_eq!(
        redeem(rig.address, &code).await.status,
        403,
        "a code is spent by its first use"
    );

    let expired = rig.remote.pair_expiring(0).unwrap().code;
    assert_eq!(redeem(rig.address, &expired).await.status, 403);

    let cookie = cookie_of(&good);
    let viewer = http(rig.address, "GET", "/", &[("Cookie", &cookie)], "").await;
    assert!(viewer.body.contains("Orochi browser"));
    assert!(viewer.headers.contains("Content-Security-Policy"));
    assert_eq!(rig.remote.devices().len(), 1);
    assert!(rig.remote.devices()[0].name.starts_with("iPhone"));
}

#[tokio::test]
async fn pairing_is_locked_after_five_wrong_guesses() {
    let rig = rig().await;
    let code = rig.remote.pair().unwrap().code;
    for _ in 0..5 {
        assert_eq!(redeem(rig.address, &"f".repeat(32)).await.status, 403);
    }
    assert_eq!(
        redeem(rig.address, &code).await.status,
        429,
        "even the right code waits"
    );
    assert!(rig.remote.devices().is_empty());
}

#[tokio::test]
async fn pairing_needs_the_pages_own_origin_and_the_listeners_own_host() {
    let rig = rig().await;
    let code = rig.remote.pair().unwrap().code;
    let from_elsewhere = http(
        rig.address,
        "POST",
        "/pair",
        &[("Origin", "http://evil.example")],
        &json!({"code": code}).to_string(),
    )
    .await;
    assert_eq!(from_elsewhere.status, 403);
    let mut stream = TcpStream::connect(rig.address).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: rebound.example\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).await.unwrap();
    assert!(text.starts_with("HTTP/1.1 421"), "{text}");
    // The code was not spent by either attempt.
    assert_eq!(redeem(rig.address, &code).await.status, 200);
}

#[tokio::test]
async fn a_paired_phone_lists_tabs_and_gets_frames_as_binary_messages() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;

    let hello = ws.call("ping", None, json!({})).await;
    let device = hello["ok"]["device"].as_str().unwrap().to_owned();
    assert_eq!(device, rig.remote.devices()[0].id);
    assert_eq!(rig.remote.connected(), vec![device.clone()]);

    let listed = ws.call("view_list", None, json!({})).await;
    assert_eq!(listed["ok"]["tabs"][0]["scope"], "user:t1:1");

    let frame = ws
        .call("view_frame", Some("user:t1:1"), json!({"since": 0}))
        .await;
    assert_eq!(frame["ok"]["frame"]["seq"], 7);
    assert!(
        frame["ok"]["frame"].get("jpeg").is_none(),
        "the picture is not also base64"
    );
    assert_eq!(frame["jpeg_ok"], true);

    let none = ws
        .call("view_frame", Some("user:t1:1"), json!({"since": 7}))
        .await;
    assert!(none["ok"].get("frame").is_none());
    assert!(
        rig.fake
            .tools()
            .contains(&("user:t1:1".into(), "view_frame".into()))
    );
}

#[tokio::test]
async fn a_phone_may_not_use_another_origin() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let wrong = upgrade(rig.address, Some(&cookie), Some("http://evil.example"))
        .await
        .err()
        .unwrap();
    assert_eq!(wrong.status, 403);
    let none = upgrade(rig.address, Some(&cookie), None)
        .await
        .err()
        .unwrap();
    assert_eq!(
        none.status, 403,
        "a socket opened without an Origin is not the page's own"
    );
}

#[tokio::test]
async fn only_view_tools_and_only_the_persons_tabs_are_reachable() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    for tool in [
        "browser_navigate",
        "remote_pair",
        "view_preview",
        "__end",
        "Page.navigate",
    ] {
        let answer = ws.call(tool, Some("user:t1:1"), json!({})).await;
        assert!(
            error_of(&answer).contains("not available"),
            "{tool}: {answer}"
        );
    }
    for scope in ["claude-1a2b3c4d", "", "agent:user:x"] {
        let answer = ws.call("view_frame", Some(scope), json!({})).await;
        assert!(
            error_of(&answer).contains("not a tab of the person"),
            "{scope:?}: {answer}"
        );
    }
    // A name the broker never listed is not made into a tab by being named.
    let unknown = ws.call("view_frame", Some("user:other:9"), json!({})).await;
    assert!(
        error_of(&unknown).contains("not one the person has open"),
        "{unknown}"
    );
    assert!(
        !rig.fake
            .tools()
            .iter()
            .any(|(scope, _)| scope == "user:other:9")
    );
    let none = ws
        .call("view_navigate", None, json!({"url": "http://localhost/"}))
        .await;
    assert!(error_of(&none).contains("a tab is needed"));
}

#[tokio::test]
async fn a_phone_opens_tabs_only_under_its_own_name_and_only_three() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    let device = rig.remote.devices()[0].id.clone();
    let own = format!("user:phone-{device}:");

    let stranger = ws
        .call(
            "view_open",
            Some("user:phone-dffffffff:a"),
            json!({"url": ""}),
        )
        .await;
    assert!(
        error_of(&stranger).contains("phone opens tabs named"),
        "{stranger}"
    );
    let window = ws
        .call("view_open", Some("user:t1:1"), json!({"url": ""}))
        .await;
    assert!(
        error_of(&window).contains("phone opens tabs named"),
        "{window}"
    );

    for name in ["a", "b", "c"] {
        let opened = ws
            .call(
                "view_open",
                Some(&format!("{own}{name}")),
                json!({"url": ""}),
            )
            .await;
        assert!(opened.get("ok").is_some(), "{opened}");
    }
    let fourth = ws
        .call("view_open", Some(&format!("{own}d")), json!({"url": ""}))
        .await;
    assert!(error_of(&fourth).contains("at most 3"), "{fourth}");
    // Closing one makes room.
    ws.call("view_close", Some(&format!("{own}a")), json!({}))
        .await;
    let again = ws
        .call("view_open", Some(&format!("{own}d")), json!({"url": ""}))
        .await;
    assert!(again.get("ok").is_some(), "{again}");
}

#[tokio::test]
async fn the_broker_s_refusal_reaches_the_phone_as_its_error() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    let own = format!("user:phone-{}:", rig.remote.devices()[0].id);
    let answer = ws
        .call(
            "view_open",
            Some(&format!("{own}x")),
            json!({"url": "refuse"}),
        )
        .await;
    assert_eq!(error_of(&answer), "refused by the broker");
}

#[tokio::test]
async fn one_viewer_drives_a_tab_until_another_takes_control() {
    let rig = rig().await;
    let (a, b) = (pair(&rig).await, pair(&rig).await);
    let mut first = connect(&rig, &a).await;
    let mut second = connect(&rig, &b).await;
    let tab = Some("user:t1:1");
    let key = json!({"type": "key", "kind": "keyDown", "key": "a"});

    assert!(
        first
            .call("view_input", tab, key.clone())
            .await
            .get("ok")
            .is_some()
    );
    let refused = second.call("view_input", tab, key.clone()).await;
    assert_eq!(error_of(&refused), "another viewer is driving");
    let resize = second
        .call(
            "view_resize",
            tab,
            json!({"width": 400, "height": 800, "ratio": 2}),
        )
        .await;
    assert_eq!(error_of(&resize), "another viewer is driving");
    // Looking is never refused.
    assert!(
        second
            .call("view_frame", tab, json!({"since": 0}))
            .await
            .get("ok")
            .is_some()
    );

    assert!(
        second
            .call("take_control", tab, json!({}))
            .await
            .get("ok")
            .is_some()
    );
    assert!(
        second
            .call("view_input", tab, key.clone())
            .await
            .get("ok")
            .is_some()
    );
    assert_eq!(
        error_of(&first.call("view_input", tab, key).await),
        "another viewer is driving"
    );
    let refused_calls = rig
        .fake
        .tools()
        .iter()
        .filter(|(_, tool)| tool == "view_input")
        .count();
    assert_eq!(
        refused_calls, 2,
        "the refused calls never reached the broker"
    );
}

#[tokio::test]
async fn a_driver_that_disconnects_hands_the_tab_back() {
    let rig = rig().await;
    let (a, b) = (pair(&rig).await, pair(&rig).await);
    let tab = Some("user:t1:1");
    let key = json!({"type": "key", "kind": "keyDown", "key": "a"});
    {
        let mut first = connect(&rig, &a).await;
        assert!(
            first
                .call("view_input", tab, key.clone())
                .await
                .get("ok")
                .is_some()
        );
    }
    let mut second = connect(&rig, &b).await;
    for _ in 0..50 {
        if second
            .call("view_input", tab, key.clone())
            .await
            .get("ok")
            .is_some()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the tab stayed with a phone that left");
}

#[tokio::test]
async fn one_frame_is_in_flight_per_socket() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    let slow = ws
        .send(
            json!({"tool": "view_frame", "scope": "user:t1:1", "args": {"slow": true, "since": 1}}),
        )
        .await;
    // `view_frame` is checked against the list first, so wait for the slow call to be in.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = ws
        .send(json!({"tool": "view_frame", "scope": "user:t1:1", "args": {"since": 1}}))
        .await;
    let mut got = std::collections::HashMap::new();
    for _ in 0..2 {
        if let Message::Text(value) = ws.recv().await {
            got.insert(value["id"].as_u64().unwrap(), value);
        }
    }
    assert_eq!(got[&second]["ok"]["busy"], true, "{got:?}");
    assert!(got[&slow]["ok"].get("busy").is_none());
}

#[tokio::test]
async fn revoking_a_device_closes_its_socket_and_its_cookie_stops_working() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    assert!(ws.call("ping", None, json!({})).await.get("ok").is_some());
    let id = rig.remote.devices()[0].id.clone();
    assert!(rig.remote.revoke(&id));
    assert!(matches!(ws.recv().await, Message::Closed));
    assert!(rig.remote.devices().is_empty());
    assert!(!rig.remote.revoke(&id));
    let origin = format!("http://{}", rig.address);
    assert_eq!(
        upgrade(rig.address, Some(&cookie), Some(&origin))
            .await
            .err()
            .unwrap()
            .status,
        401
    );
    let page = http(rig.address, "GET", "/", &[("Cookie", &cookie)], "").await;
    assert!(page.body.contains("Pair this phone"));
}

#[tokio::test]
async fn stopping_closes_every_session() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let mut ws = connect(&rig, &cookie).await;
    rig.remote.stop();
    assert!(matches!(ws.recv().await, Message::Closed));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        TcpStream::connect(rig.address).await.is_err() || {
            // The listener may still accept for a moment; a request must get nothing useful.
            http(rig.address, "GET", "/", &[], "").await.status == 0
        }
    );
}

#[tokio::test]
async fn only_four_devices_are_connected_at_once() {
    let rig = rig().await;
    let mut open = Vec::new();
    for _ in 0..4 {
        let cookie = pair(&rig).await;
        open.push(connect(&rig, &cookie).await);
    }
    let fifth = pair(&rig).await;
    let origin = format!("http://{}", rig.address);
    let refused = upgrade(rig.address, Some(&fifth), Some(&origin))
        .await
        .err()
        .unwrap();
    assert_eq!(refused.status, 503);
    assert_eq!(rig.remote.connected().len(), 4);
}

#[tokio::test]
async fn the_token_is_never_kept_only_its_hash() {
    let rig = rig().await;
    let cookie = pair(&rig).await;
    let token = cookie.strip_prefix("orochi_remote=").unwrap();
    let path = rig.data.path().join("browser/remote.json");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(token), "the cookie value is in remote.json");
    let hash: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(text.contains(&hash));
    assert!(text.contains(&rig.address.to_string()));
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let page = http(rig.address, "GET", "/", &[], "").await;
    assert!(!page.body.contains(token) && !page.headers.contains(token));
}

#[tokio::test]
async fn a_restart_keeps_the_paired_devices_and_the_port() {
    let data = tempfile::tempdir().unwrap();
    let config = RemoteConfig {
        enabled: true,
        ..RemoteConfig::default()
    };
    let first = Remote::start(
        data.path(),
        &config,
        Arc::new(Fake::default()),
        Arc::new(|| {}),
    )
    .await
    .unwrap();
    let address = first.address();
    let code = first.pair().unwrap().code;
    let cookie = cookie_of(&redeem(address, &code).await);
    first.stop();
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = Remote::start(
        data.path(),
        &config,
        Arc::new(Fake::default()),
        Arc::new(|| {}),
    )
    .await
    .unwrap();
    assert_eq!(second.address(), address);
    assert_eq!(second.devices().len(), 1);
    let page = http(address, "GET", "/", &[("Cookie", &cookie)], "").await;
    assert!(page.body.contains("Orochi browser"));
}

#[tokio::test]
async fn a_bind_that_would_not_be_encrypted_is_refused_unless_said_so() {
    let lan = |insecure| RemoteConfig {
        enabled: true,
        bind: "192.168.1.20".into(),
        insecure_lan: insecure,
        ..RemoteConfig::default()
    };
    assert!(policy::remote_bind(&lan(false)).is_err());
    assert!(policy::remote_bind(&lan(true)).is_ok());
    for bind in ["127.0.0.1", "::1", "100.64.0.1", "100.127.255.254"] {
        let config = RemoteConfig {
            bind: bind.into(),
            ..RemoteConfig::default()
        };
        assert!(policy::remote_bind(&config).is_ok(), "{bind}");
    }
    for bind in ["100.128.0.1", "100.63.0.1", "0.0.0.0", "example.com", ""] {
        let config = RemoteConfig {
            bind: bind.into(),
            ..RemoteConfig::default()
        };
        assert!(policy::remote_bind(&config).is_err(), "{bind}");
    }
    let data = tempfile::tempdir().unwrap();
    let refused = Remote::start(
        data.path(),
        &lan(false),
        Arc::new(Fake::default()),
        Arc::new(|| {}),
    )
    .await;
    assert!(refused.is_err());
    assert!(
        !data.path().join("browser/remote.json").exists(),
        "nothing is written for a refused start"
    );
}

#[tokio::test]
async fn activity_is_reported_for_requests_and_messages() {
    let data = tempfile::tempdir().unwrap();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    let remote = Remote::start(
        data.path(),
        &RemoteConfig {
            enabled: true,
            ..RemoteConfig::default()
        },
        Arc::new(Fake::default()),
        Arc::new(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }),
    )
    .await
    .unwrap();
    let address = remote.address();
    http(address, "GET", "/pair", &[], "").await;
    let before = count.load(std::sync::atomic::Ordering::SeqCst);
    assert!(before >= 1);
    let code = remote.pair().unwrap().code;
    let cookie = cookie_of(&redeem(address, &code).await);
    let origin = format!("http://{address}");
    let mut ws = match upgrade(address, Some(&cookie), Some(&origin)).await {
        Ok(ws) => ws,
        Err(a) => panic!("{}", a.status),
    };
    let upgraded = count.load(std::sync::atomic::Ordering::SeqCst);
    ws.call("ping", None, json!({})).await;
    assert!(count.load(std::sync::atomic::Ordering::SeqCst) > upgraded);
}

/// The whole chain, with only the engine faked: a phone's socket → the listener → the broker's
/// hub → an engine process speaking the real line protocol → back as a binary frame.
#[tokio::test]
async fn a_phone_opens_a_page_on_servo_and_sees_and_drives_it_through_the_whole_chain() {
    use orochi::{
        browser::{broker::Request, engine::Engines, hub::Hub},
        config::{BrowserConfig, BrowserEngine},
    };
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("browser")).unwrap();
    let config = BrowserConfig {
        engine: BrowserEngine::Servo,
        servo: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_servo.py"),
        ),
        ..BrowserConfig::default()
    };
    // The hub reads the settings the window last saved, as the broker does.
    std::fs::write(
        data.path().join("browser/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let hub = Arc::new(Hub::new(
        None,
        Engines::new(&config, data.path()),
        &config,
        data.path(),
        Arc::new(|| {}),
    ));
    let admin = |tool: &str| Request {
        scope: "admin".into(),
        tool: tool.into(),
        args: json!({}),
        read_only: false,
        cwd: None,
    };
    let reply = hub.call(admin("remote_pair")).await;
    let pairing: Value =
        serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap();
    let url = pairing["url"].as_str().unwrap();
    let address: SocketAddr = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let answer = redeem(address, pairing["code"].as_str().unwrap()).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let cookie = cookie_of(&answer);
    let origin = format!("http://{address}");
    let Ok(mut ws) = upgrade(address, Some(&cookie), Some(&origin)).await else {
        panic!("not upgraded");
    };

    let device = ws.call("ping", None, json!({})).await["ok"]["device"]
        .as_str()
        .unwrap()
        .to_owned();
    let tab = format!("user:phone-{device}:one");
    let opened = ws
        .call(
            "view_open",
            Some(&tab),
            json!({"url": "http://localhost:3000/login"}),
        )
        .await;
    assert!(opened.get("error").is_none(), "{opened}");

    // The picture arrives as a binary message the engine's own bytes are in.
    let mut seen = None;
    for _ in 0..100 {
        let reply = ws.call("view_frame", Some(&tab), json!({"since": 0})).await;
        if reply["ok"]["frame"].is_object() {
            seen = Some(reply);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let reply = seen.expect("a frame");
    assert_eq!(reply["ok"]["engine"], "servo");
    assert_eq!(reply["ok"]["url"], "http://localhost:3000/login");
    assert_eq!(reply["ok"]["frame"]["width"], 1024);
    assert!(reply["jpeg_len"].as_u64().unwrap() > 8);

    // Typing on the phone reaches the page, and the page's answer comes back as a new frame.
    let typed = ws
        .call(
            "view_input",
            Some(&tab),
            json!({"type": "text", "text": "ログイン"}),
        )
        .await;
    assert!(typed.get("error").is_none(), "{typed}");
    let mut title = String::new();
    for _ in 0..100 {
        let reply = ws.call("view_frame", Some(&tab), json!({"since": 0})).await;
        title = reply["ok"]["title"].as_str().unwrap_or("").to_owned();
        if title.starts_with("typed") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(title, "typed: ログイン");

    // The phone's tab is in the person's list, with its engine.
    let listed = ws.call("view_list", None, json!({})).await;
    assert_eq!(listed["ok"]["tabs"][0]["scope"], json!(tab));
    assert_eq!(listed["ok"]["tabs"][0]["engine"], "servo");
    // It may not take an address the person's own tab may not.
    let refused = ws
        .call(
            "view_navigate",
            Some(&tab),
            json!({"url": "file:///etc/passwd"}),
        )
        .await;
    assert!(error_of(&refused).contains("not opened"), "{refused}");
    hub.stop_remote();
}
