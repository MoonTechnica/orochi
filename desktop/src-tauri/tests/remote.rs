//! The phone's door (`docs/mobile-remote-design.md`), driven over a real socket on loopback.
//! `OROCHI_REMOTE_BIND` stands in for the Tailscale address, which is the only thing the tests
//! cannot have.
use futures_util::StreamExt;
use orochi::media::{self, MediaRef, Source};
use orochi_desktop::{remote::Remote, view::Client};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0123456789abcdefghij";

struct Door {
    _dir: tempfile::TempDir,
    data: PathBuf,
    repo: PathBuf,
    thread: String,
    remote: Remote,
    addr: SocketAddr,
}

async fn door() -> Door {
    // Every test binds the same way, so setting it once for the process is safe.
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { std::env::set_var("OROCHI_REMOTE_BIND", "127.0.0.1:0") });
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("note.txt"), "hello from the tree\n").unwrap();
    std::fs::write(repo.join("clip.mp4"), PNG).unwrap();
    std::fs::write(dir.path().join("secret.txt"), "outside").unwrap();
    let client = Client::open(&data).unwrap();
    let thread = client.new_thread(&repo, false, None).unwrap();
    let remote = Remote::new(&data, Arc::new(Mutex::new(client)), "orochi".into());
    remote.enable().await.unwrap();
    let addr = remote.address().unwrap();
    Door {
        _dir: dir,
        data,
        repo,
        thread,
        remote,
        addr,
    }
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: Option<&Value>,
) -> Answer {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(token) = token {
        head += &format!("Authorization: Bearer {token}\r\n");
    }
    for (name, value) in extra {
        head += &format!("{name}: {value}\r\n");
    }
    head += &format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let text = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = text.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    Answer {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

async fn get(door: &Door, path: &str, token: Option<&str>) -> Answer {
    request(door.addr, "GET", path, token, &[], None).await
}

async fn get_at(addr: SocketAddr, path: &str) -> Answer {
    request(addr, "GET", path, None, &[], None).await
}

async fn rpc(door: &Door, token: &str, command: &str, body: Value) -> Answer {
    request(
        door.addr,
        "POST",
        &format!("/rpc/{command}"),
        Some(token),
        &[],
        Some(&body),
    )
    .await
}

async fn pair_with(door: &Door, code: &str, device: &str) -> Answer {
    request(
        door.addr,
        "POST",
        "/pair",
        None,
        &[],
        Some(&json!({"code": code, "device": device})),
    )
    .await
}

async fn paired(door: &Door) -> String {
    let code = door.remote.pair().unwrap().code;
    let answer = pair_with(door, &code, "test phone").await;
    assert_eq!(answer.status, 200);
    answer.json()["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn health_needs_no_token_and_names_the_machine() {
    let door = door().await;
    let answer = get(&door, "/health", None).await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json()["ok"], true);
    assert!(answer.json()["name"].is_string());
}

#[tokio::test]
async fn a_code_pairs_once_and_the_token_it_gives_is_who_you_are() {
    let door = door().await;
    let pairing = door.remote.pair().unwrap();
    assert!(pairing.svg.contains("<svg"));
    assert!(
        pairing
            .url
            .starts_with("orochi://pair?host=127.0.0.1&port=")
    );
    assert!(pairing.url.contains(&format!("code={}", pairing.code)));
    let first = pair_with(&door, &pairing.code, "Pixel").await;
    assert_eq!(first.status, 200);
    let token = first.json()["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    let me = get(&door, "/me", Some(&token)).await;
    assert_eq!(me.json()["device"], "Pixel");
    // Single use.
    assert_eq!(pair_with(&door, &pairing.code, "again").await.status, 401);
    assert_eq!(door.remote.devices().len(), 1);
}

#[tokio::test]
async fn a_wrong_code_fails_and_five_of_them_burn_the_real_one() {
    let door = door().await;
    let code = door.remote.pair().unwrap().code;
    for _ in 0..5 {
        assert_eq!(pair_with(&door, "00000000x", "p").await.status, 401);
    }
    assert_eq!(pair_with(&door, &code, "p").await.status, 401);
    assert!(door.remote.devices().is_empty());
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let door = door().await;
    let code = door.remote.pair().unwrap().code;
    door.remote.expire_pairing();
    assert_eq!(pair_with(&door, &code, "p").await.status, 401);
}

#[tokio::test]
async fn only_the_hash_of_a_token_is_kept_and_the_file_is_private() {
    let door = door().await;
    let token = paired(&door).await;
    let path = door.data.join("remote/devices.json");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&token));
    let hash: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(text.contains(&hash));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn nothing_but_health_and_pair_answers_without_a_token() {
    let door = door().await;
    let _ = paired(&door).await;
    for path in ["/me", "/events", "/file?thread=x&path=y", "/media/t/abc"] {
        let answer = get(&door, path, None).await;
        assert!(
            answer.status == 401 || answer.status == 400,
            "{path} {}",
            answer.status
        );
        let wrong = get(&door, path, Some("nope")).await.status;
        assert!(wrong == 401 || wrong == 400, "{path} {wrong}");
    }
    assert_eq!(get(&door, "/me", None).await.status, 401);
    assert_eq!(get(&door, "/me", Some("wrong")).await.status, 401);
    let rpc_unauthenticated = request(
        door.addr,
        "POST",
        "/rpc/sidebar",
        None,
        &[],
        Some(&json!({})),
    )
    .await;
    assert_eq!(rpc_unauthenticated.status, 401);
    assert_eq!(
        get(&door, "/file?thread=x&path=y", Some("wrong"))
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn a_command_off_the_whitelist_is_refused_even_with_a_token() {
    let door = door().await;
    let token = paired(&door).await;
    for command in [
        "term_open",
        "settings",
        "save_settings",
        "sandboxes",
        "setup_status",
        "decision_key",
        "browser_call",
        "open_folder",
        "open_path",
        "places",
        "set_place",
        "set_roster",
        "save_memory",
        "delete_project",
        "set_clone_dir",
        "remote_set_clone_dir",
        "delete_thread",
        "notify",
        "badge",
        "remote_enable",
        "remote_pair",
        "remote_devices",
        "nothing_at_all",
    ] {
        let answer = rpc(&door, &token, command, json!({})).await;
        assert_eq!(answer.status, 404, "{command}");
    }
}

#[tokio::test]
async fn a_revoked_token_stops_working() {
    let door = door().await;
    let token = paired(&door).await;
    assert_eq!(get(&door, "/me", Some(&token)).await.status, 200);
    let id = door.remote.devices()[0].id.clone();
    assert!(door.remote.revoke(&id).unwrap());
    assert_eq!(get(&door, "/me", Some(&token)).await.status, 401);
    assert!(!door.remote.revoke(&id).unwrap());
    let text = std::fs::read_to_string(door.data.join("remote/devices.json")).unwrap();
    assert!(!text.contains(&id));
}

#[tokio::test]
async fn devices_outlive_the_server_and_the_server_can_be_turned_off() {
    let door = door().await;
    let token = paired(&door).await;
    let status = door.remote.disable().await.unwrap();
    assert!(!status.running && !status.enabled);
    // Whether the port still answers is not asked: another test's server may have taken it.
    assert!(door.remote.pair().is_err());
    door.remote.enable().await.unwrap();
    let addr = door.remote.address().unwrap();
    let me = request(addr, "GET", "/me", Some(&token), &[], None).await;
    assert_eq!(me.status, 200);
}

#[tokio::test]
async fn rpc_answers_with_the_views_the_window_reads_and_writes_what_it_writes() {
    let door = door().await;
    let token = paired(&door).await;
    let sidebar = rpc(
        &door,
        &token,
        "sidebar",
        json!({"limit": 50, "archived": false}),
    )
    .await;
    assert_eq!(sidebar.status, 200);
    let rows = sidebar.json();
    assert_eq!(rows[0]["threads"][0]["id"], door.thread);
    let sent = rpc(
        &door,
        &token,
        "send",
        json!({"thread": door.thread, "text": "hello"}),
    )
    .await;
    assert_eq!(sent.status, 200);
    let thread = rpc(&door, &token, "thread", json!({"id": door.thread})).await;
    assert_eq!(thread.status, 200);
    assert!(thread.json().is_object());
    let missing = rpc(&door, &token, "room", json!({})).await;
    assert_eq!(missing.status, 400);
    // A phone starts a thread only in a project the desktop already has.
    let elsewhere = rpc(&door, &token, "new_thread", json!({"root": "/tmp"})).await;
    assert_eq!(elsewhere.status, 400);
    let root = door.repo.to_string_lossy().into_owned();
    let started = rpc(&door, &token, "new_thread", json!({"root": root})).await;
    assert_eq!(started.status, 200);
}

#[tokio::test]
async fn a_change_in_the_store_reaches_a_connected_phone() {
    let door = door().await;
    let token = paired(&door).await;
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(format!(
            "ws://{}/events",
            door.addr
        ))
        .unwrap();
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let hello = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(hello.to_text().unwrap()).unwrap()["type"],
        "hello"
    );
    // The poller reads from its own cursor, so give it a moment to have taken its first look.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let sent = rpc(
        &door,
        &token,
        "send",
        json!({"thread": door.thread, "text": "wake up"}),
    )
    .await;
    assert_eq!(sent.status, 200);
    let seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if event["type"] == "changed" {
                return event;
            }
        }
    })
    .await
    .expect("a changed event");
    assert!(
        seen["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| *t == door.thread.as_str())
    );
}

