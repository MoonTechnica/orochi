//! Interactive conversation (`orochi` on a terminal, `orochi chat`). Each message re-enters the
//! scheduler; later messages continue the agent session the first one was routed to.
//!
//! The terminal UI follows Claude Code, and OpenHands CLI where Claude Code says nothing: the
//! input line, its attachments and a status row stay pinned at the bottom while the transcript
//! scrolls above them, messages typed during a turn queue up, tool rows are marked ✓/✗ in
//! place, Esc interrupts, and Shift-Tab cycles the approval mode the running agent actually
//! supports. Keys, the queue and questions behave as Claude Code documents them.
mod banner;
pub mod term;

use crate::{
    acp::{ExecutionEvent, FileDiff, Progress, ToolUpdate, allow_once_option},
    collaboration::{Part, Plan},
    config::{Config, PermissionMode},
    context::bounded,
    policy::Registry,
    router::roles::Role,
    scheduler::{self, Continuation, RunOptions},
    storage::{Store, workspace_lock},
    types::{
        Attachment, Feedback, Outcome, Overrides, Provider, SessionRecord, TaskDescriptor, Usage,
        now,
    },
};
use anyhow::Result;
use futures::StreamExt;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::IsTerminal,
    path::Path,
    time::{Duration, Instant},
};
use term::{Key, Keyboard, Term, width_of};
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

const USER_CHARS: usize = 8192;
const AGENT_CHARS: usize = 16384;
const TRANSCRIPT_BYTES: usize = 32768;
/// Messages kept for the end-of-session look back; the request itself keeps the newest part.
const SAID_MESSAGES: usize = 64;
const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024;

const PROMPT: &str = "> ";

struct Command {
    name: &'static str,
    aliases: &'static [&'static str],
    usage: &'static str,
    about: &'static str,
    /// Runs even while an agent is working.
    anytime: bool,
}
const COMMANDS: [Command; 11] = [
    Command {
        name: "help",
        aliases: &[],
        usage: "",
        about: "Show commands and shortcuts",
        anytime: true,
    },
    Command {
        name: "new",
        aliases: &["clear"],
        usage: "",
        about: "Start a new conversation",
        anytime: false,
    },
    Command {
        name: "resume",
        aliases: &["history"],
        usage: "[n|id]",
        about: "List or reopen recent conversations in this directory",
        anytime: false,
    },
    Command {
        name: "team",
        aliases: &["collaborate"],
        usage: "<task>",
        about: "Force the design, implement and review steps (Orochi decides on its own otherwise)",
        anytime: false,
    },
    Command {
        name: "solo",
        aliases: &[],
        usage: "<task>",
        about: "Force one agent to do the whole task in one turn",
        anytime: false,
    },
    Command {
        name: "reroute",
        aliases: &[],
        usage: "",
        about: "Pick an agent again, keeping this conversation",
        anytime: false,
    },
    Command {
        name: "confirm",
        aliases: &["permissions"],
        usage: "[ask|always|never]",
        about: "How permission requests are answered: auto, ask or never (Shift-Tab cycles)",
        anytime: true,
    },
    Command {
        name: "peers",
        aliases: &[],
        usage: "",
        about: "Show the other agents working in this repository",
        anytime: true,
    },
    Command {
        name: "status",
        aliases: &[],
        usage: "",
        about: "Show the agent, model and session in use",
        anytime: true,
    },
    Command {
        name: "memory",
        aliases: &[],
        usage: "[forget <id>]",
        about: "Show what Orochi remembers about you and this repository",
        anytime: true,
    },
    Command {
        name: "exit",
        aliases: &["quit"],
        usage: "",
        about: "Quit",
        anytime: false,
    },
];
const SHORTCUTS: [(&str, &str); 10] = [
    ("Shift-Tab", "Cycle the approval mode"),
    ("Esc", "Stop the running turn; what is queued is sent next"),
    ("Esc Esc", "Clear what you typed (↑ brings it back)"),
    (
        "Enter",
        "Send; while the agent works, the message is queued",
    ),
    ("↑", "While messages are queued, take them back to edit"),
    ("Ctrl-C", "Stop the turn, or clear the input; twice quits"),
    ("Ctrl-D", "Delete forward; twice on an empty line quits"),
    ("\\ + Enter", "Continue the message on a new line"),
    ("/ then Tab", "List and complete commands"),
    ("@ then Tab", "Attach a file or image by path"),
];

pub struct Options {
    /// Apply to the first message and to every message routed again.
    pub overrides: Overrides,
    /// A recorded session the first message continues.
    pub resume: Option<String>,
    pub permission: PermissionMode,
}

/// One queued user message.
struct Message {
    text: String,
    attachments: Vec<Attachment>,
    /// What the user asked for explicitly; otherwise Orochi decides.
    steps: Steps,
}

#[derive(PartialEq, Clone, Copy)]
enum Steps {
    Auto,
    Team,
    Solo,
}

/// A step of `/team`: its own agent, chosen for this kind of work.
struct Phase {
    name: &'static str,
    /// Wording the profiler reads, so each phase is routed on its own merits.
    instruction: &'static str,
    /// The phase passes its reply on to the next one.
    hands_over: bool,
}
const DESIGN: usize = 0;
const IMPLEMENT: usize = 1;
const REVIEW: usize = 2;
const PHASES: [Phase; 3] = [
    Phase {
        name: "design",
        instruction: "Plan the work first (設計 / architecture). Change nothing yet. Answer with what has to happen, the pieces involved, the risks, and how the result will be checked. Answer in the language the user used. If the work divides into parts that can be built without each other's code, end with one line holding only {\"parts\":[{\"id\":\"<lowercase letters, digits, ->\",\"brief\":\"what this part builds\",\"paths\":[\"<relative paths it writes>\"],\"after\":[\"<ids of parts whose code it needs>\"]}]}; two parts that only have to agree on an interface need not wait for each other. Leave it out when the work does not divide.",
        hands_over: true,
    },
    Phase {
        name: "implement",
        instruction: "Carry out the task, following the plan above. Verify the result yourself. Answer in the language the user used.",
        hands_over: true,
    },
    Phase {
        name: "review",
        instruction: "Review (レビュー) the work just done. Change nothing. Report what is wrong or missing, most important first, in the language the user used. End with one line: \"VERDICT: fix\" when something must change, or \"VERDICT: ok\" when it is sound.",
        hands_over: true,
    },
];

/// Every prompt Orochi writes is English, whatever the user writes in; without this the seats
/// answer in different languages.
const LANGUAGE: &str = "Write your answer, and every message you send to another agent, in the language the user \
     used in the request.";

/// What a key press asks the session to do.
enum Action {
    None,
    Send(Message),
    /// Esc: stops a running turn; at an idle prompt, twice clears the draft.
    Escape,
    /// Ctrl-C: stops a running turn; at an idle prompt, clears the input, then leaves.
    Interrupt,
    /// Ctrl-D on an empty line: leaves once pressed twice.
    Exit,
    /// Input ended.
    Quit,
    Cycle,
}

/// How long a second Esc or Ctrl-D still counts as the second press (Claude Code's 800 ms).
const AGAIN: Duration = Duration::from_millis(800);

/// Keys that act on their second press at an idle prompt, and when each was first pressed.
/// Any other key starts them over.
#[derive(Default)]
struct Twice {
    escape: Option<Instant>,
    interrupt: bool,
    exit: Option<Instant>,
}
impl Twice {
    fn again(first: &mut Option<Instant>) -> bool {
        match first.take() {
            Some(at) if at.elapsed() < AGAIN => true,
            _ => {
                *first = Some(Instant::now());
                false
            }
        }
    }
}

/// The approval mode: the agent's own session modes when it has them, else how Orochi answers
/// the agent's permission requests.
struct Approval {
    confirm: PermissionMode,
    modes: Vec<String>,
    mode: Option<String>,
    /// The user picked a mode, so a finished turn must not replace it.
    chosen: bool,
    /// The agent and model the modes belong to; another model may not have them.
    route: Option<String>,
}
impl Approval {
    fn cycle(&mut self) {
        self.chosen = true;
        if self.modes.len() > 1 {
            let next = self
                .mode
                .as_ref()
                .and_then(|mode| self.modes.iter().position(|m| m == mode))
                .map_or(0, |index| (index + 1) % self.modes.len());
            self.mode = Some(self.modes[next].clone());
        } else {
            self.confirm = match self.confirm {
                PermissionMode::Ask => PermissionMode::Allow,
                PermissionMode::Allow => PermissionMode::Deny,
                PermissionMode::Deny => PermissionMode::Ask,
            };
        }
    }
    fn label(&self) -> String {
        match &self.mode {
            Some(mode) => format!("{mode} mode"),
            None => format!("confirm {}", confirm_label(self.confirm)),
        }
    }
}

/// A turn's run and how it ended, until the user's next move labels it (or does not).
struct Awaiting {
    run: String,
    interrupted: bool,
}

/// Everything one chat session needs; the pinned input lives in `view.term`.
struct Session<'a> {
    config: &'a Config,
    data: &'a Path,
    store: &'a Store,
    root: &'a Path,
    options: Options,
    view: View,
    /// Whether there is a terminal to ask a question at. Without one every stdin line is a
    /// message, so a prompt would eat the next one.
    tty: bool,
    keyboard: Keyboard,
    queue: VecDeque<Message>,
    /// What the user asked this session, in memory only, for the one distillation at its end.
    /// Kept apart from `conversation`, which `/new` clears.
    said: VecDeque<String>,
    /// The last turn's run, waiting for the only verdict an unverified turn gets: what the user
    /// does next.
    awaiting: Option<Awaiting>,
    conversation: Conversation,
    feed: Option<Feed>,
    approval: Approval,
    twice: Twice,
    quit: bool,
}

pub async fn run(
    config: &Config,
    data: &Path,
    store: &Store,
    root: &Path,
    options: Options,
) -> Result<u8> {
    // Esc and Ctrl-C interrupt by raising SIGINT, so the handler must exist before any turn.
    #[cfg(unix)]
    let _sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let tty = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();
    let keyboard = Keyboard::start(tty);
    let approval = Approval {
        confirm: options.permission,
        modes: vec![],
        chosen: options.overrides.mode.is_some(),
        mode: options.overrides.mode.clone(),
        route: None,
    };
    let mut session = Session {
        config,
        data,
        store,
        root,
        conversation: Conversation {
            resume: options.resume.clone(),
            ..Default::default()
        },
        options,
        view: View::new(tty),
        tty,
        keyboard,
        queue: VecDeque::new(),
        said: VecDeque::new(),
        awaiting: None,
        feed: Feed::open(),
        approval,
        twice: Twice::default(),
        quit: false,
    };
    if tty {
        session.view.welcome(config, data, root, &session.feed);
    }
    session.refresh_status(None);
    let code = session.serve().await;
    session.distill().await;
    session.view.term.restore();
    code
}

impl Session<'_> {
    /// The idle loop: keys, queued messages and mailbox traffic until the user quits.
    async fn serve(&mut self) -> Result<u8> {
        #[cfg(unix)]
        let mut resized =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
        let mut mail = mail_ticker();
        loop {
            let message = loop {
                if self.quit {
                    break None;
                }
                // A queued line may be a command; run it in the order it was typed.
                if let Some(queued) = self.queue.pop_front() {
                    match self.dispatch(queued, false) {
                        Some(message) => break Some(message),
                        None => continue,
                    }
                }
                let key = tokio::select! {
                    key = self.keyboard.next() => key,
                    _ = mail.tick(), if self.feed.is_some() => { self.deliver(); continue }
                    _ = resized.recv() => { self.view.term.resized(); continue }
                };
                let Some(key) = key else { break None };
                let action = press(&mut self.view.term, key, self.root);
                // As in Claude Code: Esc never leaves; twice clears a draft into history.
                // Ctrl-C clears the input, and a second press leaves; Ctrl-D on an empty line
                // asks first. Anything else in between starts them over.
                let twice = std::mem::take(&mut self.twice);
                match action {
                    Action::Quit => break None,
                    Action::Send(message) => {
                        if let Some(message) = self.dispatch(message, false) {
                            break Some(message);
                        }
                    }
                    Action::Cycle => {
                        self.approval.cycle();
                        self.refresh_status(None);
                    }
                    Action::Escape => {
                        self.twice.escape = twice.escape;
                        if Twice::again(&mut self.twice.escape) {
                            self.view.term.prompt.shelve();
                            self.view.term.render();
                        }
                    }
                    Action::Interrupt => {
                        if twice.interrupt {
                            break None;
                        }
                        self.twice.interrupt = true;
                        self.view.term.prompt.clear();
                        self.view.term.render();
                        self.view.note("Press Ctrl-C again to exit");
                    }
                    Action::Exit => {
                        self.twice.exit = twice.exit;
                        if Twice::again(&mut self.twice.exit) {
                            break None;
                        }
                        self.view.note("Press Ctrl-D again to exit");
                    }
                    Action::None => {}
                }
                self.refresh_status(None);
            };
            let Some(message) = message else { break };
            self.run_turn(message).await?;
        }
        Ok(0)
    }

    /// Runs a command straight away, or hands a plain message back to be executed.
    fn dispatch(&mut self, message: Message, running: bool) -> Option<Message> {
        let Some((command, argument)) = command(&message.text) else {
            if running {
                self.view.queued(&message.text, self.queue.len() + 1);
                self.queue.push_back(message);
                return None;
            }
            self.view.sent(&message.text, &message.attachments);
            return Some(message);
        };
        let known = COMMANDS
            .iter()
            .find(|c| c.name == command || c.aliases.contains(&command));
        if running && !known.is_some_and(|c| c.anytime) {
            self.view.queued(&message.text, self.queue.len() + 1);
            self.queue.push_back(message);
            return None;
        }
        self.view.sent(&message.text, &[]);
        match command {
            "help" => self.view.help(),
            "exit" | "quit" => {
                self.queue.clear();
                self.quit = true;
            }
            "status" => {
                let status = self.view.session_status(&self.conversation, self.root);
                self.view.rows(status);
            }
            "memory" => self.memory(argument),
            "new" | "clear" => {
                // Moving on to something else says nothing about the last answer.
                self.awaiting = None;
                self.conversation = Conversation::default();
                self.view.last_route = None;
                self.approval.modes.clear();
                self.approval.mode = None;
                self.approval.chosen = false;
                self.view.note("Started a new conversation");
            }
            "reroute" => {
                // Asking for another agent right after an answer, or right after stopping one, is
                // a vote against it.
                if let Some(Awaiting { run, .. }) = self.awaiting.take() {
                    let _ = self.store.feedback(&run, &Feedback::rerouted());
                }
                self.conversation.current = None;
                self.conversation.resume = None;
                self.view.last_route = None;
                self.view
                    .note("The next message picks an agent again and keeps this conversation");
            }
            "resume" | "history" => {
                let result = self.resume(argument);
                if let Err(error) = result {
                    self.view.result(ERR, &format!("{error:#}"));
                }
            }
            "team" | "collaborate" | "solo" => {
                if argument.is_empty() {
                    self.view.result(WARN, &format!("/{command} <task>"));
                } else {
                    return Some(Message {
                        text: argument.to_owned(),
                        attachments: message.attachments,
                        steps: if command == "solo" {
                            Steps::Solo
                        } else {
                            Steps::Team
                        },
                    });
                }
            }
            "confirm" | "permissions" => self.confirm(argument),
            "peers" => {
                let rows = self.view.peer_rows(&self.feed);
                self.view.rows(rows);
            }
            other => self.view.result(
                WARN,
                &format!("Unknown command /{other} · type / and Tab to list commands"),
            ),
        }
        self.refresh_status(None);
        None
    }

    fn confirm(&mut self, argument: &str) {
        match argument {
            "ask" => self.approval.confirm = PermissionMode::Ask,
            "auto" | "always" | "allow" => self.approval.confirm = PermissionMode::Allow,
            "never" | "deny" => self.approval.confirm = PermissionMode::Deny,
            _ => {
                let rows = self.view.confirm_help(self.approval.confirm);
                self.view.rows(rows);
                return;
            }
        }
        self.approval.mode = None;
        self.approval.chosen = true;
        let label = self.approval.label();
        self.view.note(&format!("Approval: {label}"));
    }

    fn resume(&mut self, argument: &str) -> Result<()> {
        let mut sessions = self
            .store
            .sessions(Some(&self.store.repository_id(self.root)?))?;
        sessions.truncate(10);
        if argument.is_empty() {
            let rows = self.view.session_rows(&sessions);
            self.view.rows(rows);
            return Ok(());
        }
        let chosen = argument
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| sessions.get(n))
            .or_else(|| sessions.iter().find(|s| s.session_id == argument));
        match chosen {
            Some(session) => {
                self.conversation = Conversation {
                    resume: Some(session.session_id.clone()),
                    ..Default::default()
                };
                self.view.last_route = None;
                self.view.note(&format!(
                    "The next message continues {} / {}",
                    session.agent, session.model
                ));
            }
            None => self
                .view
                .result(WARN, "No such conversation · /resume lists them"),
        }
        Ok(())
    }

    fn deliver(&mut self) {
        let Some(feed) = self.feed.as_mut() else {
            return;
        };
        let events = feed.poll();
        let mine = feed.mine.clone();
        let routes = feed.routes.clone();
        for event in events {
            let text = self.view.mail(&event, &mine, &routes);
            self.view.term.note(&text);
        }
    }

    /// The status row: approval mode, queue depth and what the agent is doing.
    fn refresh_status(&mut self, phase: Option<&str>) {
        let mut parts = vec![format!("⏵⏵ {} (shift+tab)", self.approval.label())];
        if !self.queue.is_empty() {
            parts.push(format!("{} queued", self.queue.len()));
        }
        match phase {
            Some(phase) => parts.push(format!("{phase} · esc to interrupt")),
            None => parts.push("/help for commands".into()),
        }
        let status = fit(
            &parts.join(" · "),
            self.view.term.columns().saturating_sub(1),
        );
        self.view.term.status = self.view.paint(MUTED, &status);
        self.view.term.render();
    }
}

