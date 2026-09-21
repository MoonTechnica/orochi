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

/// What a seat is doing before it has said anything, in words rather than a state name.
const AT = {
  choosing: "choosing an agent…",
  starting: "starting up…",
  working: "working…",
  asking: "waiting for your answer",
  done: "done",
  failed: "failed",
};
/// The words the markup ships with, said again in the window's language once it is open.
const LABELS = {
  "new-thread": "New thread",
  "composer-hint": "Cmd-Enter to send · Enter for a new line",
  send: "Send",
  interrupt: "Stop",
  "say-hint": "Delivered when the agent next checks.",
};

/// The window's own words. It says them in the language the machine is set to, because a
/// Japanese conversation framed in English labels reads as two things at once. A language it
/// has no words for gets English; nothing is half-translated.
const WORDS = {
  ja: {
    "New thread": "新しいスレッド",
    "New conversation": "新しい会話",
    "Cmd-Enter to send · Enter for a new line": "⌘Enter で送信 · Enter で改行",
    Send: "送信",
    Stop: "停止",
    "Message…": "メッセージ…",
    "Message the team…": "チームにメモ…",
    "Delivered when the agent next checks.": "エージェントが次に確認したときに届きます。",
    "Starting the agent…": "エージェントを起動しています…",
    "choosing an agent…": "エージェントを選んでいます…",
    "starting up…": "起動しています…",
    "working…": "作業しています…",
    "waiting for your answer": "あなたの返答を待っています",
    done: "完了",
    failed: "失敗",
    everyone: "全員",
    "No folder": "フォルダ未選択",
    Recent: "最近",
    "Nowhere yet.": "まだありません。",
    "Open folder…": "フォルダを開く…",
    "No one is seated yet.": "まだ誰も席に着いていません。",
    "Nobody has come or gone yet.": "まだ誰の出入りもありません。",
    "Nothing said yet.": "まだ何も話されていません。",
    "an attempt that failed, and what it said": "失敗した試行と、その内容",
    "nothing to verify": "検証対象なし",
    Team: "チーム",
    Changes: "変更",
    Plan: "計画",
  },
};

/// One word, in the window's language.
const LANG = (typeof window !== "undefined" && window.navigator?.language) || "en";
const SAID = WORDS[LANG.slice(0, 2)] || {};
const t = (word) => SAID[word] || word;

const MARKS = {
  needs_you: "message",
  working: "loader",
  queued: "clock",
  interrupted: "pause",
  failed: "x",
  unread: "dot",
  idle: "circle",
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
      row.append(drawn(MARKS[thread.status] || "circle", "state"));
      row.append(text("span", "name", thread.title || t("New conversation")));
      if (thread.status === "working" && thread.seats > 1) {
        row.append(text("span", "seats", `${thread.seats}`));
      }
      if (thread.terminal) row.append(drawn("terminal", "term"));
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
  const chipped = text("span", "chip");
  chipped.append(drawn("git-branch"));
  chipped.append(text("span", null, parts));
  node.append(chipped);
  if (d.resumed) node.append(document.createTextNode("  continues the recorded session"));
  return node;
}

