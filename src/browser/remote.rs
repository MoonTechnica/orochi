//! The person's browser tabs on a phone (`docs/servo-browser-design.md` §6).
//!
//! A listener **inside the broker** that serves a web page and a WebSocket to a paired phone's
//! own browser. It is the window's viewer over a network: it sends the same `view_*` tools the
//! window sends, and nothing else. It is **off** unless `[browser.remote] enabled`, and is
//! reached through a Tailscale address, a tunnel to loopback, or (with `insecure_lan`) a LAN
//! address in the clear; Orochi never opens a port in a firewall, runs `tailscale`, or relays.
//!
//! # HTTP
//!
//! Hand-written HTTP/1.1, one request per connection (`Connection: close`).
//!
//! * `GET /` — the viewer (`viewer.html`) with a valid session cookie, the pairing page without.
//! * `GET /pair` — the pairing page. The pairing URL carries the code in the fragment
//!   (`/pair#<code>`), which a browser never sends to a server, so it stays out of logs.
//! * `POST /pair` `{"code": "...", "name": "..."}` — redeems a code for a device and a cookie
//!   (`orochi_remote`, `HttpOnly; SameSite=Strict; Path=/`, `Secure` only behind a loopback
//!   proxy that said `X-Forwarded-Proto: https`, i.e. `tailscale serve`).
//! * `GET /ws` — the WebSocket upgrade; needs the cookie **and** an `Origin` equal to the
//!   request's own origin.
//!
//! The `Host` header must name the listener (its address, or `localhost` on loopback), which is
//! what stops a rebinding page from talking to it; the one exception is a request that comes
//! from loopback with `X-Forwarded-Proto: https`, a local TLS proxy in front of the listener.
//! Even then nothing works without the cookie, which is the credential.
//!
//! # Pairing and devices
//!
//! A code is 128 random bits, valid five minutes and once, compared in constant time; five
//! failed redemptions in a minute lock redemption for a minute. A redeemed code makes a
//! device: a 256-bit random token goes to the phone as the cookie, and only its SHA-256 is
//! kept, in `<data>/browser/remote.json` (`0600`) beside the device's name and times and the
//! address the listener chose (so a restart tries the same port first). A device unused for
//! `device_days` is forgotten; `revoke` forgets one at once and closes its sockets. At most
//! 4 devices have a socket open at the same time and 16 are remembered.
//!
//! # WebSocket
//!
//! A client message is text JSON `{"id": n, "tool": "...", "scope": "user:...", "args": {..}}`
//! and is answered by text JSON `{"id": n, "ok": <value>}` or `{"id": n, "error": "<text>"}`.
//! The only tools are `view_list`, `view_open`, `view_navigate`, `view_back`, `view_forward`,
//! `view_reload`, `view_resize`, `view_frame`, `view_input`, `view_dialog`, `view_close`,
//! `view_agents`, `view_unwatch`, and `take_control` and `ping` (which answers `{"device": <id>}`),
//! which are answered here. Any
//! other name is refused by name. A `scope` must start `user:` (a phone has no way to reach
//! an agent's page except `view_agents` / `view_frame` with `watch`, which the broker keeps
//! read-only). A tab the phone opens is `user:phone-<device id>:<name>` (at most 3); every
//! other scope must be one `view_list` has shown, so a phone cannot make the broker invent a
//! tab by naming one. `view_list` is answered as `{"tabs": [{scope, title, url, engine}]}`.
//!
//! `view_frame` with a frame to show is answered by a **binary** message instead of base64:
//! `[u32 big-endian header length][header: the text reply JSON with `frame.jpeg` removed][JPEG]`.
//! Without a new frame it is a text reply. One `view_frame` is in flight per socket; a second
//! while one runs is answered `{"ok": {"busy": true}}` and dropped, so a slow phone gets fewer
//! frames, never a queue.
//!
//! **One driver per tab.** The device that last sent `view_open`, `view_input`, `view_resize`,
//! `view_navigate`, `view_back`, `view_forward`, `view_reload`, `view_dialog` or `view_close`
//! for a tab is its driver; another device's such call is answered
//! `another viewer is driving` until it sends `take_control` for that tab. Watching
//! (`view_frame`, `view_list`) is never refused. A device's tabs lose their driver when its
//! last socket closes.
//!
//! Request bodies and input are never logged. Every request and message calls `activity`, which
//! is what keeps the broker from stopping while a phone is looking.
use super::broker::Request;
use crate::config::RemoteConfig;
use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
};