/// Command candidates for what has been typed, as the rows drawn above the input. Only while
/// the command itself is being typed: past the first space the rest is that command's argument.
fn suggest(term: &mut Term) {
    let typed = term
        .prompt
        .text()
        .strip_prefix('/')
        .filter(|rest| !rest.contains(char::is_whitespace))
        .map(str::to_owned);
    let Some(typed) = typed else {
        term.suggestions.clear();
        return;
    };
    let matching: Vec<&Command> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&typed) || c.aliases.iter().any(|a| a.starts_with(&typed)))
        .collect();
    if matching.is_empty() {
        term.suggestions.clear();
        return;
    }
    term.selected = term.selected.min(matching.len() - 1);
    let paint = |code: &str, text: &str| {
        if term.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    };
    let names: Vec<String> = matching
        .iter()
        .map(|c| format!("/{} {}", c.name, c.usage).trim_end().to_owned())
        .collect();
    let column = names.iter().map(|n| width_of(n)).max().unwrap_or(0) + 2;
    term.suggestions = matching
        .iter()
        .zip(&names)
        .enumerate()
        .map(|(index, (command, name))| {
            // The characters already typed are the reason this row is here; show which.
            let (head, tail) = name.split_at(typed.len() + 1);
            let padding = " ".repeat(column.saturating_sub(width_of(name)));
            format!(
                "  {}{}{}{}",
                paint(BOLD, head),
                paint(if index == term.selected { BRAND } else { MUTED }, tail),
                padding,
                paint(
                    if index == term.selected { BRAND } else { MUTED },
                    command.about
                )
            )
        })
        .collect();
}

/// Turns a key into an action, editing the pinned input line as it goes.
fn press(term: &mut Term, key: Key, root: &Path) -> Action {
    // Up and Down move through open candidates rather than through history, and typing
    // anything else changes what is being matched, so the selection starts again.
    let picking = !term.suggestions.is_empty();
    let navigating = picking && matches!(key, Key::Up | Key::Down);
    match key {
        Key::Char(c) => term.prompt.insert(&c.to_string()),
        Key::Paste(text) => return paste(term, &text, root),
        // Enter takes the highlighted candidate unless what is typed is already a command,
        // so `/new` still sends on the first Enter rather than needing a second.
        Key::Enter
            if picking
                && !COMMANDS.iter().any(|c| {
                    let typed = term.prompt.text().trim_start_matches('/');
                    c.name == typed || c.aliases.contains(&typed)
                }) =>
        {
            accept(term);
        }
        Key::Enter => {
            let text = term.prompt.text().to_owned();
            if text.ends_with('\\') {
                term.prompt.backspace();
                term.prompt.insert("\n");
            } else if !text.trim().is_empty() || !term.prompt.attachments.is_empty() {
                resolve_references(term, root);
                let (text, attachments) = term.prompt.take();
                return Action::Send(Message {
                    text,
                    attachments,
                    steps: Steps::Auto,
                });
            }
        }
        Key::Backspace => term.prompt.backspace(),
        Key::Delete => term.prompt.delete(),
        Key::Left => term.prompt.left(),
        Key::Right => term.prompt.right(),
        Key::Up if navigating => {
            term.selected = (term.selected + term.suggestions.len() - 1) % term.suggestions.len();
        }
        Key::Down if navigating => {
            term.selected = (term.selected + 1) % term.suggestions.len();
        }
        Key::Up => term.prompt.recall(true),
        Key::Down => term.prompt.recall(false),
        Key::Home => term.prompt.home(),
        Key::End => term.prompt.end(),
        Key::KillLine => term.prompt.kill_line(),
        Key::KillWord => term.prompt.kill_word(),
        Key::Clear => {}
        Key::Tab if picking => accept(term),
        Key::Tab => complete(term, root),
        Key::ShiftTab => return Action::Cycle,
        Key::Escape => return Action::Escape,
        Key::Interrupt => return Action::Interrupt,
        Key::CtrlD if term.prompt.text().is_empty() => return Action::Exit,
        Key::CtrlD => term.prompt.delete(),
        Key::Eof => {
            if term.prompt.text().is_empty() {
                return Action::Quit;
            }
        }
    }
    if !navigating {
        term.selected = 0;
    }
    suggest(term);
    term.render();
    Action::None
}

/// Takes the highlighted candidate, leaving the cursor where its argument goes.
fn accept(term: &mut Term) {
    let Some(command) = COMMANDS
        .iter()
        .filter(|c| {
            let typed = term.prompt.text().trim_start_matches('/');
            c.name.starts_with(typed) || c.aliases.iter().any(|a| a.starts_with(typed))
        })
        .nth(term.selected)
    else {
        return;
    };
    term.prompt.buffer = format!("/{} ", command.name);
    term.prompt.cursor = term.prompt.buffer.len();
}

const IMAGE_TYPES: [(&str, &str); 6] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
];

/// A dropped or pasted path becomes an attachment tag; anything else is inserted as text.
fn paste(term: &mut Term, text: &str, root: &Path) -> Action {
    let candidate = text.trim().trim_matches(['"', '\'']);
    if !candidate.is_empty() && !candidate.contains('\n') && attach(term, candidate, root).is_some()
    {
        term.prompt.insert(" ");
        term.render();
        return Action::None;
    }
    term.prompt.insert(text);
    term.render();
    Action::None
}

/// Attaches an existing file, inserting `[Image #1]` or `[File #1]` where the cursor is.
fn attach(term: &mut Term, path: &str, root: &Path) -> Option<()> {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => Path::new(&std::env::var("HOME").ok()?).join(rest),
        None => Path::new(path).to_path_buf(),
    };
    let full = if expanded.is_absolute() {
        expanded
    } else {
        root.join(expanded)
    };
    let metadata = std::fs::metadata(&full).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ATTACHMENT_BYTES {
        return None;
    }
    let image = IMAGE_TYPES.iter().any(|(extension, _)| {
        full.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    });
    let count = term
        .prompt
        .attachments
        .iter()
        .filter(|a| a.image == image)
        .count()
        + 1;
    term.prompt.attach(Attachment {
        tag: format!("[{} #{count}]", if image { "Image" } else { "File" }),
        name: full
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned()),
        bytes: metadata.len(),
        image,
        path: full,
    });
    Some(())
}

/// Turns every `@path` that names a readable file into an attachment tag.
fn resolve_references(term: &mut Term, root: &Path) {
    let words: Vec<String> = term
        .prompt
        .text()
        .split_whitespace()
        .filter_map(|word| word.strip_prefix('@').map(str::to_owned))
        .collect();
    for word in words {
        let Some(start) = term.prompt.text().find(&format!("@{word}")) else {
            continue;
        };
        term.prompt.cursor = start;
        term.prompt
            .buffer
            .replace_range(start..start + word.len() + 1, "");
        if attach(term, &word, root).is_none() {
            term.prompt.insert(&format!("@{word}"));
        }
    }
    term.prompt.end();
}

/// Tab: complete a `/command` at the start of the line, or an `@path` into an attachment.
fn complete(term: &mut Term, root: &Path) {
    let line = term.prompt.text().to_owned();
    let cursor = term.prompt.cursor;
    let start = line[..cursor]
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let word = &line[start..cursor];
    if let Some(typed) = word.strip_prefix('/').filter(|_| start == 0) {
        let matching: Vec<_> = COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(typed))
            .collect();
        match matching.as_slice() {
            [command] => {
                term.prompt.buffer = format!("/{} ", command.name);
                term.prompt.cursor = term.prompt.buffer.len();
            }
            [] => {}
            many => {
                let list: Vec<String> = many
                    .iter()
                    .map(|c| {
                        format!(
                            "  /{:<22} {}",
                            format!("{} {}", c.name, c.usage).trim_end(),
                            c.about
                        )
                    })
                    .collect();
                term.write(&format!("{}\n", list.join("\n")));
            }
        }
    } else if let Some(typed) = word.strip_prefix('@') {
        let (directory, prefix) = match typed.rfind('/') {
            Some(index) => (&typed[..=index], &typed[index + 1..]),
            None => ("", typed),
        };
        let base = if directory.starts_with('/') || directory.starts_with('~') {
            Path::new(directory).to_path_buf()
        } else {
            root.join(directory)
        };
        let mut names: Vec<String> = std::fs::read_dir(&base)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let hidden = name.starts_with('.') && !prefix.starts_with('.');
                (!hidden && name.starts_with(prefix)).then(|| {
                    if entry.path().is_dir() {
                        format!("{name}/")
                    } else {
                        name
                    }
                })
            })
            .collect();
        names.sort();
        match names.as_slice() {
            [name] => {
                let path = format!("{directory}{name}");
                term.prompt.buffer.replace_range(start..cursor, "");
                term.prompt.cursor = start;
                // A directory keeps the `@` so the next Tab can go deeper.
                if path.ends_with('/') || attach(term, &path, root).is_none() {
                    term.prompt.insert(&format!("@{path}"));
                }
            }
            [] => {}
            many => {
                let list: Vec<_> = many.iter().take(20).cloned().collect();
                term.write(&format!("  {}\n", list.join("  ")));
            }
        }
    }
}

#[cfg(unix)]
fn raise_interrupt() {
    unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
}
#[cfg(not(unix))]
fn raise_interrupt() {}

/// A known command (with an optional argument), or an unknown bare `/word`. Anything else, such
/// as `/usr/bin is missing`, is a message.
fn command(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim().strip_prefix('/')?;
    let (word, argument) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(word, argument)| (word, argument.trim()));
    let known = COMMANDS
        .iter()
        .any(|c| c.name == word || c.aliases.contains(&word));
    (!word.is_empty()
        && word.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        && (known || argument.is_empty()))
    .then_some((word, argument))
}

/// As Claude Code's permission prompt, with No last and on Esc.
const CHOICES: [&str; 3] = [
    "Yes",
    "Yes, and don't ask again this session",
    "No, and say what to do instead (esc)",
];
const NO: usize = 2;

/// A permission request waiting for an answer; the panel stays on screen, the loop keeps running.
struct Pending {
    answer: tokio::sync::oneshot::Sender<Option<String>>,
    allow: String,
    selected: usize,
    /// What was asked about, for the one line that records the answer.
    title: String,
    call: Value,
}