#[tokio::test]
async fn a_socket_without_a_token_is_not_upgraded() {
    let door = door().await;
    let refused = tokio_tungstenite::connect_async(format!("ws://{}/events", door.addr)).await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn media_is_served_with_ranges_and_takes_the_token_in_the_query() {
    let door = door().await;
    let token = paired(&door).await;
    let sha = media::keep(
        &door.data,
        "t1",
        &MediaRef {
            name: "a.png".into(),
            source: Source::Inline(PNG.to_vec()),
        },
    )
    .unwrap()
    .unwrap()
    .sha;
    let path = format!("/media/t1/{sha}");
    assert_eq!(get(&door, &path, None).await.status, 401);
    let whole = get(&door, &format!("{path}?token={token}"), None).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, PNG);
    assert_eq!(whole.header("content-type"), Some("image/png"));
    let part = request(
        door.addr,
        "GET",
        &path,
        Some(&token),
        &[("Range", "bytes=8-11")],
        None,
    )
    .await;
    assert_eq!(part.status, 206);
    assert_eq!(part.body, b"0123");
    assert_eq!(
        part.header("content-range"),
        Some(format!("bytes 8-11/{}", PNG.len()).as_str())
    );
    assert_eq!(
        get(&door, "/media/t1/not-a-hash", Some(&token))
            .await
            .status,
        404
    );
}

