// The window's rendering, against view output recorded from a real run.
//
// The app's claim is that a screen is a query. These tests hold it to that: the only input is
// what the views returned, and the only output is what was drawn. If a screen needs something
// the views do not carry, it fails here rather than in a window nobody is watching.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import assert from "node:assert/strict";
import test from "node:test";
import { document, page } from "./dom.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const recorded = {
  sidebar: JSON.parse(readFileSync(join(here, "sidebar.json"), "utf8")).projects,
  thread: JSON.parse(readFileSync(join(here, "thread.json"), "utf8")),
};

/// The window's script, read once and given a fresh scope per test. Importing it again would
/// not do: an ES module is evaluated once per URL, so every test after the first would be
/// looking at the first one's screen.
const source = readFileSync(join(here, "../dist/app.js"), "utf8");
const markup = readFileSync(join(here, "../dist/index.html"), "utf8");

/// Loads the app with the recorded answers in place of a core, and returns what it drew.
async function open(answers = {}, { prompt, pick } = {}) {
  const calls = [];
  page([
    "sidebar", "projects", "sidebar-foot", "new-thread",
    "thread", "thread-head", "thread-where", "thread-title", "thread-status",
    "timeline", "composer", "message", "composer-row", "composer-hint", "interrupt", "send",
    "folder-picker", "folder", "folder-name", "folder-menu",
    "tabs", "pane-team", "pane-changes", "pane-plan",
    "roster", "room", "say-form", "say", "say-hint",
    "scopes", "files", "review-form", "review-list", "review-send",
    "notice", "account", "account-mark", "account-name", "account-chevron", "account-menu",
    "panel", "panel-head", "panel-title", "panel-close", "panel-body",
  ], markup);
  const localStorage = {
    store: new Map(),
    getItem(key) { return this.store.get(key) ?? null; },
    setItem(key, value) { this.store.set(key, value); },
  };
  const window = {
    // The page asks for a comment the way a page does.
    prompt: prompt || (() => null),
    __TAURI__: {
      // The system's own folder picker, which the window never replaces with a typed path.
      dialog: { open: async (options) => (pick ? pick(options) : null) },
      core: {
        invoke(name, args) {
          calls.push([name, args]);
          if (name in answers) return Promise.resolve(answers[name]);
          const fallback = {
            sidebar: recorded.sidebar,
            tree_files: [
              { turn: "", path: "src/auth.ts", change: "modify", added: 9, removed: 2, latest_patch: 0 },
            ],
            tree_patch: "--- a/src/auth.ts\n+++ b/src/auth.ts\n@@ -1,2 +1,2 @@\n-const a = 1;\n+const a = 2;\n",
            comment: "turn",
            folders: [
              { name: "orochi", root: "/work/orochi", threads: 2, updated_at: 2 },
              { name: "web-app", root: "/work/web-app", threads: 1, updated_at: 1 },
            ],
            open_prompts: [],
            thread: recorded.thread,
            changed: [],
            patch: "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\n+new\n",
            seen: null,
            send: "turn",
            answer: true,
            interrupt: null,
          };
          return Promise.resolve(fallback[name] ?? null);
        },
      },
    },
  };
  // A scope of its own, with the globals the page would have given it. The poll is held
  // rather than run, so a test drives time instead of waiting for it.
  let poll = () => {};
  new Function("window", "document", "localStorage", "setInterval", source)(
    window,
    document,
    localStorage,
    (fn) => {
      poll = fn;
      return 0;
    },
  );
  // The app refreshes on load; give its promises a tick to settle.
  await new Promise((resolve) => setTimeout(resolve, 20));
  const tick = async () => {
    await poll();
    await new Promise((resolve) => setTimeout(resolve, 20));
  };
  return { calls, tick, el: (id) => document.getElementById(id) };
}

test("the sidebar lists a project's threads with the mark for their state", async () => {
  const { el } = await open();
  const drawn = el("projects").render();
  assert.match(drawn, /repo/, "the project heads its section");
  assert.match(drawn, /Fix the flaky mailbox placement test/);
  assert.match(drawn, /Refactor the scheduler retry loop/);
  assert.match(drawn, /●/, "and each thread carries its status mark");
});

