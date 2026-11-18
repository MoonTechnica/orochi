//! Servo, as the engine behind the person's own tab (`docs/servo-browser-design.md` §3).
//!
//! One `orochi-servo` process per tab, started on first use and ended with the tab, the broker
//! or the person: Servo is a 0.x engine parsing hostile pages, so a crash or a runaway page
//! ends one tab and nothing else. The broker speaks to it in lines of JSON on the child's
//! stdin and stdout (§3.2), and keeps only the newest frame and the tab's state, which is what
//! `view_frame` answers from, in the shape the Chromium path already answers in — the window
//! and the phone cannot tell which engine drew the page.
//!
//! An agent session can hold a tab here too (`tools.rs`): it gets an engine of its own with no
//! profile, so nothing it stores outlives the session and nothing of the person's is in it.
mod logins;
mod tools;

use super::policy::Policy;
use crate::config::{BrowserConfig, BrowserEngine};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub const BINARY: &str = "orochi-servo";
/// Engines running at once, across every conversation.
const MAX_TABS: usize = 12;
/// One frame may not be larger than this; a bigger length is a broken engine, not a picture.
const MAX_FRAME: usize = 16 * 1024 * 1024;
const CONSOLE_LINES: usize = 200;
const TEXT_LIMIT: usize = 20_000;

/// The `orochi-servo` to run: the one the settings name, else the one `setup` or `mise run
/// servo` put in the data directory, else one beside this executable.
pub fn find_servo(config: &BrowserConfig, data: &Path) -> Option<PathBuf> {
    if let Some(path) = &config.servo {
        return path.is_file().then(|| path.clone());
    }
    let mut places = vec![data.join("toolchain/servo/current").join(BINARY)];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        places.push(dir.join(BINARY));
    }
    places.into_iter().find(|p| p.is_file())
}

struct Frame {
    seq: u64,
    jpeg: String,
    width: u64,
    height: u64,
}

#[derive(Default)]
struct State {
    url: String,
    title: String,
    back: bool,
    forward: bool,
    loading: bool,
    cursor: String,
    dialog: Option<(String, String)>,
    frame: Option<Frame>,
    /// The broker's own sequence, so a restarted engine (whose own count starts again) never
    /// makes the window wait for a number it has already drawn.
    seq: u64,
    crashed: bool,
    ended: bool,
    ready: bool,
    refused: Option<String>,
    popup: Option<String>,
    console: VecDeque<String>,
    /// Lines the console has ever written, so a cursor still means the same line after the
    /// oldest ones were dropped.
    console_total: u64,
    next_id: u64,
    evals: HashMap<u64, tokio::sync::oneshot::Sender<std::result::Result<Value, String>>>,
    shots: HashMap<u64, tokio::sync::oneshot::Sender<Vec<u8>>>,
    exports: HashMap<u64, tokio::sync::oneshot::Sender<Vec<Value>>>,
    /// Whether the engine has been told to draw frames: an agent's engine draws none until
    /// the window looks.
    frames_on: bool,
    /// The last time the window asked for a frame of this agent's page.
    watched: Option<Instant>,
    /// The agent asked for the window to show its page.
    shown: bool,
    /// The agent has handed the page to the person: why, and since when.
    handoff: Option<(String, Instant)>,
}

struct Tab {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    state: Arc<Mutex<State>>,
    child: tokio::sync::Mutex<tokio::process::Child>,
    /// Woken when the person gives a handed-over page back.
    returned: tokio::sync::Notify,
    logins: Option<Arc<logins::Logins>>,
    /// The cookies this engine was given, so only what it changed is handed back.
    given: Mutex<HashMap<String, String>>,
}

/// How long a page may stay with the person before the agent is told nobody came.
const HANDOFF_LIMIT: Duration = Duration::from_secs(30 * 60);
/// A window that has not asked for a frame this long is no longer looking.
const WATCH_IDLE: Duration = Duration::from_secs(5);

pub struct Engines {
    data: PathBuf,
    config: BrowserConfig,
    binary: Option<PathBuf>,
    policy: Policy,
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    logins: Option<Arc<logins::Logins>>,
    /// The last frame number each tab drew, so a tab started again under the same name goes
    /// on counting from there: the window asks for frames newer than the last it drew.
    drawn: Mutex<HashMap<String, u64>>,
}

fn text_of(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .take(TEXT_LIMIT)
        .collect()
}