const VIEWER: &str = include_str!("viewer.html");
const COOKIE: &str = "orochi_remote";
const CODE_SECS: u64 = 300;
const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 4096;
const MAX_MESSAGE: usize = 256 * 1024;
const MAX_CONNECTED_DEVICES: usize = 4;
const MAX_REMEMBERED_DEVICES: usize = 16;
const MAX_CONNECTIONS: usize = 64;
const MAX_OWN_TABS: usize = 3;
const FAILED_REDEEMS: usize = 5;

pub type Reply = std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send>>;

/// Where a phone's calls go: the broker's own dispatcher.
pub trait Dispatch: Send + Sync + 'static {
    fn call(&self, request: Request) -> Reply;
}

#[derive(Debug, Clone)]
pub struct Pairing {
    pub url: String,
    pub code: String,
    pub expires_secs: u64,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    name: String,
    created: i64,
    last_seen: i64,
    token_sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    address: String,
    #[serde(default)]
    devices: Vec<Stored>,
}

struct Socket {
    device: String,
    close: watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    file: File,
    code: Option<(String, Instant)>,
    failures: VecDeque<Instant>,
    locked_until: Option<Instant>,
    sockets: HashMap<u64, Socket>,
    next_socket: u64,
    /// tab scope → the device that drives it.
    drivers: HashMap<String, String>,
    /// device → the tabs it opened.
    own: HashMap<String, HashSet<String>>,
    /// device → the scopes its last `view_list` showed.
    known: HashMap<String, HashSet<String>>,
}

struct Shared {
    file: PathBuf,
    device_days: u32,
    dispatch: Arc<dyn Dispatch>,
    activity: Arc<dyn Fn() + Send + Sync>,
    address: SocketAddr,
    state: Mutex<State>,
    stop: watch::Sender<bool>,
    slots: tokio::sync::Semaphore,
}

pub struct Remote {
    shared: Arc<Shared>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn random(bytes: usize) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let mut buffer = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buffer))
        .context("no source of randomness")?;
    Ok(buffer)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        difference |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(1));
    }
    difference == 0
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, state: &State) {
        if let Err(error) = write_file(&self.file, &state.file) {
            tracing::warn!(
                "browser remote: could not write {}: {error:#}",
                self.file.display()
            );
        }
    }

    /// The device a cookie belongs to, refreshing when it was last seen.
    fn authenticate(&self, token: &str) -> Option<String> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let hash = sha256_hex(token.as_bytes());
        let limit = i64::from(self.device_days) * 86_400;
        let at = now();
        let mut state = self.lock();
        let before = state.file.devices.len();
        state.file.devices.retain(|d| at - d.last_seen <= limit);
        let mut dirty = state.file.devices.len() != before;
        let mut found = None;
        for device in &mut state.file.devices {
            if constant_time_eq(device.token_sha256.as_bytes(), hash.as_bytes()) {
                found = Some(device.id.clone());
                if at - device.last_seen >= 60 {
                    device.last_seen = at;
                    dirty = true;
                }
            }
        }
        if dirty {
            self.save(&state);
        }
        found
    }

    /// Redeems a pairing code: the new device's cookie value, or the status to answer.
    fn redeem(&self, code: &str, agent: &str) -> Result<(String, String), (u16, &'static str)> {
        let mut state = self.lock();
        let tick = Instant::now();
        if state.locked_until.is_some_and(|until| tick < until) {
            return Err((429, "too many attempts; wait a minute"));
        }
        let valid = state.code.as_ref().is_some_and(|(wanted, until)| {
            tick < *until && constant_time_eq(wanted.as_bytes(), code.as_bytes())
        });
        if !valid {
            state.failures.push_back(tick);
            while state
                .failures
                .front()
                .is_some_and(|f| tick.duration_since(*f) > Duration::from_secs(60))
            {
                state.failures.pop_front();
            }
            if state.failures.len() >= FAILED_REDEEMS {
                state.locked_until = Some(tick + Duration::from_secs(60));
                state.failures.clear();
            }
            return Err((403, "that code is wrong or has expired"));
        }
        state.code = None;
        let (Ok(id), Ok(token)) = (random(4), random(32)) else {
            return Err((500, "no source of randomness"));
        };
        let id = format!("d{}", hex(&id));
        let kind = if agent.contains("iPhone") {
            "iPhone"
        } else if agent.contains("iPad") {
            "iPad"
        } else if agent.contains("Android") {
            "Android"
        } else if agent.contains("Macintosh") {
            "Mac"
        } else if agent.contains("Windows") {
            "Windows"
        } else {
            "Phone"
        };
        let token = hex(&token);
        let at = now();
        state.file.devices.push(Stored {
            id: id.clone(),
            name: format!("{kind} ({id})"),
            created: at,
            last_seen: at,
            token_sha256: sha256_hex(token.as_bytes()),
        });
        while state.file.devices.len() > MAX_REMEMBERED_DEVICES {
            let oldest = state
                .file
                .devices
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| d.last_seen)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let gone = state.file.devices.remove(oldest);
            forget_device(&mut state, &gone.id);
        }
        self.save(&state);
        Ok((id, token))
    }
}