#[tokio::test]
async fn a_file_of_the_thread_folder_is_served_and_nothing_outside_it() {
    let door = door().await;
    let token = paired(&door).await;
    let thread = &door.thread;
    let note = get(
        &door,
        &format!("/file?thread={thread}&path=note.txt&token={token}"),
        None,
    )
    .await;
    assert_eq!(note.status, 200);
    assert_eq!(note.body, b"hello from the tree\n");
    assert!(
        note.header("content-type")
            .unwrap()
            .starts_with("text/plain")
    );
    let clip = request(
        door.addr,
        "GET",
        &format!("/file?thread={thread}&path=clip.mp4"),
        Some(&token),
        &[("Range", "bytes=-4")],
        None,
    )
    .await;
    assert_eq!(clip.status, 206);
    assert_eq!(clip.body, &PNG[PNG.len() - 4..]);
    assert_eq!(clip.header("content-type"), Some("video/mp4"));
    for path in [
        "../secret.txt",
        "%2E%2E%2Fsecret.txt",
        "sub/../../secret.txt",
        "%2Fetc%2Fpasswd",
        ".git/config",
    ] {
        let answer = get(
            &door,
            &format!("/file?thread={thread}&path={path}"),
            Some(&token),
        )
        .await;
        assert!(
            answer.status == 403 || answer.status == 404,
            "{path} {}",
            answer.status
        );
        assert_ne!(answer.body, b"outside");
    }
    assert_eq!(
        get(&door, &format!("/file?thread={thread}"), Some(&token))
            .await
            .status,
        400
    );
}