function checks(item, seats = []) {
  const d = item.data || {};
  const node = text("div", "checks");
  // A seat that may change nothing has nothing to check. "Unverified" says a check was owed
  // and not made; here none was ever owed, and the two read very differently to someone
  // looking at a discussion that went perfectly well.
  const seat = seats.find((s) => s.turn_id === item.turn && s.ordinal === item.lane);
  const nothing =
    d.outcome === "partial_success" && !(d.checks || []).length && seat?.read_only;
  const verdict = nothing
    ? t("nothing to verify")
    : {
        success: "verified",
        partial_success: "unverified",
        failure: "failed",
        cancelled: "stopped",
      }[d.outcome] || d.outcome;
  node.append(
    text("div", d.outcome === "success" || nothing ? "pass" : "fail", verdict),
  );
  for (const check of d.checks || []) {
    const row = text("div", check.passed ? "pass" : "fail");
    row.append(drawn(check.passed ? "check" : "x"));
    row.append(text("span", null, check.name));
    node.append(row);
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
  const head = text("summary", null);
  head.append(drawn("chevron-right"));
  head.append(text("span", null, summary.join(", ")));
  node.append(head);
  for (const item of items) {
    const d = item.data || {};
    const label = item.kind === "thought" ? "thinking" : `${d.title || "tool"} ${d.detail || ""}`;
    node.append(text("div", null, label.trim()));
    if (item.text) node.append(text("pre", null, item.text));
  }
  return node;
}

/// How a route was chosen — an agent that could not be reached, an account cooling down, the
/// label the classifier settled on. Worth keeping and not worth reading: four of them beside
/// one reply drowned it.
function aside(notes, what) {
  const node = document.createElement("details");
  node.className = "aside";
  const count = notes.length;
  const head = text("summary", null);
  head.append(drawn("chevron-right"));
  head.append(
    text("span", null, what || `${count} routing note${count > 1 ? "s" : ""}`),
  );
  node.append(head);
  for (const item of notes) node.append(text("div", null, item.text));
  return node;
}

function drawTimeline(thread, said = []) {
  const host = el("timeline");
  const stick = host.scrollTop + host.clientHeight >= host.scrollHeight - 40;
  host.replaceChildren();
  let turn = null;
  let pending = [];
  let notes = [];
  const flush = () => {
    if (pending.length && turn) turn.append(work(pending));
    pending = [];
    if (notes.length && turn) turn.append(aside(notes));
    notes = [];
  };
  // One conversation. What the agents say to each other belongs in it, in the order it was
  // said — a table where half the talk happens in another column is two rooms, not one.
  const spoken = said
    .filter((line) => line.kind === "message")
    .map((line) => ({ ...line, kind: "said", at: line.at }));
  // Who each peer is, so the one it is speaking to is named the same way it is.
  const roles = new Map(said.map((line) => [line.who, line.role]).filter(([, r]) => r));
  const stream = [...thread.items, ...spoken].sort(
    (a, b) => (a.at ?? 0) - (b.at ?? 0) || (a.seq ?? 0) - (b.seq ?? 0),
  );
  let voice = null;
  for (const item of stream) {
    if (item.kind === "said") {
      flush();
      if (!turn) {
        turn = text("div", "turn loose");
        host.append(turn);
      }
      const who = named(item);
      const whom =
        !item.whom || item.whom === "all"
          ? t("everyone")
          : `to ${roles.get(item.whom) || item.whom}`;
      turn.append(
        post(
          { who, to: whom, model: item.model, at: item.at, text: item.text, mine: item.via === "user" },
          voice === who,
        ),
      );
      voice = who;
      continue;
    }
    // A read-only seat's own working notes stay out; what it had to say it said in the room.
    if (item.lane !== null && item.lane !== undefined && item.lane > 0) continue;
    if (item.kind === "user_message") {
      flush();
      turn = text("div", "turn");
      turn.append(post({ who: "you", at: item.at, text: item.text, mine: true }, false));
      host.append(turn);
      voice = "you";
      continue;
    }
    if (!turn) {
      turn = text("div", "turn loose");
      host.append(turn);
    }
    if (item.kind === "thought" || item.kind === "tool_call") {
      pending.push(item);
      continue;
    }
    if (item.kind === "note" || item.kind === "unavailable") {
      notes.push(item);
      continue;
    }
    // An attempt that failed and was followed by another: what it said is almost always the
    // provider explaining itself, and reading that as the agent's own words is how a usage
    // limit came to look like something an agent had decided to say.
    if (item.failed && item.text) {
      flush();
      if (!turn) {
        turn = text("div", "turn loose");
        host.append(turn);
      }
      turn.append(aside([item], t("an attempt that failed, and what it said")));
      voice = null;
      continue;
    }
    flush();
    if (item.kind === "route") turn.append(chip(item));
    else if (item.kind === "agent_message") {
      const who = item.role || item.agent || "agent";
      turn.append(post({ who, model: item.model, at: item.at, text: item.text }, voice === who));
      voice = who;
    }
    else if (item.kind === "checks") turn.append(checks(item, thread.seats));

  }
  flush();
  // Sending starts a process, which has to find its agents and choose one before anything it
  // says can be written down. That is the longest a person waits with nothing to read, so the
  // window says what is happening rather than sitting still.
  if (thread.thread.status === "working") {
    const seats = thread.seats.filter((s) => s.turn_id === thread.seats.at(-1)?.turn_id);
    const live = text("div", "live-turn");
    if (!seats.length) {
      live.append(drawn("loader", "state"));
      live.append(text("span", null, t("Starting the agent…")));
    } else {
      for (const seat of seats) {
        const row = text("div", "at-work");
        row.append(mark(seat.role));
        row.append(text("span", "name", seat.role));
        row.append(
          text("span", "what", seat.peer_status || seat.doing?.split("\n")[0] || t(AT[seat.state] || seat.state)),
        );
        live.append(row);
      }
    }
    if (!turn) {
      turn = text("div", "turn loose");
      host.append(turn);
    }
    turn.append(live);
  }
  for (const prompt of state.prompts.filter((p) => p.thread_id === thread.thread.id)) {
    host.append(askCard(prompt));
  }
  if (stick) host.scrollTop = host.scrollHeight;
}

/// Lucide, inlined: a window that runs offline fetches nothing, and a face would be a lie —
/// nobody here has one. What is drawn is the job: a terminal for the one doing the work, an
/// eye for the one reading it, a compass for the one shaping it.
const ICONS = {
  user: ["M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2", "circle:12,7,4"],
  terminal: ["m4 17 6-6-6-6", "M12 19h8"],
  eye: ["M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z", "circle:12,12,3"],
  compass: ["m16.24 7.76-2.12 6.36-6.36 2.12 2.12-6.36z", "circle:12,12,10"],
  search: ["circle:11,11,8", "m21 21-4.3-4.3"],
  users: ["M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2", "circle:9,7,4", "M22 21v-2a4 4 0 0 0-3-3.87"],
  message: ["M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"],
  bot: ["M12 8V4H8", "rect:4,8,16,12", "M2 14h2", "M20 14h2", "M15 13v2", "M9 13v2"],
  folder: ["M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"],
  "chevron-up": ["m18 15-6-6-6 6"],
  "chevron-right": ["m9 18 6-6-6-6"],
  x: ["M18 6 6 18", "m6 6 12 12"],
  check: ["M20 6 9 17l-5-5"],
  plus: ["M5 12h14", "M12 5v14"],
  clock: ["circle:12,12,10", "M12 6v6l4 2"],
  pause: ["rect:6,4,4,16", "rect:14,4,4,16"],
  circle: ["circle:12,12,10"],
  dot: ["circle:12,12,4"],
  loader: ["M12 2v4", "m16.2 7.8 2.9-2.9", "M18 12h4", "m16.2 16.2 2.9 2.9", "M12 18v4", "m4.9 19.1 2.9-2.9", "M2 12h4", "m4.9 4.9 2.9 2.9"],
  "git-branch": ["M6 3v12", "circle:18,6,3", "circle:6,18,3", "M18 9a9 9 0 0 1-9 9"],
  "log-in": ["M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4", "m10 17 5-5-5-5", "M15 12H3"],
  "log-out": ["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4", "m16 17 5-5-5-5", "M21 12H9"],
  activity: ["M22 12h-4l-3 9L9 3l-3 9H2"],
  hexagon: ["M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"],
};

/// One drawing, wherever the window shows a picture. A character standing in for an icon is
/// whatever font the machine happens to have, at whatever weight, and never matches the ones
/// beside it.
function drawn(which, klass = "icon") {
  const host = text("span", klass);
  host.dataset.icon = which;
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [key, value] of Object.entries({
    viewBox: "0 0 24 24", fill: "none", stroke: "currentColor",
    "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
  })) {
    svg.setAttribute(key, value);
  }
  for (const shape of ICONS[which] || ICONS.circle) {
    let node;
    if (shape.startsWith("circle:")) {
      const [cx, cy, r] = shape.slice(7).split(",");
      node = document.createElementNS("http://www.w3.org/2000/svg", "circle");
      node.setAttribute("cx", cx);
      node.setAttribute("cy", cy);
      node.setAttribute("r", r);
    } else if (shape.startsWith("rect:")) {
      const [x, y, w, h] = shape.slice(5).split(",");
      node = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      for (const [key, value] of [["x", x], ["y", y], ["width", w], ["height", h], ["rx", "2"]]) {
        node.setAttribute(key, value);
      }
    } else {
      node = document.createElementNS("http://www.w3.org/2000/svg", "path");
      node.setAttribute("d", shape);
    }
    svg.append(node);
  }
  host.append(svg);
  return host;
}

