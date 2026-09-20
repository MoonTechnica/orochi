// The window. It holds no truth of its own: every screen below is a query, and the four
// things it changes — a queued turn, an answer, a control, what has been seen — are rows the
// core acts on. Polling is `changed()`, which names the threads something happened in, so a
// quiet second costs one small query rather than a re-read of the conversation.
const invoke = window.__TAURI__.core.invoke;

const el = (id) => document.getElementById(id);
const call = async (name, args) => {
  try {
    return await invoke(name, args);
  } catch (error) {
    report(String(error));
    return null;
  }
};

const state = {
  thread: null,
  projects: [],
  prompts: [],
  folders: [],
  open: new Set(JSON.parse(localStorage.getItem("open") || "[]")),
  file: null,
  // Which of Agents / Insights / Settings is open over the conversation, if any. The
  // conversation is the window; these are asked for and dismissed.
  panel: null,
  scope: "turn",
  comments: [],
};

function report(text) {
  el("notice").textContent = text;
}

function text(tag, className, value) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (value !== undefined) node.textContent = value;
  return node;
}

const MARKS = {
  needs_you: "!",
  working: "◉",
  queued: "⋯",
  interrupted: "⏸",
  failed: "×",
  unread: "●",
  idle: "○",
};

// Sidebar -------------------------------------------------------------------
// Sections are ordered by name and remember whether they were folded, so the tree the user
// left behind is the tree they come back to.
function drawSidebar() {
  const host = el("projects");
  host.replaceChildren();
  for (const project of state.projects) {
    const section = document.createElement("details");
    section.className = "project";
    section.open = !state.open.has(`closed:${project.id}`);
    section.addEventListener("toggle", () => {
      const key = `closed:${project.id}`;
      section.open ? state.open.delete(key) : state.open.add(key);
      localStorage.setItem("open", JSON.stringify([...state.open]));
    });

    const summary = document.createElement("summary");
    summary.append(text("span", "name", project.name));
    const waiting = project.threads.filter((t) => t.asking > 0).length;
    const busy = project.threads.filter((t) => t.status === "working").length;
    if (waiting || busy) {
      summary.append(text("span", "count", waiting ? `!${waiting}` : `${busy}`));
    }
    section.append(summary);

    for (const thread of project.threads) {
      const row = document.createElement("button");
      row.className = "thread";
      row.dataset.status = thread.status;
      row.setAttribute("aria-current", String(thread.id === state.thread));
      row.append(text("span", "mark", MARKS[thread.status] || "○"));
      row.append(text("span", "name", thread.title || "New conversation"));
      if (thread.status === "working" && thread.seats > 1) {
        row.append(text("span", "seats", `${thread.seats}`));
      }
      if (thread.terminal) row.append(text("span", "term", "⌘"));
      row.addEventListener("click", () => select(thread.id));
      section.append(row);
    }
    host.append(section);
  }
}

// Thread --------------------------------------------------------------------
function chip(item) {
  const d = item.data || {};
  const parts = [d.agent, d.model, d.reasoning].filter(Boolean).join(" · ");
  const node = text("div", "route");
  node.append(text("span", "chip", `⟡ ${parts}`));
  if (d.resumed) node.append(document.createTextNode("  continues the recorded session"));
  return node;
}

function checks(item) {
  const d = item.data || {};
  const node = text("div", "checks");
  const verdict = {
    success: "verified",
    partial_success: "unverified",
    failure: "failed",
    cancelled: "stopped",
  }[d.outcome] || d.outcome;
  node.append(text("div", d.outcome === "success" ? "pass" : "fail", verdict));
  for (const check of d.checks || []) {
    node.append(text("div", check.passed ? "pass" : "fail", `${check.passed ? "✓" : "✗"} ${check.name}`));
    // The one place check output is kept: the last lines of one that failed.
    if (check.tail) node.append(text("pre", null, check.tail));
  }
  return node;
}

