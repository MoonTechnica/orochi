#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The shell. It owns a window and a connection, and does nothing else: every command below
//! is one call into `view`, which is where the app's behavior lives and where it is tested.
use orochi::activity::{ProjectRow, Thread};
use orochi_desktop::view::{AgentRow, Client, Folder, Prompt, Remembered, RouteStats, Said};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};

/// The one connection this window reads through. `Client` is not `Sync` (a SQLite connection
/// is not), so the window holds it behind a mutex and never across an await.
struct Open(Arc<Mutex<Client>>);

/// The shells in the window's terminal tabs.
struct Shells(orochi_desktop::term::Terminals);

/// The person's browser tabs: one connection to the broker per conversation, set up once.
#[derive(Default)]
struct Views {
    ready: Mutex<bool>,
    #[cfg(unix)]
    clients:
        Mutex<std::collections::HashMap<String, std::sync::Arc<Mutex<orochi::browser::Client>>>>,
}

fn fail(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// The folders a thread can be started in, and starting one. Choosing where the work happens
/// is the one thing a window must ask before anything else can be done.
#[tauri::command]
fn folders(open: State<'_, Open>) -> Result<Vec<Folder>, String> {
    open.0.lock().unwrap().folders().map_err(fail)
}

/// Adds a folder as a project, with no thread in it yet.
#[tauri::command]
fn add_project(open: State<'_, Open>, root: String) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .add_project(std::path::Path::new(&root))
        .map_err(fail)
}

#[tauri::command]
fn new_thread(
    open: State<'_, Open>,
    root: String,
    worktree: Option<bool>,
    sandbox: Option<String>,
) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .new_thread(
            std::path::Path::new(&root),
            worktree.unwrap_or(false),
            sandbox.as_deref(),
        )
        .map_err(fail)
}

#[cfg(windows)]
const OROCHI_SIDECAR: &str = "orochi.exe";
#[cfg(not(windows))]
const OROCHI_SIDECAR: &str = "orochi";

/// The `orochi` this window starts: `OROCHI_BIN`, then the one shipped beside this window,
/// then the one the user would run. A window opened from Finder has no shell `PATH`, so a
/// sibling is what makes a packaged app work at all.
fn orochi() -> std::path::PathBuf {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(OROCHI_SIDECAR)))
        .filter(|path| path.is_file());
    std::env::var_os("OROCHI_BIN")
        .map(std::path::PathBuf::from)
        .or(beside)
        .unwrap_or_else(|| OROCHI_SIDECAR.into())
}