/// Which drawing a name gets. A role says what a seat is for, so the role picks the icon.
function icon(name) {
  const role = name.toLowerCase();
  if (role === "you") return "user";
  if (role.includes("review")) return "eye";
  if (role.includes("architect") || role.includes("design")) return "compass";
  if (role.includes("research") || role.includes("investigat")) return "search";
  if (role.includes("partner") || role.includes("panel")) return "users";
  if (role.includes("facilitator") || role.includes("discuss")) return "message";
  if (
    role.includes("implement") || role.includes("fix") || role.includes("edit") ||
    role.includes("refactor") || role.includes("migrat") || role.includes("test") ||
    role.includes("writ")
  ) {
    return "terminal";
  }
  return "bot";
}

/// The drawing for a seat, in its own colour.
function mark(name) {
  const host = drawn(icon(name), "mark");
  host.dataset.tone = tone(name);
  return host;
}

/// One thing one voice said. Every voice is drawn this way — the person, the agent doing the
/// work, and the agents talking to each other — because they are all in the same conversation
/// and drawing them differently made it look like two.
function post(voice, run) {
  const node = text("div", "post");
  node.dataset.who = voice.who;
  node.dataset.tone = tone(voice.who);
  if (voice.mine) node.dataset.mine = "true";
  if (run) {
    node.dataset.run = "true";
  } else {
    const who = text("div", "who");
    who.append(mark(voice.who));
    who.append(text("span", "name", voice.who));
    if (voice.to) who.append(text("span", "to", voice.to));
    if (voice.model) who.append(text("span", "model", voice.model));
    if (voice.at) who.append(text("span", "at", clock(voice.at)));
    node.append(who);
  }
  const body = text("div", "body", voice.text);
  node.append(body);
  return node;
}