/// Closes a device's sockets and drops what it held.
fn forget_device(state: &mut State, id: &str) {
    for socket in state.sockets.values().filter(|s| s.device == id) {
        let _ = socket.close.send(true);
    }
    state.drivers.retain(|_, driver| driver != id);
    state.own.remove(id);
    state.known.remove(id);
}

fn write_file(path: &Path, file: &File) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let temporary = path.with_extension("json.tmp");
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    out.write_all(&serde_json::to_vec_pretty(file)?)?;
    out.sync_all()?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

impl Remote {
    pub async fn start(
        data: &Path,
        config: &RemoteConfig,
        dispatch: Arc<dyn Dispatch>,
        activity: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Arc<Self>> {
        let ip = super::policy::remote_bind(config)?;
        let dir = data.join("browser");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("remote.json");
        let mut file: File = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        // The port a paired phone has bookmarked is the one to come back on, if it is free.
        let remembered = file
            .address
            .parse::<SocketAddr>()
            .ok()
            .filter(|a| a.ip() == ip)
            .map(|a| a.port());
        let listener = match (config.port, remembered) {
            (0, Some(port)) => match TcpListener::bind((ip, port)).await {
                Ok(listener) => listener,
                Err(_) => TcpListener::bind((ip, 0)).await?,
            },
            (port, _) => TcpListener::bind((ip, port))
                .await
                .with_context(|| format!("the phone listener could not use {ip}:{port}"))?,
        };
        let address = listener.local_addr()?;
        file.address = address.to_string();
        let limit = i64::from(config.device_days) * 86_400;
        file.devices.retain(|d| now() - d.last_seen <= limit);
        write_file(&path, &file)?;
        let (stop, stopped) = watch::channel(false);
        let shared = Arc::new(Shared {
            file: path,
            device_days: config.device_days,
            dispatch,
            activity,
            address,
            state: Mutex::new(State {
                file,
                ..State::default()
            }),
            stop,
            slots: tokio::sync::Semaphore::new(MAX_CONNECTIONS),
        });
        tokio::spawn(accept_loop(shared.clone(), listener, stopped));
        Ok(Arc::new(Self { shared }))
    }

    pub fn address(&self) -> SocketAddr {
        self.shared.address
    }

    pub fn pair(&self) -> Result<Pairing> {
        self.pair_expiring(CODE_SECS)
    }

    /// A code that stops working after `secs`; `pair` is this with five minutes.
    #[doc(hidden)]
    pub fn pair_expiring(&self, secs: u64) -> Result<Pairing> {
        let code = hex(&random(16)?);
        self.shared.lock().code = Some((code.clone(), Instant::now() + Duration::from_secs(secs)));
        Ok(Pairing {
            url: format!("http://{}/pair#{code}", self.shared.address),
            code,
            expires_secs: secs,
        })
    }

    pub fn devices(&self) -> Vec<Device> {
        self.shared
            .lock()
            .file
            .devices
            .iter()
            .map(|d| Device {
                id: d.id.clone(),
                name: d.name.clone(),
                created: d.created,
                last_seen: d.last_seen,
            })
            .collect()
    }

    /// The devices with a socket open now.
    pub fn connected(&self) -> Vec<String> {
        let state = self.shared.lock();
        let mut ids: Vec<String> = state.sockets.values().map(|s| s.device.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn revoke(&self, id: &str) -> bool {
        let mut state = self.shared.lock();
        let before = state.file.devices.len();
        state.file.devices.retain(|d| d.id != id);
        let removed = state.file.devices.len() != before;
        forget_device(&mut state, id);
        if removed {
            self.shared.save(&state);
        }
        removed
    }

    pub fn stop(&self) {
        let _ = self.shared.stop.send(true);
        let mut state = self.shared.lock();
        state.code = None;
        for socket in state.sockets.values() {
            let _ = socket.close.send(true);
        }
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn accept_loop(
    shared: Arc<Shared>,
    listener: TcpListener,
    mut stopped: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = stopped.changed() => return,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                let Ok(permit) = shared.slots.try_acquire() else { continue };
                permit.forget();
                let shared = shared.clone();
                tokio::spawn(async move {
                    let _ = stream.set_nodelay(true);
                    let _ = handle(shared.clone(), stream, peer).await;
                    shared.slots.add_permits(1);
                });
            }
        }
    }
}

// -- HTTP ---------------------------------------------------------------------------------

struct Http {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_http(stream: &mut TcpStream) -> Result<Http> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        ensure!(buffer.len() <= MAX_HEADERS, "headers too long");
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .context("timed out")??;
        ensure!(n > 0, "closed");
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buffer[..end]).context("not text")?;
    let mut lines = head.split("\r\n");
    let first = lines.next().context("no request line")?;
    let mut parts = first.split(' ');
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("");
    ensure!(
        parts.next().is_some_and(|v| v.starts_with("HTTP/1.")),
        "not HTTP/1.x"
    );
    let path = target.split('?').next().unwrap_or("").to_owned();
    let mut headers = HashMap::new();
    for line in lines.take(64) {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    ensure!(length <= MAX_BODY, "body too long");
    let mut body = buffer[end + 4..].to_vec();
    while body.len() < length {
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .context("timed out")??;
        ensure!(n > 0, "closed");
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Http {
        method,
        path,
        headers,
        body,
    })
}

fn response(
    status: u16,
    reason: &str,
    content_type: &str,
    extra: &[(&str, String)],
    body: &[u8],
) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
         X-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; \
         style-src 'unsafe-inline'; img-src 'self' blob: data:; connect-src 'self' ws: wss:; \
         manifest-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n\
         Connection: close\r\n",
        body.len()
    );
    for (name, value) in extra {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn plain(status: u16, reason: &str, message: &str) -> Vec<u8> {
    response(
        status,
        reason,
        "text/plain; charset=utf-8",
        &[],
        message.as_bytes(),
    )
}

fn json_reply(status: u16, reason: &str, value: &Value, extra: &[(&str, String)]) -> Vec<u8> {
    response(
        status,
        reason,
        "application/json",
        extra,
        value.to_string().as_bytes(),
    )
}

fn cookie_of(headers: &HashMap<String, String>) -> Option<String> {
    headers.get("cookie")?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == COOKIE).then(|| value.to_owned())
    })
}