/// The binary is the phone's way in with no window: seeded, on loopback, paired by the code it
/// prints, and answering the same routes.
struct Headless {
    child: std::process::Child,
    lines: std::io::BufReader<std::process::ChildStdout>,
    stdin: std::process::ChildStdin,
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn next_pairing(headless: &mut Headless) -> Value {
    use std::io::BufRead;
    let mut line = String::new();
    headless.lines.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|_| panic!("a JSON line, got {line:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_headless_binary_serves_demo_data_on_loopback_to_a_phone_that_pairs_with_its_code() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_orochi-remote"))
        .args(["--demo-data"])
        .arg(dir.path().join("demo"))
        .args(["--bind", "127.0.0.1:0", "--json"])
        .env_remove("OROCHI_DATA_DIR")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut headless = Headless {
        lines: std::io::BufReader::new(child.stdout.take().unwrap()),
        stdin: child.stdin.take().unwrap(),
        child,
    };
    let (first, mut headless) = tokio::task::spawn_blocking(move || {
        let first = next_pairing(&mut headless);
        (first, headless)
    })
    .await
    .unwrap();
    let addr: SocketAddr = first["base"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .parse()
        .unwrap();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert_eq!(first["port"], addr.port());
    let link = first["url"].as_str().unwrap();
    assert!(
        link.starts_with("orochi://pair?host=127.0.0.1&port="),
        "{link}"
    );
    assert!(link.contains(&format!("code={}", first["code"].as_str().unwrap())));

    let paired = request(
        addr,
        "POST",
        "/pair",
        None,
        &[],
        Some(&json!({"code": first["code"], "device": "simulator"})),
    )
    .await;
    assert_eq!(paired.status, 200);
    let token = paired.json()["token"].as_str().unwrap().to_owned();
    let call = |command: &'static str, body: Value| {
        let token = token.clone();
        async move {
            let answer = request(
                addr,
                "POST",
                &format!("/rpc/{command}"),
                Some(&token),
                &[],
                Some(&body),
            )
            .await;
            assert_eq!(answer.status, 200, "{command}");
            answer.json()
        }
    };