test("a turn reads as what was asked, who took it, and what came back", async () => {
  const { el } = await open();
  const drawn = el("timeline").render();
  assert.match(drawn, /you\s+Fix the flaky mailbox placement test/);
  assert.match(drawn, /⟡ test · sol-test/, "the route chip says which agent took it");
  assert.match(drawn, /reply/, "and the reply is in the transcript");
  assert.equal(
    el("thread-title").textContent,
    "Fix the flaky mailbox placement test",
    "the header is the thread's name",
  );
});

test("both messages of one conversation are drawn as separate turns", async () => {
  const { el } = await open();
  const turns = el("timeline").children.filter((c) => c.className === "turn");
  assert.equal(turns.length, 2, "one turn per message, not one screen per conversation");
  assert.match(turns[1].render(), /and add a regression test for it/);
});

test("the team pane seats everyone at the turn, read-only marked", async () => {
  const { el } = await open();
  const drawn = el("roster").render();
  assert.match(drawn, /fixer/);
  assert.match(drawn, /tester/);
  assert.match(drawn, /sol-test/, "each seat shows the route it took");
});

test("a question is a card with the agent's own options and no invented ones", async () => {
  const prompt = {
    id: "p1",
    thread_id: recorded.thread.thread.id,
    thread_title: recorded.thread.thread.title,
    project: "repo",
    kind: "permission",
    agent: "test",
    model: "sol-test",
    role: "fixer",
    title: "Run tests",
    detail: "cargo test",
    options: [["ok", "Allow once"], ["always", "Always allow"]],
    created_at: 0,
  };
  const { el, calls } = await open({ open_prompts: [prompt] });
  const card = el("timeline").children.find((c) => c.className === "ask");
  assert.ok(card, "the question is on the timeline where the work is");
  assert.match(card.render(), /Run tests/);
  assert.match(card.render(), /cargo test/);
  const buttons = card.children.find((c) => c.className === "row").children;
  assert.deepEqual(
    buttons.map((b) => b.textContent),
    ["Allow once", "Always allow", "No"],
    "the agent's options, plus refusing — which is the absence of a choice",
  );

  buttons[0].dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(
    calls.find(([name]) => name === "answer")[1],
    { prompt: "p1", option: "ok" },
    "answering names the option the agent offered",
  );
});

test("choosing a thread and sending a message queues a turn in it", async () => {
  const { el, calls } = await open();
  // The window opens on the most recent thread; clicking another is how you leave it.
  const rows = el("projects").querySelectorAll(".thread");
  const wanted = rows.find((row) => row.textContent.includes("Fix the flaky"));
  wanted.dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));

  el("message").value = "and one more thing";
  el("composer").dispatch("submit");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const sent = calls.find(([name]) => name === "send");
  assert.equal(sent[1].text, "and one more thing");
  assert.equal(sent[1].thread, recorded.thread.thread.id);
  assert.equal(el("message").value, "", "the box is empty, as a sent message leaves it");
});

test("the changes pane lists what the turn changed with its stats", async () => {
  const thread = structuredClone(recorded.thread);
  thread.files = [
    { turn: "t", path: "tests/mailbox.rs", change: "modify", added: 12, removed: 4, latest_patch: 1 },
  ];
  const { el } = await open({ thread });
  const drawn = el("files").render();
  assert.match(drawn, /tests\/mailbox\.rs/);
  assert.match(drawn, /\+12/);
  assert.match(drawn, /−4/);
});

test("a quiet tick asks only what changed", async () => {
  const { calls } = await open();
  const reads = calls.filter(([name]) => name === "thread").length;
  assert.equal(reads, 1, "the conversation is read once on open, not per poll");
});

test("the composer says which folder the work happens in", async () => {
  const { el } = await open();
  assert.equal(
    el("folder-name").textContent,
    "repo",
    "the chip names the folder the open thread works in",
  );
});

test("the folder menu offers what has been worked in, and a way to choose another", async () => {
  const { el } = await open();
  el("folder").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const menu = el("folder-menu");
  assert.equal(menu.hidden, false, "clicking the chip opens it");
  const drawn = menu.render();
  assert.match(drawn, /Recent/);
  assert.match(drawn, /orochi/);
  assert.match(drawn, /\/work\/web-app/, "each entry says where it is");
  assert.match(drawn, /Open folder…/, "and the system's own picker is the last resort");
});

