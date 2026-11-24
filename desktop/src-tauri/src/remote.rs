//! The phone's door (`docs/mobile-remote-design.md`).
//!
//! An HTTP + WebSocket server bound to the machine's Tailscale address and nothing else. It is
//! a library module so it is tested without a webview: every route ends in a call the window
//! already makes on `view::Client`, behind a whitelist, after a device token has been checked.
//! Off until the user turns it on in Settings → Mobile.
use crate::view::Client;
use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Router,
    body::Bytes,
    extract::{
        Path as UrlPath, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, watch};

pub const DEFAULT_PORT: u16 = 47821;
const CODE_TTL_MS: i64 = 5 * 60 * 1000;
const CODE_FAILURES: u32 = 5;
const FILE_LIMIT: u64 = crate::files::MEDIA_LIMIT;

/// Read commands, then write commands: the only names `/rpc/<command>` answers. Never here:
/// terminals, settings, sandboxes, setup, keys, the browser, opening local programs, places,
/// the roster and memory writers, deleting a project, notifications and the dock badge.
const READS: &[&str] = &[
    "sidebar",
    "thread",
    "patch",
    "room",
    "chatter",
    "members",
    "mentions",
    "team",
    "board",
    "open_prompts",
    "live_seats",
    "live_tasks",
    "search",
    "agents",
    "insights",
    "folders",
    "git_overview",
    "git_show",
    "tree_files",
    "tree_patch",
    "tree_list",
    "tree_read",
    "tree_media",
    "tree_find",
    "watch",
    "agent_icons",
    "browse",
    "residents",
    "routines",
    "routine_firings",
    "notices",
    "keeper",
];
const WRITES: &[&str] = &[
    "send",
    "say",
    "answer",
    "interrupt",
    "stop_seat",
    "seen",
    "see_mentions",
    "new_thread",
    "ensure_host",
    "rename_thread",
    "rename_member",
    "pin_thread",
    "archive_thread",
    "set_route",
    "comment",
    "add_project",
    "clone_project",
    "routine_action",
    "read_notices",
    "resident_add",
    "resident_action",
];