function work(items) {
  const node = document.createElement("details");
  node.className = "work";
  const tools = items.filter((i) => i.kind === "tool_call");
  const thoughts = items.filter((i) => i.kind === "thought");
  const summary = [];
  if (thoughts.length) summary.push("Thought");
  if (tools.length) summary.push(`${tools.length} tool call${tools.length > 1 ? "s" : ""}`);
  node.append(text("summary", null, `▸ ${summary.join(", ")}`));
  for (const item of items) {
    const d = item.data || {};
    const label = item.kind === "thought" ? "thinking" : `${d.title || "tool"} ${d.detail || ""}`;
    node.append(text("div", null, label.trim()));
    if (item.text) node.append(text("pre", null, item.text));
  }
  return node;
}

function drawTimeline(thread) {
  const host = el("timeline");
  const stick = host.scrollTop + host.clientHeight >= host.scrollHeight - 40;
  host.replaceChildren();
  let turn = null;
  let pending = [];
  const flush = () => {
    if (pending.length && turn) turn.append(work(pending));
    pending = [];
  };
  for (const item of thread.items) {
    // Only the lead's work is in the transcript; the read-only seats speak in the Team pane.
    if (item.lane !== null && item.lane !== undefined && item.lane > 0) continue;
    if (item.kind === "user_message") {
      flush();
      turn = text("div", "turn");
      turn.append(text("p", "you", item.text));
      host.append(turn);
      continue;
    }
    if (!turn) {
      turn = text("div", "turn");
      host.append(turn);
    }
    if (item.kind === "thought" || item.kind === "tool_call") {
      pending.push(item);
      continue;
    }
    flush();
    if (item.kind === "route") turn.append(chip(item));
    else if (item.kind === "agent_message") turn.append(text("p", "reply", item.text));
    else if (item.kind === "checks") turn.append(checks(item));
    else if (item.kind === "note" || item.kind === "unavailable") {
      turn.append(text("div", "note", item.text));
    }
  }
  flush();
  for (const prompt of state.prompts.filter((p) => p.thread_id === thread.thread.id)) {
    host.append(askCard(prompt));
  }
  if (stick) host.scrollTop = host.scrollHeight;
}

function askCard(prompt) {
  const node = text("div", "ask");
  node.append(text("h4", null, prompt.title));
  if (prompt.detail) node.append(text("code", null, prompt.detail));
  const row = text("div", "row");
  for (const [id, name] of prompt.options) {
    const button = document.createElement("button");
    button.className = "primary";
    button.textContent = name;
    button.addEventListener("click", () => answer(prompt.id, id));
    row.append(button);
  }
  const no = document.createElement("button");
  no.textContent = "No";
  no.addEventListener("click", () => answer(prompt.id, null));
  row.append(no);
  node.append(row);
  return node;
}

async function answer(prompt, option) {
  await call("answer", { prompt, option });
  await refresh(true);
}

// Folder picker -------------------------------------------------------------
// Where the work happens is the one thing a window has to ask before anything else can be
// done. The folders offered are the ones already worked in, from a terminal or from here, so
// a repository is one click away; the last entry opens the system's own picker.
function folderLabel() {
  const thread = state.projects.flatMap((p) => p.threads).find((t) => t.id === state.thread);
  el("folder-name").textContent = thread ? thread.project : "No folder";
}

function closeFolders() {
  el("folder-menu").hidden = true;
  el("folder").setAttribute("aria-expanded", "false");
}

async function openFolders() {
  const menu = el("folder-menu");
  menu.replaceChildren();
  const current = state.projects.flatMap((p) => p.threads).find((t) => t.id === state.thread);

  menu.append(text("div", "head", "Recent"));
  for (const folder of state.folders) {
    const row = document.createElement("button");
    row.type = "button";
    row.append(text("span", "name", folder.name));
    row.append(text("span", "where", folder.root));
    if (current && current.project === folder.name) row.append(text("span", "tick", "✓"));
    row.addEventListener("click", () => startIn(folder.root));
    menu.append(row);
  }
  if (!state.folders.length) menu.append(text("div", "where", "Nowhere yet."));

  menu.append(text("div", "sep"));
  const pick = document.createElement("button");
  pick.type = "button";
  pick.append(text("span", "name", "Open folder…"));
  pick.addEventListener("click", chooseFolder);
  menu.append(pick);

  menu.hidden = false;
  el("folder").setAttribute("aria-expanded", "true");
}

/// The system's own directory picker, so the window never asks anyone to type a path.
async function chooseFolder() {
  const dialog = window.__TAURI__.dialog;
  const root = await dialog.open({ directory: true, multiple: false, title: "Choose a folder" });
  if (root) await startIn(root);
}