impl Engines {
    pub fn new(config: &BrowserConfig, data: &Path) -> Self {
        Self {
            data: data.to_path_buf(),
            config: config.clone(),
            binary: find_servo(config, data),
            policy: Policy::new(config),
            tabs: Mutex::default(),
            logins: config.share_logins.then(|| {
                Arc::new(logins::Logins::new(
                    data.join("browser").join("servo").join("logins.json"),
                ))
            }),
            drawn: Mutex::default(),
        }
    }

    /// Servo can draw a tab: there is a binary, and the settings do not ask for Chrome.
    pub fn available(&self) -> bool {
        self.config.engine != BrowserEngine::Chromium && self.binary.is_some()
    }

    /// The settings insist on Servo, so a missing binary is an error and not a reason to use
    /// Chrome instead.
    pub fn required(&self) -> bool {
        self.config.engine == BrowserEngine::Servo
    }

    pub fn why_missing(&self) -> String {
        format!(
            "browser.engine is \"servo\" but no {BINARY} was found: build one with `mise run \
             servo`, or name it in browser.servo"
        )
    }

    pub fn owns(&self, scope: &str) -> bool {
        self.tabs.lock().unwrap().contains_key(scope)
    }

    pub async fn forget(&self, scope: &str) {
        let tab = self.tabs.lock().unwrap().remove(scope);
        if let Some(tab) = tab {
            self.drawn
                .lock()
                .unwrap()
                .insert(scope.to_owned(), tab.state.lock().unwrap().seq);
            tab.end().await;
        }
    }

    /// Ends every engine the way a tab's close does: asked to, and waited for, because the
    /// profile (cookies, storage) is written only when the engine exits on its own.
    pub async fn shutdown_all(&self) {
        let tabs: Vec<(String, Arc<Tab>)> = self.tabs.lock().unwrap().drain().collect();
        for (scope, tab) in &tabs {
            self.drawn
                .lock()
                .unwrap()
                .insert(scope.clone(), tab.state.lock().unwrap().seq);
        }
        futures::future::join_all(tabs.iter().map(|(_, tab)| tab.end())).await;
    }

    /// The tabs this engine holds, for the list a phone is offered.
    pub fn list(&self) -> Vec<Value> {
        self.tabs
            .lock()
            .unwrap()
            .iter()
            .filter(|(scope, _)| scope.starts_with("user:"))
            .filter_map(|(scope, tab)| {
                let state = tab.state.lock().unwrap();
                (!state.ended).then(|| {
                    json!({"scope": scope, "title": state.title, "url": state.url,
                           "engine": "servo"})
                })
            })
            .collect()
    }