pub fn whitelisted(command: &str) -> bool {
    READS.contains(&command) || WRITES.contains(&command)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random(bytes: &mut [u8]) {
    getrandom::fill(bytes).expect("the system has a source of randomness");
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

// ---- where to listen ---------------------------------------------------------------------

fn in_tailnet(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..=127).contains(&b)
}

fn tailscale_cli() -> Option<Ipv4Addr> {
    const BINARIES: &[&str] = &[
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/opt/homebrew/bin/tailscale",
        "/usr/local/bin/tailscale",
    ];
    BINARIES.iter().find_map(|binary| {
        let output = std::process::Command::new(binary)
            .args(["ip", "-4"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        let ip: Ipv4Addr = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()?;
        (output.status.success() && in_tailnet(ip)).then_some(ip)
    })
}

#[cfg(unix)]
fn interface_scan() -> Option<Ipv4Addr> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list that is read only until freeifaddrs below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut at = list;
    while !at.is_null() && found.is_none() {
        // SAFETY: `at` is a node of the list getifaddrs returned.
        let node = unsafe { &*at };
        if !node.ifa_addr.is_null() && unsafe { (*node.ifa_addr).sa_family } as i32 == libc::AF_INET
        {
            // SAFETY: an AF_INET address is a sockaddr_in.
            let sin = unsafe { &*(node.ifa_addr as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            if in_tailnet(ip) {
                found = Some(ip);
            }
        }
        at = node.ifa_next;
    }
    // SAFETY: `list` came from getifaddrs and is not used again.
    unsafe { libc::freeifaddrs(list) };
    found
}

#[cfg(not(unix))]
fn interface_scan() -> Option<Ipv4Addr> {
    None
}

/// The address the server may listen on. `OROCHI_REMOTE_BIND` (`ip` or `ip:port`) is for tests.
pub fn bind_address() -> Result<SocketAddr> {
    if let Some(value) = std::env::var_os("OROCHI_REMOTE_BIND") {
        let value = value.to_string_lossy().into_owned();
        return value
            .parse::<SocketAddr>()
            .or_else(|_| {
                value
                    .parse::<IpAddr>()
                    .map(|ip| SocketAddr::new(ip, DEFAULT_PORT))
            })
            .with_context(|| format!("OROCHI_REMOTE_BIND={value} is not an address"));
    }
    let ip = tailscale_cli().or_else(interface_scan).ok_or_else(|| {
        anyhow!("No Tailscale address was found. Install Tailscale and sign in on this machine.")
    })?;
    Ok(SocketAddr::new(IpAddr::V4(ip), DEFAULT_PORT))
}

/// The host name without its domain (`mac.example.net` and `mac.local` are both `mac`).
fn short_host(host: &str) -> String {
    host.split('.').next().unwrap_or(host).trim().to_owned()
}

fn default_name() -> String {
    #[cfg(unix)]
    {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is as long as the length given.
        if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0 {
            let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
            let name = short_host(&String::from_utf8_lossy(&buffer[..end]));
            if !name.is_empty() {
                return name;
            }
        }
    }
    "orochi".into()
}

const NAME_LIMIT: usize = 40;

/// What the phone is told this desktop is called: the name chosen in Settings, else the host
/// name without its domain.
fn machine_name(data: &Path) -> String {
    read_machine(data)
        .get("name")
        .and_then(Value::as_str)
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(default_name)
}

fn read_machine(data: &Path) -> serde_json::Map<String, Value> {
    std::fs::read(folder(data).join("machine.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---- devices and pairing -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    id: String,
    name: String,
    /// SHA-256 of the bearer token; the token itself is never kept.
    token_hash: String,
    paired_at: i64,
    /// The phone's own id for this install (the app sends it when pairing): pairing again with
    /// the same one replaces this row instead of leaving it beside the new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    /// The random id this phone registered with the push service (`orochi::notify`): what a
    /// notice is addressed to. It names nobody, and goes with the pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    push_id: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    devices: Vec<Stored>,
}

/// A paired phone as Settings lists it.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
}

fn folder(data: &Path) -> PathBuf {
    data.join("remote")
}

/// The id this machine answers to, made once and kept: a phone paired with several desktops
/// tells them apart by it, and a name is not unique.
fn machine_id(data: &Path) -> String {
    let path = folder(data).join("machine.json");
    let kept = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
        .filter(|id| id.len() >= 16 && id.chars().all(|c| c.is_ascii_alphanumeric()));
    if let Some(id) = kept {
        return id;
    }
    let mut bytes = [0u8; 16];
    random(&mut bytes);
    let id = hex(&bytes);
    let mut kept = read_machine(data);
    kept.insert("id".into(), json!(id));
    let _ = write_private(&path, Value::Object(kept).to_string().as_bytes());
    id
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("no folder")?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let temporary = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(&temporary)?, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// The device list. `devices.json` is the source of truth: edits made to the file while the
/// server runs (a revoked row, an emptied list) take effect on the next request, found by a
/// cheap `(modified, length)` check, and our own writes are remembered so they do not reload.
struct Devices {
    path: PathBuf,
    list: Vec<Stored>,
    seen: HashMap<String, i64>,
    stamp: Option<(SystemTime, u64)>,
}

impl Devices {
    fn load(data: &Path) -> Self {
        let mut devices = Self {
            path: folder(data).join("devices.json"),
            list: Vec::new(),
            seen: HashMap::new(),
            stamp: None,
        };
        devices.refresh();
        devices
    }

    fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// Reads the file again when it is not the one last read or written. A file that is gone
    /// is an empty list (every phone revoked); one that does not parse leaves the list as it was.
    fn refresh(&mut self) {
        let now = Self::stamp_of(&self.path);
        if now == self.stamp {
            return;
        }
        match now {
            None => self.list.clear(),
            Some(_) => match std::fs::read(&self.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
            {
                Some(file) => self.list = file.devices,
                None => return,
            },
        }
        self.stamp = now;
    }

    fn save(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&File {
            devices: self.list.clone(),
        })?;
        write_private(&self.path, &bytes)?;
        self.stamp = Self::stamp_of(&self.path);
        Ok(())
    }

    /// The device a token belongs to. Every stored hash is compared, whatever the answer.
    fn find(&mut self, token: &str) -> Option<Stored> {
        self.refresh();
        let given = token_hash(token);
        let mut found = None;
        for device in &self.list {
            if same(device.token_hash.as_bytes(), given.as_bytes()) {
                found = Some(device.clone());
            }
        }
        if let Some(device) = &found {
            self.seen.insert(device.id.clone(), now_ms());
        }
        found
    }
}

struct Code {
    code: String,
    expires_at: i64,
    failures: u32,
}

/// What Settings shows to pair a phone.
#[derive(Debug, Clone, Serialize)]
pub struct Pairing {
    pub svg: String,
    pub code: String,
    pub expires_at: i64,
    pub url: String,
    /// The same link by hand: for a phone that cannot scan.
    pub host: String,
    pub port: u16,
    pub name: String,
    pub id: String,
}

/// What Settings → Mobile shows about the server.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub running: bool,
    pub address: Option<String>,
    pub name: String,
    /// What the name is when none is chosen: the host name without its domain.
    pub default_name: String,
    pub id: String,
    pub error: Option<String>,
    pub devices: usize,
    /// Where a phone's "clone a repository" puts it; none until chosen, and then it cannot.
    pub clone_dir: Option<String>,
}

// ---- the server --------------------------------------------------------------------------

struct Shared {
    data: PathBuf,
    binary: PathBuf,
    client: Arc<Mutex<Client>>,
    name: Mutex<String>,
    id: String,
    devices: Mutex<Devices>,
    pairing: Mutex<Option<Code>>,
    events: broadcast::Sender<String>,
}

#[derive(Clone)]
struct App {
    shared: Arc<Shared>,
    stop: watch::Receiver<bool>,
}

struct Running {
    addr: SocketAddr,
    stop: watch::Sender<bool>,
    poller: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

/// The remote server of one window: its devices and pairing outlive the server being turned
/// off and on.
pub struct Remote {
    shared: Arc<Shared>,
    running: Mutex<Option<Running>>,
    error: Mutex<Option<String>>,
}

fn enabled_file(data: &Path) -> PathBuf {
    folder(data).join("enabled")
}

impl Remote {
    /// `client` is the connection the window answers through; the server shares it and never
    /// holds it across an await.
    pub fn new(data: &Path, client: Arc<Mutex<Client>>, binary: PathBuf) -> Self {
        Self {
            shared: Arc::new(Shared {
                data: data.to_path_buf(),
                binary,
                client,
                name: Mutex::new(machine_name(data)),
                id: machine_id(data),
                devices: Mutex::new(Devices::load(data)),
                pairing: Mutex::new(None),
                events: broadcast::channel(256).0,
            }),
            running: Mutex::new(None),
            error: Mutex::new(None),
        }
    }

    /// What this desktop is called to a phone.
    pub fn name(&self) -> String {
        locked(&self.shared.name).clone()
    }

    /// Settings → Mobile's display name; an empty one goes back to the host name. It is read
    /// by `/health`, `/me` and `/pair` and shown on the phone as the desktop's name.
    pub fn set_name(&self, name: &str) -> Result<Status> {
        let clean: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.chars().any(char::is_control) {
            bail!("The name cannot contain control characters");
        }
        if clean.chars().count() > NAME_LIMIT {
            bail!("The name is longer than {NAME_LIMIT} characters");
        }
        let mut kept = read_machine(&self.shared.data);
        kept.insert("id".into(), json!(self.shared.id));
        if clean.is_empty() {
            kept.remove("name");
        } else {
            kept.insert("name".into(), json!(clean));
        }
        write_private(
            &folder(&self.shared.data).join("machine.json"),
            Value::Object(kept).to_string().as_bytes(),
        )?;
        *locked(&self.shared.name) = if clean.is_empty() {
            default_name()
        } else {
            clean
        };
        Ok(self.status())
    }

    pub fn enabled(&self) -> bool {
        enabled_file(&self.shared.data).is_file()
    }

    fn set_enabled(&self, on: bool) -> Result<()> {
        let path = enabled_file(&self.shared.data);
        if on {
            write_private(&path, b"1\n")
        } else {
            match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            }
        }
    }

    /// Starts the server when the user left it on. A failure is kept for `status` to show.
    pub async fn resume(&self) {
        if self.enabled()
            && let Err(error) = self.start().await
        {
            *locked(&self.error) = Some(format!("{error:#}"));
        }
    }

    pub async fn enable(&self) -> Result<Status> {
        self.start().await?;
        self.set_enabled(true)?;
        Ok(self.status())
    }

    pub async fn disable(&self) -> Result<Status> {
        self.set_enabled(false)?;
        self.stop().await;
        *locked(&self.shared.pairing) = None;
        Ok(self.status())
    }

    async fn start(&self) -> Result<()> {
        if locked(&self.running).is_some() {
            return Ok(());
        }
        *locked(&self.error) = None;
        let wanted = blocking(bind_address).await??;
        let listener = match tokio::net::TcpListener::bind(wanted).await {
            Ok(listener) => listener,
            Err(_) if std::env::var_os("OROCHI_REMOTE_BIND").is_none() => {
                tokio::net::TcpListener::bind(SocketAddr::new(wanted.ip(), 0)).await?
            }
            Err(error) => bail!("cannot listen on {wanted}: {error}"),
        };
        let addr = listener.local_addr()?;
        let (stop, stopped) = watch::channel(false);
        let app = App {
            shared: self.shared.clone(),
            stop: stopped.clone(),
        };
        let router = Router::new()
            .route("/health", get(health))
            .route("/pair", post(pair))
            .route("/me", get(me))
            .route("/push", post(register_push))
            .route("/rpc/{command}", post(rpc))
            .route("/events", get(events))
            .route("/media/{thread}/{sha}", get(media))
            .route("/file", get(file))
            .with_state(app);
        let mut wait = stopped;
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = wait.wait_for(|stop| *stop).await;
                })
                .await;
        });
        let poller = Arc::new(AtomicBool::new(false));
        poll(self.shared.clone(), poller.clone());
        *locked(&self.running) = Some(Running {
            addr,
            stop,
            poller,
            task,
        });
        Ok(())
    }

    async fn stop(&self) {
        let running = locked(&self.running).take();
        if let Some(running) = running {
            running.poller.store(true, Ordering::SeqCst);
            let _ = running.stop.send(true);
            let mut task = running.task;
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
    }

    pub fn address(&self) -> Option<SocketAddr> {
        locked(&self.running).as_ref().map(|r| r.addr)
    }

    pub fn status(&self) -> Status {
        let address = self.address();
        Status {
            enabled: self.enabled(),
            running: address.is_some(),
            address: address.map(|a| a.to_string()),
            name: self.name(),
            default_name: default_name(),
            id: self.shared.id.clone(),
            error: locked(&self.error).clone(),
            devices: {
                let mut devices = locked(&self.shared.devices);
                devices.refresh();
                devices.list.len()
            },
            clone_dir: clone_dir(&self.shared.data).map(|p| p.display().to_string()),
        }
    }

    /// Chooses (or, empty, forgets) the folder phones clone into. It must already exist.
    pub fn set_clone_dir(&self, path: &str) -> Result<Status> {
        let file = projects_file(&self.shared.data);
        let path = path.trim();
        if path.is_empty() {
            match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => return Ok(self.status()),
            }
        }
        if !Path::new(path).is_absolute() {
            bail!("The clone folder must be an absolute path");
        }
        let dir = Path::new(path)
            .canonicalize()
            .with_context(|| format!("{path} does not exist"))?;
        if !dir.is_dir() {
            bail!("{} is not a folder", dir.display());
        }
        write_private(
            &file,
            json!({ "clone_dir": dir.display().to_string() })
                .to_string()
                .as_bytes(),
        )?;
        Ok(self.status())
    }

    /// A new single-use code, valid five minutes, and the QR that carries it. Any earlier
    /// unused code is void.
    pub fn pair(&self) -> Result<Pairing> {
        let addr = self
            .address()
            .context("Mobile access is off, or has no Tailscale address")?;
        let mut digits = [0u8; 8];
        random(&mut digits);
        let code: String = digits.iter().map(|b| char::from(b'0' + (b % 10))).collect();
        let expires_at = now_ms() + CODE_TTL_MS;
        *locked(&self.shared.pairing) = Some(Code {
            code: code.clone(),
            expires_at,
            failures: 0,
        });
        let url = format!(
            "orochi://pair?host={}&port={}&code={}&name={}&id={}",
            addr.ip(),
            addr.port(),
            code,
            encode(&self.name()),
            self.shared.id
        );
        let svg = qrcode::QrCode::new(url.as_bytes())?
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build();
        Ok(Pairing {
            svg,
            code,
            expires_at,
            url,
            host: addr.ip().to_string(),
            port: addr.port(),
            name: self.name(),
            id: self.shared.id.clone(),
        })
    }

    /// Lets a test see a code run out without waiting five minutes.
    #[doc(hidden)]
    pub fn expire_pairing(&self) {
        if let Some(code) = locked(&self.shared.pairing).as_mut() {
            code.expires_at = 0;
        }
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        let mut devices = locked(&self.shared.devices);
        devices.refresh();
        devices
            .list
            .iter()
            .map(|d| DeviceView {
                id: d.id.clone(),
                name: d.name.clone(),
                paired_at: d.paired_at,
                last_seen: devices.seen.get(&d.id).copied(),
            })
            .collect()
    }

    pub fn revoke(&self, id: &str) -> Result<bool> {
        let mut devices = locked(&self.shared.devices);
        devices.refresh();
        let before = devices.list.len();
        // A phone that is unpaired is pushed to no more.
        for gone in devices.list.iter().filter(|d| d.id == id) {
            if let Some(push) = &gone.push_id {
                let _ = orochi::notify::remove_phone(&self.shared.data, push);
            }
        }
        devices.list.retain(|d| d.id != id);
        let removed = devices.list.len() != before;
        if removed {
            devices.save()?;
        }
        Ok(removed)
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| anyhow!("{e}"))
}

// ---- the change poller -------------------------------------------------------------------

/// One reader of the change feed for every connected phone, on a connection and cursor of its
/// own: sharing the window's would take the changes the window is waiting for.
fn poll(shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let Ok(mut client) = Client::open(&shared.data) else {
            return;
        };
        // What is already there is not news; `hello` tells a phone to read everything.
        let _ = client.changed();
        let mut prompts: Option<Vec<String>> = None;
        let mut tick = 0u32;
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(250));
            tick += 1;
            let listening = shared.events.receiver_count() > 0;
            if let Ok(threads) = client.changed()
                && listening
                && !threads.is_empty()
            {
                let threads: Vec<String> = threads.into_iter().collect();
                let _ = shared
                    .events
                    .send(json!({"type": "changed", "threads": threads}).to_string());
            }
            if listening
                && tick.is_multiple_of(4)
                && let Ok(open) = client.open_prompts()
            {
                let ids: Vec<String> = open.into_iter().map(|p| p.id).collect();
                if prompts.as_ref().is_some_and(|before| *before != ids) {
                    let _ = shared.events.send(json!({"type": "prompts"}).to_string());
                }
                prompts = Some(ids);
            }
        }
    });
}