/// What to call a seat. A mailbox peer is named after the directory it started in and four
/// random characters, which says nothing about who it is; what it is doing does.
const named = (line) => line.role || line.who;

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
  el("folder-name").textContent = thread ? thread.project : t("No folder");
}

function closeFolders() {
  el("folder-menu").hidden = true;
  el("folder").setAttribute("aria-expanded", "false");
}

async function openFolders() {
  const menu = el("folder-menu");
  menu.replaceChildren();
  const current = state.projects.flatMap((p) => p.threads).find((t) => t.id === state.thread);

  menu.append(text("div", "head", t("Recent")));
  for (const folder of state.folders) {
    const row = document.createElement("button");
    row.type = "button";
    row.append(text("span", "name", folder.name));
    row.append(text("span", "where", folder.root));
    if (current && current.project === folder.name) row.append(drawn("check", "tick"));
    row.addEventListener("click", () => startIn(folder.root));
    menu.append(row);
  }
  if (!state.folders.length) menu.append(text("div", "where", t("Nowhere yet.")));

  menu.append(text("div", "sep"));
  const pick = document.createElement("button");
  pick.type = "button";
  pick.append(text("span", "name", t("Open folder…")));
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
    host.append(text("p", "empty", t("No one is seated yet.")));
    return;
  }
  // Who is in the room now: the seats of the turn in hand. Every earlier turn's seats are in
  // the store too, and listing them all showed one agent three times for having sat three
  // times.
  const latest = thread.seats[thread.seats.length - 1].turn_id;
  for (const seat of thread.seats.filter((s) => s.turn_id === latest)) {
    const node = text("div", "seat");
    node.dataset.state = seat.state;
    node.dataset.who = seat.role;
    node.dataset.tone = tone(seat.role);
    const who = text("div", "who");
    who.append(mark(seat.role));
    who.append(text("span", "name", seat.role));
    if (seat.read_only) who.append(text("span", "ro", "reads only"));
    node.append(who);
    node.append(text("div", "what", [seat.agent, seat.model, seat.reasoning].filter(Boolean).join(" · ")));
    if (seat.peer_status) node.append(text("div", "what doing", seat.peer_status));
    else if (seat.doing) node.append(text("div", "what doing", seat.doing.split("\n")[0]));
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
/// A speaker's own colour, decided from the name so it is the same in every message and in
/// the roster beside them. Eight is enough for a table this size and they stay far apart.
function tone(name) {
  let hash = 0;
  for (const ch of name) hash = (hash * 31 + ch.codePointAt(0)) % 997;
  return String(hash % 8);
}

/// Everything a client reads is timed in milliseconds; the mailbox's own seconds are turned
/// into them before they leave the core, so there is one clock here.
const clock = (at) =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/// Who is in the room, and what each of them is doing. What they said is in the conversation.
async function drawRoom(thread) {
  const host = el("room");
  host.replaceChildren();
  const events = (state.said || []).filter((line) => line.kind !== "message");
  if (!events.length) {
    host.append(text("p", "empty", t("Nobody has come or gone yet.")));
    return;
  }
  for (const line of events.slice(-12)) {
    const what = line.kind === "status" ? line.text : line.kind;
    const row = text("div", "said system");
    row.append(drawn(
      line.kind === "joined" ? "log-in" : line.kind === "left" ? "log-out" : "activity",
    ));
    row.append(text("span", null, `${named(line)} ${what}`));
    host.append(row);
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
  el("thread-title").textContent = thread.thread.title || t("New conversation");
  el("thread-status").textContent = thread.thread.status;
  el("interrupt").hidden = thread.thread.status !== "working";
  // A host can stop while this window stays open — its machine sleeps, it is killed, it
  // crashes — and nothing else here would notice: the turn would claim to be running for ever
  // under an agent that is not there. Looked at on every pass.
  const watch = await call("watch", { thread: thread.thread.id });
  if (watch?.lost) {
    report("The agent running this conversation stopped; the turn was left unfinished.");
  }
  if (watch?.queued) await call("ensure_host", { thread: thread.thread.id });
  // The room is read first: what the agents said to each other is part of the conversation,
  // so the conversation cannot be drawn without it.
  const said = (await call("room", { thread: thread.thread.id })) || [];
  state.said = said;
  drawTimeline(thread, said);
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

// Cmd-Enter sends; Enter is a new line. A message here is often several lines of thinking,
// and the terminal console's Enter is not this window's to copy: there, a stray Enter starts
// over at a prompt, and here it would send half a thought to an agent.
el("message").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
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

// The page ships English; it says the same words in the window's language before it is read.
for (const [id, word] of Object.entries(LABELS)) {
  const node = el(id);
  const label = node.querySelectorAll?.(".label")[0];
  (label || node).textContent = t(word);
}
for (const tab of document.querySelectorAll(".tab")) {
  tab.textContent = t(tab.textContent);
}
for (const [id, word] of [["message", "Message…"], ["say", "Message the team…"]]) {
  el(id).setAttribute("placeholder", t(word));
}

refresh(true);