async function startIn(root) {
  closeFolders();
  const id = await call("new_thread", { root });
  if (!id) return;
  state.thread = id;
  await refresh(true);
}

el("folder").addEventListener("click", () => {
  el("folder-menu").hidden ? openFolders() : closeFolders();
});
document.addEventListener("click", (event) => {
  if (!el("folder-picker").contains?.(event.target)) closeFolders();
});

// Side pane -----------------------------------------------------------------
function drawTeam(thread) {
  const host = el("roster");
  host.replaceChildren();
  if (!thread.seats.length) {
    host.append(text("p", "empty", "No one is seated yet."));
    return;
  }
  for (const seat of thread.seats) {
    const node = text("div", "seat");
    node.dataset.state = seat.state;
    const who = text("div", "who");
    who.append(text("span", null, seat.role));
    if (seat.read_only) who.append(text("span", "ro", "RO"));
    node.append(who);
    node.append(text("div", "what", [seat.agent, seat.model, seat.reasoning].filter(Boolean).join(" · ")));
    if (seat.peer_status) node.append(text("div", "what", `"${seat.peer_status}"`));
    if (seat.doing) node.append(text("div", "what", seat.doing.split("\n")[0]));
    host.append(node);
  }
}

async function drawChanges(thread) {
  const host = el("files");
  host.replaceChildren();
  // "Last turn" is what the agent said it changed, which is all that survives the tree moving
  // on. The git scopes are what is actually there now, and they are the authority.
  const files =
    state.scope === "turn"
      ? thread.files
      : (await call("tree_files", { thread: thread.thread.id, scope: state.scope })) || [];
  if (!files.length) {
    host.append(
      text(
        "p",
        "empty",
        state.scope === "turn"
          ? "Nothing changed in this conversation yet."
          : "Nothing in this scope.",
      ),
    );
    return;
  }
  for (const file of files) {
    const row = text("div", "file");
    row.append(text("span", "name", file.path));
    const stat = text("span", "stat");
    stat.append(text("span", "add", `+${file.added}`));
    stat.append(document.createTextNode(" "));
    stat.append(text("span", "del", `−${file.removed}`));
    row.append(stat);
    host.append(row);

    const diff = text("pre", "diff");
    diff.hidden = true;
    host.append(diff);
    row.addEventListener("click", async () => {
      diff.hidden = !diff.hidden;
      if (diff.hidden || diff.childElementCount) return;
      const patch =
        state.scope === "turn"
          ? await call("patch", { id: file.latest_patch })
          : await call("tree_patch", {
              thread: thread.thread.id,
              scope: state.scope,
              path: file.path,
            });
      diff.replaceChildren();
      let line = 0;
      for (const row of (patch || "").split("\n")) {
        const header = row.startsWith("@") || row.startsWith("+++") || row.startsWith("---");
        const kind = header ? "h" : row.startsWith("+") ? "i" : row.startsWith("-") ? "d" : null;
        if (!header) line += 1;
        const node = text("span", kind, row + "\n");
        const at = line;
        // Clicking a line is how a comment is left, which is how the next message is written.
        node.addEventListener("click", () => {
          const note = window.prompt(`Comment on ${file.path}:${at}`);
          if (!note) return;
          state.comments.push([file.path, at, note]);
          drawReview();
        });
        diff.append(node);
      }
    });
  }
}

/// The comments waiting to be sent. They are not a review mechanism of their own: they become
/// one message, routed like any other, continuing the same conversation.
function drawReview() {
  const form = el("review-form");
  const list = el("review-list");
  list.replaceChildren();
  form.hidden = state.comments.length === 0;
  state.comments.forEach(([path, line, note], index) => {
    const row = text("div", "comment");
    row.append(text("span", "at", `${path}:${line} `));
    row.append(text("span", null, note));
    const drop = document.createElement("button");
    drop.type = "button";
    drop.textContent = "×";
    drop.addEventListener("click", () => {
      state.comments.splice(index, 1);
      drawReview();
    });
    row.append(drop);
    list.append(row);
  });
}

el("review-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!state.comments.length || !state.thread) return;
  const comments = state.comments;
  state.comments = [];
  drawReview();
  await call("comment", { thread: state.thread, comments });
  await call("ensure_host", { thread: state.thread });
  await refresh(true);
});