// ---- routes ------------------------------------------------------------------------------

fn failure(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "error": message }))).into_response()
}

fn unauthorized() -> Response {
    failure(StatusCode::UNAUTHORIZED, "unauthorized")
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim().to_owned())
}

/// The paired device a request is from. `query` allows `?token=` for the routes a player or an
/// image view fetches without being able to set a header.
fn authenticate(
    app: &App,
    headers: &HeaderMap,
    query: Option<&HashMap<String, String>>,
) -> Option<Stored> {
    let token = bearer(headers).or_else(|| query.and_then(|q| q.get("token").cloned()))?;
    locked(&app.shared.devices).find(&token)
}

async fn health(State(app): State<App>) -> Response {
    axum::Json(json!({
        "ok": true,
        "name": locked(&app.shared.name).clone(),
        "id": app.shared.id,
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
    #[serde(default)]
    device: String,
    /// The phone's stable id for this install; see `Stored::device_id`.
    #[serde(default)]
    device_id: Option<String>,
    /// The phone's push id, when it has notifications on.
    #[serde(default)]
    push_id: Option<String>,
}

async fn pair(State(app): State<App>, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<PairBody>(&body) else {
        return failure(StatusCode::BAD_REQUEST, "expected {code, device}");
    };
    {
        let mut pairing = locked(&app.shared.pairing);
        let accepted = match pairing.as_mut() {
            Some(open) if open.expires_at > now_ms() => {
                if same(open.code.as_bytes(), body.code.trim().as_bytes()) {
                    true
                } else {
                    open.failures += 1;
                    if open.failures >= CODE_FAILURES {
                        *pairing = None;
                    }
                    false
                }
            }
            Some(_) => {
                *pairing = None;
                false
            }
            None => false,
        };
        if !accepted {
            return failure(StatusCode::UNAUTHORIZED, "invalid or expired code");
        }
        *pairing = None;
    }
    let mut secret = [0u8; 32];
    random(&mut secret);
    let token = hex(&secret);
    let mut id = [0u8; 8];
    random(&mut id);
    let name: String = body.device.trim().chars().take(64).collect();
    let stored = Stored {
        id: hex(&id),
        name: if name.is_empty() {
            "phone".into()
        } else {
            name
        },
        token_hash: token_hash(&token),
        paired_at: now_ms(),
        device_id: body
            .device_id
            .as_deref()
            .map(str::trim)
            .filter(|d| {
                (8..=64).contains(&d.len())
                    && d.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
            .map(str::to_owned),
        push_id: body
            .push_id
            .as_deref()
            .map(str::trim)
            .filter(|p| orochi::notify::valid_push_id(p))
            .map(str::to_owned),
    };
    let mut devices = locked(&app.shared.devices);
    devices.refresh();
    let before = devices.list.clone();
    // The same install pairing again replaces its earlier row (and so that token), never
    // another phone's: nothing is guessed from the name.
    if let Some(id) = &stored.device_id {
        for old in devices
            .list
            .iter()
            .filter(|d| d.device_id.as_deref() == Some(id.as_str()))
        {
            if let Some(push) = &old.push_id {
                let _ = orochi::notify::remove_phone(&app.shared.data, push);
            }
        }
        devices
            .list
            .retain(|d| d.device_id.as_deref() != Some(id.as_str()));
    }
    devices.list.push(stored.clone());
    if let Err(error) = devices.save() {
        devices.list = before;
        return failure(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}"));
    }
    if let Some(push) = &stored.push_id {
        let _ = orochi::notify::add_phone(&app.shared.data, push, &stored.name);
    }
    axum::Json(
        json!({ "token": token, "name": locked(&app.shared.name).clone(), "id": app.shared.id }),
    )
    .into_response()
}

#[derive(Deserialize)]
struct PushBody {
    push_id: String,
}

/// A paired phone gives (or changes) the id notices are addressed to: it only learns it once
/// the app has notification permission, which is after it paired.
async fn register_push(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(device) = authenticate(&app, &headers, None) else {
        return unauthorized();
    };
    let Ok(body) = serde_json::from_slice::<PushBody>(&body) else {
        return failure(StatusCode::BAD_REQUEST, "expected {push_id}");
    };
    let id = body.push_id.trim();
    if !orochi::notify::valid_push_id(id) {
        return failure(StatusCode::BAD_REQUEST, "that is not a push id");
    }
    let mut devices = locked(&app.shared.devices);
    devices.refresh();
    let Some(row) = devices.list.iter_mut().find(|d| d.id == device.id) else {
        return unauthorized();
    };
    if let Some(old) = row.push_id.replace(id.to_owned())
        && old != id
    {
        let _ = orochi::notify::remove_phone(&app.shared.data, &old);
    }
    let name = row.name.clone();
    if let Err(error) = devices.save() {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}"));
    }
    let _ = orochi::notify::add_phone(&app.shared.data, id, &name);
    axum::Json(json!({ "ok": true })).into_response()
}

async fn me(State(app): State<App>, headers: HeaderMap) -> Response {
    match authenticate(&app, &headers, None) {
        Some(device) => axum::Json(json!({
            "device": device.name,
            "paired_at": device.paired_at,
            "id": app.shared.id,
            "name": locked(&app.shared.name).clone(),
        }))
        .into_response(),
        None => unauthorized(),
    }
}

struct Failure(StatusCode, String);

impl Failure {
    fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}"))
    }
}

fn text(args: &Value, name: &str) -> Result<String, Failure> {
    args[name]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Failure::bad(format!("`{name}` is needed")))
}

fn maybe(args: &Value, name: &str) -> Option<String> {
    args[name].as_str().map(str::to_owned)
}

fn flag(args: &Value, name: &str) -> bool {
    args[name].as_bool().unwrap_or(false)
}

fn number(args: &Value, name: &str) -> Result<i64, Failure> {
    args[name]
        .as_i64()
        .ok_or_else(|| Failure::bad(format!("`{name}` is needed")))
}

fn value<T: Serialize>(result: Result<T>) -> Result<Value, Failure> {
    Ok(serde_json::to_value(result?).map_err(|e| anyhow!(e))?)
}

fn call(
    client: &mut Client,
    shared: &Shared,
    command: &str,
    args: &Value,
) -> Result<Value, Failure> {
    let thread = || text(args, "thread");
    match command {
        "sidebar" => value(client.sidebar(
            number(args, "limit").unwrap_or(50).max(0) as usize,
            flag(args, "archived"),
        )),
        "thread" => value(client.thread(&text(args, "id")?)),
        "patch" => value(client.patch(number(args, "id")?)),
        "room" => value(client.room(&thread()?)),
        "chatter" => value(client.chatter(number(args, "since").unwrap_or(0), 60)),
        "members" => value(client.members(&thread()?)),
        "rename_member" => {
            value(client.rename_member(&thread()?, &text(args, "member")?, &text(args, "name")?))
        }
        "mentions" => value(client.mentions()),
        "team" => value(client.team(&thread()?)),
        "board" => value(client.board(&thread()?)),
        "open_prompts" => value(client.open_prompts()),
        "live_seats" => value(client.live_seats()),
        "live_tasks" => value(client.live_tasks()),
        "search" => value(client.search(&text(args, "query")?)),
        "agents" => value(client.agents()),
        "insights" => value(client.insights()),
        "residents" => value(client.residents()),
        "routines" => value(client.routines()),
        "routine_firings" => value(client.routine_firings(&text(args, "id")?)),
        "notices" => value(client.notices(flag(args, "unread"))),
        "keeper" => value(client.keeper()),
        // What a phone may do to a routine is to run, pause or resume it, and allow or
        // decline what an agent proposed in the conversation; deleting one stays at the window.
        "routine_action" => {
            let action = text(args, "action")?;
            if !matches!(
                action.as_str(),
                "run" | "pause" | "resume" | "approve" | "decline"
            ) {
                return Err(Failure::bad(
                    "a phone can run, pause, resume, allow or decline a routine",
                ));
            }
            value(client.routine_action(&shared.binary, &text(args, "id")?, &action))
        }
        "read_notices" => value(client.read_notices(args.get("id").and_then(Value::as_i64))),
        "resident_add" => value(
            client.resident_add(
                &text(args, "name")?,
                args.get("instruction")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ),
        ),
        // The phone pauses or resumes a resident, its one switch for everything the resident
        // has scheduled; deleting one stays at the window.
        "resident_action" => {
            let action = text(args, "action")?;
            if !matches!(action.as_str(), "pause" | "resume") {
                return Err(Failure::bad("a phone can pause or resume a resident"));
            }
            value(client.resident_action(&text(args, "name")?, &action))
        }
        "folders" => value(client.folders()),
        "git_overview" => value(client.git_overview(&thread()?)),
        "git_show" => value(client.git_show(&thread()?, &text(args, "hash")?)),
        "tree_files" => value(client.tree_files(&thread()?, &text(args, "scope")?)),
        "tree_patch" => {
            value(client.tree_patch(&thread()?, &text(args, "scope")?, &text(args, "path")?))
        }
        "tree_list" => value(client.tree_list(&thread()?, &text(args, "dir")?)),
        "tree_read" => value(client.tree_read(&thread()?, &text(args, "path")?)),
        "tree_media" => value(client.tree_media(&thread()?, &text(args, "path")?)),
        "tree_find" => value(client.tree_find(
            &thread()?,
            &text(args, "query")?,
            &text(args, "by")?,
            number(args, "limit").unwrap_or(50).clamp(1, 500) as usize,
        )),
        "watch" => value(client.watch(&thread()?)),
        "agent_icons" => {
            let scratch = std::env::temp_dir().join("orochi-desktop");
            let _ = std::fs::create_dir_all(&scratch);
            Ok(serde_json::to_value(crate::view::agent_icons(&scratch)).unwrap_or(Value::Null))
        }
        "send" => value(client.send(&thread()?, &text(args, "text")?)),
        "say" => value(client.say_in(
            &thread()?,
            maybe(args, "to").as_deref(),
            &text(args, "text")?,
            maybe(args, "member").as_deref(),
        )),
        "answer" => value(client.answer(&text(args, "prompt")?, maybe(args, "option").as_deref())),
        "interrupt" => value(client.interrupt(&thread()?)),
        "stop_seat" => value(client.stop_seat(&thread()?, maybe(args, "seat").as_deref())),
        "seen" => value(client.seen(&thread()?)),
        "see_mentions" => value(client.see_mentions(&thread()?)),
        "new_thread" => {
            // The phone may start a thread only where the window already knows a project:
            // it cannot make the desktop record a thread in an arbitrary folder.
            let asked = Path::new(&text(args, "root")?).canonicalize().ok();
            let known = client
                .folders()?
                .into_iter()
                .find(|f| asked.is_some() && Path::new(&f.root).canonicalize().ok() == asked);
            let Some(known) = known else {
                return Err(Failure::bad("unknown project folder"));
            };
            value(client.new_thread(
                Path::new(&known.root),
                flag(args, "worktree"),
                maybe(args, "sandbox").as_deref(),
            ))
        }
        "browse" => browse(shared, maybe(args, "path")),
        "add_project" => {
            let root = reachable(&shared.data, Path::new(&text(args, "root")?))?;
            let id = client.add_project(&root)?;
            Ok(json!({ "id": id, "root": root.display().to_string() }))
        }
        "ensure_host" => {
            let thread = thread()?;
            if !client.needs_host(&thread)? {
                return Ok(Value::Bool(false));
            }
            let (config, data) = client.paths();
            crate::view::start_host(&shared.binary, &config, &data, &thread)?;
            Ok(Value::Bool(true))
        }
        "rename_thread" => value(client.rename_thread(&thread()?, &text(args, "title")?)),
        "pin_thread" => value(client.pin_thread(&thread()?, flag(args, "pinned"))),
        "archive_thread" => value(client.archive_thread(&thread()?, flag(args, "archived"))),
        "set_route" => value(client.set_route(&thread()?, maybe(args, "route").as_deref())),
        "comment" => {
            let comments: Vec<(String, u64, String)> =
                serde_json::from_value(args["comments"].clone())
                    .map_err(|_| Failure::bad("`comments` is needed"))?;
            value(client.comment(&thread()?, &comments))
        }
        _ => Err(Failure(StatusCode::NOT_FOUND, "unknown command".into())),
    }
}

async fn rpc(
    State(app): State<App>,
    UrlPath(command): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if authenticate(&app, &headers, None).is_none() {
        return unauthorized();
    }
    if !whitelisted(&command) {
        return failure(StatusCode::NOT_FOUND, "unknown command");
    }
    let args = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(args) if args.is_object() => args,
            _ => return failure(StatusCode::BAD_REQUEST, "the body must be a JSON object"),
        }
    };
    let shared = app.shared.clone();
    let name = command.clone();
    let result = tokio::task::spawn_blocking(move || {
        // A clone takes minutes and must not hold the window's connection while it runs.
        if name == "clone_project" {
            return clone_project(&shared, &args);
        }
        let mut client = locked(&shared.client);
        call(&mut client, &shared, &name, &args)
    })
    .await;
    match result {
        Ok(Ok(result)) => {
            if command == "answer" {
                let _ = app
                    .shared
                    .events
                    .send(json!({"type": "prompts"}).to_string());
            }
            axum::Json(result).into_response()
        }
        Ok(Err(Failure(status, message))) => failure(status, &message),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "the command failed"),
    }
}