test("choosing a folder starts a thread in it", async () => {
  const { el, calls } = await open();
  el("folder").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const entry = el("folder-menu").querySelectorAll("button").find((b) =>
    b.textContent.includes("/work/web-app"),
  );
  entry.dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.deepEqual(
    calls.find(([name]) => name === "new_thread")[1],
    { root: "/work/web-app" },
    "the folder chosen is the folder the thread works in",
  );
  assert.equal(el("folder-menu").hidden, true, "and the menu closes behind it");
});

test("new thread starts one where the work already happens", async () => {
  const { el, calls } = await open();
  el("new-thread").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.deepEqual(
    calls.find(([name]) => name === "new_thread")?.[1],
    { root: recorded.sidebar[0].root },
    "the open thread's own folder, without asking again for what is already known",
  );
  assert.equal(
    el("folder-menu").hidden,
    true,
    "and no menu is left open at the other end of the window",
  );
});

test("new thread asks where, when nowhere has been worked in yet", async () => {
  const picked = [];
  const { el, calls } = await open({ sidebar: [], thread: null, folders: [] }, {
    pick: (options) => {
      picked.push(options);
      return "/work/fresh";
    },
  });
  el("new-thread").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.equal(picked.length, 1, "the system's own picker, not a typed path");
  assert.deepEqual(
    calls.find(([name]) => name === "new_thread")?.[1],
    { root: "/work/fresh" },
    "and the thread starts in what was chosen",
  );
});

test("the composer sends on cmd-enter and breaks the line on enter", async () => {
  const { el, calls } = await open();
  const message = el("message");
  let prevented = 0;

  message.value = "half a thought";
  message.dispatch("keydown", { key: "Enter", preventDefault: () => (prevented += 1) });
  assert.equal(
    calls.some(([name]) => name === "send"),
    false,
    "a bare Enter is a new line, not a send",
  );
  assert.equal(prevented, 0, "and the textarea is left to insert it");

  message.dispatch("keydown", { key: "Enter", metaKey: true, preventDefault: () => (prevented += 1) });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(
    calls.find(([name]) => name === "send")?.[1].text,
    "half a thought",
    "cmd-Enter sends what was typed",
  );
  assert.equal(prevented, 1, "and the line break it would have made is not also inserted");
  // The hint is the page's own words, so it is the page that is read for them.
  assert.match(markup, /\u2318Enter to send/, "the hint says what the keys do");
  assert.doesNotMatch(markup, /Shift-Enter for a new line/, "and not what they used to do");
});

test("a sent message also makes sure something is running it", async () => {
  const { el, calls } = await open();
  el("message").value = "start here";
  el("composer").dispatch("submit");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(
    calls.some(([name]) => name === "ensure_host"),
    "a message is only a row until a host takes it",
  );
});

test("the room shows what agents and the person said, and lets the person answer", async () => {
  const said = [
    { seq: 1, kind: "joined", who: "reviewer", whom: null, via: "mailbox", text: "", at: 1, role: null, model: null },
    { seq: 2, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox", text: "the sleep hides it", at: 2, role: "reviewer", model: "opus" },
  ];
  const { el, calls } = await open({ room: said });
  const drawn = el("room").render();
  assert.match(drawn, /reviewer joined/, "arriving is a line in the room");
  assert.match(drawn, /the sleep hides it/);
  assert.match(drawn, /reviewer → implementer/, "and each message says who it was for");

  el("say").value = "prefer the simpler shape";
  el("say-form").dispatch("submit");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const note = calls.find(([name]) => name === "say");
  assert.deepEqual(note[1].text, "prefer the simpler shape");
  assert.equal(note[1].to, null, "a note with no name is for everyone");
});

test("a working thread is marked in the list rather than given a page", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects[0].threads[0].status = "working";
  projects[0].threads[0].seats = 2;
  const { el } = await open({ sidebar: projects });
  const drawn = el("projects").render();
  assert.match(drawn, /◉/, "the list says which thread is working");
  assert.match(drawn, /2/, "and how many seats it has open, which is what a page would add");
});