    let sidebar = call("sidebar", json!({"limit": 50, "archived": false})).await;
    let rows = sidebar[0]["threads"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    let titled = |title: &str| {
        rows.iter()
            .find(|row| row["title"] == title)
            .unwrap_or_else(|| panic!("no thread {title}"))["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(rows.iter().any(|row| row["status"] == "needs_you"));
    assert_eq!(
        call("open_prompts", json!({}))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !call("live_seats", json!({}))
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );

    let team = titled("Split the work");
    let members = call("members", json!({"thread": team})).await;
    assert_eq!(members.as_array().unwrap().len(), 2);
    let room = call("room", json!({"thread": team})).await;
    assert!(
        room.as_array()
            .unwrap()
            .iter()
            .any(|m| m["via"] == "handoff" && m["member"] == "alice")
    );

    // A picture, a video and a recording of the first thread, by hash, with a Range.
    let thread = titled("Show me what you made");
    let shown = call("thread", json!({"id": thread})).await;
    let kept: Vec<(String, String)> = shown["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "media")
        .map(|item| {
            (
                item["data"]["sha"].as_str().unwrap().to_owned(),
                item["data"]["mime"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mimes: Vec<&str> = kept.iter().map(|(_, m)| m.as_str()).collect();
    assert_eq!(mimes, ["image/png", "image/jpeg", "video/mp4", "audio/mp4"]);
    for (sha, mime) in &kept {
        let path = format!("/media/{thread}/{sha}");
        let part = request(
            addr,
            "GET",
            &path,
            Some(&token),
            &[("Range", "bytes=0-7")],
            None,
        )
        .await;
        assert_eq!(part.status, 206, "{mime}");
        assert_eq!(part.body.len(), 8);
        assert_eq!(part.header("content-type"), Some(mime.as_str()));
    }

    // A relative link of the markdown thread, out of the demo folder.
    let markdown = titled("Write up with files");
    let clip = request(
        addr,
        "GET",
        &format!("/file?thread={markdown}&path=assets/clip.mp4"),
        Some(&token),
        &[("Range", "bytes=4-7")],
        None,
    )
    .await;
    assert_eq!(clip.status, 206);
    assert_eq!(clip.body, b"ftyp");
    let notes = get_at(
        addr,
        &format!("/file?thread={markdown}&path=notes.txt&token={token}"),
    )
    .await;
    assert_eq!(notes.status, 200);

    // `pair` on stdin issues another code, and the first stays used.
    writeln!(headless.stdin, "pair").unwrap();
    headless.stdin.flush().unwrap();
    let (second, _headless) = tokio::task::spawn_blocking(move || {
        let second = next_pairing(&mut headless);
        (second, headless)
    })
    .await
    .unwrap();
    assert_ne!(second["code"], first["code"]);
    let again = request(
        addr,
        "POST",
        "/pair",
        None,
        &[],
        Some(&json!({"code": second["code"], "device": "second phone"})),
    )
    .await;
    assert_eq!(again.status, 200);
    let reuse = request(
        addr,
        "POST",
        "/pair",
        None,
        &[],
        Some(&json!({"code": first["code"], "device": "late"})),
    )
    .await;
    assert_eq!(reuse.status, 401);
}

#[tokio::test]
async fn a_machine_has_one_stable_private_id_that_health_pair_me_and_the_link_all_carry() {
    let door = door().await;
    let health = get(&door, "/health", None).await.json();
    let id = health["id"].as_str().unwrap().to_owned();
    assert!(id.len() >= 16 && id.chars().all(|c| c.is_ascii_alphanumeric()));
    let pairing = door.remote.pair().unwrap();
    assert_eq!(pairing.id, id);
    assert!(
        pairing.url.ends_with(&format!("&id={id}")),
        "{}",
        pairing.url
    );
    let paired = pair_with(&door, &pairing.code, "phone").await.json();
    assert_eq!(paired["id"], id);
    let token = paired["token"].as_str().unwrap();
    assert_eq!(get(&door, "/me", Some(token)).await.json()["id"], id);
    assert_eq!(door.remote.status().id, id);
    // Kept in a file of its own: a second Remote on the same folder is the same machine.
    let path = door.data.join("remote/machine.json");
    assert!(std::fs::read_to_string(&path).unwrap().contains(&id));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let again = Remote::new(
        &door.data,
        Arc::new(Mutex::new(Client::open(&door.data).unwrap())),
        "orochi".into(),
    );
    assert_eq!(again.status().id, id);
    // Another data folder is another machine.
    let other = door_with_new_data().await;
    assert_ne!(get(&other, "/health", None).await.json()["id"], id);
}

async fn pair_as(door: &Door, device: &str, device_id: Option<&str>) -> String {
    let code = door.remote.pair().unwrap().code;
    let body = match device_id {
        Some(id) => json!({"code": code, "device": device, "device_id": id}),
        None => json!({"code": code, "device": device}),
    };
    let answer = request(door.addr, "POST", "/pair", None, &[], Some(&body)).await;
    assert_eq!(answer.status, 200);
    answer.json()["token"].as_str().unwrap().to_owned()
}

/// A file write the server must notice: a moment past the last one, so the stamp differs.
fn edit_devices(door: &Door, text: &str) {
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(door.data.join("remote/devices.json"), text).unwrap();
}

#[tokio::test]
async fn the_device_file_is_the_truth_edits_to_it_take_effect_while_the_server_runs() {
    let door = door().await;
    let keep = pair_as(&door, "keep", None).await;
    let gone = pair_as(&door, "gone", None).await;
    assert_eq!(door.remote.devices().len(), 2);
    // A row removed by hand revokes that phone and no other.
    let path = door.data.join("remote/devices.json");
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["devices"]
        .as_array_mut()
        .unwrap()
        .retain(|d| d["name"] != "gone");
    edit_devices(&door, &file.to_string());
    assert_eq!(get(&door, "/me", Some(&gone)).await.status, 401);
    assert_eq!(get(&door, "/me", Some(&keep)).await.status, 200);
    assert_eq!(door.remote.devices().len(), 1);
    assert_eq!(door.remote.status().devices, 1);
    // A file that does not parse changes nothing; an emptied one revokes everyone.
    edit_devices(&door, "{ not json");
    assert_eq!(get(&door, "/me", Some(&keep)).await.status, 200);
    edit_devices(&door, r#"{"devices":[]}"#);
    assert_eq!(get(&door, "/me", Some(&keep)).await.status, 401);
    // And a deleted file is an empty list, not a crash.
    let again = pair_as(&door, "again", None).await;
    assert_eq!(get(&door, "/me", Some(&again)).await.status, 200);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(get(&door, "/me", Some(&again)).await.status, 401);
    assert!(door.remote.devices().is_empty());
    // The server's own writes do not make it forget what it just wrote.
    let last = pair_as(&door, "last", None).await;
    assert_eq!(get(&door, "/me", Some(&last)).await.status, 200);
}

#[tokio::test]
async fn pairing_again_from_the_same_install_replaces_its_row_and_nobody_elses() {
    let door = door().await;
    let first = pair_as(&door, "iPhone", Some("install-aaaaaaaa")).await;
    let other = pair_as(&door, "iPhone", Some("install-bbbbbbbb")).await;
    assert_eq!(door.remote.devices().len(), 2, "same name, two installs");
    let second = pair_as(&door, "iPhone", Some("install-aaaaaaaa")).await;
    let rows = door.remote.devices();
    assert_eq!(rows.len(), 2, "the old row of install a was replaced");
    assert_eq!(get(&door, "/me", Some(&first)).await.status, 401);
    assert_eq!(get(&door, "/me", Some(&second)).await.status, 200);
    assert_eq!(get(&door, "/me", Some(&other)).await.status, 200);
    // The id is kept in the file, the token is not.
    let text = std::fs::read_to_string(door.data.join("remote/devices.json")).unwrap();
    assert!(text.contains("install-aaaaaaaa") && !text.contains(&second));
    // Without an id (an older app, a browser) nothing is guessed from the name.
    pair_as(&door, "iPhone", None).await;
    pair_as(&door, "iPhone", None).await;
    assert_eq!(door.remote.devices().len(), 4);
    // An id that is not an id is dropped rather than matching anything.
    pair_as(&door, "x", Some("short")).await;
    pair_as(&door, "x", Some("short")).await;
    assert_eq!(door.remote.devices().len(), 6);
}

#[tokio::test]
async fn the_name_a_phone_sees_is_chosen_in_settings_and_defaults_to_the_host_without_its_domain() {
    let door = door().await;
    let default = door.remote.status().default_name;
    assert!(!default.is_empty() && !default.contains('.'), "{default}");
    assert_eq!(door.remote.name(), default);
    assert_eq!(get(&door, "/health", None).await.json()["name"], default);
    let id = door.remote.status().id;

    let status = door.remote.set_name("  Work   Mac ").unwrap();
    assert_eq!(status.name, "Work Mac");
    assert_eq!(get(&door, "/health", None).await.json()["name"], "Work Mac");
    let pairing = door.remote.pair().unwrap();
    assert_eq!(pairing.name, "Work Mac");
    assert!(pairing.url.contains("name=Work%20Mac"), "{}", pairing.url);
    let paired = pair_with(&door, &pairing.code, "phone").await.json();
    assert_eq!(paired["name"], "Work Mac");
    let token = paired["token"].as_str().unwrap();
    assert_eq!(
        get(&door, "/me", Some(token)).await.json()["name"],
        "Work Mac"
    );

    // Kept beside the id, which does not change; a new Remote on the folder reads it.
    let kept = std::fs::read_to_string(door.data.join("remote/machine.json")).unwrap();
    assert!(kept.contains("Work Mac") && kept.contains(&id));
    let again = Remote::new(
        &door.data,
        Arc::new(Mutex::new(Client::open(&door.data).unwrap())),
        "orochi".into(),
    );
    assert_eq!(
        (again.name(), again.status().id),
        ("Work Mac".into(), id.clone())
    );

    // Empty goes back to the default; nonsense is refused and changes nothing.
    assert_eq!(door.remote.set_name("   ").unwrap().name, default);
    assert!(door.remote.set_name(&"x".repeat(41)).is_err());
    assert!(door.remote.set_name("bad\u{7}name").is_err());
    assert_eq!(door.remote.name(), default);
    assert_eq!(door.remote.status().id, id);
}

async fn door_with_new_data() -> Door {
    door().await
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.test", "-C"])
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[tokio::test]
async fn a_phone_browses_and_adds_folders_only_inside_the_clone_folder_or_home() {
    let door = door().await;
    let token = paired(&door).await;
    let clones = door._dir.path().join("clones");
    std::fs::create_dir_all(clones.join("mine/.git")).unwrap();
    std::fs::create_dir_all(clones.join("plain")).unwrap();
    std::fs::create_dir_all(clones.join(".hidden")).unwrap();
    std::fs::write(clones.join("file.txt"), "x").unwrap();

    // Until a clone folder is chosen the tempdir is outside everything a phone may open.
    let outside = clones.to_string_lossy().into_owned();
    let refused = rpc(&door, &token, "browse", json!({"path": outside})).await;
    assert_eq!(refused.status, 400);

    let status = door.remote.set_clone_dir(&outside).unwrap();
    assert_eq!(
        status.clone_dir.as_deref(),
        Some(clones.canonicalize().unwrap().to_str().unwrap())
    );
    let listing = rpc(&door, &token, "browse", json!({})).await;
    assert_eq!(listing.status, 200);
    let listing = listing.json();
    let names: Vec<&str> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["mine", "plain"]);
    assert_eq!(listing["entries"][0]["git"], true);
    assert_eq!(listing["entries"][1]["git"], false);

    let escape = rpc(&door, &token, "browse", json!({"path": "/"})).await;
    assert_eq!(escape.status, 400);
    let up = format!("{outside}/../..");
    assert_eq!(
        rpc(&door, &token, "add_project", json!({"root": up}))
            .await
            .status,
        400
    );

    let mine = clones.join("mine").to_string_lossy().into_owned();
    let added = rpc(&door, &token, "add_project", json!({"root": mine})).await;
    assert_eq!(
        added.status,
        200,
        "{}",
        String::from_utf8_lossy(&added.body)
    );
    let started = rpc(&door, &token, "new_thread", json!({"root": mine})).await;
    assert_eq!(started.status, 200);

    // Choosing nothing turns it off again.
    assert!(door.remote.set_clone_dir("").unwrap().clone_dir.is_none());
    assert_eq!(
        rpc(&door, &token, "browse", json!({"path": outside}))
            .await
            .status,
        400
    );
    assert!(door.remote.set_clone_dir("relative/path").is_err());
}

#[tokio::test]
async fn a_phone_clones_a_repository_into_the_chosen_folder_and_nothing_else() {
    let door = door().await;
    let token = paired(&door).await;
    let clones = door._dir.path().join("clones");
    std::fs::create_dir_all(&clones).unwrap();

    // Standing in for a hosting site: an address under example.test is a local repository.
    let origin = door._dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "--quiet"]);
    git(
        &origin,
        &["commit", "--quiet", "--allow-empty", "-m", "first"],
    );
    unsafe {
        std::env::set_var("GIT_CONFIG_COUNT", "1");
        std::env::set_var(
            "GIT_CONFIG_KEY_0",
            format!("url.{}/.insteadOf", origin.display()),
        );
        std::env::set_var("GIT_CONFIG_VALUE_0", "https://example.test/origin");
    }

    let url = "https://example.test/origin";
    let none = rpc(&door, &token, "clone_project", json!({"url": url})).await;
    assert_eq!(none.status, 400, "no clone folder chosen yet");
    door.remote.set_clone_dir(clones.to_str().unwrap()).unwrap();

    for bad in [
        "",
        "--upload-pack=touch /tmp/x",
        "-oProxyCommand=x",
        "/etc/passwd",
        "file:///etc",
        "ext::sh -c id",
        "ssh://-oProxyCommand=x/repo",
        "https://host with space/x",
        "git@host",
        "https://example.test",
    ] {
        let answer = rpc(&door, &token, "clone_project", json!({"url": bad})).await;
        assert_eq!(answer.status, 400, "{bad:?}");
    }
    let wrong_name = rpc(
        &door,
        &token,
        "clone_project",
        json!({"url": url, "name": "../escape"}),
    )
    .await;
    assert_eq!(wrong_name.status, 400);
    assert!(!door._dir.path().join("escape").exists());

    let made = rpc(&door, &token, "clone_project", json!({"url": url})).await;
    assert_eq!(made.status, 200, "{}", String::from_utf8_lossy(&made.body));
    let made = made.json();
    assert_eq!(made["name"], "origin");
    assert!(clones.join("origin/.git").exists());
    let root = made["root"].as_str().unwrap().to_owned();
    let started = rpc(&door, &token, "new_thread", json!({"root": root})).await;
    assert_eq!(started.status, 200);

    let again = rpc(&door, &token, "clone_project", json!({"url": url})).await;
    assert_eq!(again.status, 400, "the folder is taken");
    let failed = rpc(
        &door,
        &token,
        "clone_project",
        json!({"url": "https://example.test/missing", "name": "gone"}),
    )
    .await;
    assert_eq!(failed.status, 400);
    assert!(
        !clones.join("gone").exists(),
        "a failed clone leaves nothing"
    );
}

const PUSH_A: &str = "3f9a1c0de2b74a65b3c1d0e9f8a7b6c5";
const PUSH_B: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn phones(door: &Door) -> Vec<String> {
    orochi::notify::phones(&door.data)
        .into_iter()
        .map(|p| p.id)
        .collect()
}

/// What a notice is addressed to is a random id the phone makes, and it goes where the pairing
/// goes: registered when the phone pairs or says so later, replaced when it changes, and gone
/// when the phone is revoked — a revoked phone is pushed to no more.
#[tokio::test]
async fn a_phones_push_id_is_registered_with_its_pairing_and_goes_when_the_pairing_does() {
    let door = door().await;
    let code = door.remote.pair().unwrap().code;
    let body = json!({"code": code, "device": "iPhone", "device_id": "install-aaaaaaaa", "push_id": PUSH_A});
    let answer = request(door.addr, "POST", "/pair", None, &[], Some(&body)).await;
    assert_eq!(answer.status, 200);
    let token = answer.json()["token"].as_str().unwrap().to_owned();
    assert_eq!(phones(&door), vec![PUSH_A], "registered when it paired");

    // Learned later (notifications are allowed after pairing), and changed.
    let told = request(
        door.addr,
        "POST",
        "/push",
        Some(&token),
        &[],
        Some(&json!({"push_id": PUSH_B})),
    )
    .await;
    assert_eq!(told.status, 200);
    assert_eq!(
        phones(&door),
        vec![PUSH_B],
        "the old one is replaced, not kept beside it"
    );

    // Nothing that is not a push id is taken, and nobody unauthenticated may set one.
    for bad in ["short", "me@example.com-aaaaaaaa", ""] {
        let refused = request(
            door.addr,
            "POST",
            "/push",
            Some(&token),
            &[],
            Some(&json!({"push_id": bad})),
        )
        .await;
        assert_eq!(refused.status, 400, "{bad}");
    }
    let anonymous = request(
        door.addr,
        "POST",
        "/push",
        None,
        &[],
        Some(&json!({"push_id": PUSH_A})),
    )
    .await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(phones(&door), vec![PUSH_B]);

    // Pairing again from the same install replaces the row and its push id.
    let again = door.remote.pair().unwrap().code;
    let body = json!({"code": again, "device": "iPhone", "device_id": "install-aaaaaaaa", "push_id": PUSH_A});
    assert_eq!(
        request(door.addr, "POST", "/pair", None, &[], Some(&body))
            .await
            .status,
        200
    );
    assert_eq!(phones(&door), vec![PUSH_A]);

    let id = door.remote.devices().first().unwrap().id.clone();
    assert!(door.remote.revoke(&id).unwrap());
    assert!(
        phones(&door).is_empty(),
        "a revoked phone is pushed to no more"
    );
}

/// A phone may look at routines, residents and notices, run, pause or resume a routine, allow
/// or decline what an agent proposed, and make a resident to talk to; it may not make or delete
/// a routine by hand, or change the settings.
#[tokio::test]
async fn a_phone_can_read_routines_answer_proposals_and_make_or_pause_a_resident_but_not_delete_either()
 {
    let door = door().await;
    let token = paired(&door).await;
    for command in ["residents", "routines", "notices", "keeper"] {
        let answer = rpc(&door, &token, command, json!({})).await;
        assert_eq!(
            answer.status,
            200,
            "{command}: {}",
            String::from_utf8_lossy(&answer.body)
        );
    }
    assert!(rpc(&door, &token, "keeper", json!({})).await.json()["enabled"].is_boolean());
    for command in [
        "routine_add",
        "routine_try",
        "save_section",
        "notify_key",
        "keeper_login",
        "notify_test",
        "save_resident",
    ] {
        assert_eq!(
            rpc(&door, &token, command, json!({})).await.status,
            404,
            "{command} stays at the desktop"
        );
    }
    let refused = rpc(
        &door,
        &token,
        "routine_action",
        json!({"id": "x", "action": "delete"}),
    )
    .await;
    assert_eq!(refused.status, 400, "a phone cannot delete a routine");
    let made = rpc(
        &door,
        &token,
        "resident_add",
        json!({"name": "assistant", "instruction": "Keep me up with my mail."}),
    )
    .await;
    assert_eq!(made.status, 200, "{}", String::from_utf8_lossy(&made.body));
    let listed = rpc(&door, &token, "residents", json!({})).await.json();
    assert_eq!(listed[0]["name"], "assistant");
    // The one switch for everything a resident has scheduled reaches the phone; deleting does not.
    let paused = rpc(
        &door,
        &token,
        "resident_action",
        json!({"name": "assistant", "action": "pause"}),
    )
    .await;
    assert_eq!(
        paused.status,
        200,
        "{}",
        String::from_utf8_lossy(&paused.body)
    );
    assert_eq!(
        rpc(&door, &token, "residents", json!({})).await.json()[0]["paused"],
        true
    );
    let refused = rpc(
        &door,
        &token,
        "resident_action",
        json!({"name": "assistant", "action": "delete"}),
    )
    .await;
    assert_eq!(refused.status, 400, "a phone cannot delete a resident");
    let missing = rpc(
        &door,
        &token,
        "routine_action",
        json!({"id": "nope", "action": "pause"}),
    )
    .await;
    assert_ne!(
        missing.status, 200,
        "and an unknown routine is an error, not silence"
    );
}