/// The Sandboxes screen. It asks Incus through the VM, so it is read off the connection.
#[tauri::command]
async fn sandboxes(open: State<'_, Open>) -> Result<orochi_desktop::view::Sandboxes, String> {
    let (config, data) = open.0.lock().unwrap().paths();
    tauri::async_runtime::spawn_blocking(move || orochi_desktop::view::sandboxes(&config, &data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(fail)
}

/// The Setup screen: what is installed and signed in, and what a window should offer first.
#[tauri::command]
async fn setup_status(open: State<'_, Open>) -> Result<orochi_desktop::view::Setup, String> {
    let (config, data) = open.0.lock().unwrap().paths();
    tauri::async_runtime::spawn_blocking(move || orochi_desktop::view::setup(&config, &data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(fail)
}

/// Stores the key a cloud decision model is reached with.
#[tauri::command]
fn decision_key(open: State<'_, Open>, provider: String, key: String) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    orochi_desktop::view::decision_key(&config, &data, &provider, &key).map_err(fail)
}

/// Opens Terminal on an agent's own sign-in command, with Orochi's toolchain in front.
#[tauri::command]
fn setup_login(open: State<'_, Open>, agent: String) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    let command =
        orochi_desktop::view::setup_login_command(&config, &data, &agent).map_err(fail)?;
    in_terminal(&command)
}

/// Stops a setup job: the whole process group, so nothing it started is left downloading.
#[tauri::command]
fn cancel_job(open: State<'_, Open>, id: String) -> Result<(), String> {
    let (_, data) = open.0.lock().unwrap().paths();
    orochi_desktop::view::cancel_job(&data, &id).map_err(fail)
}

/// Opens an agent's install instructions in the browser, from the Setup screen.
#[tauri::command]
fn open_docs(open: State<'_, Open>, url: String) -> Result<(), String> {
    let (config, _) = open.0.lock().unwrap().paths();
    orochi_desktop::view::open_docs(&config, &url).map_err(fail)
}

/// Stores an API key for agents in sandboxes; the value goes straight to the keychain.
#[tauri::command]
fn sandbox_secret(open: State<'_, Open>, name: String, value: String) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    orochi_desktop::view::sandbox_secret(&config, &data, &name, &value).map_err(fail)
}

/// Opens Terminal on `orochi sandbox login <agent>`: a browser sign-in and a token pasted
/// back, which only a terminal can take.
#[tauri::command]
fn sandbox_login(open: State<'_, Open>, agent: String) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    let command = orochi_desktop::view::sandbox_login_command(&orochi(), &config, &data, &agent)
        .map_err(fail)?;
    in_terminal(&command)
}

/// Runs one command in a new Terminal window.
fn in_terminal(command: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        command.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let status = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Terminal could not be opened".into())
    }
}

// The sidebar's context menus: a thread or a project renamed, pinned, archived, hidden or
// deleted is a row changed, and the window redraws from the view like anyone else.

#[tauri::command]
fn rename_thread(open: State<'_, Open>, thread: String, title: String) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .rename_thread(&thread, &title)
        .map_err(fail)
}

#[tauri::command]
fn pin_thread(open: State<'_, Open>, thread: String, pinned: bool) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .pin_thread(&thread, pinned)
        .map_err(fail)
}

#[tauri::command]
fn archive_thread(open: State<'_, Open>, thread: String, archived: bool) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .archive_thread(&thread, archived)
        .map_err(fail)
}

/// Deletes one conversation. The window asks first; this does it.
#[tauri::command]
fn delete_thread(open: State<'_, Open>, thread: String) -> Result<(), String> {
    open.0.lock().unwrap().delete_thread(&thread).map_err(fail)
}

#[tauri::command]
fn rename_project(open: State<'_, Open>, project: String, name: String) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .rename_project(&project, &name)
        .map_err(fail)
}

#[tauri::command]
fn pin_project(open: State<'_, Open>, project: String, pinned: bool) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .pin_project(&project, pinned)
        .map_err(fail)
}

#[tauri::command]
fn hide_project(open: State<'_, Open>, project: String, hidden: bool) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .hide_project(&project, hidden)
        .map_err(fail)
}

/// Deletes a project and every conversation in it. The window asks first; this does it.
#[tauri::command]
fn delete_project(open: State<'_, Open>, project: String) -> Result<usize, String> {
    open.0
        .lock()
        .unwrap()
        .delete_project(&project)
        .map_err(fail)
}

/// Shows a folder in the Finder.
#[tauri::command]
fn open_folder(open: State<'_, Open>, root: String) -> Result<(), String> {
    open.0.lock().unwrap().open_folder(&root).map_err(fail)
}

/// Opens Terminal on `orochi chat --thread <id>` in the thread's folder: the conversation
/// carries on there, as `/desktop` hands one the other way in Claude Code.
#[tauri::command]
fn open_terminal(open: State<'_, Open>, thread: String) -> Result<(), String> {
    let command = open
        .0
        .lock()
        .unwrap()
        .terminal_command(&orochi(), &thread)
        .map_err(fail)?;
    in_terminal(&command)
}

/// One sandbox operation, started in the background; the screen follows its output.
#[tauri::command]
fn sandbox_job(
    open: State<'_, Open>,
    request: orochi::sandbox::jobs::Request,
) -> Result<String, String> {
    let (config, data) = open.0.lock().unwrap().paths();
    orochi_desktop::view::sandbox_job(&orochi(), &config, &data, request).map_err(fail)
}

/// Where a folder's agents run, and whether a new project should be asked.
#[tauri::command]
fn placement(
    open: State<'_, Open>,
    root: String,
) -> Result<orochi_desktop::view::Placement, String> {
    open.0
        .lock()
        .unwrap()
        .placement(std::path::Path::new(&root))
        .map_err(fail)
}

/// Decides where a folder's agents run. Making a sandbox takes a while, so it runs off the
/// window's connection and the window stays responsive.
#[tauri::command]
async fn place(
    open: State<'_, Open>,
    root: String,
    mode: String,
    docker: bool,
) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    orochi_desktop::view::prepare_place(&config, &data, &mode)
        .await
        .map_err(fail)?;
    tauri::async_runtime::spawn_blocking(move || {
        orochi_desktop::view::place(&config, &data, std::path::Path::new(&root), &mode, docker)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(fail)
}

/// One look at a thread: has the agent running it gone, and is anything waiting to be run.
#[tauri::command]
fn watch(open: State<'_, Open>, thread: String) -> Result<orochi_desktop::view::Watch, String> {
    open.0.lock().unwrap().watch(&thread).map_err(fail)
}

/// Starts the thread's host if it has none, so a message sent here actually runs. A thread a
/// terminal is holding keeps its owner; this never takes one over.
#[tauri::command]
fn ensure_host(open: State<'_, Open>, thread: String) -> Result<bool, String> {
    let needed = open.0.lock().unwrap().needs_host(&thread).map_err(fail)?;
    if !needed {
        return Ok(false);
    }
    orochi_desktop::view::start_host(&orochi(), &thread).map_err(fail)?;
    Ok(true)
}

#[tauri::command]
fn room(open: State<'_, Open>, thread: String) -> Result<Vec<Said>, String> {
    open.0.lock().unwrap().room(&thread).map_err(fail)
}

/// What agents said to each other lately across every conversation, for the office.
#[tauri::command]
fn chatter(open: State<'_, Open>, since: i64) -> Result<Vec<orochi_desktop::view::Talk>, String> {
    open.0.lock().unwrap().chatter(since, 60).map_err(fail)
}

/// Leaves a note in the room. It reaches the agents when they next read their messages; with
/// a `member` it is a reply in that member's thread, and a mention reaches whoever it names.
#[tauri::command]
fn say(
    open: State<'_, Open>,
    thread: String,
    to: Option<String>,
    text: String,
    member: Option<String>,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .say_in(&thread, to.as_deref(), &text, member.as_deref())
        .map_err(fail)
}

/// The agents the lead works with in one conversation: a thread each in the channel.
#[tauri::command]
fn members(open: State<'_, Open>, thread: String) -> Result<Vec<orochi::activity::Member>, String> {
    open.0.lock().unwrap().members(&thread).map_err(fail)
}

/// What the agents have said to the user that they have not seen yet.
#[tauri::command]
fn mentions(open: State<'_, Open>) -> Result<Vec<orochi::activity::Mention>, String> {
    open.0.lock().unwrap().mentions().map_err(fail)
}

#[tauri::command]
fn see_mentions(open: State<'_, Open>, thread: String) -> Result<(), String> {
    open.0.lock().unwrap().see_mentions(&thread).map_err(fail)
}

#[tauri::command]
fn agents(open: State<'_, Open>) -> Result<Vec<AgentRow>, String> {
    open.0.lock().unwrap().agents().map_err(fail)
}

/// Reads again the quota of each account whose last reading has expired, so the status bar keeps
/// what each account has left (the console does the same before each run). A probe only reads what
/// the CLI shows; an account whose reading is still fresh is not asked again.
#[tauri::command]
async fn refresh_quota(open: State<'_, Open>) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    // The store is not `Send`, so the probes run on a blocking thread that owns it.
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let config = orochi::config::Config::load(&config)?;
        let store = orochi::storage::Store::open(&data)?;
        tauri::async_runtime::block_on(orochi::quota_sources::refresh_stale(&config, &store))?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(fail)
}

#[tauri::command]
fn insights(open: State<'_, Open>) -> Result<Vec<RouteStats>, String> {
    open.0.lock().unwrap().insights().map_err(fail)
}

#[tauri::command]
fn board(open: State<'_, Open>, thread: String) -> Result<Vec<serde_json::Value>, String> {
    open.0.lock().unwrap().board(&thread).map_err(fail)
}

#[tauri::command]
fn team(open: State<'_, Open>, thread: String) -> Result<orochi::activity::Team, String> {
    open.0.lock().unwrap().team(&thread).map_err(fail)
}

#[tauri::command]
fn tree_files(
    open: State<'_, Open>,
    thread: String,
    scope: String,
) -> Result<Vec<orochi::activity::FileRow>, String> {
    open.0
        .lock()
        .unwrap()
        .tree_files(&thread, &scope)
        .map_err(fail)
}

#[tauri::command]
fn git_overview(
    open: State<'_, Open>,
    thread: String,
) -> Result<orochi_desktop::view::GitOverview, String> {
    open.0.lock().unwrap().git_overview(&thread).map_err(fail)
}

#[tauri::command]
fn git_show(open: State<'_, Open>, thread: String, hash: String) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .git_show(&thread, &hash)
        .map_err(fail)
}

#[tauri::command]
fn tree_patch(
    open: State<'_, Open>,
    thread: String,
    scope: String,
    path: String,
) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .tree_patch(&thread, &scope, &path)
        .map_err(fail)
}

#[tauri::command]
fn tree_refresh(
    open: State<'_, Open>,
    thread: String,
) -> Result<orochi_desktop::files::TreeState, String> {
    open.0.lock().unwrap().tree_refresh(&thread).map_err(fail)
}

#[tauri::command]
fn tree_list(
    open: State<'_, Open>,
    thread: String,
    dir: String,
) -> Result<Vec<orochi_desktop::files::Entry>, String> {
    open.0
        .lock()
        .unwrap()
        .tree_list(&thread, &dir)
        .map_err(fail)
}

#[tauri::command]
fn tree_read(
    open: State<'_, Open>,
    thread: String,
    path: String,
) -> Result<orochi_desktop::files::FileText, String> {
    open.0
        .lock()
        .unwrap()
        .tree_read(&thread, &path)
        .map_err(fail)
}

#[tauri::command]
fn tree_media(
    open: State<'_, Open>,
    thread: String,
    path: String,
) -> Result<orochi_desktop::files::Media, String> {
    open.0
        .lock()
        .unwrap()
        .tree_media(&thread, &path)
        .map_err(fail)
}

#[tauri::command]
fn tree_find(
    open: State<'_, Open>,
    thread: String,
    query: String,
    by: String,
    limit: usize,
) -> Result<Option<Vec<orochi_desktop::files::Hit>>, String> {
    open.0
        .lock()
        .unwrap()
        .tree_find(&thread, &query, &by, limit)
        .map_err(fail)
}

#[tauri::command]
fn open_path(
    open: State<'_, Open>,
    thread: String,
    path: String,
    reveal: bool,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .open_path(&thread, &path, reveal)
        .map_err(fail)
}

/// Comments left on a diff, sent as the next message.
#[tauri::command]
fn comment(
    open: State<'_, Open>,
    thread: String,
    comments: Vec<(String, u64, String)>,
) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .comment(&thread, &comments)
        .map_err(fail)
}

#[tauri::command]
fn settings(open: State<'_, Open>) -> Result<serde_json::Value, String> {
    open.0.lock().unwrap().settings().map_err(fail)
}

#[tauri::command]
fn save_settings(open: State<'_, Open>, settings: serde_json::Value) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .save_settings(&settings)
        .map_err(fail)
}

#[tauri::command]
fn roster(open: State<'_, Open>) -> Result<orochi::roster::Listing, String> {
    open.0.lock().unwrap().roster().map_err(fail)
}

/// Where worktrees, a local decision model and the sandbox VM are kept. Reading the VM's
/// state runs `limactl`, so it is off the window's connection.
#[tauri::command]
async fn places(open: State<'_, Open>) -> Result<Vec<orochi::places::Place>, String> {
    let (config, data) = open.0.lock().unwrap().paths();
    tauri::async_runtime::spawn_blocking(move || orochi_desktop::view::places(&config, &data))
        .await
        .map_err(|e| e.to_string())?
        .map_err(fail)
}

#[tauri::command]
fn set_place(
    open: State<'_, Open>,
    which: String,
    path: Option<String>,
    force: bool,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .set_place(&which, path.as_deref(), force)
        .map_err(fail)
}

#[tauri::command]
fn set_roster(
    open: State<'_, Open>,
    agent: String,
    models: Vec<String>,
    off: bool,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .set_roster(&agent, &models, off)
        .map_err(fail)
}

#[tauri::command]
fn set_roster_note(
    open: State<'_, Open>,
    agent: String,
    model: String,
    text: String,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .set_roster_note(&agent, &model, &text)
        .map_err(fail)
}

/// Asks an agent for a roster and writes it down; a few seconds, off the window's connection.
#[tauri::command]
async fn suggest_roster(open: State<'_, Open>) -> Result<orochi::roster::Suggestion, String> {
    let (config, data) = open.0.lock().unwrap().paths();
    tauri::async_runtime::spawn_blocking(move || {
        orochi_desktop::view::suggest_roster(&orochi(), &config, &data)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(fail)
}

/// Starts every agent once so the roster has their models; tens of seconds, off the connection.
#[tauri::command]
async fn refresh_models(open: State<'_, Open>) -> Result<(), String> {
    let (config, data) = open.0.lock().unwrap().paths();
    tauri::async_runtime::spawn_blocking(move || {
        orochi_desktop::view::refresh_models(&orochi(), &config, &data)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(fail)
}

#[tauri::command]
fn memory(open: State<'_, Open>) -> Result<Remembered, String> {
    open.0.lock().unwrap().memory().map_err(fail)
}

#[tauri::command]
fn save_memory(open: State<'_, Open>, text: String) -> Result<(), String> {
    open.0.lock().unwrap().save_memory(&text).map_err(fail)
}

/// Deletes every conversation. The window asks first; this does it.
#[tauri::command]
fn forget_all(open: State<'_, Open>) -> Result<usize, String> {
    open.0.lock().unwrap().forget_all().map_err(fail)
}

#[tauri::command]
fn sidebar(open: State<'_, Open>, limit: usize, archived: bool) -> Result<Vec<ProjectRow>, String> {
    open.0
        .lock()
        .unwrap()
        .sidebar(limit, archived)
        .map_err(fail)
}

#[tauri::command]
fn thread(open: State<'_, Open>, id: String) -> Result<Option<Thread>, String> {
    open.0.lock().unwrap().thread(&id).map_err(fail)
}

#[tauri::command]
fn patch(open: State<'_, Open>, id: i64) -> Result<Option<String>, String> {
    open.0.lock().unwrap().patch(id).map_err(fail)
}

#[tauri::command]
fn open_prompts(open: State<'_, Open>) -> Result<Vec<Prompt>, String> {
    open.0.lock().unwrap().open_prompts().map_err(fail)
}

#[tauri::command]
fn answer(open: State<'_, Open>, prompt: String, option: Option<String>) -> Result<bool, String> {
    open.0
        .lock()
        .unwrap()
        .answer(&prompt, option.as_deref())
        .map_err(fail)
}

#[tauri::command]
fn send(open: State<'_, Open>, thread: String, text: String) -> Result<String, String> {
    open.0.lock().unwrap().send(&thread, &text).map_err(fail)
}

#[tauri::command]
fn interrupt(open: State<'_, Open>, thread: String) -> Result<(), String> {
    open.0.lock().unwrap().interrupt(&thread).map_err(fail)
}

#[tauri::command]
fn seen(open: State<'_, Open>, thread: String) -> Result<(), String> {
    open.0.lock().unwrap().seen(&thread).map_err(fail)
}

/// The threads something has happened in since the window last asked. The window polls this
/// and re-reads only what it names, rather than the whole conversation.
#[tauri::command]
fn changed(open: State<'_, Open>) -> Result<Vec<String>, String> {
    open.0
        .lock()
        .unwrap()
        .changed()
        .map(|threads| threads.into_iter().collect())
        .map_err(fail)
}

/// A notification from the system, the window's way of saying something while it is not in
/// front. Sent from here rather than from the page, so the page needs no permission of its own.
#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: String, sound: bool) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let mut builder = app.notification().builder().title(title).body(body);
    if sound {
        builder = builder.sound("default");
    }
    builder.show().map_err(|error| error.to_string())
}

/// The open questions, on the dock.
#[tauri::command]
fn badge(app: tauri::AppHandle, count: i64) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    window
        .set_badge_count((count > 0).then_some(count))
        .map_err(|error| error.to_string())
}

/// The installed vendors' own icons, read once per window.
#[tauri::command]
fn agent_icons() -> std::collections::BTreeMap<String, String> {
    let scratch = std::env::temp_dir().join("orochi-desktop");
    let _ = std::fs::create_dir_all(&scratch);
    orochi_desktop::view::agent_icons(&scratch)
}

#[tauri::command]
fn set_route(open: State<'_, Open>, thread: String, route: Option<String>) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .set_route(&thread, route.as_deref())
        .map_err(fail)
}

#[tauri::command]
fn search(
    open: State<'_, Open>,
    query: String,
) -> Result<Vec<orochi::activity::SearchHit>, String> {
    open.0.lock().unwrap().search(&query).map_err(fail)
}

#[tauri::command]
fn stop_seat(open: State<'_, Open>, thread: String, seat: Option<String>) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .stop_seat(&thread, seat.as_deref())
        .map_err(fail)
}

#[tauri::command]
fn live_seats(open: State<'_, Open>) -> Result<Vec<orochi::activity::LiveSeat>, String> {
    open.0.lock().unwrap().live_seats().map_err(fail)
}

/// Every agent a seat's own CLI runs inside a `Task` right now, drawn under that seat.
#[tauri::command]
fn live_tasks(open: State<'_, Open>) -> Result<Vec<orochi::activity::LiveTask>, String> {
    open.0.lock().unwrap().live_tasks().map_err(fail)
}

// Settings → Mobile (`docs/mobile-remote-design.md`). These are the window's own commands: the
// phone's `/rpc` whitelist does not include any of them.

#[tauri::command]
fn remote_status(
    remote: State<'_, orochi_desktop::remote::Remote>,
) -> orochi_desktop::remote::Status {
    remote.status()
}

#[tauri::command]
async fn remote_enable(
    remote: State<'_, orochi_desktop::remote::Remote>,
) -> Result<orochi_desktop::remote::Status, String> {
    remote.enable().await.map_err(fail)
}

#[tauri::command]
async fn remote_disable(
    remote: State<'_, orochi_desktop::remote::Remote>,
) -> Result<orochi_desktop::remote::Status, String> {
    remote.disable().await.map_err(fail)
}

#[tauri::command]
fn remote_pair(
    remote: State<'_, orochi_desktop::remote::Remote>,
) -> Result<orochi_desktop::remote::Pairing, String> {
    remote.pair().map_err(fail)
}

#[tauri::command]
fn remote_set_name(
    remote: State<'_, orochi_desktop::remote::Remote>,
    name: String,
) -> Result<orochi_desktop::remote::Status, String> {
    remote.set_name(&name).map_err(fail)
}

#[tauri::command]
fn remote_devices(
    remote: State<'_, orochi_desktop::remote::Remote>,
) -> Vec<orochi_desktop::remote::DeviceView> {
    remote.devices()
}

#[tauri::command]
fn remote_revoke(
    remote: State<'_, orochi_desktop::remote::Remote>,
    id: String,
) -> Result<bool, String> {
    remote.revoke(&id).map_err(fail)
}

/// Where the core keeps its data, by the same rule the CLI uses, so the window and the
/// terminal are looking at one store.
fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    // The same rule the CLI uses, `OROCHI_DATA_DIR` included, so the window and the terminal
    // are always looking at one store.
    let chosen = std::env::var_os("OROCHI_DATA_DIR").map(std::path::PathBuf::from);
    Ok(orochi::config::Paths::resolve(None, chosen)?.data)
}

/// Whether the browser is on and which Chrome and `orochi-servo` it would use. The window
/// draws a tab when either exists: a machine with only Servo has a browser too.
#[tauri::command]
fn browser_status(open: State<'_, Open>) -> serde_json::Value {
    let (config, data) = open.0.lock().unwrap().paths();
    let config = orochi::config::Config::load(&config).unwrap_or_default();
    #[cfg(unix)]
    let (chrome, servo) = (
        // Chrome is not run unless the settings ask for it, so it is not offered either.
        (config.browser.engine != orochi::config::BrowserEngine::Servo)
            .then(|| orochi::browser::find_chrome(&config.browser).ok())
            .flatten()
            .map(|p| p.display().to_string()),
        orochi::browser::engine::find_servo(&config.browser, &data)
            .map(|p| p.display().to_string()),
    );
    #[cfg(not(unix))]
    let (chrome, servo): (Option<String>, Option<String>) = {
        let _ = &data;
        (None, None)
    };
    serde_json::json!({ "enabled": config.browser.enabled, "chrome": chrome, "servo": servo })
}

/// The connection to the broker for one key (a conversation, or the settings page's own),
/// opened on first use after the browser has been registered with the current settings.
#[cfg(unix)]
fn broker_client(
    open: &State<'_, Open>,
    views: &State<'_, Views>,
    key: &str,
    refresh: bool,
) -> Result<std::sync::Arc<Mutex<orochi::browser::Client>>, String> {
    let (config_path, data) = open.0.lock().unwrap().paths();
    // The broker reads its settings from a file `register` writes; the settings page changes
    // them while the window runs, so it asks for the file to be brought up to date.
    if refresh || !*views.ready.lock().unwrap() {
        let config = orochi::config::Config::load(&config_path).map_err(fail)?;
        if !config.browser.enabled {
            return Err("the browser is off: set `enabled = true` under [browser]".into());
        }
        orochi::browser::register(&data, orochi(), &config.browser).map_err(fail)?;
        *views.ready.lock().unwrap() = true;
    }
    let mut clients = views.clients.lock().unwrap();
    if !clients.contains_key(key) {
        let opened = orochi::browser::Client::open(&orochi(), &data).map_err(fail)?;
        clients.insert(key.to_owned(), std::sync::Arc::new(Mutex::new(opened)));
    }
    Ok(clients[key].clone())
}

/// What the broker answered, as JSON, or its error text.
#[cfg(unix)]
fn broker_answer(reply: serde_json::Value) -> Result<serde_json::Value, String> {
    let said = reply["content"][0]["text"].as_str().unwrap_or("");
    if reply["isError"] == true {
        return Err(said.to_owned());
    }
    serde_json::from_str(said).map_err(|e| e.to_string())
}

/// The broker's own administration (`remote_*`: the phone listener), which has no tab and no
/// conversation. Anything but a `remote_` tool is refused here, so the window cannot be made
/// to speak an agent's tools.
#[tauri::command]
async fn browser_admin(
    open: State<'_, Open>,
    views: State<'_, Views>,
    tool: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(not(unix))]
    {
        let _ = (&open, &views, &tool, &args);
        Err("the browser is not available on this platform yet".to_owned())
    }
    #[cfg(unix)]
    {
        if !tool.starts_with("remote_") {
            return Err(format!("{tool} is not an administration call"));
        }
        const KEY: &str = "\u{0}admin";
        let client = broker_client(&open, &views, KEY, true)?;
        let request = orochi::browser::broker::Request {
            scope: "admin".into(),
            tool,
            args,
            read_only: false,
            cwd: None,
        };
        let reply =
            tauri::async_runtime::spawn_blocking(move || client.lock().unwrap().call(&request))
                .await
                .map_err(|e| e.to_string())?;
        let reply = reply.map_err(|error| {
            views.clients.lock().unwrap().remove(KEY);
            fail(error)
        })?;
        broker_answer(reply)
    }
}

/// One call of the person's browser tab (`view_*`): the answer is the broker's JSON.
#[tauri::command]
async fn browser_call(
    open: State<'_, Open>,
    views: State<'_, Views>,
    thread: String,
    tab: String,
    tool: String,
    mut args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(not(unix))]
    {
        let _ = (&open, &views, &thread, &tab, &tool, &args);
        Err("the browser tab is not available on this platform yet".to_owned())
    }
    #[cfg(unix)]
    {
        let client = broker_client(&open, &views, &thread, false)?;
        // A preview serves a file of this conversation's folder, and only that: the folder is
        // the window's to say, never the page's.
        if tool == "view_preview" {
            let cwd = open.0.lock().unwrap().cwd(&thread).map_err(fail)?;
            args["root"] = serde_json::json!(cwd);
        }
        let scope = format!("user:{thread}:{tab}");
        let request = orochi::browser::broker::Request {
            scope,
            tool,
            args,
            read_only: false,
            cwd: None,
        };
        let closing = request.tool == "view_close";
        let reply =
            tauri::async_runtime::spawn_blocking(move || client.lock().unwrap().call(&request))
                .await
                .map_err(|e| e.to_string())?;
        if closing {
            views.clients.lock().unwrap().remove(&thread);
        }
        let reply = reply.map_err(|error| {
            // The connection is broken (the broker stopped); the next call starts afresh.
            views.clients.lock().unwrap().remove(&thread);
            fail(error)
        })?;
        broker_answer(reply)
    }
}

#[tauri::command]
fn term_open(
    app: tauri::AppHandle,
    open: State<'_, Open>,
    shells: State<'_, Shells>,
    thread: String,
    cols: u16,
    rows: u16,
    on_data: tauri::ipc::Channel<Vec<u8>>,
) -> Result<orochi_desktop::term::Opened, String> {
    use tauri::Emitter;
    let cwd = open.0.lock().unwrap().cwd(&thread).map_err(fail)?;
    shells
        .0
        .open(
            &cwd,
            None,
            (cols, rows),
            move |bytes| {
                let _ = on_data.send(bytes);
            },
            move |id, code| {
                let _ = app.emit("term-exit", serde_json::json!({ "id": id, "code": code }));
            },
        )
        .map_err(fail)
}

#[tauri::command]
fn term_write(shells: State<'_, Shells>, id: String, data: String) -> Result<(), String> {
    shells.0.write(&id, data.as_bytes()).map_err(fail)
}

#[tauri::command]
fn term_resize(shells: State<'_, Shells>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    shells.0.resize(&id, cols, rows).map_err(fail)
}

#[tauri::command]
fn term_close(shells: State<'_, Shells>, id: String) -> Result<(), String> {
    shells.0.close(&id).map_err(fail)
}

fn main() {
    tauri::Builder::default()
        .register_uri_scheme_protocol("orochi-media", |_context, request| {
            let data = data_dir().unwrap_or_default();
            let range = request
                .headers()
                .get("range")
                .and_then(|value| value.to_str().ok());
            let reply = orochi_desktop::media::serve(&data, request.uri().path(), range);
            let mut response = tauri::http::Response::builder().status(reply.status);
            for (name, value) in &reply.headers {
                response = response.header(*name, value);
            }
            response
                .body(std::borrow::Cow::Owned(reply.body))
                .unwrap_or_default()
        })
        .setup(|app| {
            let data = data_dir()?;
            let client = Arc::new(Mutex::new(Client::open(&data)?));
            app.manage(Open(client.clone()));
            app.manage(orochi_desktop::remote::Remote::new(&data, client, orochi()));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                handle
                    .state::<orochi_desktop::remote::Remote>()
                    .resume()
                    .await;
            });
            app.manage(Shells(orochi_desktop::term::Terminals::default()));
            app.manage(Views::default());
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            add_project,
            members,
            mentions,
            see_mentions,
            folders,
            new_thread,
            placement,
            place,
            sandboxes,
            sandbox_job,
            setup_status,
            decision_key,
            places,
            set_place,
            setup_login,
            open_docs,
            cancel_job,
            sandbox_secret,
            sandbox_login,
            rename_thread,
            pin_thread,
            archive_thread,
            delete_thread,
            rename_project,
            pin_project,
            hide_project,
            delete_project,
            open_folder,
            open_terminal,
            ensure_host,
            watch,
            room,
            chatter,
            say,
            agents,
            refresh_quota,
            insights,
            board,
            team,
            tree_files,
            git_overview,
            git_show,
            tree_patch,
            tree_refresh,
            tree_list,
            tree_read,
            tree_media,
            tree_find,
            open_path,
            comment,
            settings,
            save_settings,
            roster,
            set_roster,
            set_roster_note,
            suggest_roster,
            refresh_models,
            memory,
            save_memory,
            forget_all,
            sidebar,
            thread,
            patch,
            open_prompts,
            answer,
            send,
            interrupt,
            seen,
            changed,
            live_seats,
            live_tasks,
            stop_seat,
            search,
            set_route,
            notify,
            badge,
            agent_icons,
            term_open,
            term_write,
            term_resize,
            term_close,
            browser_status,
            browser_call,
            remote_status,
            remote_enable,
            remote_disable,
            remote_pair,
            remote_set_name,
            remote_devices,
            remote_revoke,
            browser_admin
        ])
        .build(tauri::generate_context!())
        .expect("orochi desktop failed to start")
        // Nothing a terminal started outlives the window.
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<Shells>().0.close_all();
            }
        });
}