impl Session<'_> {
    async fn run_turn(&mut self, message: Message) -> Result<()> {
        // Carrying on after reading the answer is a (small) vote for how it was routed. Carrying
        // on after stopping it is not: that is usually the user adding what they forgot to say.
        if let Some(Awaiting {
            run,
            interrupted: false,
        }) = self.awaiting.take()
        {
            let _ = self.store.feedback(&run, &Feedback::continued());
        }
        self.said.push_back(bounded(&message.text, USER_CHARS));
        while self.said.len() > SAID_MESSAGES {
            self.said.pop_front();
        }
        let Some(task) = self.profile(&message.text).await else {
            self.view.result(WARN, "Interrupted");
            self.refresh_status(None);
            return Ok(());
        };
        let steps = self.steps(&message, &task);
        if !steps.is_empty() {
            return self.run_team(message, steps, &task).await;
        }
        let pinned = self.conversation.current.is_some();
        let result = self.execute(message, None, Some(task)).await;
        match &result {
            Err(error) => self.view.result(ERR, &format!("✗ {error:#}")),
            Ok((code, _)) if *code != 0 && *code != 130 && pinned => self
                .view
                .note("/reroute picks another agent and keeps this conversation"),
            Ok(_) => {}
        }
        self.refresh_status(None);
        Ok(())
    }

    /// Lists what is remembered, with the ids `forget` takes. For looking, not for using:
    /// nothing here is needed for memory to work.
    fn memory(&mut self, argument: &str) {
        let Some(memory) = crate::memory::Memory::open(self.data, &self.config.memory) else {
            self.view.note("memory is off (memory.enabled = false)");
            return;
        };
        let Ok(repository) = self.store.repository_id(self.root) else {
            return;
        };
        let now = crate::types::now();
        if let Some(id) = argument.strip_prefix("forget").map(str::trim) {
            let result = match crate::memory::position(id) {
                Some((scope, position)) => memory.forget(scope, &repository, position, now),
                None => Ok(None),
            };
            match result {
                Ok(Some(text)) => self.view.note(&format!("forgot: {text}")),
                Ok(None) => self
                    .view
                    .result(WARN, &format!("no memory {id}; /memory lists them")),
                Err(error) => self.view.result(ERR, &format!("✗ {error:#}")),
            }
            return;
        }
        let mut rows = vec![];
        for (scope, prefix, title) in [
            (crate::memory::Scope::User, "u", "About you"),
            (crate::memory::Scope::Repo, "r", "About this repository"),
        ] {
            let items = memory.items(scope, &repository, now);
            rows.push(format!(
                "{} · {}",
                self.view.paint(BOLD, title),
                self.view.paint(
                    MUTED,
                    &memory.path(scope, &repository).display().to_string()
                )
            ));
            if items.is_empty() {
                rows.push(self.view.paint(MUTED, "  nothing yet"));
            }
            for (index, item) in items.iter().enumerate() {
                let origin = match item.auto {
                    None => "yours".to_owned(),
                    Some(seen) => format!("heard {}×", seen.count),
                };
                rows.push(format!(
                    "  {} {}  {}",
                    self.view.paint(MUTED, &format!("{prefix}{}", index + 1)),
                    crate::memory::clean(&item.text, 400),
                    self.view.paint(MUTED, &origin)
                ));
            }
        }
        self.view.rows(rows);
    }

    /// The session's one look back at everything the user said, for what held across it.
    /// The user is on their way out, so it says it is happening and Esc skips it.
    async fn distill(&mut self) {
        if self.said.len() < 2 {
            return;
        }
        let Ok(policies) = Registry::load(self.data) else {
            return;
        };
        let said: Vec<String> = self.said.iter().cloned().collect();
        let (config, store, root) = (self.config, self.store, self.root);
        let mut notes = vec![];
        self.view.note("looking back over this session · Esc skips");
        {
            let work = crate::router::classifier::distill(
                config,
                &policies,
                store,
                root,
                &said,
                |progress| {
                    if let Progress::Note(note) = progress {
                        notes.push(note);
                    }
                },
            );
            tokio::pin!(work);
            let mut listening = true;
            loop {
                tokio::select! {
                    _ = &mut work => break,
                    key = self.keyboard.next(), if listening => match key {
                        Some(Key::Escape | Key::Interrupt) => {
                            self.view.note("skipped");
                            return;
                        }
                        // Input already ended (the usual way out of a piped session).
                        None | Some(Key::Eof) => listening = false,
                        _ => {}
                    },
                }
            }
        }
        for note in notes {
            self.view.note(&note);
        }
    }

    /// One classification per turn. The escalation below, the phase split and the routing
    /// all read this same answer instead of asking again about the same words. `None` when the
    /// user stopped the turn while it was being classified: Esc stops whatever is running.
    async fn profile(&mut self, text: &str) -> Option<TaskDescriptor> {
        let mut task = crate::router::profiler::profile(text, self.root);
        let Ok(policies) = Registry::load(self.data) else {
            return Some(task);
        };
        let (config, store, root) = (self.config, self.store, self.root);
        let mut notes = vec![];
        let stopped = {
            let work = crate::router::classifier::refine(
                config,
                &policies,
                store,
                root,
                text,
                &mut task,
                |progress| {
                    if let Progress::Note(note) = progress {
                        notes.push(note);
                    }
                },
            );
            tokio::pin!(work);
            let mut listening = true;
            loop {
                tokio::select! {
                    _ = &mut work => break false,
                    key = self.keyboard.next(), if listening => {
                        let Some(key) = key else { listening = false; continue };
                        match press(&mut self.view.term, key, root) {
                            Action::Escape | Action::Interrupt => break true,
                            Action::Send(message) => {
                                let _ = self.dispatch(message, true);
                            }
                            Action::Cycle => {
                                self.approval.cycle();
                                self.refresh_status(None);
                            }
                            Action::Quit => listening = false,
                            Action::Exit | Action::None => {}
                        }
                    }
                }
            }
        };
        for note in notes {
            self.view.note(&note);
        }
        (!stopped).then_some(task)
    }

    /// Work the design step divided into parts that can run side by side is past what a console
    /// turn does in the user's tree. It is also the only escalation that starts several agents
    /// at once and writes back, so it is the one thing the console asks about first — showing
    /// the order it would run in, which only something that has read the code can propose.
    /// Asked as Claude Code asks to approve a plan: Esc builds nothing.
    async fn divide(
        &mut self,
        message: &Message,
        task: &TaskDescriptor,
        parts: Vec<Part>,
    ) -> Option<Proceed> {
        if !self.tty || !matches!(message.steps, Steps::Auto) {
            return None;
        }
        let plan = crate::collaboration::plan::with_parts(task, parts)?;
        plan.validate(self.config).ok()?;
        let waves = plan.waves()?;
        let limit = self.view.width().saturating_sub(4).max(20);
        let heading = vec![
            format!(
                " {} {}",
                self.view.paint(BRAND, "◆"),
                self.view
                    .paint(BOLD, "This divides into parts that can run side by side")
            ),
            format!(
                "   {}",
                self.view.paint(
                    CODE,
                    &fit(&crate::collaboration::graph::summary(&waves), limit)
                )
            ),
            format!(
                "   {}",
                self.view.paint(MUTED, &fit(&plan.summary(), limit))
            ),
            format!(
                "   {}",
                self.view.paint(
                    MUTED,
                    &fit(
                        "each part works in its own copy; a verified result is merged back",
                        limit
                    )
                )
            ),
        ];
        let choice = self
            .choose(
                heading,
                &[
                    "Yes, run the parts side by side",
                    "Yes, one agent carries it out here",
                    "No, keep the design (esc)",
                ],
            )
            .await;
        Some(match choice {
            Some(0) => Proceed::Team(plan),
            Some(1) => Proceed::Alone,
            _ => Proceed::Keep,
        })
    }

    /// A question in the pinned area, answered as Claude Code's dialogs are: ↑↓ and Enter, or
    /// an option's number. Esc and Ctrl-C close it without choosing.
    async fn choose(&mut self, heading: Vec<String>, options: &[&str]) -> Option<usize> {
        let mut selected = 0;
        let choice = loop {
            let mut rows = vec![self.view.paint(MUTED, &"─".repeat(self.view.width()))];
            rows.extend(heading.iter().cloned());
            rows.push(String::new());
            for (index, label) in options.iter().enumerate() {
                let label = format!("{}. {label}", index + 1);
                rows.push(if index == selected {
                    format!(
                        " {} {}",
                        self.view.paint(BRAND, "❯"),
                        self.view.paint(&format!("1;{BRAND}"), &label)
                    )
                } else {
                    format!("   {}", self.view.paint(MUTED, &label))
                });
            }
            self.view.term.overlay = Some(rows);
            self.view.term.status = self
                .view
                .paint(MUTED, " Enter to select · ↑↓ to navigate · Esc to cancel");
            self.view.term.render();
            match self.keyboard.next().await {
                Some(Key::Up) => selected = (selected + options.len() - 1) % options.len(),
                Some(Key::Down) => selected = (selected + 1) % options.len(),
                Some(Key::Enter) => break Some(selected),
                Some(Key::Char(c))
                    if c.to_digit(10)
                        .is_some_and(|n| (1..=options.len()).contains(&(n as usize))) =>
                {
                    break c.to_digit(10).map(|n| n as usize - 1);
                }
                None | Some(Key::Escape | Key::Interrupt | Key::Eof) => break None,
                _ => {}
            }
        };
        self.view.term.overlay = None;
        self.refresh_status(None);
        choice
    }

    /// Runs the derived team and reports where it got to. The report lives under the data
    /// directory so an interrupted run can be resumed with `collaborate-resume`.
    async fn collaborate(&mut self, message: Message, plan: Plan) -> Result<()> {
        let output = self
            .data
            .join("collaborations")
            .join(uuid::Uuid::new_v4().to_string());
        self.view.note(&format!("report: {}", output.display()));
        let policies = Registry::load(self.data)?;
        let (events, mut received) = mpsc::unbounded_channel();
        let result = {
            let run = crate::collaboration::run(
                self.config,
                &policies,
                self.store,
                self.root,
                &message.text,
                &plan,
                &output,
                // The user asked for the work, not for a report about it; `apply` still refuses
                // anything whose real checks did not pass.
                true,
                // Progress goes through the screen: raw stderr would land on the pinned input.
                Some(events),
            );
            tokio::pin!(run);
            let mut listening = true;
            loop {
                tokio::select! {
                    biased;
                    Some(event) = received.recv() => {
                        if let ExecutionEvent::Progress(Progress::Note(note)) = event {
                            self.view.note(&note);
                        }
                    }
                    key = self.keyboard.next(), if listening => match key {
                        // The collaboration stops on the interrupt it already listens for.
                        Some(Key::Escape | Key::Interrupt) => raise_interrupt(),
                        None | Some(Key::Eof) => listening = false,
                        _ => {}
                    },
                    result = &mut run => break result,
                }
            }
        };
        while let Ok(event) = received.try_recv() {
            if let ExecutionEvent::Progress(Progress::Note(note)) = event {
                self.view.note(&note);
            }
        }
        match result {
            Ok(report) => {
                let resume = format!(
                    "resume with `orochi collaborate-resume --output {}`",
                    output.display()
                );
                if report.finished() {
                    self.view.result(
                        BRAND,
                        "team finished · result merged into your working tree",
                    );
                } else if report.status == crate::collaboration::RunStatus::Cancelled {
                    self.view.result(WARN, &format!("team stopped · {resume}"));
                } else {
                    self.view.result(
                        WARN,
                        &format!("team finished without a verified result · {resume}"),
                    );
                }
            }
            Err(error) => self.view.result(ERR, &format!("✗ {error:#}")),
        }
        self.refresh_status(None);
        Ok(())
    }

    /// Splits work Orochi judges too big for one turn: a design pass for the hard thinking,
    /// then implementation, then a review when the result needs checking.
    fn steps(&mut self, message: &Message, task: &TaskDescriptor) -> Vec<usize> {
        match message.steps {
            Steps::Solo => return vec![],
            Steps::Team => return vec![DESIGN, IMPLEMENT, REVIEW],
            Steps::Auto => {}
        }
        // Questions, reviews and one-line edits stay a single turn.
        if matches!(
            task.task_type.as_str(),
            "review" | "investigation" | "documentation" | "small_edit" | "discussion"
        ) {
            return vec![];
        }
        let steps = match task.complexity {
            crate::types::Complexity::Extreme => vec![DESIGN, IMPLEMENT, REVIEW],
            crate::types::Complexity::Complex => vec![DESIGN, IMPLEMENT],
            _ if task.requires_architecture_change => vec![DESIGN, IMPLEMENT],
            _ => vec![],
        };
        if !steps.is_empty() {
            let names: Vec<_> = steps.iter().map(|i| PHASES[*i].name).collect();
            self.view.note(&format!(
                "{} task · {}",
                task.complexity.key(),
                names.join(" → ")
            ));
        }
        steps
    }

    /// Each step is routed on its own and hands its reply to the next one. A design that
    /// divides the work may turn the rest of it into a team, if the user says so.
    async fn run_team(
        &mut self,
        message: Message,
        steps: Vec<usize>,
        descriptor: &TaskDescriptor,
    ) -> Result<()> {
        let task = message.text.clone();
        let mut handover = String::new();
        let mut steps = steps;
        let mut index = 0;
        while index < steps.len() {
            let phase = &PHASES[steps[index]];
            self.view.phase(phase.name, index + 1, steps.len());
            let text = format!(
                "{}\n\nTask:\n{task}{}",
                phase.instruction,
                if handover.is_empty() {
                    String::new()
                } else {
                    format!("\n\nFrom the previous step:\n{handover}")
                }
            );
            let step = Message {
                text,
                attachments: if index == 0 {
                    message.attachments.clone()
                } else {
                    vec![]
                },
                steps: Steps::Solo,
            };
            let hands_over = phase.hands_over;
            let implementing = steps[index] == IMPLEMENT;
            let reviewing = steps[index] == REVIEW && index + 1 == steps.len();
            let (code, reply) = match self.execute(step, Some(phase), None).await {
                Ok(result) => result,
                Err(error) => {
                    self.view.result(ERR, &format!("✗ {error:#}"));
                    break;
                }
            };
            let designed = steps[index] == DESIGN;
            let (reply, parts) = if designed {
                divided(&reply)
            } else {
                (reply, None)
            };
            if hands_over {
                handover = bounded(&reply, AGENT_CHARS);
            }
            if designed
                && code == 0
                && let Some(parts) = parts
                && let Some(proceed) = self.divide(&message, descriptor, parts).await
            {
                match proceed {
                    Proceed::Team(plan) => {
                        let team = Message {
                            text: format!(
                                "{}\n\nThe design step concluded:\n{handover}",
                                message.text
                            ),
                            attachments: vec![],
                            steps: Steps::Solo,
                        };
                        return self.collaborate(team, plan).await;
                    }
                    Proceed::Alone => self.view.note("carrying on as one agent"),
                    Proceed::Keep => {
                        // As declining a plan in Claude Code: nothing is built, and the next
                        // message carries on from the design.
                        self.conversation.remember(&message.text, &handover);
                        self.view.note("keeping the design · say what to change");
                        break;
                    }
                }
            }
            // An implementation that did not verify itself gets a review, and then one pass
            // to act on what the review found.
            if implementing && code != 0 && code != 130 && !steps.contains(&REVIEW) {
                self.view
                    .note("the step did not finish cleanly · adding a review");
                steps.push(REVIEW);
            }
            // Only a review that asks for changes costs another pass.
            if reviewing && steps.len() < 5 && reply.to_lowercase().contains("verdict: fix") {
                self.view
                    .note("the review asked for changes · one more pass");
                steps.push(IMPLEMENT);
            }
            if self.quit || code == 130 {
                break;
            }
            index += 1;
        }
        self.refresh_status(None);
        Ok(())
    }

    /// Whether the agent holding this conversation can do what the message needs.
    fn capable(&self, agent: &str, text: &str) -> bool {
        let task = crate::router::profiler::profile(text, self.root);
        self.config
            .agents
            .iter()
            .find(|a| a.id == agent)
            .is_some_and(|a| {
                (!task.requires_image || a.image)
                    && (!task.requires_browser || a.browser)
                    && (!task.requires_web || a.web)
            })
    }

    /// Whether the model holding this conversation is still good enough for what was asked.
    /// A conversation that started small must not keep a small model for work it would never
    /// have been chosen for.
    fn strong_enough(
        &self,
        session: &SessionRecord,
        task: &TaskDescriptor,
        policies: &Registry,
    ) -> bool {
        let Some(agent) = self.config.agents.iter().find(|a| a.id == session.agent) else {
            return true;
        };
        let rule = policies.get(agent.provider).model_rule(&session.model);
        rule.success_prior[task.complexity.index()] >= self.config.scheduler.required_success
    }

    /// Runs one turn and returns its exit code with the agent's reply.
    async fn execute(
        &mut self,
        message: Message,
        phase: Option<&Phase>,
        classified: Option<TaskDescriptor>,
    ) -> Result<(u8, String)> {
        let _lock = workspace_lock(
            self.data,
            &self.store.repository_id(self.root)?,
            self.config.scheduler.shared_workspace,
        )?;
        let policies = Registry::load(self.data)?;
        let mut overrides = self.options.overrides.clone();
        // One message may need something this conversation's agent cannot do (generating an
        // image, say). That one goes to an agent that can, and the conversation stays put.
        let guest = phase.is_none()
            && self
                .conversation
                .current
                .as_ref()
                .is_some_and(|c| !self.capable(&c.session.agent, &message.text));
        if guest {
            self.view
                .note("this needs an agent with other capabilities · handing it over");
            // The conversation's agent (even one the user asked for) cannot do this one.
            overrides = Overrides::default();
        }
        // Seats come from the task rather than from a fixed pairing: most turns are one
        // agent, and a big or unclear one gets a second, read-only seat beside it. The two
        // share this working tree and talk over the mailbox, so both must be able to reach it.
        // A phase carries text Orochi wrote for it and is routed on its own merits; every
        // other turn was already classified by `run_turn`, once, before any of this printed.
        let mut profile = match classified {
            Some(profile) => profile,
            None => match self.profile(&message.text).await {
                Some(profile) => profile,
                None => {
                    self.view.result(WARN, "Interrupted");
                    return Ok((130, String::new()));
                }
            },
        };
        // The seats of the last turn are gone; a follow-up to a discussion seats them again,
        // with what was said passed on as context, rather than answering into an empty room.
        if let Some((count, talking)) = self.conversation.panel.filter(|_| phase.is_none()) {
            profile.collaborative = true;
            profile.seats = profile.seats.or(Some(count));
            if talking {
                profile.task_type = "discussion".into();
            }
        }
        let seats = crate::router::roles::seats(&profile);
        let beside: Vec<Role> = if !guest
            && self.feed.is_some()
            // `/solo` asks for one agent; a step of a plan carries Solo for its own reason.
            && match phase {
                Some(phase) => phase.name == "implement",
                None => !matches!(message.steps, Steps::Solo),
            } {
            seats[1..].to_vec()
        } else {
            vec![]
        };
        let own_session = phase.is_none() && !guest;
        // A phase is routed on its own merits, in its own session.
        let mut resume = own_session
            .then(|| self.conversation.resume.take())
            .flatten();
        // The conversation's own model is kept only while it would still be picked for the
        // work; a follow-up that outgrows it is routed again rather than answered badly.
        let outgrown = own_session
            && self
                .conversation
                .current
                .as_ref()
                .is_some_and(|c| !self.strong_enough(&c.session, &profile, &policies));
        if outgrown {
            self.view
                .note("this needs more than the model in this conversation · picking again");
            resume = None;
        }
        if let Some(current) = self
            .conversation
            .current
            .as_ref()
            .filter(|_| own_session && !outgrown)
        {
            let session = &current.session;
            overrides = Overrides {
                agent: Some(session.agent.clone()),
                model: Some(session.model.clone()),
                reasoning: session.reasoning.clone(),
                mode: session.mode.clone(),
            };
            if current.loadable {
                resume = Some(session.session_id.clone());
            }
        }
        // Session modes are per model: only reuse one for the route it came from.
        let route = overrides
            .agent
            .as_deref()
            .zip(overrides.model.as_deref())
            .map(|(agent, model)| format!("{agent}/{model}"));
        if self.approval.mode.is_some() && (route.is_none() || route == self.approval.route) {
            overrides.mode = self.approval.mode.clone();
        }
        let mut task = if resume.is_some() || self.conversation.turns.is_empty() {
            message.text.clone()
        } else {
            format!(
                "Previous conversation (context):\n{}\nCurrent user request:\n{}",
                self.conversation.transcript(),
                message.text
            )
        };
        if let Some(pending) = self.conversation.pending.take() {
            task = format!("{pending}\n\n{task}");
        }
        // Inside a plan the seat keeps the step's name; on its own it takes the task's.
        let lead = phase.map_or(seats[0].name, |p| p.name);
        if !beside.is_empty() {
            task = format!("{task}\n\n{}", alongside(lead, &beside));
            let names: Vec<&str> = beside.iter().map(|role| role.name).collect();
            self.view.note(&format!(
                "beside {lead}: {} · read only, talking over the mailbox",
                names.join(", ")
            ));
        }
        let mut asides: Vec<(&'static str, mpsc::UnboundedReceiver<ExecutionEvent>)> = Vec::new();
        let mut aside_slots: Vec<Option<Continuation>> = Vec::new();
        let mut aside_runs = Vec::new();
        for role in &beside {
            let (sender, receiver) = mpsc::unbounded_channel();
            asides.push((role.name, receiver));
            aside_slots.push(None);
            aside_runs.push((
                role.clone(),
                beside_seat(lead, role, &seats, &message.text),
                sender,
            ));
        }
        let (events, mut received) = mpsc::unbounded_channel();
        let mut continuation = None;
        let mut finished: BTreeSet<&'static str> = BTreeSet::new();
        let mut reply = String::new();
        // The design's division of the work is for Orochi, which shows the order instead.
        let mut withhold = Withhold::new(phase.is_some_and(|p| p.name == PHASES[DESIGN].name));
        let mut pending: Option<Pending> = None;
        let mut signalled = false;
        let mut stopping = false;
        let mut exiting: Option<Instant> = None;
        let mut listening = true;
        let approval = &mut self.approval;
        let queue = &mut self.queue;
        let keyboard = &mut self.keyboard;
        let feed = &mut self.feed;
        let root = self.root;
        let mut screen = Screen::new(&mut self.view);
        // The lead takes its place first and each seat after the one before it, so each sees
        // what the earlier ones took and finds them in the mailbox, however long its own
        // discovery ran.
        let order = crate::mailbox::Order::new(1 + beside.len());
        let result = {
            let run = scheduler::run_turn(
                self.config,
                &policies,
                self.store,
                self.root,
                RunOptions {
                    task,
                    descriptor: Some(profile.clone()),
                    overrides,
                    dry_run: false,
                    json: false,
                    resume,
                    permission: approval.confirm,
                    interactive: true,
                    attachments: message.attachments,
                    peer: (!beside.is_empty() || phase.is_some()).then(|| lead.to_owned()),
                    verify: seats[0].writes,
                    // A discussion changes nothing in the repository; only the talking matters.
                    read_only: !seats[0].writes,
                    place: Some(order.place(0)),
                },
                Some(events),
                &mut continuation,
            );
            let (config, store, rules, order) = (self.config, self.store, &policies, &order);
            // Every other seat runs beside the lead, in this same working tree, reading only.
            let others: futures::stream::FuturesUnordered<_> = aside_runs
                .into_iter()
                .zip(aside_slots.iter_mut())
                .enumerate()
                .map(|(index, ((role, task, sender), slot))| async move {
                    let _ = scheduler::run_turn(
                        config,
                        rules,
                        store,
                        root,
                        RunOptions {
                            task,
                            descriptor: None,
                            overrides: Overrides::default(),
                            dry_run: false,
                            json: false,
                            resume: None,
                            permission: PermissionMode::Allow,
                            interactive: true,
                            attachments: vec![],
                            peer: Some(role.name.to_owned()),
                            verify: false,
                            read_only: true,
                            place: Some(order.place(index + 1)),
                        },
                        Some(sender),
                        slot,
                    )
                    .await;
                    role.name
                })
                .collect();
            let interrupt = tokio::signal::ctrl_c();
            let mut ticker = tokio::time::interval(Duration::from_millis(100));
            let mut mail = mail_ticker();
            #[cfg(unix)]
            let mut resized =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
            tokio::pin!(run, interrupt, others);
            loop {
                tokio::select! {
                    biased;
                    Some(event) = received.recv() => {
                        handle(event, &mut screen, &mut reply, &mut pending, &mut withhold);
                    }
                    Some((name, event)) = next_aside(&mut asides), if !asides.is_empty() => {
                        aside(event, &mut screen, name);
                    }
                    Some(name) = others.next(), if !others.is_empty() => {
                        finished.insert(name);
                        let text = screen.view.paint(MUTED, "· done");
                        screen.aside(name, &text);
                    }
                    _ = &mut interrupt, if !signalled => {
                        signalled = true;
                        screen.phase = "Stopping".into();
                    }
                    key = keyboard.next(), if listening => {
                        let Some(key) = key else { listening = false; continue };
                        if pending.is_some() {
                            answer(key, &mut pending, &mut screen);
                        } else if matches!(key, Key::Up) && !queue.is_empty() {
                            // As in Claude Code, Up takes back what is queued, to edit or drop.
                            take_back(queue, &mut screen);
                        } else {
                            let action = press(&mut screen.view.term, key, root);
                            let first_exit = exiting.take();
                            match action {
                                Action::Send(message) => enqueue(message, queue, approval, &mut screen),
                                Action::Cycle => approval.cycle(),
                                // Both stop the turn and keep what is typed; what is queued is
                                // sent next, as in Claude Code.
                                Action::Escape | Action::Interrupt => {
                                    screen.phase = "Stopping".into();
                                    raise_interrupt();
                                }
                                Action::Exit => {
                                    exiting = first_exit;
                                    if Twice::again(&mut exiting) {
                                        stopping = true;
                                        screen.phase = "Stopping".into();
                                        raise_interrupt();
                                    } else {
                                        screen.result(MUTED, "Press Ctrl-D again to exit");
                                    }
                                }
                                // A closed pipe or terminal just means no more input.
                                Action::Quit => {
                                    if screen.view.term.tty {
                                        stopping = true;
                                    } else {
                                        listening = false;
                                    }
                                }
                                Action::None => {}
                            }
                        }
                        screen.set_status(&approval.label(), queue.len());
                    }
                    _ = ticker.tick(), if screen.view.term.tty => {
                        screen.tick();
                        screen.set_status(&approval.label(), queue.len());
                    }
                    _ = mail.tick(), if feed.is_some() => {
                        screen.mail(feed.as_mut().expect("feed"));
                    }
                    _ = resized.recv() => screen.view.term.resized(),
                    result = &mut run => break result,
                }
            }
        };
        while let Ok(event) = received.try_recv() {
            handle(event, &mut screen, &mut reply, &mut pending, &mut withhold);
        }
        let held = withhold.finish();
        if !held.is_empty() {
            screen.text(&held);
        }
        for (name, receiver) in &mut asides {
            while let Ok(event) = receiver.try_recv() {
                aside(event, &mut screen, name);
            }
        }
        // The lead is done, so the seats beside it have nothing left to answer.
        for name in beside
            .iter()
            .map(|role| role.name)
            .filter(|name| !finished.contains(name))
        {
            let text = screen.view.paint(MUTED, "· stopped, the work is done");
            screen.aside(name, &text);
        }
        if let Some(feed) = feed.as_mut() {
            screen.mail(feed);
        }
        screen.done();
        // Only a turn holding the conversation can be judged by what the user does next; a guest
        // or a phase is answered for elsewhere. A message queued while it ran was written before
        // the answer was seen, so it is no verdict on it.
        self.awaiting = None;
        // Stopping a turn says only that something was wrong, not what: the user may have
        // forgotten to say something as easily as the agent may have gone astray. What they
        // do next tells which, so it waits for that too.
        if let Some(run) = continuation
            .as_ref()
            .filter(|_| !guest && phase.is_none() && self.queue.is_empty())
            .map(|c| c.run_id.clone())
        {
            self.awaiting = Some(Awaiting {
                run,
                interrupted: signalled,
            });
        }
        // Follow-up messages continue the phase that did the work; a guest turn changes nothing.
        let continues = !guest && phase.is_none_or(|p| p.name == "implement");
        if let Some(continuation) = continuation.filter(|_| continues) {
            self.approval.modes = continuation.modes.clone();
            let session = &continuation.session;
            let route = format!("{}/{}", session.agent, session.model);
            if !self.approval.chosen || self.approval.route.as_ref() != Some(&route) {
                self.approval.mode = session.mode.clone();
                self.approval.chosen = false;
            }
            self.approval.route = Some(route);
            self.conversation.current = Some(continuation);
        }
        if !reply.is_empty() && phase.is_none() {
            self.conversation.remember(&message.text, &reply);
        }
        if phase.is_none() {
            // Only a discussion keeps its table for the next message: work is seated per
            // message on its own merits, and `/solo` ends the table either way.
            self.conversation.panel = match message.steps {
                Steps::Solo => None,
                _ if !beside.is_empty() && !seats[0].writes => Some((seats.len(), true)),
                _ if !beside.is_empty() => None,
                _ => self.conversation.panel,
            };
        }
        if guest && !reply.is_empty() {
            // The agent holding the conversation did not see this turn; tell it next time.
            self.conversation.pending = Some(format!(
                "Another agent handled the previous request for the user:\n{}",
                bounded(&reply, 4000)
            ));
        }
        if stopping {
            self.queue.clear();
            self.quit = true;
        }
        let code = result?;
        let code = if signalled && !self.view.term.tty {
            130
        } else {
            code
        };
        Ok((code, reply))
    }
}