/// The origin a request says it is for, if its `Host` names this listener.
fn origin_of(
    shared: &Shared,
    headers: &HashMap<String, String>,
    peer: SocketAddr,
) -> Option<String> {
    let host = headers.get("host")?;
    let forwarded = peer.ip().is_loopback()
        && headers
            .get("x-forwarded-proto")
            .is_some_and(|p| p.eq_ignore_ascii_case("https"));
    if forwarded {
        let sane = !host.is_empty()
            && host.len() <= 255
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b));
        return sane.then(|| format!("https://{host}"));
    }
    let address = shared.address;
    let mut names = vec![address.to_string()];
    if address.ip().is_loopback() {
        names.push(format!("localhost:{}", address.port()));
    }
    names
        .iter()
        .any(|n| n.eq_ignore_ascii_case(host))
        .then(|| format!("http://{host}"))
}

async fn handle(shared: Arc<Shared>, mut stream: TcpStream, peer: SocketAddr) -> Result<()> {
    (shared.activity)();
    let request = match read_http(&mut stream).await {
        Ok(request) => request,
        Err(_) => {
            let _ = stream
                .write_all(&plain(400, "Bad Request", "bad request"))
                .await;
            return Ok(());
        }
    };
    let Some(origin) = origin_of(&shared, &request.headers, peer) else {
        stream
            .write_all(&plain(
                421,
                "Misdirected Request",
                "that is not this listener's name",
            ))
            .await?;
        return Ok(());
    };
    let secure = origin.starts_with("https://");
    let device = cookie_of(&request.headers).and_then(|t| shared.authenticate(&t));
    let upgrade = request
        .headers
        .get("upgrade")
        .is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
    let bytes = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/ws") if upgrade => {
            return websocket(shared, stream, &request, &origin, device).await;
        }
        ("GET", "/") if device.is_some() => response(
            200,
            "OK",
            "text/html; charset=utf-8",
            &[],
            VIEWER.as_bytes(),
        ),
        ("GET", "/" | "/pair") => response(
            200,
            "OK",
            "text/html; charset=utf-8",
            &[],
            PAIR_PAGE.as_bytes(),
        ),
        ("POST", "/pair") => {
            // A browser always sends `Origin` on a POST; one that does not, or that names
            // another site, is not the pairing page.
            if request.headers.get("origin").map(String::as_str) != Some(origin.as_str()) {
                plain(403, "Forbidden", "wrong origin")
            } else {
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let code = body["code"].as_str().unwrap_or("");
                let agent = request.headers.get("user-agent").map_or("", String::as_str);
                match shared.redeem(code, agent) {
                    Ok((_, token)) => {
                        let max_age = i64::from(shared.device_days) * 86_400;
                        let cookie = format!(
                            "{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{}",
                            if secure { "; Secure" } else { "" }
                        );
                        json_reply(200, "OK", &json!({"ok": true}), &[("Set-Cookie", cookie)])
                    }
                    Err((status, why)) => json_reply(
                        status,
                        if status == 429 {
                            "Too Many Requests"
                        } else {
                            "Forbidden"
                        },
                        &json!({"ok": false, "error": why}),
                        &[],
                    ),
                }
            }
        }
        _ => plain(404, "Not Found", "not found"),
    };
    stream.write_all(&bytes).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