for (const button of document.querySelectorAll(".scope")) {
  button.addEventListener("click", async () => {
    state.scope = button.dataset.scope;
    for (const other of document.querySelectorAll(".scope")) {
      other.setAttribute("aria-selected", String(other === button));
    }
    const thread = await call("thread", { id: state.thread });
    if (thread) await drawChanges(thread);
  });
}

// The room ------------------------------------------------------------------
// Agents talk to each other here, and so can the person — delivered when the agent next reads
// its messages, which is a note on the table rather than an interruption.
async function drawRoom(thread) {
  const host = el("room");
  host.replaceChildren();
  const said = await call("room", { thread: thread.thread.id });
  if (!said || !said.length) {
    host.append(text("p", "empty", "Nothing said yet."));
    return;
  }
  for (const line of said) {
    if (line.kind !== "message") {
      const mark = line.kind === "joined" ? "●" : line.kind === "left" ? "○" : "⎿";
      host.append(text("div", "said system", `${mark} ${line.who} ${line.kind === "status" ? line.text : line.kind}`));
      continue;
    }
    const node = text("div", "said");
    node.dataset.via = line.via;
    node.append(text("div", "who", `${line.who} → ${line.whom || "all"}`));
    node.append(text("div", "body", line.text));
    host.append(node);
  }
}

el("say-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const box = el("say");
  const value = box.value.trim();
  if (!value || !state.thread) return;
  box.value = "";
  await call("say", { thread: state.thread, to: null, text: value });
  const thread = await call("thread", { id: state.thread });
  if (thread) await drawRoom(thread);
});

// The work graph of a turn, when it has one.
async function drawPlan(thread) {
  const host = el("pane-plan");
  host.replaceChildren();
  const parts = await call("board", { thread: thread.thread.id });
  if (!parts || !parts.length) {
    host.append(text("p", "empty", "This conversation was not divided into parts."));
    return;
  }
  let wave = null;
  for (const part of parts) {
    if (part.wave !== wave) {
      wave = part.wave;
      host.append(text("div", "wave", wave === null ? "Ungated" : `Wave ${wave + 1}`));
    }
    const node = text("div", "part");
    node.dataset.state = part.state;
    node.append(text("span", "id", part.id));
    node.append(text("div", null, part.brief));
    if (part.paths?.length) node.append(text("div", "paths", part.paths.join(", ")));
    host.append(node);
  }
}

// Screens --------------------------------------------------------------------
// Everything that is not one conversation: what is working anywhere, which agents are ready,
// and what the routes have actually done.
function screenRows(head, rows) {
  const table = document.createElement("table");
  const header = document.createElement("tr");
  for (const [label, numeric] of head) {
    const cell = text("th", numeric ? "num" : null, label);
    header.append(cell);
  }
  table.append(header);
  for (const row of rows) {
    const line = document.createElement("tr");
    for (const [value, numeric] of row) {
      if (value && value.tagName) {
        const cell = text("td", numeric ? "num" : null);
        cell.append(value);
        line.append(cell);
      } else {
        line.append(text("td", numeric ? "num" : null, value));
      }
    }
    table.append(line);
  }
  return table;
}

function gauge(fraction) {
  const node = text("span", fraction < 0.2 ? "gauge low" : "gauge");
  const fill = text("i");
  fill.style.width = `${Math.round(Math.max(0, Math.min(1, fraction)) * 100)}%`;
  node.append(fill);
  return node;
}

/// Opens one of the panels over the conversation. Esc and the backdrop close it, as a dialog
/// does; the conversation underneath is never replaced.
async function openPanel(panel) {
  state.panel = panel;
  el("panel-title").textContent = PANELS.find(([p]) => p === panel)?.[1] || "";
  el("panel").showModal();
  await drawScreen();
}

function closePanel() {
  state.panel = null;
  el("panel").close();
}

el("panel-close").addEventListener("click", closePanel);
el("panel").addEventListener("close", () => {
  state.panel = null;
});

