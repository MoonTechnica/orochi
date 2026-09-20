#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! The shell. It owns a window and a connection, and does nothing else: every command below
//! is one call into `view`, which is where the app's behavior lives and where it is tested.
use orochi_desktop::view::{
    AgentRow, Client, Folder, Prompt, Remembered, RouteStats, Said, Working,
};
use orochi::activity::{ProjectRow, Thread};
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

/// Starts the thread's host if it has none, so a message sent here actually runs. A thread a
/// terminal is holding keeps its owner; this never takes one over.
#[tauri::command]
fn ensure_host(open: State<'_, Open>, thread: String) -> Result<bool, String> {
    let needed = open.0.lock().unwrap().needs_host(&thread).map_err(fail)?;
    if !needed {
        return Ok(false);
    }
    // The same binary the user would run, found the way a user would find it.
    let orochi = std::env::var_os("OROCHI_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "orochi".into());
    std::process::Command::new(orochi)
        .args(["host", "--thread", &thread])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start a host for this thread: {error}"))?;
    Ok(true)
}

/// One screen of every seat working anywhere, so many threads can be watched without opening
/// any of them.
#[tauri::command]
fn working(open: State<'_, Open>) -> Result<Vec<Working>, String> {
    open.0.lock().unwrap().working().map_err(fail)
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
fn settings(open: State<'_, Open>) -> Result<serde_json::Value, String> {
    open.0.lock().unwrap().settings().map_err(fail)
}

#[tauri::command]
fn save_settings(open: State<'_, Open>, settings: serde_json::Value) -> Result<(), String> {
    open.0.lock().unwrap().save_settings(&settings).map_err(fail)
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
    open.0.lock().unwrap().sidebar(limit, archived).map_err(fail)
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
        .invoke_handler(tauri::generate_handler![
            folders,
            new_thread,
            ensure_host,
            working,
            room,
            say,
            agents,
            insights,
            board,
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
            changed
        ])
        .run(tauri::generate_context!())
        .expect("orochi desktop failed to start");
}
