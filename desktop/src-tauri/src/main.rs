#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The shell. It owns a window and a connection, and does nothing else: every command below
//! is one call into `view`, which is where the app's behavior lives and where it is tested.
use orochi::activity::{ProjectRow, Thread};
use orochi_desktop::view::{AgentRow, Client, Folder, Prompt, Remembered, RouteStats, Said};
use std::sync::Mutex;
use tauri::{Manager, State};

/// The one connection this window reads through. `Client` is not `Sync` (a SQLite connection
/// is not), so the window holds it behind a mutex and never across an await.
struct Open(Mutex<Client>);

fn fail(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// The folders a thread can be started in, and starting one. Choosing where the work happens
/// is the one thing a window must ask before anything else can be done.
#[tauri::command]
fn folders(open: State<'_, Open>) -> Result<Vec<Folder>, String> {
    open.0.lock().unwrap().folders().map_err(fail)
}

#[tauri::command]
fn new_thread(open: State<'_, Open>, root: String) -> Result<String, String> {
    open.0
        .lock()
        .unwrap()
        .new_thread(std::path::Path::new(&root))
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

/// Leaves a note in the room. It reaches the agents when they next read their messages.
#[tauri::command]
fn say(
    open: State<'_, Open>,
    thread: String,
    to: Option<String>,
    text: String,
) -> Result<(), String> {
    open.0
        .lock()
        .unwrap()
        .say(&thread, to.as_deref(), &text)
        .map_err(fail)
}

#[tauri::command]
fn agents(open: State<'_, Open>) -> Result<Vec<AgentRow>, String> {
    open.0.lock().unwrap().agents().map_err(fail)
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

/// Where the core keeps its data, by the same rule the CLI uses, so the window and the
/// terminal are looking at one store.
fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    // The same rule the CLI uses, `OROCHI_DATA_DIR` included, so the window and the terminal
    // are always looking at one store.
    let chosen = std::env::var_os("OROCHI_DATA_DIR").map(std::path::PathBuf::from);
    Ok(orochi::config::Paths::resolve(None, chosen)?.data)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let client = Client::open(&data_dir()?)?;
            app.manage(Open(Mutex::new(client)));
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            folders,
            new_thread,
            placement,
            place,
            sandboxes,
            sandbox_job,
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
            say,
            agents,
            insights,
            board,
            tree_files,
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
            stop_seat,
            search,
            set_route,
            notify,
            badge,
            agent_icons
        ])
        .run(tauri::generate_context!())
        .expect("orochi desktop failed to start");
}