    async fn ensure(&self, scope: &str) -> Result<Arc<Tab>> {
        {
            let mut tabs = self.tabs.lock().unwrap();
            if let Some(tab) = tabs.get(scope) {
                if !tab.state.lock().unwrap().ended {
                    return Ok(tab.clone());
                }
                if let Some(old) = tabs.remove(scope) {
                    self.drawn
                        .lock()
                        .unwrap()
                        .insert(scope.to_owned(), old.state.lock().unwrap().seq);
                }
            }
            tabs.retain(|_, t| !t.state.lock().unwrap().ended);
            if tabs.len() >= MAX_TABS {
                bail!("that is as many browser tabs as this machine runs at once ({MAX_TABS})");
            }
        }
        let binary = self.binary.as_ref().with_context(|| self.why_missing())?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data.join("browser").join("servo.log"))?;
        let mut command = tokio::process::Command::new(binary);
        if scope.starts_with("user:") {
            let profile = self.data.join("browser").join("servo").join("profile");
            std::fs::create_dir_all(&profile)?;
            command.arg("--profile").arg(&profile);
        } else {
            // An agent's engine has no profile, so what it stores ends with it, and no frames
            // are drawn for a window that is not looking.
            command.args(["--frames", "0"]);
            let name: String = scope
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let home = self
                .data
                .join("browser")
                .join("servo")
                .join("agents")
                .join(name);
            std::fs::create_dir_all(&home)?;
            command.current_dir(home);
        }
        command
            .args(["--width", "1024", "--height", "768", "--quality", "70"])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .kill_on_drop(true);
        // A page's engine inherits nothing of the person's environment but what it needs to
        // find fonts and a temporary directory.
        for name in ["HOME", "TMPDIR", "LANG", "LC_ALL", "PATH", "USER"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("{} could not be started", binary.display()))?;
        let stdin = child.stdin.take().context("the engine has no input")?;
        let stdout = child.stdout.take().context("the engine has no output")?;
        let state = Arc::new(Mutex::new(State {
            seq: self.drawn.lock().unwrap().get(scope).copied().unwrap_or(0),
            ..State::default()
        }));
        tokio::spawn(read_events(stdout, state.clone()));
        let tab = Arc::new(Tab {
            stdin: tokio::sync::Mutex::new(stdin),
            state,
            child: tokio::sync::Mutex::new(child),
            returned: tokio::sync::Notify::new(),
            logins: self.logins.clone(),
            given: Mutex::default(),
        });
        self.tabs
            .lock()
            .unwrap()
            .insert(scope.to_owned(), tab.clone());
        if let Some(logins) = &self.logins {
            let items = logins.load();
            if !items.is_empty() {
                tab.given.lock().unwrap().extend(items.iter().map(|i| {
                    (
                        logins::key(i),
                        i["cookie"].as_str().unwrap_or("").to_owned(),
                    )
                }));
                // Sent before anything else is, so the first page already has them.
                let _ = tab
                    .send(&json!({"cmd": "cookies_import", "items": items}))
                    .await;
            }
        }
        Ok(tab)
    }

    /// One `view_*` call, answered as the Chromium path answers it.
    pub async fn view(&self, scope: &str, tool: &str, args: &Value) -> Result<Value> {
        let number = |key: &str| args[key].as_f64();
        match tool {
            "view_open" | "view_navigate" => {
                let tab = self.ensure(scope).await?;
                let url = args["url"].as_str().filter(|u| !u.is_empty());
                if let Some(url) = url {
                    Policy::web(url).map_err(|r| anyhow::anyhow!("{r}"))?;
                    tab.state.lock().unwrap().refused = None;
                    tab.send(&json!({"cmd": "navigate", "url": url})).await?;
                }
            }
            "view_back" | "view_forward" | "view_reload" | "view_stop" => {
                let tab = self.ensure(scope).await?;
                let cmd = tool.trim_start_matches("view_");
                tab.send(&json!({ "cmd": cmd })).await?;
            }
            "view_resize" => {
                let tab = self.ensure(scope).await?;
                let (width, height) = (
                    number("width").unwrap_or(1024.0).clamp(100.0, 4000.0),
                    number("height").unwrap_or(768.0).clamp(100.0, 4000.0),
                );
                let ratio = number("ratio").unwrap_or(1.0).clamp(1.0, 3.0);
                // A new size is a new picture: nothing of the old size outlasts the answer.
                tab.state.lock().unwrap().frame = None;
                tab.send(&json!({"cmd": "resize", "width": width.round(),
                                 "height": height.round(), "ratio": ratio}))
                    .await?;
            }
            "view_frame" => {
                let tab = self.ensure(scope).await?;
                let since = args["since"].as_u64().unwrap_or(0);
                let state = tab.state.lock().unwrap();
                return Ok(frame_reply(&state, since));
            }
            "view_input" => {
                let tab = self.ensure(scope).await?;
                let kind = args["type"].as_str().unwrap_or("");
                anyhow::ensure!(
                    ["mouse", "key", "text", "ime"].contains(&kind),
                    "not an input: {kind}"
                );
                let mut command = args.clone();
                if let Some(text) = command["text"].as_str() {
                    command["text"] = json!(text.chars().take(TEXT_LIMIT).collect::<String>());
                }
                command["cmd"] = json!("input");
                tab.send(&command).await?;
            }
            "view_dialog" => {
                let tab = self.ensure(scope).await?;
                tab.send(&json!({"cmd": "dialog",
                                 "accept": args["accept"].as_bool().unwrap_or(false),
                                 "text": text_of(&args["text"])}))
                    .await?;
                tab.state.lock().unwrap().dialog = None;
            }
            "view_close" => {
                self.forget(scope).await;
                return Ok(json!({"closed": true}));
            }
            other => bail!("{other} is not something this tab does"),
        }
        // The same acknowledgement the Chromium path gives: the tab's state, with no frame.
        let tab = self.ensure(scope).await?;
        let state = tab.state.lock().unwrap();
        Ok(json!({"url": state.url, "title": state.title, "engine": "servo"}))
    }

    /// The agents that have a page here, for the window to offer: what a tab strip shows, and
    /// whether the agent asked to be seen or to hand the page over.
    pub async fn agents(&self) -> Vec<Value> {
        let tabs: Vec<(String, Arc<Tab>)> = self
            .tabs
            .lock()
            .unwrap()
            .iter()
            .filter(|(scope, _)| !scope.starts_with("user:"))
            .map(|(scope, tab)| (scope.clone(), tab.clone()))
            .collect();
        let mut agents = vec![];
        for (scope, tab) in tabs {
            tab.stop_unwatched().await;
            let state = tab.state.lock().unwrap();
            if state.ended || !state.ready {
                continue;
            }
            agents.push(json!({
                "scope": scope,
                "label": scope.rsplit_once('-').map_or(scope.as_str(), |(label, _)| label),
                "title": state.title, "url": state.url, "tabs": 1, "engine": "servo",
                "shown": state.shown,
                "handoff": state.handoff.as_ref().map(|(reason, _)| reason),
            }));
        }
        agents.sort_by(|a, b| a["scope"].as_str().cmp(&b["scope"].as_str()));
        agents
    }

    /// A `view_*` call about an agent's page. The window may look at it and, only while the
    /// agent has handed it over, drive it.
    pub async fn watch(&self, agent: &str, tool: &str, args: &Value) -> Result<Value> {
        let tab = self
            .tabs
            .lock()
            .unwrap()
            .get(agent)
            .cloned()
            .context("that agent has no page open")?;
        match tool {
            "view_frame" => {
                tab.state.lock().unwrap().watched = Some(Instant::now());
                tab.frames(true).await?;
                let since = args["since"].as_u64().unwrap_or(0);
                let state = tab.state.lock().unwrap();
                let mut reply = frame_reply(&state, since);
                reply["handoff"] = json!(state.handoff.as_ref().map(|(reason, _)| reason));
                Ok(reply)
            }
            "view_unwatch" => {
                tab.state.lock().unwrap().watched = None;
                tab.frames(false).await?;
                Ok(json!({"unwatched": true}))
            }
            "view_input" | "view_dialog" => {
                anyhow::ensure!(
                    tab.state.lock().unwrap().handoff.is_some(),
                    "that is the agent's page: it is watched, not driven, until it hands it over"
                );
                if tool == "view_input" {
                    let kind = args["type"].as_str().unwrap_or("");
                    anyhow::ensure!(
                        ["mouse", "key", "text", "ime"].contains(&kind),
                        "not an input: {kind}"
                    );
                    let mut command = args.clone();
                    if let Some(text) = command["text"].as_str() {
                        command["text"] = json!(text.chars().take(TEXT_LIMIT).collect::<String>());
                    }
                    command["cmd"] = json!("input");
                    tab.send(&command).await?;
                } else {
                    tab.send(&json!({"cmd": "dialog",
                                     "accept": args["accept"].as_bool().unwrap_or(false),
                                     "text": text_of(&args["text"])}))
                        .await?;
                    tab.state.lock().unwrap().dialog = None;
                }
                Ok(json!({"ok": true}))
            }
            "view_handoff_done" => {
                // What the person signed in with is kept before the agent goes on.
                tab.save_logins().await;
                let had = tab.state.lock().unwrap().handoff.take().is_some();
                tab.returned.notify_waiters();
                Ok(json!({"returned": had}))
            }
            other => bail!("{other} is not something the window does to an agent's page"),
        }
    }

    /// What the engine has written to its page's console since `since`, newest last.
    pub fn console(&self, scope: &str) -> Vec<String> {
        self.tabs
            .lock()
            .unwrap()
            .get(scope)
            .map_or_else(Vec::new, |tab| {
                tab.state.lock().unwrap().console.iter().cloned().collect()
            })
    }

    /// Waits until the engine says it is up, so the first command is not lost to a start.
    pub async fn warm(&self, scope: &str) -> Result<()> {
        let tab = self.ensure(scope).await?;
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            {
                let state = tab.state.lock().unwrap();
                if state.ready {
                    return Ok(());
                }
                if state.ended {
                    bail!("the engine ended while starting; see browser/servo.log");
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("the engine did not start within 30 seconds")
    }
}

fn frame_reply(state: &State, since: u64) -> Value {
    let mut reply = json!({"url": state.url, "title": state.title,
                           "engine": "servo", "loading": state.loading,
                           "back": state.back, "forward": state.forward});
    if !state.cursor.is_empty() {
        reply["cursor"] = json!(state.cursor);
    }
    if state.crashed || state.ended {
        reply["crashed"] = json!(true);
    }
    if let Some(url) = &state.refused {
        reply["refused"] = json!(url);
    }
    if let Some(url) = &state.popup {
        reply["popup"] = json!(url);
    }
    if let Some((kind, message)) = &state.dialog {
        reply["dialog"] = json!({"type": kind, "message": message});
    }
    if let Some(frame) = state.frame.as_ref().filter(|f| f.seq > since) {
        reply["frame"] = json!({"seq": frame.seq, "jpeg": frame.jpeg,
                                "width": frame.width, "height": frame.height});
    }
    reply
}

impl Tab {
    /// Starts or stops the engine's frames, once per change.
    async fn frames(&self, on: bool) -> Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            if state.frames_on == on {
                return Ok(());
            }
            state.frames_on = on;
        }
        self.send(&json!({"cmd": "frames", "on": on})).await
    }

    /// Frames cost an agent's engine encoding it would not otherwise do, so a window that
    /// stopped asking (closed, crashed) does not leave them running.
    async fn stop_unwatched(&self) {
        let idle = {
            let state = self.state.lock().unwrap();
            state.frames_on && state.watched.is_none_or(|at| at.elapsed() > WATCH_IDLE)
        };
        if idle {
            let _ = self.frames(false).await;
        }
    }

    /// The engine's cookies, as `{url, cookie}` items.
    async fn export_cookies(&self) -> Result<Vec<Value>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let id = {
            let mut state = self.state.lock().unwrap();
            state.next_id += 1;
            let id = state.next_id;
            state.exports.insert(id, sender);
            id
        };
        self.send(&json!({"cmd": "cookies_export", "id": id}))
            .await?;
        match tokio::time::timeout(Duration::from_secs(5), receiver).await {
            Ok(Ok(items)) => Ok(items),
            Ok(Err(_)) => bail!("the tab's engine has ended"),
            Err(_) => {
                self.state.lock().unwrap().exports.remove(&id);
                bail!("the engine did not answer in 5 seconds")
            }
        }
    }

    /// Hands what this engine changed in its cookies to the shared file.
    async fn save_logins(&self) {
        let Some(logins) = &self.logins else {
            return;
        };
        {
            let state = self.state.lock().unwrap();
            if state.ended || !state.ready {
                return;
            }
        }
        let Ok(items) = self.export_cookies().await else {
            return;
        };
        let mut given = self.given.lock().unwrap();
        let changed: Vec<Value> = items
            .into_iter()
            .filter(|item| {
                given.get(&logins::key(item)).map(String::as_str) != item["cookie"].as_str()
            })
            .collect();
        for item in &changed {
            given.insert(
                logins::key(item),
                item["cookie"].as_str().unwrap_or("").to_owned(),
            );
        }
        drop(given);
        logins.merge(&changed);
    }

    /// Asks the engine to exit and waits for it, killing it only if it will not.
    async fn end(&self) {
        self.save_logins().await;
        let mut line = br#"{"cmd":"shutdown"}"#.to_vec();
        line.push(b'\n');
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.write_all(&line).await;
            let _ = stdin.flush().await;
        }
        let mut child = self.child.lock().await;
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }

    /// Runs a script in the page and returns what it evaluated to.
    async fn eval(&self, script: &str) -> Result<Value> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let id = {
            let mut state = self.state.lock().unwrap();
            state.next_id += 1;
            let id = state.next_id;
            state.evals.insert(id, sender);
            id
        };
        self.send(&json!({"cmd": "eval", "id": id, "script": script}))
            .await?;
        match tokio::time::timeout(Duration::from_secs(20), receiver).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => bail!("the script threw: {error}"),
            Ok(Err(_)) => bail!("the tab's engine has ended"),
            Err(_) => {
                self.state.lock().unwrap().evals.remove(&id);
                bail!("the page did not answer a script in 20 seconds")
            }
        }
    }

    /// A PNG of the page, or of the clip `[x, y, width, height]`.
    async fn screenshot(&self, clip: Option<[f64; 4]>) -> Result<Vec<u8>> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let id = {
            let mut state = self.state.lock().unwrap();
            state.next_id += 1;
            let id = state.next_id;
            state.shots.insert(id, sender);
            id
        };
        let mut command = json!({"cmd": "screenshot", "id": id});
        if let Some([x, y, width, height]) = clip {
            command["x"] = json!(x);
            command["y"] = json!(y);
            command["width"] = json!(width);
            command["height"] = json!(height);
        }
        self.send(&command).await?;
        match tokio::time::timeout(Duration::from_secs(20), receiver).await {
            Ok(Ok(bytes)) if !bytes.is_empty() => Ok(bytes),
            Ok(Ok(_)) => bail!("the engine could not take a picture"),
            Ok(Err(_)) => bail!("the tab's engine has ended"),
            Err(_) => {
                self.state.lock().unwrap().shots.remove(&id);
                bail!("the engine did not answer in 20 seconds")
            }
        }
    }

    async fn send(&self, command: &Value) -> Result<()> {
        // The engine says `ready` once its first page has loaded, and a command sent before
        // that is dropped (measured with the real engine), so nothing is sent until then.
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            {
                let state = self.state.lock().unwrap();
                if state.ended {
                    bail!("the tab's engine has ended");
                }
                if state.ready {
                    break;
                }
            }
            if Instant::now() >= end {
                bail!("the tab's engine did not start within 30 seconds");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut line = serde_json::to_vec(command)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|_| anyhow::anyhow!("the tab's engine has ended"))?;
        stdin.flush().await.ok();
        Ok(())
    }
}