/// A message typed while the agent works waits its turn; `/confirm` still applies at once.
/// Moves every queued message back into the input, one per line, ahead of what is typed.
fn take_back(queue: &mut VecDeque<Message>, screen: &mut Screen<'_>) {
    let taken: Vec<Message> = queue.drain(..).collect();
    let text = taken
        .iter()
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = taken
        .into_iter()
        .flat_map(|message| message.attachments)
        .collect();
    screen.view.term.prompt.restore(&text, attachments);
    screen.view.term.render();
}

fn enqueue(
    message: Message,
    queue: &mut VecDeque<Message>,
    approval: &mut Approval,
    screen: &mut Screen<'_>,
) {
    if let Some(("confirm" | "permissions", argument)) = command(&message.text) {
        match argument {
            "ask" => approval.confirm = PermissionMode::Ask,
            "auto" | "always" | "allow" => approval.confirm = PermissionMode::Allow,
            "never" | "deny" => approval.confirm = PermissionMode::Deny,
            _ => {}
        }
        approval.mode = None;
        approval.chosen = true;
        screen.result(MUTED, &format!("Approval: {}", approval.label()));
        return;
    }
    queue.push_back(message);
    let text = queue.back().expect("queued message").text.clone();
    let line = screen.view.queued(&text, queue.len());
    screen.interject(line);
}