// ---- projects started from the phone -------------------------------------------------------
// A phone may add a folder it can see from the places below, or clone into the folder chosen in
// Settings; it can never name another place for a project to live.

fn projects_file(data: &Path) -> PathBuf {
    folder(data).join("projects.json")
}

/// The folder chosen in Settings for clones; none when unset or no longer a folder.
fn clone_dir(data: &Path) -> Option<PathBuf> {
    let saved: Value = serde_json::from_slice(&std::fs::read(projects_file(data)).ok()?).ok()?;
    PathBuf::from(saved["clone_dir"].as_str()?)
        .canonicalize()
        .ok()
        .filter(|p| p.is_dir())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .and_then(|h| PathBuf::from(h).canonicalize().ok())
}

/// A folder inside the user's home or the clone folder, symlinks resolved first.
fn reachable(data: &Path, path: &Path) -> Result<PathBuf, Failure> {
    let path = path
        .canonicalize()
        .map_err(|_| Failure::bad("that folder does not exist"))?;
    let inside = [clone_dir(data), home_dir()]
        .iter()
        .flatten()
        .any(|root| path.starts_with(root));
    if !path.is_dir() || !inside {
        return Err(Failure::bad(
            "that folder is not a place a phone may use (the home folder or the clone folder)",
        ));
    }
    Ok(path)
}