test("insights keeps verified evidence and weak signals apart, and says so", async () => {
  const insights = [
    {
      agent: "claude", model: "opus", task_type: "implementation", reasoning: "high",
      verified: 7, failures: 1, weak: 3, success_rate: 6 / 7,
      mean_tokens: 12000, mean_duration_ms: 42000, last_at: 1,
    },
  ];
  const { el } = await open({ insights });
  el("account").dispatch("click");
  el("account-menu").querySelectorAll("button")
    .find((b) => b.dataset.screen === "insights")
    .dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const drawn = el("panel-body").render();
  assert.match(drawn, /86%/, "the success rate is of the verified runs");
  assert.match(drawn, /12,000/);
  assert.match(drawn, /its own estimates/, "and the screen says whose numbers these are");
});

test("agents shows readiness and what is left of a quota", async () => {
  const agents = [
    {
      agent: "codex", model: "*", status: "cooldown", cooling: 300, reset_at: null,
      quota_estimate: 0.1, failures: 2, windows: [["5h", 0.12, null]],
    },
  ];
  const { el } = await open({ agents });
  el("account").dispatch("click");
  el("account-menu").querySelectorAll("button")
    .find((b) => b.dataset.screen === "agents")
    .dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const drawn = el("panel-body").render();
  assert.match(drawn, /codex/);
  assert.match(drawn, /whole account/, "a `*` model is the account, not a model named star");
  assert.match(drawn, /300s/);
});

test("the plan pane draws the waves a divided turn runs in", async () => {
  const board = [
    { id: "alpha", brief: "build alpha", paths: ["alpha"], after: [], wave: 0, state: "merged", strays: 0, seat: null },
    { id: "beta", brief: "build beta", paths: ["beta"], after: [], wave: 0, state: "merged", strays: 0, seat: null },
    { id: "gamma", brief: "build gamma", paths: ["gamma"], after: ["alpha"], wave: 1, state: "waiting", strays: null, seat: null },
  ];
  const { el } = await open({ board });
  const drawn = el("pane-plan").render();
  assert.match(drawn, /Wave 1/);
  assert.match(drawn, /Wave 2/, "each wave is a heading of its own");
  assert.match(drawn, /build gamma/);
});

test("settings edit the conversation store and what is remembered", async () => {
  const settings = { activity: { enabled: true, retention_days: 30, thinking: true } };
  const { el, calls } = await open({
    settings,
    memory: { user: "Prefers small commits.\n", path: "/data/memory/USER.md" },
  });
  el("account").dispatch("click");
  el("account-menu").querySelectorAll("button")
    .find((b) => b.dataset.screen === "settings")
    .dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const host = el("panel-body");
  assert.match(host.render(), /Keep conversations for/);
  assert.match(host.render(), /Prefers small commits/, "memory is shown as the text it is");
  assert.match(host.render(), /never sent to a routing adviser/);

  const wipe = host.querySelectorAll("button").find((b) =>
    b.textContent.startsWith("Delete every"),
  );
  wipe.dispatch("click");
  assert.equal(
    calls.some(([name]) => name === "forget_all"),
    false,
    "deleting everything takes two clicks, not one",
  );
  wipe.dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(calls.some(([name]) => name === "forget_all"), "and then it happens");
});

test("the changes pane switches between what was recorded and what is in the tree", async () => {
  const thread = structuredClone(recorded.thread);
  thread.files = [
    { turn: "t", path: "tests/mailbox.rs", change: "modify", added: 12, removed: 4, latest_patch: 1 },
  ];
  const { el, calls } = await open({ thread });
  assert.match(el("files").render(), /tests\/mailbox\.rs/, "it opens on the turn's own patches");

  el("scopes").querySelectorAll("button")
    .find((b) => b.dataset.scope === "unstaged")
    .dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(
    calls.find(([name]) => name === "tree_files")[1],
    { thread: thread.thread.id, scope: "unstaged" },
    "and asks git for the scope that was chosen",
  );
  assert.match(el("files").render(), /src\/auth\.ts/, "showing what is actually in the tree");
});