async function drawScreen() {
  const host = el("panel-body");
  host.replaceChildren();
  if (state.panel === "agents") {
    const agents = (await call("agents", {})) || [];
    if (!agents.length) {
      host.append(text("p", "empty", "Nothing has been run yet."));
      return;
    }
    host.append(
      screenRows(
        [["Agent"], ["Model"], ["State"], ["Cooling", true], ["Quota"]],
        agents.map((a) => [
          [a.agent],
          [a.model === "*" ? "whole account" : a.model],
          [a.status],
          [a.cooling ? `${a.cooling}s` : "—", true],
          [a.windows.length ? gauge(a.windows[0][1]) : "—"],
        ]),
      ),
    );
    return;
  }
  if (state.panel === "settings") {
    const [config, remembered] = await Promise.all([
      call("settings", {}),
      call("memory", {}),
    ]);
    if (!config) return;

    // The conversation store is the one place Orochi keeps your own text, so it is the one
    // place with a switch, a window and a way to remove what is there.
    const activity = config.activity || {};
    const field = (label, node) => {
      const row = text("div", "card");
      row.append(text("div", "where", label));
      row.append(node);
      return row;
    };
    const keep = document.createElement("input");
    keep.type = "number";
    keep.value = String(activity.retention_days ?? 30);
    keep.addEventListener("change", async () => {
      config.activity.retention_days = Number(keep.value);
      await call("save_settings", { settings: config });
      await drawScreen();
    });
    host.append(field("Keep conversations for (days; 0 keeps them until deleted)", keep));

    const on = document.createElement("input");
    on.type = "checkbox";
    on.checked = activity.enabled !== false;
    on.addEventListener("change", async () => {
      config.activity.enabled = on.checked;
      await call("save_settings", { settings: config });
      await drawScreen();
    });
    host.append(field("Record conversations at all", on));

    const wipe = document.createElement("button");
    wipe.textContent = "Delete every conversation";
    wipe.addEventListener("click", async () => {
      if (wipe.dataset.sure !== "yes") {
        wipe.dataset.sure = "yes";
        wipe.textContent = "This cannot be undone — click again";
        return;
      }
      const removed = await call("forget_all", {});
      report(`Deleted ${removed} conversation${removed === 1 ? "" : "s"}.`);
      state.thread = null;
      await refresh(true);
    });
    host.append(field("History", wipe));

    // Memory is the user's own text, so it is edited as text.
    const notes = document.createElement("textarea");
    notes.rows = 8;
    notes.value = remembered?.user || "";
    notes.addEventListener("change", () => call("save_memory", { text: notes.value }));
    host.append(field("What Orochi remembers about you", notes));

    host.append(
      text("p", "caveat", "Agents never write this, and it is never sent to a routing adviser."),
    );
    return;
  }
  if (state.panel === "insights") {
    const stats = (await call("insights", {})) || [];
    host.append(text("h3", null, "What the routes have done"));
    if (!stats.length) {
      host.append(text("p", "empty", "No verified runs yet."));
      return;
    }
    host.append(
      screenRows(
        [["Agent"], ["Model"], ["Task"], ["Verified", true], ["Failed", true], ["Success", true], ["Tokens", true], ["Weak", true]],
        stats.map((s) => [
          [s.agent],
          [s.model],
          [s.task_type],
          [String(s.verified), true],
          [String(s.failures), true],
          [s.verified ? `${Math.round(s.success_rate * 100)}%` : "—", true],
          [s.verified ? Math.round(s.mean_tokens).toLocaleString() : "—", true],
          [String(s.weak), true],
        ]),
      ),
    );
    host.append(
      text(
        "p",
        "caveat",
        "Verified counts are measured; weak signals are what you did next, which Orochi weighs far less. These are its own estimates.",
      ),
    );
  }
}

// Everything that is not a conversation lives behind the row at the foot of the sidebar,
// rather than as a row of buttons competing with the threads for attention.
const PANELS = [
  ["agents", "Agents"],
  ["insights", "Insights"],
  ["sep"],
  ["settings", "Settings"],
];

function closeAccount() {
  el("account-menu").hidden = true;
  el("account").setAttribute("aria-expanded", "false");
}

function openAccount() {
  const menu = el("account-menu");
  menu.replaceChildren();
  for (const [panel, label] of PANELS) {
    if (panel === "sep") {
      menu.append(text("div", "sep"));
      continue;
    }
    const row = document.createElement("button");
    row.type = "button";
    row.dataset.screen = panel;
    row.append(text("span", "name", label));
    row.addEventListener("click", () => {
      closeAccount();
      openPanel(panel);
    });
    menu.append(row);
  }
  menu.hidden = false;
  el("account").setAttribute("aria-expanded", "true");
}