/// The next event from any seat running beside the lead.
async fn next_aside(
    seats: &mut [(&'static str, mpsc::UnboundedReceiver<ExecutionEvent>)],
) -> Option<(&'static str, ExecutionEvent)> {
    // A seat that has finished stays closed for good. Returning on it would hide every seat
    // after it until the lead is done, so it is skipped, and only all closed ends the stream.
    std::future::poll_fn(|cx| {
        let mut open = false;
        for (name, receiver) in seats.iter_mut() {
            match receiver.poll_recv(cx) {
                std::task::Poll::Ready(Some(event)) => {
                    return std::task::Poll::Ready(Some((*name, event)));
                }
                std::task::Poll::Ready(None) => {}
                std::task::Poll::Pending => open = true,
            }
        }
        if open {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(None)
        }
    })
    .await
}

/// What the lead is told about the seats beside it.
fn alongside(lead: &str, beside: &[Role]) -> String {
    let who: Vec<String> = beside
        .iter()
        .map(|role| format!("`{}` — {}", role.name, role.brief))
        .collect();
    let (count, plural) = (beside.len(), if beside.len() == 1 { "" } else { "s" });
    format!(
        "{count} other agent{plural} {} working beside you on this, right now, in this same \
         working tree:\n{}\n\nThey read everything and change nothing — every write tool is \
         refused for them — so they cannot collide with your work, and they are already \
         running: never start another agent or another Orochi yourself. Reach them with the \
         mailbox tools: send_message (to one name, or to \"all\") and read_messages, which waits \
         for a reply. You are `{lead}` to them, you own every change, and you tell them when the \
         work is done. Report back to the user yourself: what was said, what you concluded, and \
         what you did.\n\n{LANGUAGE}",
        if count == 1 { "is" } else { "are" },
        who.join("\n")
    )
}

/// The prompt for a seat beside the lead: same task, same tree, no way to change it.
fn beside_seat(lead: &str, role: &Role, seats: &[Role], text: &str) -> String {
    let others: Vec<String> = seats
        .iter()
        .filter(|seat| seat.name != role.name)
        .map(|seat| format!("`{}`", seat.name))
        .collect();
    format!(
        "You are `{name}`, one of {total} agents working on this at the same time, in one \
         repository. {brief}\n\nThe others are running right now: {others}. `{lead}` leads and \
         is the only one that may change anything; you cannot, every write tool is refused for \
         this session, deliberately. Never start another agent or another Orochi yourself — the \
         others are already there.\n\nWhat the user asked:\n{text}\n\nRead what you need, then \
         send_message with what you have — to `{lead}`, to another seat by name, or to \"all\" \
         when everyone should hear it. Short and concrete, no preamble, and answer what others \
         send you rather than repeating yourself. Then read_messages (it waits for a reply) and \
         keep going until `{lead}` says it is done. End your turn then, or once you have nothing \
         left worth saying.\n\n{LANGUAGE}",
        name = role.name,
        total = seats.len(),
        brief = role.brief,
        others = others.join(", ")
    )
}

/// The second seat speaks to the user through the messages it sends, which the feed shows as
/// chat; only where it is running, and what stopped it, belong in the transcript.
fn aside(event: ExecutionEvent, screen: &mut Screen<'_>, name: &str) {
    match event {
        // Refused at the source, but an agent that still asks is told no rather than the user.
        ExecutionEvent::Permission(_, answer) => {
            let _ = answer.send(None);
        }
        ExecutionEvent::Progress(Progress::Route {
            agent,
            provider,
            model,
            reasoning,
            ..
        }) => {
            let route = screen
                .view
                .route(&agent, Some(provider), &model, reasoning.as_deref());
            screen.aside(name, &route);
        }
        ExecutionEvent::Progress(Progress::Unavailable { agent, error }) => {
            let text = screen
                .view
                .paint(WARN, &format!("· {agent} unavailable: {error}"));
            screen.aside(name, &text);
        }
        _ => {}
    }
}

fn handle(
    event: ExecutionEvent,
    screen: &mut Screen<'_>,
    reply: &mut String,
    pending: &mut Option<Pending>,
    withhold: &mut Withhold,
) {
    match event {
        ExecutionEvent::Text(text) => {
            if reply.len() < AGENT_CHARS * 4 {
                reply.push_str(&text);
            }
            let shown = withhold.feed(&text);
            if !shown.is_empty() {
                screen.text(&shown);
            }
        }
        ExecutionEvent::Permission(request, answer) => {
            let Some(allow) = allow_once_option(&request) else {
                let _ = answer.send(None);
                return;
            };
            match screen.view.permission {
                PermissionMode::Allow => {
                    screen.result(MUTED, "Allowed (approval: auto)");
                    let _ = answer.send(Some(allow));
                }
                PermissionMode::Deny => {
                    screen.denied = true;
                    screen.result(WARN, "Denied (approval: never)");
                    let _ = answer.send(None);
                }
                PermissionMode::Ask if !screen.view.term.tty => {
                    screen.denied = true;
                    screen.result(
                        ERR,
                        "Permission requested without a terminal; use --permission allow or deny",
                    );
                    let _ = answer.send(None);
                }
                PermissionMode::Ask => {
                    screen.ask_permission(&request["toolCall"], 0);
                    *pending = Some(Pending {
                        answer,
                        allow,
                        selected: 0,
                        title: request["toolCall"]["title"]
                            .as_str()
                            .unwrap_or("Tool call")
                            .to_owned(),
                        call: request["toolCall"].clone(),
                    });
                }
            }
        }
        ExecutionEvent::Finished => {
            let held = withhold.finish();
            if !held.is_empty() {
                screen.text(&held);
            }
            screen.end_text();
        }
        ExecutionEvent::Progress(progress) => screen.progress(progress),
    }
}

/// What the user chose once a design divided the work.
enum Proceed {
    Team(Plan),
    Alone,
    Keep,
}

/// A design step's trailing division of the work, `{"parts": …}`, optionally fenced.
#[derive(serde::Deserialize)]
struct Division {
    parts: Value,
}

/// The design's reply without its division of the work, and the parts it proposed.
fn divided(reply: &str) -> (String, Option<Vec<Part>>) {
    let parts = crate::context::trailing_json::<Division>(reply)
        .and_then(|division| serde_json::from_value(division.parts).ok());
    let mut withhold = Withhold::new(true);
    let mut said = withhold.feed(reply);
    said.push_str(&withhold.finish());
    (said.trim_end().to_owned(), parts)
}

/// Keeps a design step's trailing `{"parts": …}` object (and any fence around it) out of the
/// transcript while it streams. Text that only looked like its start is shown as soon as that
/// is clear, and an object that turns out not to end the reply is shown at the end, so nothing
/// but the division itself is ever held back.
struct Withhold {
    active: bool,
    /// Unshown text; it always begins at the start of a line.
    held: String,
    /// The last text shown ended mid-line, so what follows cannot start the object.
    mid_line: bool,
    hiding: bool,
}
impl Withhold {
    const OPENING: &str = "{\"parts\"";
    fn new(active: bool) -> Self {
        Self {
            active,
            held: String::new(),
            mid_line: false,
            hiding: false,
        }
    }
    fn feed(&mut self, text: &str) -> String {
        if !self.active {
            return text.to_owned();
        }
        let mut shown = String::new();
        let mut text = text;
        if self.mid_line && !self.hiding {
            match text.find('\n') {
                Some(end) => {
                    shown.push_str(&text[..=end]);
                    text = &text[end + 1..];
                    self.mid_line = false;
                }
                None => return text.to_owned(),
            }
        }
        self.held.push_str(text);
        while !self.hiding {
            match self.probe() {
                Some(true) => self.hiding = true,
                // Could still become the object: wait for more.
                None => break,
                Some(false) => match self.held.find('\n') {
                    Some(end) => shown.extend(self.held.drain(..=end)),
                    None => {
                        shown.push_str(&std::mem::take(&mut self.held));
                        self.mid_line = !shown.is_empty();
                        break;
                    }
                },
            }
        }
        shown
    }
    /// Whether the held text opens the object (`Some(true)`), cannot (`Some(false)`), or may
    /// yet (`None`). Fence lines are looked through; models often wrap the object in ```json.
    fn probe(&self) -> Option<bool> {
        let mut compact = String::new();
        for line in self.held.split_inclusive('\n') {
            let trimmed = line.trim();
            let complete = line.ends_with('\n');
            if trimmed.starts_with("```")
                || !complete && !trimmed.is_empty() && "```".starts_with(trimmed)
            {
                continue;
            }
            compact.extend(trimmed.chars().filter(|c| !c.is_whitespace()));
            if compact.len() >= Self::OPENING.len() {
                break;
            }
        }
        if compact.starts_with(Self::OPENING) {
            Some(true)
        } else if Self::OPENING.starts_with(&compact) {
            None
        } else {
            Some(false)
        }
    }
    /// Whatever is still held once the reply is complete: nothing if it was the division.
    fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        let division = self.hiding && crate::context::trailing_json::<Division>(&held).is_some();
        self.hiding = false;
        self.mid_line = false;
        if division { String::new() } else { held }
    }
}

/// Answers the permission panel: arrows move, 1/2/3 and y/a/n choose, Esc is No.
fn answer(key: Key, slot: &mut Option<Pending>, screen: &mut Screen<'_>) {
    let dialog = slot.as_mut().expect("pending permission");
    let choice = match key {
        Key::Up => {
            dialog.selected = (dialog.selected + CHOICES.len() - 1) % CHOICES.len();
            None
        }
        Key::Down => {
            dialog.selected = (dialog.selected + 1) % CHOICES.len();
            None
        }
        Key::Enter => Some(dialog.selected),
        Key::Char('1' | 'y' | 'Y') => Some(0),
        Key::Char('2' | 'a' | 'A') => Some(1),
        Key::Char('3' | 'n' | 'N') => Some(NO),
        Key::Escape | Key::Interrupt => Some(NO),
        _ => None,
    };
    let Some(choice) = choice else {
        let (selected, call) = (dialog.selected, dialog.call.clone());
        screen.ask_permission(&call, selected);
        return;
    };
    let dialog = slot.take().expect("pending permission");
    // Answered: the input area comes back and one line records the decision.
    screen.view.term.overlay = None;
    let (code, answer) = match choice {
        0 => (OK, "allowed once"),
        1 => (OK, "allowed · approval is now auto"),
        _ => (WARN, "denied"),
    };
    match choice {
        // No without a word stops the turn, as in Claude Code: an agent told only "no" would
        // otherwise go looking for another way to do the same thing.
        NO => {
            screen.denied = true;
            screen.phase = "Stopping".into();
            raise_interrupt();
        }
        1 => screen.view.permission = PermissionMode::Allow,
        _ => {}
    }
    let limit = screen.view.width().saturating_sub(30);
    let line = format!(
        "{} {} {} {}",
        screen.view.paint(code, "⏺"),
        screen.view.paint(BOLD, &fit(&dialog.title, limit)),
        screen.view.paint(MUTED, "→"),
        screen.view.paint(code, answer)
    );
    screen.begin();
    screen.view.line(&line);
    let _ = dialog.answer.send((choice != NO).then_some(dialog.allow));
}

#[derive(Default)]
struct Conversation {
    current: Option<Continuation>,
    resume: Option<String>,
    /// How many seats the last turn had, and whether they were only talking. The seats end
    /// with the turn, so a follow-up has to seat them again or it is talking to nobody.
    panel: Option<(usize, bool)>,
    /// Something the next turn should know, such as what another agent just did.
    pending: Option<String>,
    /// Kept in memory only, for agents without `session/load` and for `/reroute`.
    turns: VecDeque<(String, String)>,
}
impl Conversation {
    fn remember(&mut self, user: &str, agent: &str) {
        self.turns
            .push_back((bounded(user, USER_CHARS), bounded(agent, AGENT_CHARS)));
        while self.turns.len() > 1
            && self
                .turns
                .iter()
                .map(|(u, a)| u.len() + a.len())
                .sum::<usize>()
                > TRANSCRIPT_BYTES
        {
            self.turns.pop_front();
        }
    }
    fn transcript(&self) -> String {
        self.turns
            .iter()
            .map(|(user, agent)| format!("User: {user}\nAgent: {agent}\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// One meaning per color: brand (logo, prompt, spinner), agent replies, success, failure,
// attention, and grey for secondary information.
const BRAND: &str = "38;5;204";
const AGENT: &str = "38;5;75";
const OK: &str = "38;5;78";
const ERR: &str = "38;5;203";
const WARN: &str = "38;5;221";
const MUTED: &str = "38;5;245";
const CODE: &str = "38;5;180";
const BOLD: &str = "1";

fn provider_color(provider: Provider) -> &'static str {
    match provider {
        Provider::Openai => "38;5;79",
        Provider::Anthropic => "38;5;209",
        Provider::Google => "38;5;111",
    }
}

fn confirm_label(permission: PermissionMode) -> &'static str {
    match permission {
        PermissionMode::Ask => "ask",
        PermissionMode::Allow => "auto",
        PermissionMode::Deny => "never",
    }
}

fn home(path: &Path) -> String {
    let text = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => {
            format!("~{}", &text[home.len()..])
        }
        _ => text,
    }
}

fn ago(time: i64) -> String {
    match now().saturating_sub(time) {
        ..60 => "just now".into(),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

/// Session-wide terminal state: colors, the pinned input and what was already shown.
struct View {
    term: Term,
    permission: PermissionMode,
    last_route: Option<String>,
    warned: BTreeSet<String>,
}
impl View {
    fn new(tty: bool) -> Self {
        let color = tty
            && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
            && std::env::var("TERM").map_or(true, |t| t != "dumb");
        let marker = if color {
            format!("\x1b[1;{BRAND}m{PROMPT}\x1b[0m")
        } else {
            PROMPT.to_owned()
        };
        Self {
            term: Term::start(tty, color, marker),
            permission: PermissionMode::Ask,
            last_route: None,
            warned: BTreeSet::new(),
        }
    }
    fn paint(&self, code: &str, text: &str) -> String {
        if self.term.color && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }
    fn width(&self) -> usize {
        self.term.columns()
    }
    /// One transcript line (notes, results, headers) above the pinned input.
    fn line(&mut self, text: &str) {
        self.term.note(&format!("{text}\n"));
    }
    fn rows(&mut self, rows: Vec<String>) {
        for row in rows {
            self.line(&row);
        }
        self.line("");
    }
    fn note(&mut self, text: &str) {
        self.result(MUTED, text);
    }
    /// `⎿ text`, wrapped rather than cut so an error stays readable.
    fn result(&mut self, code: &str, text: &str) {
        let limit = self.width().saturating_sub(6).max(20);
        let mut rest = text.replace('\n', " ");
        let mut lead = "⎿ ";
        for _ in 0..6 {
            if rest.is_empty() {
                return;
            }
            let row = if visible_width(&rest) > limit {
                wrap(&mut rest, limit)
            } else {
                std::mem::take(&mut rest)
            };
            let row = self.paint(code, &format!("{lead}{row}"));
            self.line(&format!("  {row}"));
            lead = "  ";
        }
        if !rest.is_empty() {
            let row = self.paint(MUTED, "  …");
            self.line(&format!("  {row}"));
        }
    }
    /// Echoes a submitted message into the transcript, since the input line is reused.
    fn sent(&mut self, text: &str, attachments: &[Attachment]) {
        let limit = self.width().saturating_sub(4).max(20);
        for (index, line) in text.lines().enumerate() {
            let marker = if index == 0 { PROMPT } else { "  " };
            let line = format!("{marker}{}", fit(line, limit));
            let line = self.paint(BOLD, &line);
            self.line(&line);
        }
        for attachment in attachments {
            let text = format!(
                "  ⎿ {} {} ({})",
                attachment.tag,
                attachment.name,
                term::size(attachment.bytes)
            );
            let line = self.paint(MUTED, &text);
            self.line(&line);
        }
    }
    /// A `/team` step header, so it is clear which agent is about to work.
    fn phase(&mut self, name: &str, step: usize, total: usize) {
        self.line("");
        let line = format!(
            "{} {} {}",
            self.paint(BRAND, "⏵"),
            self.paint(BOLD, name),
            self.paint(MUTED, &format!("step {step}/{total}"))
        );
        self.line(&line);
    }
    fn queued(&self, text: &str, position: usize) -> String {
        let limit = self.width().saturating_sub(16).max(20);
        let text = format!("⏸ queued ({position}): {}", fit(text, limit));
        self.paint(MUTED, &text)
    }
    fn route(
        &self,
        agent: &str,
        provider: Option<Provider>,
        model: &str,
        reasoning: Option<&str>,
    ) -> String {
        let agent = match provider {
            Some(provider) => self.paint(&format!("1;{}", provider_color(provider)), agent),
            None => self.paint(BOLD, agent),
        };
        let rest = match reasoning {
            Some(level) => format!(" · {model} · {level}"),
            None => format!(" · {model}"),
        };
        format!("{agent}{}", self.paint(MUTED, &rest))
    }

    fn welcome(&mut self, config: &Config, data: &Path, root: &Path, feed: &Option<Feed>) {
        let compact = self.term.tty && self.term.rows() < 27;
        let mut out = String::new();
        let agents: Vec<_> = config
            .agents
            .iter()
            .filter(|a| a.enabled && !a.routing_only)
            .filter(|a| {
                matches!(
                    crate::discovery::inspect(a, &config.discovery, data).status,
                    "ready" | "adapter_required"
                )
            })
            .map(|a| self.paint(&format!("1;{}", provider_color(a.provider)), &a.id))
            .collect();
        let ready = if agents.is_empty() {
            self.paint(WARN, "No agent is ready · run `orochi status`")
        } else {
            self.paint(OK, "All set up!")
        };
        out.push_str(&format!(
            "  {} {}\n",
            self.paint(BOLD, &format!("Orochi v{} ·", env!("CARGO_PKG_VERSION"))),
            ready
        ));
        let mut facts = vec![self.paint(MUTED, &fit(&home(root), self.width().saturating_sub(40)))];
        if !agents.is_empty() {
            facts.push(format!(
                "{} {}",
                self.paint(MUTED, "agents"),
                agents.join(", ")
            ));
        }
        out.push_str(&format!("  {}\n", facts.join(&self.paint(MUTED, " · "))));
        if let Some(feed) = feed {
            let others: Vec<_> = feed
                .peers
                .values()
                .map(|p| self.peer(&p.name, &feed.mine, true))
                .collect();
            let mut line = format!(
                "{} {}",
                self.paint(MUTED, "mailbox"),
                self.paint(
                    &format!("1;{BRAND}"),
                    &crate::mailbox::name_prefix().unwrap_or_else(|| "-".into())
                )
            );
            if !others.is_empty() {
                line.push_str(&format!(
                    " {} {}",
                    self.paint(MUTED, "· with"),
                    others.join(", ")
                ));
            }
            out.push_str(&format!("  {line}\n"));
        }
        if !compact {
            out.push('\n');
        }
        out.push_str(&format!(
            "  {}\n",
            self.paint(&format!("1;{BRAND}"), "What do you want to build?")
        ));
        if compact {
            out.push_str(&format!(
                "  {}\n",
                self.paint(
                    MUTED,
                    &fit(
                        "/ commands · @ files · Shift-Tab approvals",
                        self.width().saturating_sub(4)
                    )
                )
            ));
        } else {
            for (index, line) in [
                "Ask questions, edit files, or run commands.",
                "Orochi picks the agent; follow-ups stay in its session.",
                "Type / for commands, @ to attach a file, Shift-Tab for approval mode.",
            ]
            .iter()
            .enumerate()
            {
                out.push_str(&format!(
                    "  {} {}\n",
                    self.paint(MUTED, &format!("{}.", index + 1)),
                    fit(line, self.width().saturating_sub(5))
                ));
            }
        }
        out.push('\n');
        // Account for wrapped session details as well as the pinned input/status and
        // the trailing newline, so the serpent's heads stay on screen at startup.
        let footer_lines: usize = out
            .lines()
            .map(|line| visible_width(line).max(1).div_ceil(self.width().max(1)))
            .sum();
        let banner_lines = if self.term.tty {
            self.term.rows().saturating_sub(footer_lines + 3)
        } else {
            usize::MAX
        };
        out.insert_str(
            0,
            &banner::render(self.width(), banner_lines, self.term.color),
        );
        self.term.note(&out);
    }

    fn help(&mut self) {
        let mut rows = vec![self.paint(BOLD, "  Commands")];
        for c in &COMMANDS {
            let mut name = format!("/{} {}", c.name, c.usage);
            for alias in c.aliases {
                name.push_str(&format!(" /{alias}"));
            }
            rows.push(format!(
                "    {}  {}",
                self.paint(BRAND, &format!("{name:<30}")),
                self.paint(MUTED, c.about)
            ));
        }
        rows.push(String::new());
        rows.push(self.paint(BOLD, "  Shortcuts"));
        for (key, about) in SHORTCUTS {
            rows.push(format!(
                "    {}  {}",
                self.paint(BRAND, &format!("{key:<30}")),
                self.paint(MUTED, about)
            ));
        }
        self.rows(rows);
    }

    fn confirm_help(&self, current: PermissionMode) -> Vec<String> {
        let mut rows = vec![format!(
            "  {:<13}{}",
            "Approval",
            self.paint(BOLD, confirm_label(current))
        )];
        for (mode, about) in [
            (
                "auto",
                "Answer every request with a one-time allow (the default)",
            ),
            ("ask", "Ask you before each tool the agent wants to run"),
            ("never", "Deny every request"),
        ] {
            rows.push(format!(
                "    {}  {}",
                self.paint(BRAND, &format!("/confirm {mode:<8}")),
                self.paint(MUTED, about)
            ));
        }
        rows
    }

    fn session_status(&self, conversation: &Conversation, root: &Path) -> Vec<String> {
        let row = |label: &str, value: &str| {
            format!(
                "  {label:<13}{}",
                self.paint(MUTED, &fit(value, self.width().saturating_sub(15)))
            )
        };
        let mut rows = vec![row("Directory", &home(root))];
        match &conversation.current {
            Some(current) => {
                let session = &current.session;
                rows.push(format!(
                    "  {:<13}{}",
                    "Agent",
                    self.route(
                        &session.agent,
                        None,
                        &session.model,
                        session.reasoning.as_deref()
                    )
                ));
                rows.push(row("Session", &session.session_id));
                rows.push(row(
                    "Continues",
                    if current.loadable {
                        "by reloading the agent session"
                    } else {
                        "by sending earlier messages as context"
                    },
                ));
            }
            None => rows.push(row(
                "Agent",
                match &conversation.resume {
                    Some(_) => "the next message continues the recorded session",
                    None => "chosen when you send the next message",
                },
            )),
        }
        rows.push(row(
            "Context",
            &format!(
                "{} earlier exchange(s) kept in memory",
                conversation.turns.len()
            ),
        ));
        rows.push(row("Approval", confirm_label(self.permission)));
        rows
    }

    fn session_rows(&self, sessions: &[crate::types::SessionRecord]) -> Vec<String> {
        if sessions.is_empty() {
            return vec![self.paint(MUTED, "  ⎿ No conversations recorded in this directory yet")];
        }
        let mut rows = Vec::new();
        for (index, s) in sessions.iter().enumerate() {
            let (code, outcome) = match s.outcome {
                Outcome::Success => (OK, "checks passed"),
                Outcome::PartialSuccess => (MUTED, "unverified"),
                Outcome::Failure => (ERR, "failed"),
                Outcome::Cancelled => (WARN, "interrupted"),
            };
            rows.push(format!(
                "  {} {}  {}  {}",
                self.paint(BRAND, &format!("{:>2}.", index + 1)),
                self.route(&s.agent, None, &s.model, s.reasoning.as_deref()),
                self.paint(MUTED, &ago(s.updated_at)),
                self.paint(code, outcome)
            ));
            rows.push(format!("      {}", self.paint(MUTED, &s.session_id)));
        }
        rows.push(self.paint(MUTED, "  ⎿ /resume <n> continues one of these"));
        rows
    }
}

#[derive(Default, Clone)]
struct ToolState {
    title: Option<String>,
    detail: Option<String>,
    output: Option<String>,
    diffs: Vec<FileDiff>,
    shown: bool,
    done: bool,
}

#[derive(Default, Clone, Copy)]
struct Inline {
    bold: bool,
    code: bool,
}

#[derive(PartialEq)]
enum Live {
    Nothing,
    /// Part of a line of agent text, not yet styled.
    Text,
    /// A running tool's row, directly above the cursor, updated in place when it finishes.
    Tool(String),
}

/// One turn's transcript. Agent text goes to stdout; everything else to stderr.
struct Screen<'v> {
    view: &'v mut View,
    started: Instant,
    phase: String,
    frame: usize,
    printed: bool,
    live: Live,
    denied: bool,
    failures: usize,
    // Agent text streams raw, then each finished row is redrawn with markdown styling.
    in_text: bool,
    first_row: bool,
    partial: String,
    shown: usize,
    continued: bool,
    fence: bool,
    inline: Inline,
    raw_open: bool,
    thought: String,
    tools: BTreeMap<String, ToolState>,
    order: Vec<String>,
    plan: Vec<(String, String)>,
    /// Lines waiting for the row the agent is writing to end.
    postponed: Vec<String>,
    usage: Usage,
    route: Option<String>,
}
impl<'v> Screen<'v> {
    fn new(view: &'v mut View) -> Self {
        Self {
            view,
            started: Instant::now(),
            phase: "Routing".into(),
            frame: 0,
            printed: false,
            live: Live::Nothing,
            denied: false,
            failures: 0,
            in_text: false,
            first_row: false,
            partial: String::new(),
            shown: 0,
            continued: false,
            fence: false,
            inline: Inline::default(),
            raw_open: false,
            thought: String::new(),
            tools: BTreeMap::new(),
            postponed: Vec::new(),
            order: vec![],
            plan: vec![],
            usage: Usage::default(),
            route: None,
        }
    }

    fn out(&mut self, text: &str) {
        self.view.term.write(text);
    }
    fn set_status(&mut self, approval: &str, queued: usize) {
        // A question owns the bottom rows until it is answered.
        if self.view.term.overlay.is_some() {
            return;
        }
        let mut parts = vec![format!("{} {approval} (shift+tab)", self.spinner())];
        if queued > 0 {
            parts.push(format!("{queued} queued"));
        }
        parts.push(format!(
            "{} ({}s) · esc to interrupt",
            self.phase,
            self.started.elapsed().as_secs()
        ));
        let status = fit(&parts.join(" · "), self.view.width().saturating_sub(1));
        self.view.term.status = self.view.paint(MUTED, &status);
        self.view.term.render();
    }
    fn tick(&mut self) {
        self.frame += 1;
    }
    fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[self.frame % FRAMES.len()]
    }
    /// Finishes whatever is on the current row so another item can be printed.
    fn settle(&mut self) {
        if !self.partial.is_empty() {
            let rest = std::mem::take(&mut self.partial);
            self.finish_row(&rest, false);
        }
        if std::mem::take(&mut self.raw_open) {
            self.out("\n");
        }
        self.flush_postponed();
        self.flush_thought();
        self.live = Live::Nothing;
    }
    /// Starts a transcript item, separated from the previous one by a blank line.
    fn begin(&mut self) {
        self.settle();
        self.end_block();
        if self.printed && self.view.term.tty {
            self.view.line("");
        }
        self.printed = true;
    }
    fn end_block(&mut self) {
        self.in_text = false;
        self.fence = false;
        self.continued = false;
        self.inline = Inline::default();
    }
    fn result(&mut self, code: &str, text: &str) {
        self.settle();
        self.view.result(code, text);
    }

    /// A line of Orochi's own that belongs between two of the agent's, such as a queued
    /// message. It waits for the row being written to end instead of landing on top of it.
    fn interject(&mut self, line: String) {
        self.interject_block(format!("{line}\n"));
    }
    /// Same, for something several rows tall such as a message from another agent.
    fn interject_block(&mut self, block: String) {
        self.postponed.push(block);
        self.flush_postponed();
    }
    fn flush_postponed(&mut self) {
        if self.postponed.is_empty() || !self.partial.is_empty() && self.view.term.tty {
            return;
        }
        // A running tool's row is reprinted when it finishes, so drop it rather than leave
        // it above with its finished copy below.
        if let Live::Tool(_) = &self.live {
            self.view.term.step_back(1);
            self.live = Live::Nothing;
        }
        if !self.in_text && self.printed {
            self.view.line("");
        }
        for block in std::mem::take(&mut self.postponed) {
            self.view.term.note(&block);
        }
        self.printed = true;
    }

    fn text(&mut self, text: &str) {
        let text = text.replace('\u{1b}', "\\x1b");
        if !self.view.term.tty {
            self.out(&text);
            self.raw_open = !text.ends_with('\n');
            return;
        }
        if !self.thought.is_empty() {
            self.settle();
        }
        if !self.in_text {
            if text.trim().is_empty() {
                return;
            }
            self.begin();
            self.in_text = true;
            self.first_row = true;
        }
        self.phase = "Writing".into();
        self.partial.push_str(&text);
        let limit = width().saturating_sub(3).max(20);
        loop {
            if let Some(end) = self.partial.find('\n') {
                let line: String = self.partial.drain(..=end).collect();
                self.finish_row(line.trim_end_matches(['\n', '\r']), false);
            } else if visible_width(&self.partial) > limit {
                let segment = wrap(&mut self.partial, limit);
                self.finish_row(&segment, true);
            } else {
                break;
            }
        }
        self.flush_postponed();
        if self.partial.len() > self.shown {
            let mut raw = String::new();
            if self.shown == 0 {
                raw.push_str(&self.row_prefix());
            }
            raw.push_str(&self.partial[self.shown..]);
            self.out(&raw);
            self.shown = self.partial.len();
            self.live = Live::Text;
        }
    }
    fn row_prefix(&self) -> String {
        if self.first_row {
            format!("{} ", self.view.paint(AGENT, "⏺"))
        } else {
            "  ".into()
        }
    }
    /// Redraws the current row with styling and moves to the next one.
    fn finish_row(&mut self, line: &str, wrapped: bool) {
        if self.first_row && self.shown == 0 && line.trim().is_empty() {
            return;
        }
        let continued = std::mem::replace(&mut self.continued, wrapped);
        let body = self.style_line(line, continued);
        let prefix = if line.is_empty() {
            String::new()
        } else {
            self.row_prefix()
        };
        self.view.term.rewind();
        self.out(&format!("{prefix}{body}\n"));
        self.first_row = false;
        self.shown = 0;
        self.live = Live::Nothing;
    }
    fn end_text(&mut self) {
        self.settle();
        self.end_block();
    }
    fn style_line(&mut self, line: &str, continued: bool) -> String {
        let trimmed = line.trim_start();
        if !continued && trimmed.starts_with("```") {
            self.fence = !self.fence;
            self.view.paint(MUTED, line)
        } else if self.fence {
            self.view.paint(CODE, line)
        } else if continued {
            self.styled(line)
        } else if let Some(heading) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("# "))
        {
            self.inline = Inline::default();
            self.view.paint(&format!("1;{BRAND}"), heading)
        } else if let Some(item) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|marker| trimmed.strip_prefix(marker))
        {
            let indent = &line[..line.len() - trimmed.len()];
            let item = self.styled(item);
            format!("{indent}{} {item}", self.view.paint(MUTED, "-"))
        } else if let Some(quote) = trimmed.strip_prefix("> ") {
            self.view.paint(MUTED, quote)
        } else if !trimmed.is_empty() && trimmed.chars().all(|c| c == '-' || c == '─') {
            self.view.paint(MUTED, line)
        } else {
            self.styled(line)
        }
    }
    /// `**bold**` and `` `code` `` spans, carried across wrapped rows.
    fn styled(&mut self, text: &str) -> String {
        if !self.view.term.color {
            return text.to_owned();
        }
        let style = |state: Inline| {
            let mut codes = vec!["0"];
            if state.bold {
                codes.push(BOLD);
            }
            if state.code {
                codes.push(CODE);
            }
            format!("\x1b[{}m", codes.join(";"))
        };
        let mut out = style(self.inline);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '`' {
                self.inline.code = !self.inline.code;
                out.push_str(&style(self.inline));
            } else if c == '*' && chars.peek() == Some(&'*') && !self.inline.code {
                chars.next();
                self.inline.bold = !self.inline.bold;
                out.push_str(&style(self.inline));
            } else {
                out.push(c);
            }
        }
        out.push_str("\x1b[0m");
        out
    }

    /// Messages arrive while the agent is writing: they go between its rows, and its own
    /// answer carries on afterwards rather than starting again as a new block.
    fn mail(&mut self, feed: &mut Feed) {
        for event in feed.poll() {
            let text = self.view.mail(&event, &feed.mine, &feed.routes);
            self.interject_block(text);
        }
    }

    fn thinking(&mut self, text: &str) {
        self.phase = "Thinking".into();
        if self.thought.len() < 4096 {
            self.thought.push_str(text);
        }
    }
    /// Shows the gist of a finished reasoning block (its first line) as its own item.
    fn flush_thought(&mut self) {
        let thought = std::mem::take(&mut self.thought);
        let Some(gist) = thought
            .lines()
            .map(|l| l.trim().trim_matches('*').trim())
            .find(|l| !l.is_empty())
        else {
            return;
        };
        if !self.view.term.tty {
            return;
        }
        self.end_block();
        if self.printed {
            self.view.line("");
        }
        self.printed = true;
        let line = format!(
            "{} {}",
            self.view.paint(MUTED, "✻"),
            self.view.paint(
                &format!("3;{MUTED}"),
                &fit(
                    &gist.replace('\u{1b}', "\\x1b"),
                    self.view.width().saturating_sub(4)
                )
            )
        );
        self.view.line(&line);
    }

    /// One line about the seat beside this turn; its own output stays out of the transcript.
    fn aside(&mut self, name: &str, text: &str) {
        let line = format!(
            "  {} {text}",
            self.view.paint(&peer_color(name), &format!("⎿ {name}"))
        );
        self.interject(line);
    }

    fn progress(&mut self, progress: Progress) {
        match progress {
            Progress::Thinking(text) => self.thinking(&text),
            Progress::Tool(update) => self.tool(update),
            Progress::Plan(entries) => self.show_plan(entries),
            Progress::Route {
                agent,
                provider,
                model,
                reasoning,
                ..
            } => {
                let label = format!("{agent} · {model}");
                self.route = Some(label.clone());
                let changed = self.view.last_route.as_ref() != Some(&label);
                if changed || self.failures > 0 {
                    let route =
                        self.view
                            .route(&agent, Some(provider), &model, reasoning.as_deref());
                    self.settle();
                    let lead = match (self.failures > 0, changed) {
                        (true, true) => "⎿ Trying ",
                        (true, false) => "⎿ Retrying with ",
                        _ => "⎿ ",
                    };
                    let line = format!("  {}{route}", self.view.paint(MUTED, lead));
                    self.view.line(&line);
                    self.view.last_route = Some(label);
                }
                self.phase = "Working".into();
            }
            Progress::Unavailable { agent, error } => {
                if self.view.warned.insert(format!("{agent}\n{error}")) {
                    self.result(WARN, &format!("⚠ {agent} unavailable: {error}"));
                }
            }
            Progress::Note(text) => {
                if self.view.warned.insert(text.clone()) {
                    self.result(MUTED, &format!("⎿ {text}"));
                }
            }
            Progress::Checking => self.phase = "Running checks".into(),
            Progress::Attempt {
                outcome,
                checks,
                error,
                usage,
            } => {
                for (total, part) in [
                    (&mut self.usage.input_tokens, usage.input_tokens),
                    (&mut self.usage.output_tokens, usage.output_tokens),
                    (&mut self.usage.cached_tokens, usage.cached_tokens),
                    (&mut self.usage.total_tokens, usage.total()),
                ] {
                    if let Some(part) = part {
                        *total = Some(total.unwrap_or(0) + part);
                    }
                }
                self.attempt(outcome, &checks, error);
            }
        }
    }

    fn attempt(
        &mut self,
        outcome: Outcome,
        checks: &[crate::types::CheckResult],
        error: Option<String>,
    ) {
        if outcome == Outcome::Failure {
            self.failures += 1;
        }
        let names = |passed: bool| {
            checks
                .iter()
                .filter(|c| c.passed == passed)
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (code, text) = match (outcome, error) {
            (Outcome::PartialSuccess, _) => return,
            (Outcome::Success, _) => (OK, format!("✓ Checks passed: {}", names(true))),
            (Outcome::Cancelled, _) if self.denied => (
                WARN,
                "Stopped because the request was denied · say what to do instead".into(),
            ),
            (Outcome::Cancelled, _) => (WARN, "Interrupted".into()),
            (Outcome::Failure, Some(error)) => (ERR, format!("✗ {}", friendly(&error))),
            (Outcome::Failure, None) if checks.iter().any(|c| !c.passed) => {
                (ERR, format!("✗ Checks failed: {}", names(false)))
            }
            (Outcome::Failure, None) => {
                (ERR, "✗ The agent stopped before finishing its turn".into())
            }
        };
        self.begin();
        self.view.result(code, &text);
    }

    fn tool(&mut self, update: ToolUpdate) {
        let id = update.id.clone();
        if !self.tools.contains_key(&id) {
            self.order.push(id.clone());
        }
        let state = self.tools.entry(id.clone()).or_default();
        if update
            .title
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            state.title = update.title;
        }
        if update.detail.is_some() {
            state.detail = update.detail;
        }
        if update.output.is_some() {
            state.output = update.output;
        }
        if !update.diffs.is_empty() {
            state.diffs = update.diffs;
        }
        let finished = match update.status.as_deref() {
            Some("completed") => Some(false),
            Some("failed") => Some(true),
            _ => None,
        };
        // Mailbox traffic is shown as chat messages; its tool calls only set the status.
        if let Some(activity) = mailbox_activity(state) {
            if finished.is_some() {
                state.done = true;
            }
            let snapshot = state.clone();
            match finished {
                Some(true) => self.finish_tool(&id, &snapshot, Some(true)),
                Some(false) => self.phase = "Working".into(),
                None => self.phase = activity.into(),
            }
            return;
        }
        match finished {
            Some(failed) if !state.done => {
                state.done = true;
                let snapshot = state.clone();
                self.finish_tool(&id, &snapshot, Some(failed));
                self.phase = "Working".into();
            }
            Some(_) => {}
            None => {
                // Only one running row at a time: a row that is not the last one printed
                // cannot be replaced in place, and would be left behind when the tool ends.
                let show = !state.shown && self.view.term.tty && self.live == Live::Nothing;
                state.shown = true;
                let snapshot = state.clone();
                self.phase = format!("Running {}", fit(&tool_label(&snapshot), 48));
                if show {
                    self.begin();
                    let limit = self.view.width().saturating_sub(4);
                    let line = format!(
                        "{} {}",
                        self.view.paint(MUTED, "⏺"),
                        self.view.paint(MUTED, &fit(&tool_label(&snapshot), limit))
                    );
                    self.view.line(&line);
                    self.live = Live::Tool(id);
                }
            }
        }
    }

    /// Replaces the tool's running row in place when it is still the last one printed.
    fn finish_tool(&mut self, id: &str, state: &ToolState, failed: Option<bool>) {
        let replace = self.live == Live::Tool(id.to_owned());
        if replace {
            self.live = Live::Nothing;
            self.view.term.step_back(1);
        } else {
            self.begin();
        }
        self.tool_row(state, failed);
    }

    fn tool_heading(&self, state: &ToolState, limit: usize) -> String {
        let title = state
            .title
            .as_deref()
            .map(|t| t.trim().trim_matches('`').to_owned());
        let bold = |text: &str| self.view.paint(BOLD, &fit(text, limit));
        match (title, state.detail.as_deref()) {
            (Some(title), Some(detail))
                if detail == title || detail.strip_prefix("$ ") == Some(title.as_str()) =>
            {
                bold(detail)
            }
            (Some(title), Some(detail)) => {
                let title = fit(&title, limit);
                let rest = limit.saturating_sub(visible_width(&title));
                format!(
                    "{}{}",
                    self.view.paint(BOLD, &title),
                    self.view.paint(MUTED, &fit(&format!(": {detail}"), rest))
                )
            }
            (Some(title), None) => bold(&title),
            (None, Some(detail)) => bold(detail),
            (None, None) => bold("Tool"),
        }
    }

    /// A finished (or, with `None`, abandoned) tool: status mark, then its result.
    fn tool_row(&mut self, state: &ToolState, failed: Option<bool>) {
        let limit = width().saturating_sub(18).max(20);
        let (dot, mark) = match failed {
            Some(false) => (self.view.paint(OK, "⏺"), self.view.paint(OK, " ✓")),
            Some(true) => (self.view.paint(ERR, "⏺"), self.view.paint(ERR, " ✗")),
            None => (
                self.view.paint(WARN, "⏺"),
                self.view.paint(WARN, " (not finished)"),
            ),
        };
        let line = format!("{dot} {}{mark}", self.tool_heading(state, limit));
        self.view.line(&line);
        for diff in state.diffs.iter().take(3) {
            self.print_diff(diff);
        }
        let lines: Vec<&str> = state
            .output
            .as_deref()
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).collect())
            .unwrap_or_default();
        if let Some(first) = lines.first() {
            let more = if lines.len() > 1 {
                format!(" … +{} lines", lines.len() - 1)
            } else {
                String::new()
            };
            let code = if failed == Some(true) { ERR } else { MUTED };
            self.view
                .result(code, &format!("{}{more}", first.replace('\u{1b}', "\\x1b")));
        }
    }

    fn print_diff(&mut self, diff: &FileDiff) {
        let path = diff.path.replace('\u{1b}', "\\x1b");
        let old = diff.old.as_deref().unwrap_or("");
        if old.is_empty() {
            self.view.result(
                MUTED,
                &format!("Created {path} ({} lines)", diff.new.lines().count()),
            );
            return;
        }
        let Some(ops) = line_diff(old, &diff.new) else {
            self.view.result(MUTED, &format!("Updated {path}"));
            return;
        };
        let added = ops.iter().filter(|(op, _)| *op == '+').count();
        let removed = ops.iter().filter(|(op, _)| *op == '-').count();
        self.view.result(
            MUTED,
            &format!(
                "Updated {path} with {added} addition{} and {removed} removal{}",
                if added == 1 { "" } else { "s" },
                if removed == 1 { "" } else { "s" }
            ),
        );
        let changes: Vec<_> = ops.iter().filter(|(op, _)| *op != ' ').collect();
        let limit = self.view.width().saturating_sub(8);
        for (op, text) in changes.iter().take(8) {
            let code = if *op == '+' { OK } else { ERR };
            let row = self.view.paint(
                code,
                &fit(&format!("{op} {}", text.replace('\u{1b}', "\\x1b")), limit),
            );
            self.view.line(&format!("      {row}"));
        }
        if changes.len() > 8 {
            let row = self
                .view
                .paint(MUTED, &format!("… +{} lines", changes.len() - 8));
            self.view.line(&format!("      {row}"));
        }
    }

    fn show_plan(&mut self, entries: Vec<(String, String)>) {
        if entries.is_empty() || entries == self.plan {
            return;
        }
        self.begin();
        let header = format!(
            "{} {}",
            self.view.paint(BRAND, "⏺"),
            self.view.paint(BOLD, "Plan")
        );
        self.view.line(&header);
        let limit = self.view.width().saturating_sub(8);
        for (index, (status, content)) in entries.iter().enumerate() {
            let lead = if index == 0 { "  ⎿ " } else { "    " };
            let content = fit(&content.replace('\u{1b}', "\\x1b"), limit);
            let item = match status.as_str() {
                "completed" => format!(
                    "{} {}",
                    self.view.paint(OK, "☒"),
                    self.view.paint(&format!("9;{MUTED}"), &content)
                ),
                "in_progress" => format!(
                    "{} {}",
                    self.view.paint(BRAND, "☐"),
                    self.view.paint(&format!("1;{BRAND}"), &content)
                ),
                _ => format!("☐ {content}"),
            };
            let row = format!("{}{item}", self.view.paint(MUTED, lead));
            self.view.line(&row);
        }
        self.plan = entries;
    }

    /// The question replaces the input area: the answer is given where the cursor already is.
    fn ask_permission(&mut self, call: &Value, selected: usize) {
        let mut state = ToolState::default();
        if let Some(update) = crate::acp::tool_update(call) {
            state.title = update.title;
            state.detail = update.detail;
            state.diffs = update.diffs;
        }
        let limit = self.view.width().saturating_sub(4).max(20);
        let clean = |text: &str| text.replace('\u{1b}', "\\x1b");
        let mut rows = vec![
            self.view.paint(MUTED, &"─".repeat(self.view.width())),
            format!(
                " {} {}",
                self.view.paint(WARN, "Permission required"),
                self.tool_heading(&state, limit.saturating_sub(24))
            ),
        ];
        if let Some(detail) = state.detail.as_deref() {
            let detail: Vec<_> = detail.lines().collect();
            for line in detail.iter().take(6) {
                rows.push(format!(
                    "   {}",
                    self.view.paint(CODE, &fit(&clean(line), limit))
                ));
            }
            if detail.len() > 6 {
                rows.push(format!(
                    "   {}",
                    self.view
                        .paint(MUTED, &format!("… +{} lines", detail.len() - 6))
                ));
            }
        }
        for diff in state.diffs.iter().take(3) {
            let ops = line_diff(diff.old.as_deref().unwrap_or(""), &diff.new).unwrap_or_default();
            let count = |kind: char| ops.iter().filter(|(op, _)| *op == kind).count();
            rows.push(format!(
                "   {} {} {}",
                self.view.paint(MUTED, &clean(&diff.path)),
                self.view.paint(OK, &format!("+{}", count('+'))),
                self.view.paint(ERR, &format!("-{}", count('-')))
            ));
        }
        rows.push(String::new());
        for (index, label) in CHOICES.iter().enumerate() {
            let label = format!("{}. {label}", index + 1);
            rows.push(if index == selected {
                format!(
                    " {} {}",
                    self.view.paint(BRAND, "❯"),
                    self.view.paint(&format!("1;{BRAND}"), &label)
                )
            } else {
                format!("   {}", self.view.paint(MUTED, &label))
            });
        }
        self.view.term.overlay = Some(rows);
        self.view.term.status = self
            .view
            .paint(MUTED, " Enter to select · ↑↓ to navigate · Esc to cancel");
        self.view.term.render();
    }

    fn done(&mut self) {
        self.settle();
        self.end_block();
        let unfinished: Vec<_> = self
            .order
            .iter()
            .filter(|id| self.tools.get(*id).is_some_and(|t| !t.done))
            .cloned()
            .collect();
        for id in unfinished {
            let state = self.tools[&id].clone();
            self.finish_tool(&id, &state, None);
        }
        self.settle();
        if !self.view.term.tty || !self.printed {
            return;
        }
        let mut parts = vec![format!("{}s", self.started.elapsed().as_secs())];
        if let Some(route) = &self.route {
            parts.push(route.clone());
        }
        match (self.usage.input_tokens, self.usage.output_tokens) {
            (Some(input), Some(output)) => {
                parts.push(format!("↑ {} ↓ {}", compact(input), compact(output)));
                if let Some(cached) = self.usage.cached_tokens.filter(|_| input > 0) {
                    // Some agents count cache reads inside the input, others beside it.
                    let context = if cached >= input {
                        input + cached
                    } else {
                        input
                    };
                    parts.push(format!("cache {}%", (cached * 100 / context).min(100)));
                }
            }
            _ => {
                if let Some(total) = self.usage.total_tokens {
                    parts.push(format!("{} tokens", compact(total)));
                }
            }
        }
        let line = format!(
            "{} {}",
            self.view.paint(BRAND, "✻"),
            self.view.paint(MUTED, &parts.join(" · "))
        );
        self.view.line("");
        self.view.line(&line);
    }
}