// -- WebSocket ----------------------------------------------------------------------------

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn accept_key(key: &str) -> String {
    use sha1::Digest as _;
    let mut hasher = sha1::Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(GUID.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= 0xffff => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

/// One client frame: (opcode, final, payload). Client frames must be masked.
async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<(u8, bool, Vec<u8>)> {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head).await?;
    let fin = head[0] & 0x80 != 0;
    ensure!(head[0] & 0x70 == 0, "reserved bits");
    let opcode = head[0] & 0x0f;
    ensure!(head[1] & 0x80 != 0, "client frames are masked");
    let length = match head[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b).await?;
            usize::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            reader.read_exact(&mut b).await?;
            usize::try_from(u64::from_be_bytes(b)).unwrap_or(usize::MAX)
        }
        n => usize::from(n),
    };
    ensure!(length <= MAX_MESSAGE, "message too long");
    let mut mask = [0u8; 4];
    reader.read_exact(&mut mask).await?;
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }
    Ok((opcode, fin, payload))
}

enum Out {
    Text(String),
    Binary(Vec<u8>),
    Pong(Vec<u8>),
    Ping,
    Close(u16),
}

async fn websocket(
    shared: Arc<Shared>,
    mut stream: TcpStream,
    request: &Http,
    origin: &str,
    device: Option<String>,
) -> Result<()> {
    let headers = &request.headers;
    let key = headers.get("sec-websocket-key");
    let connection_upgrade = headers
        .get("connection")
        .is_some_and(|c| c.to_ascii_lowercase().contains("upgrade"));
    let refusal = if device.is_none() {
        Some((401, "Unauthorized", "pair this device first"))
    } else if headers.get("origin").map(String::as_str) != Some(origin) {
        Some((403, "Forbidden", "wrong origin"))
    } else if key.is_none()
        || !connection_upgrade
        || headers.get("sec-websocket-version").map(String::as_str) != Some("13")
    {
        Some((400, "Bad Request", "not a WebSocket upgrade"))
    } else {
        None
    };
    if let Some((status, reason, message)) = refusal {
        stream.write_all(&plain(status, reason, message)).await?;
        return Ok(());
    }
    let device = device.expect("checked above");
    let joined = {
        let mut state = shared.lock();
        let connected: HashSet<String> = state.sockets.values().map(|s| s.device.clone()).collect();
        if !connected.contains(&device) && connected.len() >= MAX_CONNECTED_DEVICES {
            None
        } else {
            let (close, closed) = watch::channel(false);
            let id = state.next_socket;
            state.next_socket += 1;
            state.sockets.insert(
                id,
                Socket {
                    device: device.clone(),
                    close,
                },
            );
            Some((id, closed))
        }
    };
    let Some((id, mut closed)) = joined else {
        stream
            .write_all(&plain(
                503,
                "Service Unavailable",
                "too many devices are connected",
            ))
            .await?;
        return Ok(());
    };
    let mut stopped = shared.stop.subscribe();
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key.expect("checked above"))
    );
    if stream.write_all(handshake.as_bytes()).await.is_err() {
        end_socket(&shared, id, &device);
        return Ok(());
    }
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (out, mut outgoing) = mpsc::channel::<Out>(64);
    let writer_task = tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            let (bytes, last) = match message {
                Out::Text(text) => (frame(0x1, text.as_bytes()), false),
                Out::Binary(bytes) => (frame(0x2, &bytes), false),
                Out::Pong(payload) => (frame(0xa, &payload), false),
                Out::Ping => (frame(0x9, b""), false),
                Out::Close(code) => (frame(0x8, &code.to_be_bytes()), true),
            };
            if write_all(&mut writer, &bytes).await.is_err() || last {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    let (incoming, mut messages) = mpsc::channel::<String>(256);
    let pong = out.clone();
    let reader_task = tokio::spawn(async move {
        let mut partial: Vec<u8> = Vec::new();
        loop {
            let read =
                tokio::time::timeout(Duration::from_secs(120), read_frame(&mut reader)).await;
            let Ok(Ok((opcode, fin, payload))) = read else {
                break;
            };
            match opcode {
                0x8 => break,
                0x9 => {
                    let _ = pong.send(Out::Pong(payload)).await;
                }
                0xa => {}
                0x1 | 0x0 => {
                    partial.extend_from_slice(&payload);
                    if partial.len() > MAX_MESSAGE {
                        break;
                    }
                    if fin {
                        let Ok(text) = String::from_utf8(std::mem::take(&mut partial)) else {
                            break;
                        };
                        if incoming.send(text).await.is_err() {
                            break;
                        }
                    }
                }
                // Binary from a client is not part of the protocol.
                _ => break,
            }
        }
    });
    let in_flight = Arc::new(AtomicBool::new(false));
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    tick.tick().await;
    let mut ended = 1000u16;
    loop {
        tokio::select! {
            _ = closed.changed() => { ended = 1008; break }
            _ = stopped.changed() => { ended = 1001; break }
            _ = tick.tick() => { let _ = out.send(Out::Ping).await; }
            message = messages.recv() => {
                let Some(text) = message else { break };
                (shared.activity)();
                let Ok(call) = serde_json::from_str::<Value>(&text) else {
                    let _ = out.send(Out::Text(json!({"error": "not JSON"}).to_string())).await;
                    continue;
                };
                if call["tool"] == "view_frame" {
                    if in_flight.swap(true, Ordering::SeqCst) {
                        let _ = out.send(Out::Text(json!({"id": call["id"], "ok": {"busy": true}}).to_string())).await;
                        continue;
                    }
                    let (shared, device, out, in_flight) =
                        (shared.clone(), device.clone(), out.clone(), in_flight.clone());
                    tokio::spawn(async move {
                        let reply = answer(&shared, &device, &call).await;
                        in_flight.store(false, Ordering::SeqCst);
                        let _ = out.send(reply).await;
                    });
                } else {
                    let reply = answer(&shared, &device, &call).await;
                    if out.send(reply).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
    let _ = out.send(Out::Close(ended)).await;
    drop(out);
    reader_task.abort();
    let _ = tokio::time::timeout(Duration::from_secs(2), writer_task).await;
    end_socket(&shared, id, &device);
    Ok(())
}

async fn write_all<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes).await?;
    writer.flush().await
}

fn end_socket(shared: &Shared, id: u64, device: &str) {
    let mut state = shared.lock();
    state.sockets.remove(&id);
    if !state.sockets.values().any(|s| s.device == device) {
        state.drivers.retain(|_, driver| driver != device);
    }
}

// -- what a phone may ask -----------------------------------------------------------------

const TOOLS: &[&str] = &[
    "view_list",
    "view_open",
    "view_navigate",
    "view_back",
    "view_forward",
    "view_reload",
    "view_resize",
    "view_frame",
    "view_input",
    "view_dialog",
    "view_close",
    "view_agents",
    "view_unwatch",
];

/// Calls that change what the page is doing: the driver's alone.
const DRIVING: &[&str] = &[
    "view_open",
    "view_navigate",
    "view_back",
    "view_forward",
    "view_reload",
    "view_resize",
    "view_input",
    "view_dialog",
    "view_close",
];

fn own_prefix(device: &str) -> String {
    format!("user:phone-{device}:")
}

fn error(id: &Value, message: impl Into<String>) -> Out {
    Out::Text(json!({"id": id, "error": message.into()}).to_string())
}

fn unwrap_reply(reply: &Value) -> Result<Value, String> {
    let text = reply["content"][0]["text"].as_str().unwrap_or("");
    if reply["isError"] == true {
        return Err(text.to_owned());
    }
    Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
}

async fn dispatch(shared: &Shared, scope: &str, tool: &str, args: Value) -> Result<Value, String> {
    let reply = shared
        .dispatch
        .call(Request {
            scope: scope.to_owned(),
            tool: tool.to_owned(),
            args,
            read_only: false,
            cwd: None,
        })
        .await;
    unwrap_reply(&reply)
}

/// The tabs the person has, from the broker; remembered as what this device may name.
async fn list_tabs(shared: &Shared, device: &str) -> Result<Value, String> {
    let scope = format!("{}list", own_prefix(device));
    let listed = dispatch(shared, &scope, "view_list", Value::Null).await?;
    let tabs = match &listed {
        Value::Array(_) => listed.clone(),
        other => other["tabs"].clone(),
    };
    let scopes: HashSet<String> = tabs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["scope"].as_str().map(str::to_owned))
        .collect();
    shared.lock().known.insert(device.to_owned(), scopes);
    Ok(json!({"tabs": tabs}))
}

async fn may_name(shared: &Shared, device: &str, scope: &str) -> bool {
    if scope.starts_with(&own_prefix(device)) {
        return true;
    }
    if shared
        .lock()
        .known
        .get(device)
        .is_some_and(|k| k.contains(scope))
    {
        return true;
    }
    list_tabs(shared, device).await.is_ok()
        && shared
            .lock()
            .known
            .get(device)
            .is_some_and(|k| k.contains(scope))
}

fn drive(shared: &Shared, device: &str, scope: &str) -> bool {
    let mut state = shared.lock();
    match state.drivers.get(scope) {
        Some(other) if other != device => false,
        _ => {
            state.drivers.insert(scope.to_owned(), device.to_owned());
            true
        }
    }
}

async fn answer(shared: &Shared, device: &str, call: &Value) -> Out {
    let id = &call["id"];
    let Some(tool) = call["tool"].as_str() else {
        return error(id, "a call names a tool");
    };
    let args = if call["args"].is_object() {
        call["args"].clone()
    } else {
        json!({})
    };
    if tool == "ping" {
        return Out::Text(json!({"id": id, "ok": {"device": device}}).to_string());
    }
    if tool != "take_control" && !TOOLS.contains(&tool) {
        return error(id, format!("{tool} is not available here"));
    }
    if tool == "view_list" {
        return match list_tabs(shared, device).await {
            Ok(tabs) => Out::Text(json!({"id": id, "ok": tabs}).to_string()),
            Err(why) => error(id, why),
        };
    }
    let own = own_prefix(device);
    let scope = match (call["scope"].as_str(), tool) {
        (Some(scope), _) => scope.to_owned(),
        (None, "view_agents") => format!("{own}list"),
        (None, _) => return error(id, "a tab is needed"),
    };
    if !scope.starts_with("user:") || scope.len() > 96 || scope.chars().any(char::is_control) {
        return error(id, "that is not a tab of the person's");
    }
    if tool == "view_open" {
        let name = scope.strip_prefix(&own).unwrap_or("");
        if name.is_empty()
            || name.len() > 24
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return error(id, "a phone opens tabs named user:phone-<its id>:<name>");
        }
        let mut state = shared.lock();
        let tabs = state.own.entry(device.to_owned()).or_default();
        if !tabs.contains(&scope) && tabs.len() >= MAX_OWN_TABS {
            return error(
                id,
                format!("a phone has at most {MAX_OWN_TABS} tabs of its own"),
            );
        }
        tabs.insert(scope.clone());
    } else if tool != "view_agents" && !may_name(shared, device, &scope).await {
        return error(id, "that tab is not one the person has open");
    }
    if tool == "take_control" {
        shared
            .lock()
            .drivers
            .insert(scope.clone(), device.to_owned());
        return Out::Text(json!({"id": id, "ok": {"driver": true}}).to_string());
    }
    // A watched agent's page is read-only at the broker; only the person's own tabs need a driver.
    let watching = args["watch"].is_string();
    if DRIVING.contains(&tool) && !watching && !drive(shared, device, &scope) {
        return error(id, "another viewer is driving");
    }
    match dispatch(shared, &scope, tool, args).await {
        Ok(mut value) => {
            if tool == "view_close" {
                let mut state = shared.lock();
                state.drivers.remove(&scope);
                if let Some(tabs) = state.own.get_mut(device) {
                    tabs.remove(&scope);
                }
            }
            if tool == "view_frame"
                && let Some(jpeg) = value["frame"]["jpeg"].as_str().map(str::to_owned)
            {
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(jpeg) else {
                    return error(id, "the frame was not an image");
                };
                value["frame"]["jpeg"] = Value::Null;
                if let Some(frame) = value["frame"].as_object_mut() {
                    frame.remove("jpeg");
                }
                let header = json!({"id": id, "ok": value}).to_string().into_bytes();
                let mut message = Vec::with_capacity(4 + header.len() + bytes.len());
                message.extend_from_slice(&(header.len() as u32).to_be_bytes());
                message.extend_from_slice(&header);
                message.extend_from_slice(&bytes);
                return Out::Binary(message);
            }
            Out::Text(json!({"id": id, "ok": value}).to_string())
        }
        Err(why) => error(id, why),
    }
}