/// The folders under `path` (visible ones only), to pick a project from the phone.
fn browse(shared: &Shared, path: Option<String>) -> Result<Value, Failure> {
    let start = path
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| clone_dir(&shared.data))
        .or_else(home_dir)
        .ok_or_else(|| Failure::bad("this desktop has no home folder to start from"))?;
    let path = reachable(&shared.data, &start)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| anyhow!(e))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A symlink is followed only if it ends in a folder the phone may use anyway.
        let full = entry.path();
        if name.starts_with('.') || !(kind.is_dir() || kind.is_symlink() && full.is_dir()) {
            continue;
        }
        if reachable(&shared.data, &full).is_err() {
            continue;
        }
        entries.push(json!({ "name": name, "path": full.display().to_string(), "git": full.join(".git").exists() }));
    }
    entries.sort_by_key(|e| e["name"].as_str().unwrap_or("").to_lowercase());
    entries.truncate(500);
    let parent = path
        .parent()
        .filter(|p| reachable(&shared.data, p).is_ok())
        .map(|p| p.display().to_string());
    Ok(json!({
        "path": path.display().to_string(),
        "parent": parent,
        "git": path.join(".git").exists(),
        "clone_dir": clone_dir(&shared.data).map(|p| p.display().to_string()),
        "entries": entries,
    }))
}