/// Plain wording for the protocol-level failures a user can act on.
fn friendly(error: &str) -> String {
    for (marker, text) in [
        ("transport closed", "the agent process stopped unexpectedly"),
        ("never received", "the agent stopped without answering"),
        ("timed out", "the agent did not answer in time"),
        ("rate limit", "the agent hit its rate limit"),
        ("authentication", "the agent is not logged in"),
        ("credit", "the account is out of credit"),
    ] {
        if error.to_ascii_lowercase().contains(marker) {
            return format!("{text} ({})", first_sentence(error));
        }
    }
    error.to_owned()
}
fn first_sentence(error: &str) -> String {
    let text = error.replace('\n', " ");
    let end = text
        .char_indices()
        .find(|(index, c)| *c == '.' && *index > 40)
        .map_or(text.len(), |(index, _)| index);
    text[..end].trim().to_owned()
}

fn mailbox_activity(state: &ToolState) -> Option<&'static str> {
    let title = state.title.as_deref()?.to_ascii_lowercase();
    if !title.contains("mailbox") {
        return None;
    }
    [
        ("send_message", "Messaging another agent"),
        ("read_messages", "Waiting for messages from other agents"),
        ("list_peers", "Looking for other agents"),
        ("set_status", "Sharing status with other agents"),
    ]
    .iter()
    .find(|(tool, _)| title.contains(tool))
    .map(|(_, activity)| *activity)
}