const PAIR_PAGE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>Pair this phone</title>
<style>
:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--mut:#6b6b70;--line:#d8d8dc;--acc:#2f6fed}
@media (prefers-color-scheme:dark){:root{--bg:#141416;--fg:#f2f2f4;--mut:#9a9aa0;--line:#38383c;--acc:#6b9cff}}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 -apple-system,system-ui,sans-serif;
padding:max(24px,env(safe-area-inset-top)) 20px 24px}
main{max-width:30rem;margin:0 auto}h1{font-size:1.25rem}p{color:var(--mut)}
input,button{font:inherit;padding:.7rem .8rem;border-radius:10px;border:1px solid var(--line);
background:transparent;color:inherit;width:100%;box-sizing:border-box;margin:.4rem 0}
button{background:var(--acc);color:#fff;border-color:var(--acc)}#error{color:#d33}
</style></head><body><main>
<h1>Pair this phone</h1>
<p id="about">Open the address Orochi showed you on your computer (Settings → Use from a phone), or paste the code here.</p>
<input id="code" placeholder="Pairing code" autocomplete="off" autocapitalize="off" spellcheck="false">
<button id="go" type="button">Pair</button>
<p id="error" role="alert"></p>
</main><script>
(function(){
var code=document.getElementById("code"),error=document.getElementById("error");
var fromUrl=decodeURIComponent((location.hash||"").replace(/^#/,""));
if(fromUrl){code.value=fromUrl;history.replaceState(null,"",location.pathname);}
function pair(){
  error.textContent="";
  fetch("/pair",{method:"POST",headers:{"Content-Type":"application/json"},
    body:JSON.stringify({code:code.value.trim()})}).then(function(r){return r.json().catch(function(){return{};});})
  .then(function(j){if(j.ok){location.replace("/");}else{error.textContent=j.error||"That did not work.";}})
  .catch(function(){error.textContent="Could not reach Orochi.";});
}
document.getElementById("go").addEventListener("click",pair);
code.addEventListener("keydown",function(e){if(e.key==="Enter")pair();});
if(fromUrl)pair();
})();
</script></body></html>
"##;