/// Only addresses a person would paste from a hosting site: https, ssh, or `user@host:path`.
/// No local path, no `file://`, no helper-program transport, nothing that reads as an option.
fn check_clone_url(url: &str) -> Result<(), Failure> {
    let bad =
        || Failure::bad("that is not a repository address (https://…, ssh://… or git@host:path)");
    if url.is_empty() || url.len() > 2048 || url.starts_with('-') {
        return Err(bad());
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let host_ok = |authority: &str| {
        let host = authority.rsplit('@').next().unwrap_or("");
        host.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c))
    };
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        let authority = rest.split('/').next().unwrap_or("");
        return if host_ok(authority) && rest.len() > authority.len() + 1 {
            Ok(())
        } else {
            Err(bad())
        };
    }
    if url.contains("://") {
        return Err(bad());
    }
    match url.split_once(':') {
        Some((user_host, path))
            if user_host.contains('@') && host_ok(user_host) && !path.is_empty() =>
        {
            Ok(())
        }
        _ => Err(bad()),
    }
}

fn clone_name(url: &str) -> String {
    let last = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

const CLONE_SECONDS: u64 = 15 * 60;

/// `git clone` into a new folder of the clone folder, then the folder becomes a project.
fn clone_project(shared: &Shared, args: &Value) -> Result<Value, Failure> {
    let url = text(args, "url")?;
    let url = url.trim();
    check_clone_url(url)?;
    let Some(dir) = clone_dir(&shared.data) else {
        return Err(Failure::bad(
            "choose the clone folder in Settings → Orochi mobile app on the desktop first",
        ));
    };
    let name = maybe(args, "name")
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| clone_name(url));
    let plain = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    if name.is_empty() || name.len() > 100 || name.starts_with('.') || !name.chars().all(plain) {
        return Err(Failure::bad(
            "the folder name may use letters, digits, `.`, `_` and `-` only",
        ));
    }
    let dest = dir.join(&name);
    if dest.symlink_metadata().is_ok() {
        return Err(Failure::bad(format!(
            "a folder named {name} already exists in the clone folder"
        )));
    }
    let mut command = std::process::Command::new("git");
    command
        .args([
            "-c",
            "protocol.ext.allow=never",
            "clone",
            "--quiet",
            "--",
            url,
        ])
        .arg(&dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = command
        .spawn()
        .map_err(|e| Failure::bad(format!("git could not start: {e}")))?;
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| anyhow!(e))? {
            Some(status) => break Some(status),
            None if started.elapsed().as_secs() > CLONE_SECONDS => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    };
    let mut reason = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        reason = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(300)
            .collect();
    }
    if !status.is_some_and(|s| s.success()) {
        // The folder did not exist before, so whatever is there is this attempt's leftovers.
        let _ = std::fs::remove_dir_all(&dest);
        return Err(Failure::bad(match status {
            None => "the clone took too long and was stopped".to_owned(),
            Some(_) if reason.is_empty() => "git could not clone that repository".to_owned(),
            Some(_) => reason,
        }));
    }
    let root = dest.canonicalize().map_err(|e| anyhow!(e))?;
    let id = locked(&shared.client).add_project(&root)?;
    Ok(json!({ "id": id, "root": root.display().to_string(), "name": name }))
}