async fn read_events(stdout: tokio::process::ChildStdout, state: Arc<Mutex<State>>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(event) = serde_json::from_str::<Value>(line.trim_end()) else {
            continue;
        };
        if event["event"] == "frame" {
            let len = event["len"].as_u64().unwrap_or(0) as usize;
            if len == 0 || len > MAX_FRAME {
                break;
            }
            let mut bytes = vec![0u8; len];
            if reader.read_exact(&mut bytes).await.is_err() {
                break;
            }
            let mut state = state.lock().unwrap();
            state.seq += 1;
            let seq = state.seq;
            state.frame = Some(Frame {
                seq,
                jpeg: base64::engine::general_purpose::STANDARD.encode(&bytes),
                width: event["width"].as_u64().unwrap_or(0),
                height: event["height"].as_u64().unwrap_or(0),
            });
            continue;
        }
        if event["event"] == "screenshot" {
            let len = event["len"].as_u64().unwrap_or(0) as usize;
            if len > MAX_FRAME {
                break;
            }
            let mut bytes = vec![0u8; len];
            if len > 0 && reader.read_exact(&mut bytes).await.is_err() {
                break;
            }
            let id = event["id"].as_u64().unwrap_or(0);
            if let Some(waiting) = state.lock().unwrap().shots.remove(&id) {
                let _ = waiting.send(bytes);
            }
            continue;
        }
        let mut state = state.lock().unwrap();
        match event["event"].as_str().unwrap_or("") {
            "eval" => {
                let id = event["id"].as_u64().unwrap_or(0);
                if let Some(waiting) = state.evals.remove(&id) {
                    let _ = waiting.send(if event["ok"].as_bool().unwrap_or(false) {
                        Ok(event["value"].clone())
                    } else {
                        Err(text_of(&event["error"]))
                    });
                }
            }
            "cookies" => {
                let id = event["id"].as_u64().unwrap_or(0);
                if let Some(waiting) = state.exports.remove(&id) {
                    let _ = waiting.send(event["items"].as_array().cloned().unwrap_or_default());
                }
            }
            "ready" => state.ready = true,
            "url" => state.url = text_of(&event["url"]),
            "title" => state.title = text_of(&event["title"]),
            "history" => {
                state.back = event["back"].as_bool().unwrap_or(false);
                state.forward = event["forward"].as_bool().unwrap_or(false);
            }
            "loading" => state.loading = event["state"] == "started",
            "cursor" => state.cursor = text_of(&event["cursor"]),
            "dialog" => {
                state.dialog = Some((text_of(&event["kind"]), text_of(&event["message"])));
            }
            "dialog_closed" => state.dialog = None,
            "popup" => state.popup = Some(text_of(&event["url"])),
            "refused" => state.refused = Some(text_of(&event["url"])),
            "crashed" => state.crashed = true,
            "console" => {
                state.console_total += 1;
                let line = format!("{}: {}", text_of(&event["level"]), text_of(&event["text"]));
                state.console.push_back(line);
                while state.console.len() > CONSOLE_LINES {
                    state.console.pop_front();
                }
            }
            _ => {}
        }
    }
    state.lock().unwrap().ended = true;
}