el("account").addEventListener("click", () => {
  el("account-menu").hidden ? openAccount() : closeAccount();
});
document.addEventListener("click", (event) => {
  if (!el("sidebar-foot").contains?.(event.target)) closeAccount();
});

for (const tab of document.querySelectorAll(".tab")) {
  tab.addEventListener("click", () => {
    for (const other of document.querySelectorAll(".tab")) {
      const on = other === tab;
      other.setAttribute("aria-selected", String(on));
      el(`pane-${other.dataset.pane}`).hidden = !on;
    }
  });
}

// Loop ----------------------------------------------------------------------
async function select(id) {
  state.thread = id;
  await call("seen", { thread: id });
  await refresh(true);
}

async function refresh(full) {
  const [projects, prompts, folders] = await Promise.all([
    call("sidebar", { limit: 30, archived: false }),
    call("open_prompts", {}),
    call("folders", {}),
  ]);
  state.projects = projects || [];
  state.prompts = prompts || [];
  state.folders = folders || [];
  if (!state.thread) {
    state.thread = state.projects.flatMap((p) => p.threads)[0]?.id || null;
    // A thread chosen here was not on screen a moment ago, whatever the feed said moved:
    // drawing the list without drawing the conversation leaves a window that lists one
    // thing and shows another.
    full = full || state.thread !== null;
  }
  drawSidebar();
  folderLabel();
  // An open panel follows what it is showing; it never takes the conversation's place.
  if (state.panel) await drawScreen();

  if (!state.thread) {
    el("thread-title").textContent = "Nothing selected";
    el("timeline").replaceChildren(
      text("p", "empty", "Choose a folder below to start working in it."),
    );
    return;
  }
  if (!full) return;
  const thread = await call("thread", { id: state.thread });
  if (!thread) return;
  el("thread-where").textContent = `${thread.thread.project} · ${thread.thread.branch || "—"}`;
  // The first message names it; until then it is a conversation you have not started.
  el("thread-title").textContent = thread.thread.title || "New conversation";
  el("thread-status").textContent = thread.thread.status;
  el("interrupt").hidden = thread.thread.status !== "working";
  drawTimeline(thread);
  drawTeam(thread);
  drawChanges(thread);
  drawRoom(thread);
  drawPlan(thread);
  // Looking at a conversation is what makes it read.
  await call("seen", { thread: state.thread });
}

el("composer").addEventListener("submit", async (event) => {
  event.preventDefault();
  const box = el("message");
  const value = box.value.trim();
  if (!value || !state.thread) return;
  box.value = "";
  box.style.height = "auto";
  await call("send", { thread: state.thread, text: value });
  // A message is only a row until something runs it.
  await call("ensure_host", { thread: state.thread });
  await refresh(true);
});

el("message").addEventListener("input", (event) => {
  event.target.style.height = "auto";
  event.target.style.height = `${event.target.scrollHeight}px`;
});

// Enter sends and Shift-Enter breaks the line, as the console's input does.
el("message").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    el("composer").requestSubmit();
  }
});

el("interrupt").addEventListener("click", () => call("interrupt", { thread: state.thread }));

// A quiet tick costs one query over the change feed; the conversation is re-read only when
// the feed says this thread moved.
setInterval(async () => {
  const changed = await call("changed", {});
  if (!changed) return;
  if (changed.length === 0) return;
  await refresh(changed.includes(state.thread));
}, 200);

// Where the work already happens: the open thread's folder, else the one worked in last.
// Only somewhere that has never been worked in has to be chosen, and then by the system's own
// picker. Opening the composer's menu from up here would put it a screen away from the hand
// that asked for it, and the click that opened it would close it again on its way out.
el("new-thread").addEventListener("click", async (event) => {
  event.stopPropagation();
  closeFolders();
  const open = state.projects.flatMap((p) => p.threads.map((t) => [p, t]))
    .find(([, t]) => t.id === state.thread);
  const root = open?.[0].root ?? state.folders[0]?.root;
  await (root ? startIn(root) : chooseFolder());
});

refresh(true);