fn reply(reply: crate::media::Reply) -> Response {
    let mut response = Response::builder().status(reply.status);
    for (name, value) in &reply.headers {
        response = response.header(*name, value);
    }
    response
        .body(axum::body::Body::from(reply.body))
        .unwrap_or_else(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "no reply"))
}

fn range(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn media(
    State(app): State<App>,
    UrlPath((thread, sha)): UrlPath<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if authenticate(&app, &headers, Some(&query)).is_none() {
        return unauthorized();
    }
    let data = app.shared.data.clone();
    let range = range(&headers);
    match blocking(move || {
        crate::media::serve(&data, &format!("/{thread}/{sha}"), range.as_deref())
    })
    .await
    {
        Ok(answer) => reply(answer),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "no reply"),
    }
}

fn serve_file(
    shared: &Shared,
    thread: &str,
    relative: &str,
    range: Option<&str>,
) -> crate::media::Reply {
    use crate::media::plain_reply;
    let cwd = match locked(&shared.client).cwd(thread) {
        Ok(cwd) => cwd,
        Err(_) => return plain_reply(404, "not found"),
    };
    let Ok(full) = crate::files::within(&cwd, relative) else {
        return plain_reply(403, "outside this thread's folder");
    };
    let Ok(mut file) = std::fs::File::open(&full) else {
        return plain_reply(404, "not found");
    };
    let Ok(meta) = file.metadata() else {
        return plain_reply(404, "not found");
    };
    if !meta.is_file() {
        return plain_reply(404, "not found");
    }
    if meta.len() > FILE_LIMIT {
        return plain_reply(413, "too large");
    }
    let ext = full
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut mime = crate::files::mime_for_extension(&ext).to_owned();
    if mime == "application/octet-stream" {
        let mut head = [0u8; 2048];
        let read = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
        let _ = std::io::Seek::rewind(&mut file);
        if let Some(kind) = orochi::media::sniff(&head[..read]) {
            mime = kind.mime.to_string();
        } else if !head[..read].contains(&0) && std::str::from_utf8(&head[..read]).is_ok() {
            mime = "text/plain; charset=utf-8".into();
        }
    }
    let headers = vec![
        ("Content-Type", mime),
        ("Accept-Ranges", "bytes".into()),
        ("X-Content-Type-Options", "nosniff".into()),
        (
            "Content-Security-Policy",
            "default-src 'none'; sandbox".into(),
        ),
        ("Cache-Control", "private, no-store".into()),
    ];
    crate::media::ranged(file, meta.len(), headers, range)
}