test("comments on a diff collect, and go as one message", async () => {
  const thread = structuredClone(recorded.thread);
  thread.files = [
    { turn: "t", path: "tests/mailbox.rs", change: "modify", added: 12, removed: 4, latest_patch: 1 },
  ];
  const { el, calls } = await open(
    { thread, patch: "@@ -1,1 +1,1 @@\n-old\n+new\n" },
    { prompt: () => "this allocates in a loop" },
  );
  assert.equal(el("review-form").hidden, true, "nothing to send yet");

  // Open the file, then comment on a line.
  el("files").querySelectorAll("div").find((d) => d.className === "file").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const lines = el("files").querySelectorAll("span").filter((n) => n.className === "d");
  lines[0].dispatch("click");

  assert.equal(el("review-form").hidden, false, "a comment makes the review sendable");
  assert.match(el("review-list").render(), /this allocates in a loop/);

  el("review-form").dispatch("submit");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const sent = calls.find(([name]) => name === "comment");
  assert.equal(sent[1].comments.length, 1);
  assert.equal(sent[1].comments[0][0], "tests/mailbox.rs");
  assert.equal(sent[1].comments[0][2], "this allocates in a loop");
  assert.equal(el("review-form").hidden, true, "and the list empties behind it");
});

test("a thread that appears while the window is open is drawn, not just listed", async () => {
  // The window opens on an empty store, as a first run does.
  const answers = { sidebar: [], thread: null, folders: [], changed: [] };
  const { el, calls, tick } = await open(answers);
  assert.match(el("timeline").render(), /Choose a folder/, "nothing to show yet");

  // Someone starts a thread in a terminal; the feed names it on the next tick.
  const project = structuredClone(recorded.sidebar[0]);
  answers.sidebar = [project];
  answers.thread = recorded.thread;
  answers.changed = [project.threads[0].id];
  await tick();

  assert.equal(
    el("thread-title").textContent,
    recorded.thread.thread.title,
    "the first thread to appear is drawn, not only added to the list",
  );
  assert.ok(
    calls.some(([name]) => name === "seen"),
    "and looking at it is what makes it read",
  );
});

test("everything that is not a conversation lives behind the row at the foot", async () => {
  const { el } = await open();
  assert.equal(el("account-menu").hidden, true, "it is a menu, not a row of buttons");
  assert.equal(el("panel").open, false, "and nothing is over the conversation");

  el("account").dispatch("click");
  const items = el("account-menu")
    .querySelectorAll("button")
    .map((b) => b.textContent.replace("✓", ""));
  assert.deepEqual(
    items,
    ["Agents", "Insights", "Settings"],
    "with settings kept apart from the two above it",
  );

  el("account-menu").querySelectorAll("button")
    .find((b) => b.dataset.screen === "agents")
    .dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("account-menu").hidden, true, "choosing one closes the menu");
  assert.equal(el("panel").open, true, "and opens it over the conversation");
  assert.equal(el("panel-title").textContent, "Agents");
  assert.equal(
    el("timeline").hidden,
    false,
    "the conversation stays where it is; a panel is asked for and dismissed",
  );

  el("panel-close").dispatch("click");
  assert.equal(el("panel").open, false, "and dismissed is dismissed");
});

test("a thread nobody has written in yet says so, rather than being Untitled", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects[0].threads[0].title = "";
  const { el } = await open({ sidebar: projects, thread: { ...recorded.thread, thread: { ...recorded.thread.thread, title: "" } } });
  assert.match(
    el("projects").render(),
    /New conversation/,
    "an empty thread is one you have not started, not one nobody named",
  );
  assert.equal(el("thread-title").textContent, "New conversation");
});

test("the room fills in as the agents talk, without being asked", async () => {
  const answers = {
    room: [
      { seq: 1, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox",
        text: "the sleep hides it", at: 1, role: "reviewer", model: "opus" },
    ],
    changed: [],
  };
  const { el, tick } = await open(answers);
  assert.match(el("room").render(), /the sleep hides it/);

  // A seat says something else while the window is open; the feed names the conversation.
  answers.room = [
    ...answers.room,
    { seq: 2, kind: "message", who: "implementer", whom: "all", via: "mailbox",
      text: "taking tests/mailbox.rs", at: 2, role: "implementer", model: "opus" },
  ];
  // The conversation the window is showing: the first in the sidebar.
  answers.changed = [recorded.sidebar[0].threads[0].id];
  await tick();

  assert.match(
    el("room").render(),
    /taking tests\/mailbox\.rs/,
    "what the agents say appears as they say it, not when you next click",
  );
});