fn mail_ticker() -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker
}

fn tool_label(state: &ToolState) -> String {
    let title = state.title.as_deref().map(|t| t.trim().trim_matches('`'));
    match (title, state.detail.as_deref()) {
        (Some(title), Some(detail))
            if detail == title || detail.strip_prefix("$ ") == Some(title) =>
        {
            detail.to_owned()
        }
        (Some(title), Some(detail)) => format!("{title}: {detail}"),
        (Some(title), None) => title.to_owned(),
        (None, Some(detail)) => detail.to_owned(),
        (None, None) => "a tool".into(),
    }
}

/// Changed lines between two texts (`+`, `-`, or ` ` for kept lines inside the changed span).
/// `None` when the changed span is too large to compare cheaply.
fn line_diff(old: &str, new: &str) -> Option<Vec<(char, String)>> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    if a.len().saturating_mul(b.len()) > 250_000 {
        return None;
    }
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut ops) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            ops.push((' ', a[i].to_owned()));
            i += 1;
            j += 1;
        } else if j == b.len() || (i < a.len() && lcs[i + 1][j] >= lcs[i][j + 1]) {
            ops.push(('-', a[i].to_owned()));
            i += 1;
        } else {
            ops.push(('+', b[j].to_owned()));
            j += 1;
        }
    }
    Some(ops)
}