async fn file(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if authenticate(&app, &headers, Some(&query)).is_none() {
        return unauthorized();
    }
    let (Some(thread), Some(path)) = (query.get("thread").cloned(), query.get("path").cloned())
    else {
        return failure(StatusCode::BAD_REQUEST, "`thread` and `path` are needed");
    };
    let shared = app.shared.clone();
    let range = range(&headers);
    match blocking(move || serve_file(&shared, &thread, &path, range.as_deref())).await {
        Ok(answer) => reply(answer),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "no reply"),
    }
}

async fn events(
    State(app): State<App>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let device = match authenticate(&app, &headers, Some(&query)) {
        Some(device) => device,
        None => return unauthorized(),
    };
    upgrade.on_upgrade(move |socket| push(app, device.id, socket))
}

async fn push(app: App, device: String, mut socket: WebSocket) {
    let mut feed = app.shared.events.subscribe();
    let mut stop = app.stop.clone();
    let mut check = tokio::time::interval(Duration::from_secs(5));
    let hello = json!({"type": "hello"}).to_string();
    if socket
        .send(Message::Text(hello.clone().into()))
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            event = feed.recv() => {
                let line = match event {
                    Ok(line) => line,
                    // A slow phone missed some: it is told to read everything again.
                    Err(broadcast::error::RecvError::Lagged(_)) => hello.clone(),
                    Err(_) => break,
                };
                if socket.send(Message::Text(line.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = check.tick() => {
                // A revoked phone is cut off, not left listening.
                let mut devices = locked(&app.shared.devices);
                devices.refresh();
                if !devices.list.iter().any(|d| d.id == device) {
                    break;
                }
            }
            _ = stop.changed() => break,
        }
    }
}