/// Takes one line of at most `limit` columns off the front of `text`, breaking at a space
/// when there is one.
fn wrap(text: &mut String, limit: usize) -> String {
    let mut used = 0;
    let mut end = text.len();
    let mut space = None;
    for (index, c) in text.char_indices() {
        let w = c.width().unwrap_or(0);
        if used + w > limit {
            end = index;
            break;
        }
        if c == ' ' {
            space = Some(index);
        }
        used += w;
    }
    match space.filter(|&s| s > 0) {
        Some(space) => {
            let line = text[..space].to_owned();
            text.drain(..=space);
            line
        }
        None => text.drain(..end.max(1)).collect(),
    }
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Display width ignoring ANSI color sequences.
fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut escape = false;
    for c in text.chars() {
        match (escape, c) {
            (false, '\u{1b}') => escape = true,
            (true, 'm') => escape = false,
            (true, _) => {}
            (false, c) => width += c.width().unwrap_or(0),
        }
    }
    width
}

/// One terminal line of at most `limit` columns; control characters become spaces.
fn fit(text: &str, limit: usize) -> String {
    if visible_width(text) <= limit && !text.contains(['\n', '\r', '\t']) {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    let mut escape = false;
    for c in text.chars() {
        if escape || c == '\u{1b}' {
            escape = c != 'm';
            out.push(c);
            continue;
        }
        let c = if c.is_control() { ' ' } else { c };
        let w = c.width().unwrap_or(0);
        if used + w + 1 > limit {
            out.push('…');
            if text.contains('\u{1b}') {
                out.push_str("\x1b[0m");
            }
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

fn width() -> usize {
    #[cfg(unix)]
    {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
            && size.ws_col > 0
        {
            return size.ws_col.into();
        }
    }
    80
}

/// Messages and arrivals on this repository's mailbox channel (all worktrees and processes).
struct Feed {
    /// Peer name to the agent and model it is running, for the transcript.
    routes: BTreeMap<String, String>,
    mailbox: crate::mailbox::Mailbox,
    channel: String,
    /// Names of the agent sessions this process runs right now.
    mine: BTreeSet<String>,
    last: i64,
    peers: BTreeMap<String, crate::mailbox::Peer>,
}
enum Mail {
    Message(crate::mailbox::Message),
    Joined(crate::mailbox::Peer),
    Left(crate::mailbox::Peer),
}
impl Feed {
    fn open() -> Option<Self> {
        let (data, channel, config) = crate::mailbox::active_channel()?;
        let mailbox = crate::mailbox::Mailbox::open(&data, &config).ok()?;
        let last = mailbox
            .history(&channel, 1)
            .ok()?
            .last()
            .map_or(0, |m| m.id);
        let mut feed = Self {
            routes: BTreeMap::new(),
            mailbox,
            channel,
            mine: BTreeSet::new(),
            last,
            peers: BTreeMap::new(),
        };
        feed.peers = feed.others()?;
        Some(feed)
    }
    /// Peers other people's processes run; this process's own sessions are shown as ours.
    fn others(&self) -> Option<BTreeMap<String, crate::mailbox::Peer>> {
        let ours = crate::mailbox::ours();
        Some(
            self.mailbox
                .peers(&self.channel)
                .ok()?
                .into_iter()
                .filter(|p| !ours.iter().any(|(id, _)| *id == p.id))
                .map(|p| (p.id.clone(), p))
                .collect(),
        )
    }
    /// Arrivals, then messages, then departures, so a short-lived peer reads in order.
    fn poll(&mut self) -> Vec<Mail> {
        let mut events = Vec::new();
        self.mine = crate::mailbox::our_names();
        let current = self.others();
        if let Some(current) = &current {
            for (id, peer) in current {
                if let Some(route) = &peer.route {
                    self.routes
                        .insert(peer.name.clone(), route.replace(" / ", " · "));
                }
                if !self.peers.contains_key(id) {
                    events.push(Mail::Joined(peer.clone()));
                }
            }
        }
        for (_, name) in crate::mailbox::ours() {
            if let Some(route) = crate::mailbox::route_of(&name) {
                self.routes.insert(name, route.replace(" / ", " · "));
            }
        }
        if let Ok(messages) = self.mailbox.history(&self.channel, 50) {
            for message in messages {
                if message.id > self.last {
                    self.last = message.id;
                    events.push(Mail::Message(message));
                }
            }
        }
        if let Some(current) = current {
            for (id, peer) in &self.peers {
                if !current.contains_key(id) {
                    events.push(Mail::Left(peer.clone()));
                }
            }
            self.peers = current;
        }
        events
    }
}

fn peer_color(name: &str) -> String {
    const PALETTE: [u8; 8] = [81, 213, 149, 215, 141, 117, 179, 43];
    let hash = name
        .bytes()
        .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
    format!("38;5;{}", PALETTE[hash % PALETTE.len()])
}

fn clock(time: i64) -> String {
    #[cfg(unix)]
    {
        let at = time as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if !unsafe { libc::localtime_r(&at, &mut tm) }.is_null() {
            return format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
        }
    }
    let seconds = time.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", seconds / 3600, seconds % 3600 / 60)
}

impl View {
    /// `tag` marks one of this process's own sessions as such. Two seats of the same turn
    /// are both ours, so there the tag says nothing and only the colours tell them apart.
    fn peer(&self, name: &str, mine: &BTreeSet<String>, tag: bool) -> String {
        if mine.contains(name) && tag {
            format!(
                "{} {}",
                self.paint(&format!("1;{BRAND}"), name),
                self.paint(MUTED, "(this session)")
            )
        } else if name == "all" {
            self.paint(BOLD, "everyone")
        } else {
            self.paint(&format!("1;{}", peer_color(name)), name)
        }
    }
    /// A mailbox event as chat lines: sender-colored header and quoted body.
    fn mail(
        &self,
        mail: &Mail,
        mine: &BTreeSet<String>,
        routes: &BTreeMap<String, String>,
    ) -> String {
        match mail {
            Mail::Message(message) => {
                // Several seats of this process are all "this session": the tag stops meaning
                // anything, and only the colours tell them apart.
                let between_ours = mine.len() > 1;
                let color = if mine.contains(&message.from) && !between_ours {
                    BRAND.to_owned()
                } else {
                    peer_color(&message.from)
                };
                // `(this session)` already says whose it is, and the status row says what it
                // is running: annotating it again would only render the same message two ways
                // depending on whether its route had arrived yet.
                let route = routes
                    .get(&message.from)
                    .filter(|_| between_ours || !mine.contains(&message.from))
                    .map(|route| self.paint(MUTED, &format!(" ({route})")))
                    .unwrap_or_default();
                let mut out = format!(
                    "{} {}{route} {} {}  {}\n",
                    self.paint(&color, "✉"),
                    self.peer(&message.from, mine, !between_ours),
                    self.paint(MUTED, "→"),
                    self.peer(&message.to, mine, !between_ours),
                    self.paint(MUTED, &clock(message.sent_at))
                );
                let limit = width().saturating_sub(5).max(20);
                let mut rows = Vec::new();
                for line in message.body.replace('\u{1b}', "\\x1b").lines() {
                    let mut rest = line.to_owned();
                    if rest.is_empty() {
                        rows.push(String::new());
                    }
                    while !rest.is_empty() {
                        rows.push(if visible_width(&rest) > limit {
                            wrap(&mut rest, limit)
                        } else {
                            std::mem::take(&mut rest)
                        });
                    }
                }
                let bar = self.paint(&color, "▎");
                for row in rows.iter().take(12) {
                    out.push_str(&format!("  {bar} {row}\n"));
                }
                if rows.len() > 12 {
                    out.push_str(&format!(
                        "  {bar} {}\n",
                        self.paint(MUTED, &format!("… +{} lines", rows.len() - 12))
                    ));
                }
                out
            }
            Mail::Joined(peer) => format!(
                "{} {} {}\n",
                self.paint(OK, "●"),
                self.peer(&peer.name, mine, true),
                self.paint(
                    MUTED,
                    &fit(&peer_summary("joined", peer), width().saturating_sub(30))
                )
            ),
            Mail::Left(peer) => format!(
                "{} {} {}\n",
                self.paint(MUTED, "○"),
                self.peer(&peer.name, mine, true),
                self.paint(MUTED, "left")
            ),
        }
    }
    fn peer_rows(&self, feed: &Option<Feed>) -> Vec<String> {
        let Some(feed) = feed else {
            return vec![self.paint(
                MUTED,
                "  ⎿ The mailbox is disabled (mailbox.enabled = false)",
            )];
        };
        let mut rows = Vec::new();
        for (_, name) in crate::mailbox::ours() {
            let route = feed
                .routes
                .get(&name)
                .map(|route| self.paint(MUTED, &format!(" {route}")))
                .unwrap_or_default();
            rows.push(format!(
                "  {} {}{route}",
                self.paint(BRAND, "●"),
                self.peer(&name, &feed.mine, true)
            ));
        }
        if feed.peers.is_empty() {
            rows.push(self.paint(MUTED, "  ⎿ No other agent is running in this repository"));
        }
        for peer in feed.peers.values() {
            rows.push(format!(
                "  {} {} {}",
                self.paint(OK, "●"),
                self.peer(&peer.name, &feed.mine, true),
                self.paint(
                    MUTED,
                    &fit(&peer_summary("", peer), self.width().saturating_sub(30))
                )
            ));
            if !peer.status.is_empty() {
                rows.push(format!(
                    "      {}",
                    self.paint("3", &fit(&peer.status, self.width().saturating_sub(8)))
                ));
            }
        }
        rows
    }
}

fn peer_summary(lead: &str, peer: &crate::mailbox::Peer) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !lead.is_empty() {
        parts.push(lead.into());
    }
    if let Some(route) = &peer.route {
        parts.push(route.replace(" / ", " · "));
    }
    parts.push(home(Path::new(&peer.worktree)));
    if let Some(branch) = &peer.branch {
        parts.push(branch.clone());
    }
    parts.join(" · ")
}
