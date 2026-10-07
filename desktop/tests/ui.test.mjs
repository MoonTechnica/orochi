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
const css = readFileSync(join(here, "../dist/app.css"), "utf8");

/// Loads the app with the recorded answers in place of a core, and returns what it drew.
async function open(answers = {}, { prompt, pick, language, stored = {} } = {}) {
  const calls = [];
  page([
    "sidebar", "projects", "projects-head", "projects-label", "add-project", "sidebar-foot", "new-thread", "activity", "activity-count",
    "status-bar", "accounts", "counts", "route-picker", "route", "route-name", "route-menu",
    "effort", "effort-name",
    "thread", "thread-head", "thread-where", "thread-title", "thread-status",
    "timeline", "composer", "message", "composer-row", "composer-hint", "interrupt", "send",
    "folder-picker", "folder", "folder-name", "folder-menu",
    "tabs", "pane-team", "pane-changes", "pane-plan",
    "roster", "room", "say-form", "say", "say-hint",
    "scopes", "files", "review-form", "review-list", "review-send",
    "notice", "account", "account-mark", "account-name", "account-chevron", "account-menu",
    "panel", "panel-head", "panel-title", "panel-close", "panel-body",
    "place", "place-head", "place-title", "place-close", "place-body", "place-root-label",
    "place-root", "place-modes-label", "place-modes", "place-note", "place-more",
    "place-advanced", "place-docker", "place-error", "place-foot", "place-cancel", "place-go",
    "app", "side", "sidebar-resizer", "side-resizer",
    "pane-files", "files-head", "files-where", "files-refresh",
    "tree-search", "tree-by", "tree", "viewer",
    "context-menu", "confirm", "confirm-title", "confirm-body", "confirm-foot", "confirm-cancel", "confirm-go",
  ], markup);
  const localStorage = {
    store: new Map(Object.entries(stored)),
    getItem(key) { return this.store.get(key) ?? null; },
    setItem(key, value) { this.store.set(key, value); },
  };
  const window = {
    // The page asks for a comment the way a page does.
    prompt: prompt || (() => null),
    navigator: { language: language || "en-US" },
    innerWidth: 1400,
    listeners: new Map(),
    addEventListener(name, handler) { this.listeners.set(name, handler); },
    __TAURI__: {
      // The system's own folder picker, which the window never replaces with a typed path.
      dialog: { open: async (options) => (pick ? pick(options) : null) },
      core: {
        invoke(name, args) {
          calls.push([name, args]);
          if (name in answers) {
            const answer = answers[name];
            if (answer instanceof Error) return Promise.reject(answer.message);
            return Promise.resolve(typeof answer === "function" ? answer(args) : answer);
          }
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
  return { calls, tick, window, localStorage, el: (id) => document.getElementById(id) };
}

/// Every test here builds rows by hand, and a name invented in one of them is a name the page
/// can then read for ever without anything failing: `seats[].turn_id` did not exist, the page
/// looked for it, no seat was ever found, and three tests passed over it. What the core sends
/// is what these fixtures are made of, so a field that is not there is caught here.
test("the fixtures carry the fields the core actually sends", async () => {
  const seat = recorded.thread.seats[0];
  const item = recorded.thread.items[0];
  for (const [what, row, fields] of [
    ["seat", seat, ["seat_id", "turn", "ordinal", "role", "lead", "read_only", "state"]],
    ["item", item, ["seq", "turn", "lane", "kind", "text", "at"]],
  ]) {
    for (const field of fields) {
      assert.ok(field in row, `${what} has no ${field}: ${Object.keys(row)}`);
    }
  }
  // And the page reads no seat field the core does not send. `seat.` only: `s.` is any short
  // name in the file, and guessing at those is how a wrong field got read in the first place.
  const source = readFileSync(join(here, "../dist/app.js"), "utf8");
  for (const [, field] of source.matchAll(/\bseat\.([a-z_]+)/g)) {
    assert.ok(field in seat, `the page reads seat.${field}, which is not sent`);
  }
});

test("the sidebar lists a project's threads with the mark for their state", async () => {
  const { el } = await open();
  const drawn = el("projects").render();
  assert.match(drawn, /repo/, "the project heads its section");
  assert.match(drawn, /Fix the flaky mailbox placement test/);
  assert.match(drawn, /Refactor the scheduler retry loop/);
  assert.ok(
    el("projects").querySelectorAll(".state").length >= 2,
    "and each thread carries its status mark",
  );
});

test("a turn reads as what was asked, who took it, and what came back", async () => {
  const { el } = await open();
  const drawn = el("timeline").render();
  assert.match(drawn, /you[\s\S]*Fix the flaky mailbox placement test/);
  assert.match(drawn, /test · sol-test/, "the route chip says which agent took it");
  assert.match(drawn, /Fixture completed\./, "and the reply is in the conversation");
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

test("the team pane seats whoever is at the turn in hand, read-only marked", async () => {
  const { el } = await open();
  const drawn = el("roster").render();
  // The recorded conversation has two turns, one seat each. The room is the second one's.
  assert.match(drawn, /tester/);
  assert.doesNotMatch(drawn, /fixer/, "not everyone who ever sat in this conversation");
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
    read_only: false,
    lead: true,
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

test("a project's own plus starts a thread in that project", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects.push({ ...structuredClone(projects[0]), id: "other", name: "web-app", root: "/work/web-app" });
  const { el, calls } = await open({ sidebar: projects });
  const adds = el("projects").querySelectorAll(".add");
  assert.equal(adds.length, projects.length, "one on every project");
  let prevented = 0;
  adds[1].dispatch("click", { preventDefault: () => (prevented += 1) });
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.deepEqual(
    calls.find(([name]) => name === "new_thread")?.[1],
    { root: "/work/web-app" },
    "the project clicked, not the one in view",
  );
  assert.equal(prevented, 1, "and the click does not also fold the project");
});

test("adding a project opens its dialog first, and the folder is chosen inside it", async () => {
  const picked = [];
  const { el, calls } = await open({ placement: NEW }, {
    pick: (options) => {
      picked.push(options);
      return "/work/fresh";
    },
  });
  el("add-project").dispatch("click");
  await settle();

  assert.equal(el("place").open, true, "even with a thread open: this is for somewhere new");
  assert.equal(picked.length, 0, "the picker waits to be asked for");
  assert.equal(el("place-go").disabled, true, "nothing to create before there is a folder");

  el("place-root").dispatch("click");
  await settle();
  assert.equal(picked.length, 1, "the system's own picker, from inside the dialog");
  assert.match(el("place-root").textContent, /\/work\/fresh/);
  assert.equal(el("place-go").disabled, false);
  choose(el, "container");
  el("place-go").dispatch("click");
  await settle();

  assert.deepEqual(calls.find(([name]) => name === "place")[1], { root: "/work/fresh", mode: "container", docker: true });
  assert.deepEqual(calls.find(([name]) => name === "new_thread")?.[1], { root: "/work/fresh" });
  assert.equal(el("place").open, false);
});

test("adding a folder that is already a project keeps where it runs", async () => {
  const { el, calls } = await open(
    { placement: { ask: false, mode: "container", tier: "container", docker: true, name: "repo" } },
    { pick: () => "/work/orochi" },
  );
  el("add-project").dispatch("click");
  await settle();
  el("place-root").dispatch("click");
  await settle();

  const modes = el("place-modes").querySelectorAll("button");
  assert.equal(modes.find((b) => b.dataset.mode === "container").getAttribute("aria-checked"), "true");
  assert.ok(modes.every((b) => b.disabled), "changed under Sandboxes, not here");
  el("place-go").dispatch("click");
  await settle();
  assert.equal(calls.some(([name]) => name === "place"), false);
  assert.deepEqual(calls.find(([name]) => name === "new_thread")?.[1], { root: "/work/orochi" });
});

test("dismissing a new project before choosing a folder starts nothing", async () => {
  const { el, calls } = await open({}, { pick: () => "/work/fresh" });
  el("add-project").dispatch("click");
  await settle();
  el("place-cancel").dispatch("click");
  await settle();
  assert.equal(el("place").open, false);
  assert.equal(calls.some(([name, args]) => name === "new_thread" || args?.root === "/work/fresh"), false);
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

const NEW = { ask: true, mode: "host", tier: null, docker: true, name: null };
async function freshFolder(answers = {}) {
  const opened = await open(
    { sidebar: [], thread: null, folders: [], placement: NEW, ...answers },
    { pick: () => "/work/fresh" },
  );
  opened.el("new-thread").dispatch("click");
  await settle();
  return opened;
}
const choose = (el, mode) =>
  el("place-modes").querySelectorAll("button").find((b) => b.dataset.mode === mode).dispatch("click");

test("a new project is asked where it runs before its first thread, this Mac first", async () => {
  const { el, calls } = await freshFolder();
  assert.equal(el("place").open, true, "the question is open");
  assert.match(el("place-root").textContent, /\/work\/fresh/);
  const modes = el("place-modes").querySelectorAll("button");
  assert.deepEqual(modes.map((b) => b.dataset.mode), ["host", "runner", "container", "vm"]);
  assert.equal(modes[0].getAttribute("aria-checked"), "true", "running here stays the default");
  assert.equal(el("place-docker").disabled, true, "Docker is a sandbox's to have");
  assert.equal(calls.some(([name]) => name === "new_thread"), false, "nothing starts before the answer");
});

test("choosing a sandbox makes it before the thread starts", async () => {
  const { el, calls } = await freshFolder();
  choose(el, "container");
  assert.equal(el("place-docker").disabled, false);
  assert.equal(el("place-docker").getAttribute("aria-checked"), "true", "with Docker inside by default");
  assert.match(el("place-note").textContent, /localhost:1355/, "and where its services will be");
  el("place-go").dispatch("click");
  await settle();
  const names = calls.map(([name]) => name);
  assert.deepEqual(calls.find(([name]) => name === "place")[1], {
    root: "/work/fresh",
    mode: "container",
    docker: true,
  });
  assert.ok(names.indexOf("place") < names.indexOf("new_thread"), "the sandbox exists before the thread");
  assert.equal(el("place").open, false);
});

test("running here with the project in a sandbox is a choice of its own, with Docker inside", async () => {
  const { el, calls } = await freshFolder();
  choose(el, "runner");
  assert.equal(el("place-docker").disabled, false, "the project's runtime has Docker");
  el("place-go").dispatch("click");
  await settle();
  assert.deepEqual(calls.find(([name]) => name === "place")[1], { root: "/work/fresh", mode: "runner", docker: true });
});

test("a sandbox without Docker is asked for under advanced", async () => {
  const { el, calls } = await freshFolder();
  choose(el, "container");
  el("place-more").dispatch("click");
  assert.equal(el("place-advanced").hidden, false);
  el("place-docker").dispatch("click");
  el("place-go").dispatch("click");
  await settle();
  assert.equal(calls.find(([name]) => name === "place")[1].docker, false);
});

test("dismissing the question starts nothing", async () => {
  const { el, calls } = await freshFolder();
  el("place-cancel").dispatch("click");
  await settle();
  assert.equal(el("place").open, false);
  assert.equal(calls.some(([name]) => name === "place" || name === "new_thread"), false);
});

test("a sandbox that cannot be made says why and keeps the question open", async () => {
  const { el, calls } = await freshFolder({
    place: new Error("no sbx-golden image; run `orochi sandbox up` and `orochi sandbox image build` first"),
  });
  choose(el, "container");
  el("place-go").dispatch("click");
  await settle();
  assert.equal(el("place").open, true);
  assert.equal(el("place-error").hidden, false);
  assert.match(el("place-error").textContent, /image build/);
  assert.equal(el("place-go").disabled, false, "and it can be tried again or changed");
  assert.equal(calls.some(([name]) => name === "new_thread"), false);
});

test("a folder already worked in is not asked again", async () => {
  const { el, calls } = await open({ placement: { ...NEW, ask: false } });
  el("new-thread").dispatch("click");
  await settle();
  assert.equal(el("place").open ?? false, false);
  assert.ok(calls.some(([name]) => name === "new_thread"));
});

const SANDBOXES = {
  client: "lima",
  vm: "Running",
  reachable: true,
  projects: [
    {
      name: "web-app", root: "/work/web-app", mode: "container", tier: "container", docker: true,
      status: "running", address: "10.203.0.7", url: "http://<port>-web-app.localhost:1355", focused: [54323], busy: false,
    },
    {
      name: "api", root: "/work/api", mode: "host", tier: "container", docker: true,
      status: "stopped", address: null, url: "http://<port>-api.localhost:1355", focused: [], busy: false,
    },
  ],
  auth: [
    { agent: "codex", files: [".codex/auth.json"], variables: [], missing: false, hint: null },
    { agent: "claude", files: [], variables: [], missing: true, hint: "`orochi sandbox login claude` runs `claude setup-token`" },
    { agent: "gemini", files: [], variables: [["GEMINI_API_KEY", "secret"]], missing: false, hint: null },
  ],
  host_ports: [11434, 1234],
  jobs: [
    { id: "2", request: { op: "image", vm: false }, started_at: 2, exit: null, tail: "image: installing docker" },
    { id: "1", request: { op: "focus", root: "/work/web-app" }, started_at: 1, exit: 0, tail: "http://127.0.0.1:54323" },
  ],
};
async function sandboxScreen(answers = {}) {
  const opened = await open({ sandboxes: SANDBOXES, sandbox_job: "3", ...answers });
  opened.el("account").dispatch("click");
  opened.el("account-menu").querySelectorAll("button")
    .find((b) => b.dataset.screen === "sandboxes")
    .dispatch("click");
  await settle();
  const project = (name) =>
    opened.el("panel-body").querySelectorAll("div").find((d) => d.dataset.name === name);
  const press = async (node, label) => {
    node.querySelectorAll("button").find((b) => b.textContent === label).dispatch("click");
    await settle();
  };
  return { ...opened, project, press };
}
const jobs = (calls) => calls.filter(([name]) => name === "sandbox_job").map(([, a]) => a.request);

test("the sandboxes screen shows the host, each project, where it runs and how to reach it", async () => {
  const { el, project } = await sandboxScreen();
  const drawn = el("panel-body").render();
  assert.match(drawn, /Host VM: running/);
  const web = project("web-app").render();
  assert.match(web, /\/work\/web-app/);
  assert.match(web, /http:\/\/<port>-web-app\.localhost:1355/, "its services through the gateway, nothing to set up here");
  assert.match(web, /127\.0\.0\.1:54323/, "and the focused ports as its tools print them");
  const checked = (name) =>
    project(name).querySelectorAll("button").find((b) => b.getAttribute("aria-checked") === "true").dataset.mode;
  assert.equal(checked("web-app"), "container");
  assert.equal(checked("api"), "host");
  assert.doesNotMatch(project("api").render(), /api\.localhost/, "a project running here has no sandbox address to offer");
  assert.match(drawn, /installing docker/, "a running operation shows what it is saying");
});

test("changing where a project runs, focusing and building are operations the core runs", async () => {
  const { el, calls, project, press } = await sandboxScreen();
  project("api").querySelectorAll("button").find((b) => b.dataset.mode === "container").dispatch("click");
  await settle();
  await press(project("web-app"), "Unfocus");
  assert.equal(
    project("api").querySelectorAll("button").some((b) => b.textContent === "Snapshot"),
    false,
    "a project running here has no sandbox to snapshot",
  );
  assert.deepEqual(jobs(calls).slice(0, 2), [
    { op: "mode", root: "/work/api", mode: "container" },
    { op: "unfocus" },
  ]);
  assert.equal(
    el("panel-body").querySelectorAll("button").some((b) => b.textContent === "Set up network"),
    false,
    "nothing here asks for an administrator: the gateway needs no setup on this machine",
  );
  const build = el("panel-body").querySelectorAll("button").find((b) => b.textContent === "Build image");
  assert.equal(build.disabled, true, "an image already being built is not started twice");
});

test("deleting a sandbox asks by being pressed twice", async () => {
  const { calls, project, press } = await sandboxScreen();
  await press(project("web-app"), "Delete");
  assert.deepEqual(jobs(calls), [], "the first press only asks");
  assert.match(project("web-app").render(), /Delete\? The folder is kept/);
  await press(project("web-app"), "Delete? The folder is kept");
  assert.deepEqual(jobs(calls), [{ op: "remove", root: "/work/web-app" }]);
});

test("a host VM that is not there is offered to be set up, and nothing that needs it is", async () => {
  const { el, calls } = await sandboxScreen({
    sandboxes: { ...SANDBOXES, vm: null, reachable: false, projects: [], jobs: [] },
  });
  const drawn = el("panel-body").render();
  assert.match(drawn, /not created/);
  assert.match(drawn, /asked where it runs/);
  const buttons = el("panel-body").querySelectorAll("button");
  assert.equal(buttons.find((b) => b.textContent === "Build image").disabled, true);
  buttons.find((b) => b.textContent === "Set up VM").dispatch("click");
  await settle();
  assert.deepEqual(jobs(calls), [{ op: "setup" }]);
});

test("environment initialization stays visible while running and prevents conflicting host jobs", async () => {
  const { el } = await sandboxScreen({
    sandboxes: { ...SANDBOXES, jobs: [{ id: "setup-1", request: { op: "setup" }, exit: null, tail: "Installing Lima…", started_at: 1 }] },
  });
  const buttons = el("panel-body").querySelectorAll("button");
  for (const label of ["Initialize sandbox environment", "Stop VM", "Build image"]) {
    assert.equal(buttons.find((b) => b.textContent === label).disabled, true);
  }
  assert.match(el("panel-body").render(), /Installing Lima/);
});

test("a local Linux host offers Incus initialization and lets a failed setup be retried", async () => {
  const { el, calls } = await sandboxScreen({
    sandboxes: { ...SANDBOXES, client: "incus", local: true, vm: null, reachable: false,
      projects: [], jobs: [{ id: "setup-1", request: { op: "setup" }, exit: 1,
        tail: "Run orochi init in a terminal to authorize setup", started_at: 1 }] },
  });
  assert.match(el("panel-body").render(), /Local Incus host/);
  assert.match(el("panel-body").render(), /authorize setup/);
  const buttons = el("panel-body").querySelectorAll("button");
  assert.equal(buttons.some((b) => b.textContent === "Set up VM"), false);
  const setup = buttons.find((b) => b.textContent === "Initialize sandbox environment");
  assert.equal(setup.disabled, false);
  setup.dispatch("click");
  await settle();
  assert.deepEqual(jobs(calls), [{ op: "setup" }]);
});

test("the sandboxes screen says how each agent is signed in inside, by name and never by value", async () => {
  const { el } = await sandboxScreen();
  const auth = el("panel-body").querySelectorAll("div").filter((d) => d.dataset.agent);
  const said = (agent) => auth.find((d) => d.dataset.agent === agent).render();
  assert.match(said("codex"), /~\/\.codex\/auth\.json/, "its sign-in file goes in");
  assert.match(said("gemini"), /GEMINI_API_KEY \(stored key\)/);
  assert.match(said("claude"), /nothing carried/);
  assert.match(said("claude"), /claude setup-token/, "with what to do about it");
  assert.match(el("panel-body").render(), /11434, 1234/, "and which local servers it reaches");
});

test("a Claude token is made in a terminal, and an API key goes to the keychain from a password field", async () => {
  const { el, calls } = await sandboxScreen({ sandbox_login: null, sandbox_secret: null });
  const claude = el("panel-body").querySelectorAll("div").find((d) => d.dataset.agent === "claude");
  claude.querySelectorAll("button").find((b) => b.textContent === "Get a token").dispatch("click");
  await settle();
  assert.deepEqual(calls.find(([n]) => n === "sandbox_login")[1], { agent: "claude" });

  const inputs = el("panel-body").querySelectorAll("input");
  const name = inputs.find((i) => i.id === "sbx-secret-name");
  const value = inputs.find((i) => i.id === "sbx-secret-value");
  assert.equal(value.type, "password");
  name.value = "OPENROUTER_API_KEY";
  value.value = "sk-or-secret";
  name.dispatch("input");
  el("panel-body").querySelectorAll("button").find((b) => b.textContent === "Store key").dispatch("click");
  await settle();
  assert.deepEqual(calls.find(([n]) => n === "sandbox_secret")[1], { name: "OPENROUTER_API_KEY", value: "sk-or-secret" });
  assert.equal(value.value, "", "and the field does not keep it");
});

test("a thread in a sandboxed project says so in its header", async () => {
  const { el } = await open({
    placement: { ask: false, mode: "container", tier: "container", docker: true, name: "repo" },
  });
  assert.match(el("thread-where").render(), /Sandbox · container · repo/);
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
  assert.match(markup, /Cmd-Enter to send/, "the hint says what the keys do");
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

test("what the agents said reaches the conversation, and the person can answer them", async () => {
  const said = [
    { seq: 1, kind: "joined", who: "reviewer", whom: null, via: "mailbox", text: "", at: 1, role: null, model: null },
    { seq: 2, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox", text: "the sleep hides it", at: 2, role: "reviewer", model: "opus" },
  ];
  const { el, calls } = await open({ room: said });
  assert.match(el("room").render(), /reviewer joined/, "arriving is a line beside the room");
  const drawn = el("timeline").render();
  assert.match(drawn, /the sleep hides it/, "and what was said is in the conversation");
  assert.match(drawn, /to implementer/, "and each message says who it was for");

  el("say").value = "prefer the simpler shape";
  el("say-form").dispatch("submit");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const note = calls.find(([name]) => name === "say");
  assert.deepEqual(note[1].text, "prefer the simpler shape");
  assert.equal(note[1].to, null, "a note with no name is for everyone");
});

test("the conversation reads as a group chat: who is speaking, to whom, and when", async () => {
  const said = [
    { seq: 1, kind: "joined", who: "implementer", whom: null, via: "mailbox", text: "", at: 100, role: null, model: null },
    { seq: 2, kind: "message", who: "implementer", whom: "all", via: "mailbox", text: "anything to know before I touch the parser?", at: 120, role: "implementer", model: "sonnet" },
    { seq: 3, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox", text: "the sleep hides it", at: 140, role: "reviewer", model: "opus" },
    { seq: 4, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox", text: "and the retry never fires", at: 150, role: "reviewer", model: "opus" },
    { seq: 5, kind: "message", who: "you", whom: "all", via: "user", text: "prefer the simpler shape", at: 160, role: null, model: null },
  ];
  const { el } = await open({ room: said });
  const room = el("timeline");
  const drawn = room.render();

  // Every speaker is a face, and the same speaker is always the same face.
  assert.ok(room.querySelectorAll(".mark").length >= 3, `every speaker is marked: ${drawn}`);
  const tone = (name) => room.querySelectorAll(".post").find((n) => n.dataset.who === name)?.dataset.tone;
  assert.ok(tone("reviewer"), "a speaker's colour is decided from the name");
  assert.equal(
    room.querySelectorAll(".post").filter((n) => n.dataset.who === "reviewer").map((n) => n.dataset.tone)
      .every((t, _, all) => t === all[0]),
    true,
    "and never changes between their messages",
  );

  // One header for a run of messages from the same speaker, as a chat groups them: the
  // reviewer says two things and is named once.
  const reviewer = room.querySelectorAll(".post").filter((n) => n.dataset.who === "reviewer");
  assert.equal(reviewer.length, 2, `both are shown: ${drawn}`);
  assert.equal(
    reviewer.filter((n) => n.querySelectorAll(".who").length).length,
    1,
    "and named once between them",
  );
  assert.match(drawn, /and the retry never fires/, "the follow-up is still shown");

  // Who it was for, in words rather than an arrow.
  assert.match(drawn, /everyone/, "a broadcast is to everyone");
  assert.doesNotMatch(drawn, /→ all/, "and never says 'all'");
  assert.match(drawn, /to implementer/, "a direct message names the one it was for");

  // The person is in the room too, and is marked as themselves.
  const mine = room.querySelectorAll(".post").find((n) => n.dataset.who === "you");
  assert.equal(mine.dataset.mine, "true", `the person's own note is theirs: ${drawn}`);

  // Arriving and leaving are not messages, and are not in the conversation.
  assert.doesNotMatch(drawn, /implementer joined/);
  assert.equal(
    el("room").querySelectorAll(".said.system").length,
    1,
    "a join is one system line, beside the conversation",
  );
});

test("the conversation is the group chat: the agents talk to each other in it", async () => {
  const thread = structuredClone(recorded.thread);
  thread.items = [
    { seq: 1, turn: "t1", turn_ordinal: 1, lane: 0, role: "implementer", agent: "claude", model: "sonnet", kind: "user_message", status: null, text: "split the parser", data: null, truncated: false, patches: 0, at: 100 },
    { seq: 2, turn: "t1", turn_ordinal: 1, lane: 0, role: "implementer", agent: "claude", model: "sonnet", kind: "agent_message", status: null, text: "done, one function per rule", data: null, truncated: false, patches: 0, at: 160 },
  ];
  const said = [
    { seq: 1, kind: "message", who: "implementer", whom: "all", via: "mailbox", text: "anything to know before I touch it?", at: 120, role: "implementer", model: "sonnet" },
    { seq: 2, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox", text: "the retry never fires", at: 140, role: "reviewer", model: "opus" },
  ];
  const { el } = await open({ thread, room: said });
  const drawn = el("timeline").render();

  assert.match(drawn, /split the parser/, "what the person asked");
  assert.match(drawn, /anything to know before I touch it\?/, "what one agent asked the other");
  assert.match(drawn, /the retry never fires/, "and what it answered");
  assert.match(drawn, /done, one function per rule/, "and the reply to the person");

  // In the order they were said, not the transcript first and the room after.
  const at = (needle) => drawn.indexOf(needle);
  assert.ok(
    at("split the parser") < at("anything to know") &&
      at("anything to know") < at("the retry never fires") &&
      at("the retry never fires") < at("done, one function"),
    `one conversation, in time order: ${drawn}`,
  );

  // Each voice is a face, and the reviewer is in the middle of the page rather than beside it.
  assert.ok(
    el("timeline").querySelectorAll(".mark").length >= 2,
    `the agents are marked here: ${drawn}`,
  );

  // The side pane is who is in the room, not a second copy of what was said.
  assert.doesNotMatch(
    el("room").render(),
    /the retry never fires/,
    "the same message is not shown twice in two places",
  );
});

test("the window speaks the language the person set their machine to", async () => {
  const japanese = await open({}, { language: "ja-JP" });
  const drawn = japanese.el("composer-row").render() + japanese.el("sidebar").render();
  assert.match(drawn, /\u9001\u4fe1/, `its own words are in the same language: ${drawn}`);
  assert.doesNotMatch(drawn, /Cmd-Enter to send/);

  // Anywhere else, and where the language is not one it has words for, it says it in English.
  for (const language of ["en-GB", "fi-FI"]) {
    const other = await open({}, { language });
    assert.match(other.el("composer-row").render(), /Cmd-Enter to send/, language);
  }
});

test("a turn with nothing to verify says so, rather than that it was not verified", async () => {
  const thread = structuredClone(recorded.thread);
  const seat = thread.seats[0];
  thread.seats = [{ ...seat, seat_id: "s1", turn: "t1", ordinal: 0, role: "facilitator", read_only: true }];
  const base = {
    turn: "t1", turn_ordinal: 1, lane: 0, role: "facilitator", agent: "claude", model: "haiku",
    status: null, data: null, truncated: false, patches: 0, failed: false,
  };
  thread.items = [
    { ...base, seq: 1, kind: "user_message", text: "discuss it", at: 1000 },
    { ...base, seq: 2, kind: "checks", text: "", at: 2000,
      data: { outcome: "partial_success", checks: [] } },
  ];
  const { el } = await open({ thread, room: [] });
  const drawn = el("timeline").render();
  assert.doesNotMatch(drawn, /unverified/, `nothing was there to verify: ${drawn}`);
  assert.match(drawn, /nothing to verify/i);

  // A seat that could have changed something, and was not checked, still says unverified.
  thread.seats = [{ ...seat, seat_id: "s1", turn: "t1", ordinal: 0, role: "implementer", read_only: false }];
  thread.items[0].role = "implementer";
  thread.items[1].role = "implementer";
  const wrote = await open({ thread, room: [] });
  assert.match(wrote.el("timeline").render(), /unverified/);
});

test("what a failed attempt said is kept, folded, and not read as the agent's own words", async () => {
  const thread = structuredClone(recorded.thread);
  const item = (seq, over) => ({
    seq, turn: "t1", turn_ordinal: 1, lane: 0, role: "facilitator", agent: "codex",
    model: "gpt-5.6-luna", kind: "agent_message", status: "completed", text: "", data: null,
    truncated: false, patches: 0, at: 1000 + seq, failed: false, ...over,
  });
  thread.items = [
    item(1, { kind: "user_message", text: "discuss it", role: null }),
    item(2, { text: "You've hit your usage limit. Upgrade to Pro", failed: true }),
    item(3, { text: "So: tests before, or after?", model: "haiku", agent: "claude" }),
  ];
  const { el } = await open({ thread, room: [] });
  const timeline = el("timeline");
  const drawn = timeline.render();

  assert.match(drawn, /So: tests before, or after\?/, "what the agent did say is what is read");
  const posts = timeline.querySelectorAll(".post");
  assert.deepEqual(
    posts.map((p) => p.dataset.who),
    ["you", "facilitator"],
    "the provider explaining itself is not one of the voices",
  );

  const folds = timeline.querySelectorAll("details").filter((d) => d.className === "aside");
  assert.equal(folds.length, 1, `it is kept, out of the way: ${drawn}`);
  assert.match(folds[0].render(), /attempt that failed/i, "and says what it is");
  assert.match(folds[0].render(), /usage limit/, "and still holds every word of it");
});

test("an agent that stopped is noticed and said, and queued work is picked up again", async () => {
  const answers = { watch: { lost: true, queued: true }, changed: [] };
  const { el, calls } = await open(answers);
  await new Promise((resolve) => setTimeout(resolve, 20));

  assert.match(
    el("notice").render(),
    /stopped/i,
    `the window says the agent went, rather than showing one at work that is not there: ${el("notice").render()}`,
  );
  assert.ok(
    calls.some(([name]) => name === "ensure_host"),
    "and starts one again for the work that was waiting",
  );

  // Nothing wrong, nothing said, and nothing started behind the person's back.
  answers.watch = { lost: false, queued: false };
  const quiet = await open(answers);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.doesNotMatch(quiet.el("notice").render(), /stopped/i);
  assert.equal(
    quiet.calls.some(([name]) => name === "ensure_host"),
    false,
    "a thread with nothing to run is left alone",
  );
});

test("a message that has been sent says so, from the moment it is sent", async () => {
  const thread = structuredClone(recorded.thread);
  thread.thread.status = "working";
  thread.items = [
    { seq: 1, turn: "t1", turn_ordinal: 1, lane: 0, role: null, agent: null, model: null, kind: "user_message", status: null, text: "split the parser", data: null, truncated: false, patches: 0, at: 1000 },
  ];
  // Sent, claimed, and nothing chosen yet: the longest a person waits with nothing to read.
  thread.seats = [];
  const { el } = await open({ thread, room: [] });
  const drawn = el("timeline").render();
  assert.match(drawn, /Starting/i, `something happens the moment it is sent: ${drawn}`);

  // Once a seat exists, it says what that seat is doing rather than that something is.
  thread.seats = [
    { ...recorded.thread.seats[0], seat_id: "s1", turn: "t1", ordinal: 0, role: "implementer",
      state: "working", doing: "reading tests/mailbox.rs", read_only: false },
  ];
  const second = await open({ thread, room: [] });
  const live = second.el("timeline").render();
  assert.match(live, /implementer/, "who is at work");
  assert.match(live, /reading tests\/mailbox\.rs/, "and what they are doing");

  // A thread that is not working says nothing of the sort.
  thread.thread.status = "idle";
  const quiet = await open({ thread, room: [] });
  assert.doesNotMatch(quiet.el("timeline").render(), /Starting/i);
});

test("what the window draws as an icon is drawn, not typed", async () => {
  const { el } = await open();
  // No emoji, and no character standing in for a picture: either is whatever font the machine
  // happens to have, at whatever weight, and never matches the icons beside it. Arrows, box
  // drawing, dingbats, every pictograph block, a keycap and the variation selector that forces
  // emoji presentation. Ordinary punctuation — an em dash, an ellipsis, a minus — is not an
  // icon and is left alone.
  const glyphs =
    /[\u2190-\u21FF\u2300-\u23FF\u25A0-\u27BF\u2B00-\u2BFF\u20E3\uFE0F\u{1F000}-\u{1FBFF}]/u;
  for (const id of ["timeline", "projects", "roster", "room", "composer-row", "sidebar-foot"]) {
    const drawn = el(id).render();
    assert.doesNotMatch(drawn, glyphs, `${id} draws its icons: ${drawn}`);
  }
  // And the markup does not type them either.
  assert.doesNotMatch(markup, glyphs, "nor does the page they sit on");
  assert.doesNotMatch(css, glyphs, "nor the stylesheet, through `content`");
});

test("every voice is drawn the same way, and named by what it is doing", async () => {
  const thread = structuredClone(recorded.thread);
  thread.items = [
    { seq: 1, turn: "t1", turn_ordinal: 1, lane: 0, role: "implementer", agent: "claude", model: "sonnet", kind: "user_message", status: null, text: "split the parser", data: null, truncated: false, patches: 0, at: 1000 },
    { seq: 2, turn: "t1", turn_ordinal: 1, lane: 0, role: "implementer", agent: "claude", model: "sonnet", kind: "agent_message", status: null, text: "done, one function per rule", data: null, truncated: false, patches: 0, at: 4000 },
  ];
  const said = [
    { seq: 1, kind: "message", who: "src-tauri-5e6a", whom: "all", via: "mailbox", text: "anything to know?", at: 2000, role: "implementer", model: "sonnet", agent: "claude", provider: "anthropic" },
    { seq: 2, kind: "message", who: "orochi-8a60", whom: "src-tauri-5e6a", via: "mailbox", text: "the retry never fires", at: 3000, role: "reviewer", model: "opus", agent: "claude", provider: "anthropic" },
  ];
  const { el } = await open({ thread, room: said });
  const timeline = el("timeline");
  const drawn = timeline.render();

  // A session's code is not a name anyone can use.
  assert.doesNotMatch(drawn, /src-tauri-5e6a/, `no session codes: ${drawn}`);
  assert.doesNotMatch(drawn, /orochi-8a60/, "nor in the reply to one");
  assert.match(drawn, /implementer/, "what it is doing is its name");
  assert.match(drawn, /to implementer/, "and that is what it is called when spoken to");
  assert.match(drawn, /reviewer/);

  // One shape for every voice: the person is a speaker like any other.
  const posts = timeline.querySelectorAll(".post");
  assert.equal(posts.length, 4, `you, two agents and the reply: ${drawn}`);
  assert.deepEqual(
    posts.map((p) => p.dataset.who),
    ["you", "implementer", "reviewer", "implementer"],
    "in the order they spoke",
  );
  // Everyone is marked, including you; an agent as the agent it is, as the list draws it.
  const person = posts[0].querySelectorAll(".mark");
  assert.equal(person.length, 1, `you are marked: ${posts[0].render()}`);
  assert.equal(person[0].dataset.icon, "user");
  for (const post of posts.slice(1)) {
    const faces = post.querySelectorAll(".face");
    assert.equal(faces.length, 1, `an agent speaks as its own icon: ${post.render()}`);
    assert.equal(faces[0].getAttribute("title"), "claude");
  }
  assert.equal(
    timeline.querySelectorAll(".you").length,
    0,
    "and nobody is drawn a second way",
  );
});

test("the routing notes are folded away, and the route itself is not", async () => {
  const thread = structuredClone(recorded.thread);
  const note = (seq, text) => ({
    seq, turn: "t1", turn_ordinal: 1, lane: 0, role: "implementer", agent: "claude",
    model: "sonnet", kind: "note", status: null, text, data: null, truncated: false,
    patches: 0, at: 100 + seq,
  });
  thread.items = [
    { ...note(1, "split the parser"), kind: "user_message" },
    note(2, "classifier codex unavailable: Unavailable: cooldown; keeping the local profile"),
    note(3, "classifier gemini unavailable: executable not found: gemini"),
    note(4, "classified as discussion / normal"),
    { ...note(5, "agent/account is in cooldown"), kind: "unavailable" },
    { ...note(6, ""), kind: "route", data: { agent: "claude", model: "sonnet", reasoning: "medium" } },
    { ...note(7, "done"), kind: "agent_message" },
  ];
  const { el } = await open({ thread });
  const timeline = el("timeline");
  const drawn = timeline.render();

  // The route is the one thing here worth a line of its own.
  assert.match(drawn, /claude · sonnet · medium/, "which agent ran is not noise");
  assert.match(drawn, /done/, "and neither is the reply");

  // Four notes about how the route was chosen are one fold, not four lines of body text.
  const folds = timeline.querySelectorAll("details").filter((d) => d.className === "aside");
  assert.equal(folds.length, 1, `the notes are gathered into one: ${drawn}`);
  assert.match(folds[0].render(), /4 routing notes/, "which says how many it holds");
  assert.match(folds[0].render(), /executable not found: gemini/, "and still holds them");
  assert.equal(
    timeline.querySelectorAll(".note").length,
    0,
    "none of them is left loose in the conversation",
  );
});

test("the roster is who is in the room now, not everyone who ever sat", async () => {
  const thread = structuredClone(recorded.thread);
  const seat = thread.seats[0];
  thread.seats = [
    { ...seat, seat_id: "s1", turn: "t1", ordinal: 0, role: "implementer", state: "done" },
    { ...seat, seat_id: "s2", turn: "t2", ordinal: 0, role: "implementer", state: "working" },
    { ...seat, seat_id: "s3", turn: "t2", ordinal: 1, role: "reviewer", state: "working" },
  ];
  const { el } = await open({ thread });
  const roster = el("roster");
  const seats = roster.querySelectorAll(".seat");
  assert.equal(seats.length, 2, `the seats of the turn in hand: ${roster.render()}`);
  assert.deepEqual(
    seats.map((n) => n.dataset.who),
    ["implementer", "reviewer"],
    "and the same name is not listed once per turn it has taken",
  );
});

test("the roster reads as who is in the room, not as a list of machines", async () => {
  const { el } = await open();
  const roster = el("roster");
  const drawn = roster.render();
  assert.ok(roster.querySelectorAll(".mark").length >= 1, `everyone is marked: ${drawn}`);
  // The same name is the same colour here as it is in what they said.
  const seat = roster.querySelectorAll(".seat")[0];
  assert.ok(seat.dataset.tone, `a seat carries its speaker's colour: ${drawn}`);
  assert.equal(seat.dataset.tone, seat.querySelectorAll(".mark")[0].dataset.tone);
});

test("a working thread is marked in the list rather than given a page", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects[0].threads[0].status = "working";
  projects[0].threads[0].seats = 2;
  const { el } = await open({ sidebar: projects });
  const drawn = el("projects").render();
  assert.ok(
    el("projects").querySelectorAll(".state").some((n) => n.dataset.icon === "loader"),
    "the list says which thread is working",
  );
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
    ["Agents", "Insights", "Sandboxes", "Settings"],
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

test("the conversation fills in as the agents talk, without being asked", async () => {
  const answers = {
    room: [
      { seq: 1, kind: "message", who: "reviewer", whom: "implementer", via: "mailbox",
        text: "the sleep hides it", at: 1, role: "reviewer", model: "opus" },
    ],
    changed: [],
  };
  const { el, tick } = await open(answers);
  assert.match(el("timeline").render(), /the sleep hides it/);

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
    el("timeline").render(),
    /taking tests\/mailbox\.rs/,
    "what the agents say appears as they say it, not when you next click",
  );
});

test("the status bar says what each account has left and how many conversations need what", async () => {
  const now = Math.floor(Date.now() / 1000);
  const agents = [
    { agent: "claude", model: "*", status: "available", cooling: 0, reset_at: null, quota_estimate: null, failures: 0,
      windows: [["five_hour", 0.87, now + 71 * 60], ["seven_day", 0.95, now + 86400]] },
    { agent: "codex", model: "*", status: "cooldown", cooling: 240, reset_at: null, quota_estimate: null, failures: 1, windows: [] },
    { agent: "gemini", model: "*", status: "available", cooling: 0, reset_at: null, quota_estimate: null, failures: 0, windows: [] },
  ];
  const projects = structuredClone(recorded.sidebar);
  projects[0].threads[0].status = "needs_you";
  projects[0].threads[1].status = "background";
  const { el } = await open({ agents, sidebar: projects });
  const accounts = el("accounts").render();
  assert.match(accounts, /claude[\s\S]*13%[\s\S]*1h 1[01]m/, "the most spent window, and when it resets");
  assert.match(accounts, /codex[\s\S]*cooling 4m/, "an account cooling down says for how long");
  assert.doesNotMatch(accounts, /gemini/, "an account nothing is known about is left out");
  const counts = el("counts").render();
  assert.match(counts, /1 in the background/);
  assert.match(counts, /1 needs you/);
  assert.equal(el("activity-count").textContent, "1", "and the Activity entry carries what needs you");
});

test("a thread's row says what its lead is doing and lists the helpers still working under it", async () => {
  const projects = structuredClone(recorded.sidebar);
  const thread = projects[0].threads[0];
  thread.status = "working";
  thread.doing = "$ curl -sL https://example.com/llms.txt";
  thread.running_since = Date.now() - 3 * 60 * 1000;
  const live_seats = [
    { seat_id: "a", thread_id: thread.id, project_id: projects[0].id, turn: "t", ordinal: 0, role: "fixer", lead: true,
      read_only: false, title: null, background: false, state: "working", started_at: Date.now(), agent: "claude",
      model: "opus", doing: null, usage: null },
    { seat_id: "b", thread_id: thread.id, project_id: projects[0].id, turn: "t", ordinal: 1, role: "spec", lead: false,
      read_only: true, title: "Research Agents API model support", background: true, state: "working",
      started_at: Date.now(), agent: "codex", model: "gpt", doing: null, usage: null },
  ];
  thread.agents = [
    { role: "fixer", agent: "claude", provider: "anthropic", model: "opus", seats: 1, live: 1, lead: true, background: false, last_at: 1 },
    { role: "spec", agent: "codex", provider: "openai", model: "gpt", seats: 1, live: 1, lead: false, background: true, last_at: 1 },
  ];
  const { el } = await open({ sidebar: projects, live_seats });
  const drawn = el("projects").render();
  assert.match(drawn, /curl -sL https:\/\/example\.com\/llms\.txt/, "what the lead is doing");
  assert.match(drawn, /3m/, "and for how long");
  assert.match(drawn, /spec[\s\S]*Research Agents API model support/, "each helper, with what it is for");
  assert.equal(el("projects").querySelectorAll(".member").length, 2, "the lead and the helper, each a member");
});

test("the Activity screen sorts every conversation by what it needs, and puts done ones away until they change", async () => {
  const projects = structuredClone(recorded.sidebar);
  const [asking, background, ...rest] = projects.flatMap((p) => p.threads);
  asking.status = "needs_you";
  background.status = "background";
  for (const th of rest) th.status = "idle";
  const { el, calls } = await open({ sidebar: projects });
  el("activity").dispatch("click");
  const body = () => el("panel-body");
  const bucket = (name) => body().querySelectorAll(".bucket").find((b) => b.dataset.bucket === name);
  assert.match(bucket("Needs you").render(), new RegExp(asking.title));
  assert.match(bucket("In the background").render(), new RegExp(background.title));
  assert.ok(rest.every((th) => bucket("Done").render().includes(th.title)), bucket("Done").render());

  body().querySelectorAll("button").find((b) => b.textContent === "Clear done").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(rest.every((th) => !bucket("Done").render().includes(th.title)), "done ones are put away");
  assert.match(bucket("Needs you").render(), new RegExp(asking.title), "and nothing else is");
  el("notice").querySelectorAll("button")[0].dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(rest.every((th) => bucket("Done").render().includes(th.title)), "undo brings them back");

  bucket("Needs you").querySelectorAll(".row")[0].dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(calls.some(([name, args]) => name === "seen" && args.thread === asking.id), "a row opens its conversation");
});

test("a finished turn says how long it took, when it ended, what it changed and how much context it left", async () => {
  const thread = structuredClone(recorded.thread);
  thread.thread.status = "idle";
  const turn = thread.items[0].turn;
  thread.items = [
    { ...thread.items[0], kind: "user_message", text: "make the model selectable", at: 1_000_000 },
    { ...thread.items[0], seq: 90, kind: "context", text: "", data: { used: 81000, size: 200000 }, at: 1_010_000 },
    { ...thread.items[0], seq: 91, kind: "agent_message", text: "done", role: "fixer", at: 1_016_000 },
    { ...thread.items[0], seq: 92, turn: "t2", turn_origin: "background", kind: "user_message",
      text: "Background agent `spec` finished", at: 1_100_000 },
  ];
  thread.files = [
    { turn, path: "a.ts", change: "modify", added: 1, removed: 1, latest_patch: 1 },
    { turn, path: "b.ts", change: "modify", added: 2, removed: 0, latest_patch: 2 },
  ];
  const { el } = await open({ thread });
  const drawn = el("timeline").render();
  assert.match(drawn, /Worked for 16s · done \d\d:\d\d/);
  assert.match(drawn, /2 files changed/);
  assert.match(drawn, /context 81k \/ 200k \(41%\)/);
  assert.match(drawn, /from the helpers[\s\S]*Background agent[\s\S]*code spec[\s\S]*finished/, "a turn Orochi wrote is not the person's");
});

test("a conversation's helpers are listed where it is read, each with a way to stop it", async () => {
  const thread = structuredClone(recorded.thread);
  const live_seats = [
    { seat_id: "s1", thread_id: thread.thread.id, project_id: "p", turn: "t", ordinal: 100, role: "spec", lead: false,
      read_only: true, title: "Research Agents API model support", background: true, state: "working",
      started_at: Date.now(), agent: "codex", model: "gpt", doing: null, usage: null },
  ];
  const { el, calls } = await open({ thread, live_seats });
  const drawn = el("timeline").render();
  assert.match(drawn, /1 background agent[\s\S]*spec[\s\S]*Research Agents API model support/);
  const buttons = el("timeline").querySelectorAll("button");
  buttons.find((b) => b.textContent === "Stop").dispatch("click");
  buttons.find((b) => b.textContent === "Stop all").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const stops = calls.filter(([name]) => name === "stop_seat").map(([, args]) => args.seat);
  assert.deepEqual(stops, ["s1", null], "one by its seat, then all of them");
});

test("a question from a seat that is not the lead names it and says why it is asking", async () => {
  const open_prompts = [
    { id: "p1", thread_id: recorded.thread.thread.id, thread_title: "t", project: "repo", kind: "permission",
      agent: "codex", model: "gpt", role: "spec", title: "curl -sL https://example.com/llms.txt",
      detail: "curl -sL https://example.com/llms.txt", options: [["allow", "Allow once"]], created_at: 1,
      read_only: true, lead: false },
  ];
  const { el } = await open({ open_prompts });
  const drawn = el("timeline").render();
  assert.match(drawn, /from spec/);
  assert.match(drawn, /spec is read only · this command runs in your working tree/);
});

test("cmd-K finds a conversation by what was said in it and opens it", async () => {
  const hits = [{ thread_id: recorded.thread.thread.id, title: "Fix the flaky mailbox placement test", project: "repo", snippet: "the [placement] race" }];
  const { el, calls } = await open({ search: hits });
  for (const handler of document.listeners.get("keydown") || []) handler({ key: "k", metaKey: true, preventDefault() {} });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("panel-title").textContent, "Search");
  const box = el("panel-body").querySelectorAll("input")[0];
  box.value = "placement";
  box.dispatch("input");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(calls.some(([name, args]) => name === "search" && args.query === "placement"));
  const drawn = el("panel-body").render();
  assert.match(drawn, /Fix the flaky mailbox placement test[\s\S]*the \[placement\] race/);
  el("panel-body").querySelectorAll(".row")[0].dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.ok(calls.some(([name]) => name === "seen"), "a result opens its conversation");
});

test("the route is Auto until a person pins one, and pinning goes to the thread's host", async () => {
  const insights = [
    { agent: "claude", model: "opus", task_type: "implementation", reasoning: "high", verified: 1, failures: 0, weak: 0, success_rate: 1, mean_tokens: 1, mean_duration_ms: 1, last_at: 1 },
  ];
  const { el, calls } = await open({ insights });
  assert.equal(el("route-name").textContent, "Auto");
  assert.equal(el("effort").hidden, true, "effort is the pinned agent's, so it waits for one");
  el("route").dispatch("click");
  const choices = el("route-menu").querySelectorAll("button");
  const labels = choices.map((b) => b.querySelectorAll(".name")[0].textContent);
  assert.deepEqual(labels.slice(0, 2), ["Auto", "claude · opus"]);
  choices[1].dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const pinned = calls.find(([name]) => name === "set_route");
  assert.deepEqual(pinned[1].route, "claude/opus");
  assert.ok(calls.some(([name]) => name === "ensure_host"), "and something is there to act on it");
});

test("while the window is away, a new question and a finished conversation are notified once", async () => {
  const projects = structuredClone(recorded.sidebar);
  const thread = projects[0].threads[0];
  thread.status = "working";
  const answers = { sidebar: projects, open_prompts: [] };
  const { calls, tick } = await open(answers);
  document.hasFocus = () => false;
  try {
    const later = structuredClone(projects);
    later[0].threads[0].status = "unread";
    answers.sidebar = later;
    answers.open_prompts = [{ id: "p9", thread_id: thread.id, thread_title: thread.title, project: "repo", kind: "permission",
      agent: "test", model: "m", role: "fixer", title: "Run tests", detail: null, options: [], created_at: 1, read_only: false, lead: true }];
    answers.changed = [thread.id];
    await tick();
    await tick();
    const sent = calls.filter(([name]) => name === "notify").map(([, args]) => args.title);
    assert.deepEqual(sent.sort(), ["Finished", "Needs you"], "each change once, not on every look");
    const badges = calls.filter(([name]) => name === "badge").map(([, args]) => args.count);
    assert.equal(badges.at(-1), 1, "the dock carries the open question");
  } finally {
    delete document.hasFocus;
  }
});

test("a project's conversations sit on the card of the checkout they work in, and fold to one mark each", async () => {
  const projects = structuredClone(recorded.sidebar);
  const threads = projects[0].threads;
  for (const th of threads) {
    th.cwd = "/work/orochi";
    th.branch = "main";
    th.worktree = false;
  }
  threads[0].status = "working";
  threads[0].doing = "$ cargo test";
  const { el } = await open({ sidebar: projects });
  const cards = () => el("projects").querySelectorAll(".place");
  assert.equal(cards().length, 1, "one checkout, one card");
  const drawn = cards()[0].render();
  assert.match(drawn, /main[\s\S]*primary/, "known by its branch, marked as the original");
  assert.doesNotMatch(drawn, /span\.name orochi/, "the project's name is not said again under it");
  assert.equal(cards()[0].querySelectorAll(".face")[0].dataset.provider, "openai", "and who is working in it");
  assert.match(drawn, /- \$ cargo test/, "a row says what its lead is doing, on its own line");
  assert.equal(el("projects").querySelectorAll(".thread").length, threads.length);
  cards()[0].querySelectorAll(".place-head")[0].dispatch("click");
  assert.equal(el("projects").querySelectorAll(".thread").length, 0, "folded, the rows go");
  const folded = cards()[0].querySelectorAll(".marks")[0];
  assert.match(folded.render(), new RegExp(`${threads.length} conversations`), "and one line says how many");
  assert.equal(folded.querySelectorAll(".state").length, 1, "with a mark only for what is working");
});

test("an agent is drawn with its vendor's own icon where that app is installed, and a letter where not", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects[0].threads[0].agents = [{ role: "fixer", agent: "claude", provider: "anthropic", model: "opus", seats: 1, live: 0, lead: true, background: false, last_at: 1 }];
  projects[0].threads[1].agents = [{ role: "fixer", agent: "gemini", provider: "google", model: "pro", seats: 1, live: 0, lead: true, background: false, last_at: 1 }];
  const { el } = await open({ sidebar: projects, agent_icons: { anthropic: "data:image/png;base64,AAAA" } });
  const faces = el("projects").querySelectorAll(".face");
  const claude = faces.find((f) => f.dataset.provider === "anthropic");
  assert.equal(claude.querySelectorAll("img")[0].getAttribute("src"), "data:image/png;base64,AAAA");
  const gemini = faces.find((f) => f.dataset.provider === "google");
  assert.equal(gemini.textContent, "G", "no icon installed: its letter");
});

test("a conversation opens onto the agents inside it: role, agent, model, and who is working now", async () => {
  const projects = structuredClone(recorded.sidebar);
  const thread = projects[0].threads[0];
  thread.status = "working";
  thread.agents = [
    { role: "facilitator", agent: "claude", provider: "anthropic", model: "haiku", seats: 3, live: 1, lead: true, background: false, last_at: 1 },
    { role: "partner", agent: "claude", provider: "anthropic", model: "opus", seats: 3, live: 1, lead: false, background: false, last_at: 1 },
    { role: "spec", agent: "codex", provider: "openai", model: "gpt-6", seats: 1, live: 0, lead: false, background: true, last_at: 1 },
  ];
  const live_seats = [
    { seat_id: "s", thread_id: thread.id, project_id: "p", turn: "t", ordinal: 1, role: "partner", lead: false, read_only: true,
      title: null, background: false, state: "working", started_at: Date.now(), agent: "claude", model: "opus",
      doing: "Reading src/router", usage: null, provider: "anthropic" },
  ];
  const { el } = await open({ sidebar: projects, live_seats });
  const members = el("projects").querySelectorAll(".member");
  assert.equal(members.length, 3, "one row per member, two of them the same vendor on different models");
  const row = el("projects").querySelectorAll(".thread")[0];
  const own = row.children.filter((c) => c.className === "face");
  assert.equal(own.length, 0, "several agents are listed under the row, not stamped on it");
  const drawn = members.map((m) => m.render()).join("\n");
  assert.match(drawn, /facilitator[\s\S]*claude · haiku/);
  assert.match(drawn, /partner[\s\S]*claude · opus[\s\S]*Reading src\/router/, "what a working member is doing");
  assert.match(drawn, /spec[\s\S]*codex · gpt-6/);
  assert.deepEqual(members.map((m) => m.dataset.live), ["true", "true", "false"]);
  const chip = el("projects").querySelectorAll(".members")[0];
  assert.match(chip.render(), /2\/3/, "the row says how many of its agents are working");
  chip.dispatch("click");
  assert.equal(el("projects").querySelectorAll(".member").length, 0, "and closes to the row alone");
});

test("in the conversation an agent speaks as its own icon, and the person as theirs", async () => {
  const { el } = await open({ agent_icons: { openai: "data:image/png;base64,AAAA" } });
  const posts = el("timeline").querySelectorAll(".post");
  const agent = posts.find((p) => p.dataset.who !== "you");
  assert.equal(agent.querySelectorAll(".face")[0].dataset.provider, "openai");
  assert.equal(agent.querySelectorAll("img")[0].getAttribute("src"), "data:image/png;base64,AAAA");
  const person = posts.find((p) => p.dataset.who === "you");
  assert.equal(person.querySelectorAll(".face").length, 0);
});

test("two projects with one name say which folder each is", async () => {
  const projects = structuredClone(recorded.sidebar);
  projects[1] = { ...structuredClone(projects[0]), id: "other", name: projects[0].name, root: "/tmp/s7/" + projects[0].name };
  projects[0].root = "/Users/me/Development/" + projects[0].name;
  for (const th of projects[1].threads) th.id += "-2";
  const { el } = await open({ sidebar: projects });
  const heads = el("projects").querySelectorAll(".hint").map((h) => h.textContent);
  assert.deepEqual(heads, ["Development", "s7"]);
});

// Right-click on the sidebar. Each verb is a row changed; the two that cannot be undone ask.
const menuLabels = (el) =>
  el("context-menu").querySelectorAll("button").map((b) => b.querySelectorAll(".name")[0].textContent);
const menuItem = (el, key) =>
  el("context-menu").querySelectorAll("button").find((b) => b.dataset.action === key);
const rightClick = (node) => node.dispatch("contextmenu", { clientX: 40, clientY: 120 });

test("a right-click on a conversation offers what can be done with it, and nothing else", async () => {
  const { el, calls } = await open();
  assert.equal(el("context-menu").hidden, true, "nothing is shown until asked");
  const rows = el("projects").querySelectorAll(".thread");
  rightClick(rows[0]);
  assert.equal(el("context-menu").hidden, false);
  assert.deepEqual(menuLabels(el), [
    "Rename", "Pin", "Open in Terminal", "Reveal in Finder", "Copy ID", "Archive", "Delete conversation",
  ]);
  assert.equal(el("context-menu").style.left, "40px", "drawn where the pointer is");
  assert.equal(menuItem(el, "delete").className, "danger", "and the one that cannot be undone says so");

  menuItem(el, "pin").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("context-menu").hidden, true, "choosing closes the menu");
  const pinned = calls.find(([name]) => name === "pin_thread");
  assert.deepEqual(pinned[1], { thread: recorded.sidebar[0].threads[0].id, pinned: true });

  // Esc and a click elsewhere both close it without choosing.
  rightClick(rows[1]);
  el("projects").dispatch("keydown", { key: "Escape" });
  assert.equal(el("context-menu").hidden, true);
  rightClick(rows[1]);
  el("timeline").dispatch("click");
  assert.equal(el("context-menu").hidden, true);
  assert.equal(calls.filter(([name]) => name === "pin_thread").length, 1, "and nothing was asked for");
});

test("deleting a conversation asks first, and only the red button does it", async () => {
  const { el, calls } = await open();
  const row = el("projects").querySelectorAll(".thread")[1];
  const id = recorded.sidebar[0].threads[1].id;
  rightClick(row);
  menuItem(el, "delete").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("confirm").open, true, "a question, not a deletion");
  assert.equal(el("confirm-title").textContent, "Delete this conversation?");
  el("confirm-cancel").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("confirm").open, false);
  assert.equal(calls.find(([name]) => name === "delete_thread"), undefined, "Cancel deletes nothing");

  rightClick(row);
  menuItem(el, "delete").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  el("confirm-go").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "delete_thread")[1], { thread: id });
});

/// The recorded sidebar plus a second project with a thread an agent is working in and one
/// a terminal is sitting in: what holds a conversation, and what a right-click must respect.
function withHeldThreads() {
  const projects = structuredClone(recorded.sidebar);
  const base = projects[0].threads[0];
  const held = (id, title, status, terminal) => ({
    ...base, id, title, status, terminal, project: "web-app", project_id: "p2",
    project_root: "/work/web-app", cwd: "/work/web-app", branch: "main",
  });
  projects.push({
    id: "p2", name: "web-app", root: "/work/web-app", pinned: false, collapsed: false,
    threads: [
      held("w1", "Add sign-in with a magic link", "needs_you", true),
      held("w2", "Dark mode for the settings page", "working", false),
      held("w3", "Retire the old API", "background", false),
    ],
  });
  return projects;
}

test("a conversation an agent is working in, or a terminal holds, cannot be deleted from here", async () => {
  const { el, calls } = await open({ sidebar: withHeldThreads() });
  const rows = el("projects").querySelectorAll(".thread");
  assert.equal(rows.length, 5);
  rightClick(rows[3]);
  assert.equal(menuItem(el, "delete").disabled, true);
  assert.match(menuItem(el, "delete").render(), /stop it first/);
  menuItem(el, "delete").dispatch("click");
  assert.equal(el("confirm").open, false, "a disabled line asks nothing");
  assert.ok(menuItem(el, "stop"), "what it can do instead is stop it");
  menuItem(el, "stop").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "interrupt")[1], { thread: "w2" }, "Esc, from here");

  rightClick(rows[2]);
  assert.equal(menuItem(el, "delete").disabled, true, "a question waiting is a turn still running");
  el("timeline").dispatch("click");

  rightClick(rows[4]);
  menuItem(el, "stop").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(
    calls.find(([name]) => name === "stop_seat")[1],
    { thread: "w3", seat: null },
    "helpers that outlived their turn are stopped as the Activity screen stops them",
  );
});

test("renaming a conversation is typed over its name, and Enter keeps it", async () => {
  const { el, calls } = await open();
  const id = recorded.sidebar[0].threads[0].id;
  rightClick(el("projects").querySelectorAll(".thread")[0]);
  menuItem(el, "rename").dispatch("click");
  const field = el("projects").querySelectorAll(".rename")[0];
  assert.ok(field, "the row becomes a field");
  assert.equal(field.value, "Refactor the scheduler retry loop", "holding the name it has");
  field.value = "Retry loop";
  field.dispatch("keydown", { key: "Enter" });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "rename_thread")[1], { thread: id, title: "Retry loop" });
  assert.equal(el("projects").querySelectorAll(".rename").length, 0, "and the row is a row again");

  // Esc puts the old name back without asking anything.
  rightClick(el("projects").querySelectorAll(".thread")[0]);
  menuItem(el, "rename").dispatch("click");
  const again = el("projects").querySelectorAll(".rename")[0];
  again.value = "something else";
  again.dispatch("keydown", { key: "Escape" });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(calls.filter(([name]) => name === "rename_thread").length, 1);
});

test("a right-click on a project offers its own verbs, and hiding it keeps its conversations", async () => {
  const { el, calls, localStorage } = await open();
  const head = el("projects").querySelectorAll("summary")[0];
  rightClick(head);
  assert.deepEqual(menuLabels(el), [
    "New thread", "Rename", "Pin", "Where it runs…", "Reveal in Finder", "Copy path",
    "Show archived conversations", "Hide project", "Delete project",
  ]);
  menuItem(el, "hide").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "hide_project")[1], { project: recorded.sidebar[0].id, hidden: true });
  assert.equal(calls.find(([name]) => name === "delete_project"), undefined, "hidden is not deleted");

  // Archived conversations are shown on request, and the request is remembered.
  rightClick(el("projects").querySelectorAll("summary")[0]);
  menuItem(el, "archived").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  const asked = calls.filter(([name]) => name === "sidebar").at(-1);
  assert.equal(asked[1].archived, true);
  assert.equal(localStorage.getItem("archived"), "true");
  rightClick(el("projects").querySelectorAll("summary")[0]);
  assert.ok(menuItem(el, "archived").querySelectorAll(".tick").length, "and the line says it is on");

  // Renaming a project is the same field, in its heading.
  menuItem(el, "rename").dispatch("click");
  const field = el("projects").querySelectorAll("summary")[0].querySelectorAll(".rename")[0];
  assert.equal(field.value, "repo");
  field.value = "Orochi";
  field.dispatch("keydown", { key: "Enter" });
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "rename_project")[1], { project: recorded.sidebar[0].id, name: "Orochi" });
});

test("deleting a project says how much goes with it, and is refused while an agent works in it", async () => {
  const { el, calls } = await open();
  rightClick(el("projects").querySelectorAll("summary")[0]);
  menuItem(el, "delete").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(el("confirm").open, true);
  assert.match(el("confirm-body").textContent, /repo — 2 conversations are deleted for good/);
  el("confirm-go").dispatch("click");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(calls.find(([name]) => name === "delete_project")[1], { project: recorded.sidebar[0].id });

  el("timeline").dispatch("click");
  const { el: held } = await open({ sidebar: withHeldThreads() });
  rightClick(held("projects").querySelectorAll("summary")[1]);
  assert.equal(menuItem(held, "delete").disabled, true, "web-app has a thread working");
  assert.match(menuItem(held, "delete").render(), /stop it first/);
});

test("the menu speaks the window's language too", async () => {
  const { el } = await open({}, { language: "ja-JP" });
  rightClick(el("projects").querySelectorAll(".thread")[0]);
  assert.deepEqual(menuLabels(el), [
    "名前を変更", "ピン留め", "ターミナルで開く", "Finder で表示", "ID をコピー", "アーカイブ", "会話を削除",
  ]);
});

test("what an agent writes is drawn as Markdown, and none of it becomes markup", async () => {
  const thread = structuredClone(recorded.thread);
  const reply = [
    "## Summary",
    "The **race** is in `Order::place`, see [scheduler](/src/scheduler/mod.rs:519).",
    "",
    "- first",
    "  - nested",
    "- second",
    "",
    "1. one",
    "2. two",
    "",
    "| file | change |",
    "|---|---|",
    "| a.rs | +1 |",
    "",
    "```rust",
    "fn main() {}",
    "```",
    "> quoted",
    "<script>alert(1)</script>",
  ].join("\n");
  thread.items = thread.items.map((item) => (item.kind === "agent_message" ? { ...item, text: reply } : item));
  const { el } = await open({ thread });
  const body = el("timeline").querySelectorAll(".md").find((m) => m.render().includes("Summary"));
  const has = (tag) => body.querySelectorAll(tag).length;
  assert.equal(has("h4"), 1, "a heading");
  assert.equal(body.querySelectorAll("strong")[0].render().trim().split("\n").pop().trim(), "race");
  assert.ok(body.querySelectorAll("code").some((c) => c.textContent === "Order::place"), "inline code");
  const link = body.querySelectorAll(".link")[0];
  assert.equal(link.getAttribute("title"), "/src/scheduler/mod.rs:519", "a link says where it goes");
  assert.equal(has("ul"), 2, "a list and the list nested in it");
  assert.equal(has("ol"), 1);
  assert.equal(has("td"), 2, "a table");
  assert.equal(body.querySelectorAll("pre")[0].querySelectorAll("code")[0].textContent, "fn main() {}");
  assert.equal(has("blockquote"), 1);
  assert.equal(has("script"), 0, "text that looks like markup stays text");
  assert.match(body.render(), /<script>alert\(1\)<\/script>/);
});

// Files ---------------------------------------------------------------------
// The thread's working tree. Its answers are recorded in `files.json`, whose shapes a Rust test
// compares with what the commands serialize.
const files = JSON.parse(readFileSync(join(here, "files.json"), "utf8"));
const settle = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));
const tree = (more = {}) => ({
  tree_refresh: files.refresh,
  tree_list: ({ dir }) => files[dir === "" ? "root" : dir] ?? [],
  tree_read: ({ path }) =>
    path === "README.md" ? files.markdown : path === "blob.bin" ? files.binary : files.text,
  tree_find: ({ by }) => files[by],
  tree_files: [
    { turn: "", path: "src/chat/term.rs", change: "modify", added: 1, removed: 1, latest_patch: 0 },
  ],
  ...more,
});
async function filesTab(el) {
  el("tabs").querySelectorAll("button").find((b) => b.dataset.pane === "files").dispatch("click");
  await settle();
}
const rows = (el) => el("tree").querySelectorAll(".entry");
const row = (el, path) => rows(el).find((r) => r.dataset.path === path);

test("the Files tab draws the thread's folder, folders first, each row with its git state", async () => {
  const { el, calls } = await open(tree());
  assert.equal(el("pane-files").hidden, true, "the tree is not read while nobody looks at it");
  assert.ok(!calls.some(([name]) => name === "tree_list"));
  await filesTab(el);
  assert.equal(el("pane-files").hidden, false);
  assert.deepEqual(
    rows(el).map((r) => r.dataset.path),
    ["docs", "src", "target", ".gitignore", "blob.bin", "Cargo.toml", "new.txt", "README.md"],
  );
  assert.equal(row(el, "src").dataset.status, "modified", "a folder shows what is under it");
  assert.match(row(el, "src").render(), /span\.badge M/);
  assert.equal(row(el, "target").dataset.status, "ignored");
  assert.equal(row(el, "new.txt").dataset.status, "untracked");
  assert.equal(row(el, "README.md").dataset.status, undefined);
  assert.match(el("files-where").textContent, /repo · —/, "and says which folder and branch it is");
});

test("opening a folder asks for that folder alone, and a deleted file is still where it was", async () => {
  const { el, calls } = await open(tree());
  await filesTab(el);
  row(el, "src").dispatch("click");
  await settle();
  assert.deepEqual(
    calls.filter(([name]) => name === "tree_list").map(([, args]) => args.dir),
    ["", "src"],
  );
  assert.equal(row(el, "src/gone.rs").dataset.missing, "true");
  assert.equal(row(el, "src/chat").getAttribute("aria-expanded"), "false");
  row(el, "src").dispatch("click");
  await settle();
  assert.equal(row(el, "src/chat"), undefined, "and closing it folds it away");
});

test("a file opens in the tree's place with numbered lines, and back returns to the tree as it was", async () => {
  const { el, calls } = await open(tree());
  await filesTab(el);
  row(el, "src").dispatch("click");
  await settle();
  row(el, "src/chat").dispatch("click");
  await settle();
  row(el, "src/chat/term.rs").dispatch("click");
  await settle();
  assert.equal(el("tree").hidden, true);
  assert.equal(el("viewer").hidden, false);
  const lines = el("viewer").querySelectorAll(".line");
  assert.equal(lines.length, 3);
  assert.match(lines[1].render(), /button\.no 2\n\s*span\.src\s+changed\(\)/);
  const crumbs = el("viewer").querySelectorAll(".crumbs")[0].render();
  assert.match(crumbs, /button\.crumb src[\s\S]*button\.crumb chat[\s\S]*span\.here term\.rs/);
  assert.match(el("viewer").render(), /3 lines · 44 B/);
  assert.deepEqual(
    calls.find(([name]) => name === "tree_files")[1],
    { thread: recorded.thread.thread.id, scope: "unstaged" },
    "a modified file asks what it has against HEAD",
  );
  assert.match(el("viewer").querySelectorAll(".stat")[0].render(), /\+1[\s\S]*−1/);

  el("viewer").querySelectorAll(".back")[0].dispatch("click");
  assert.equal(el("viewer").hidden, true);
  assert.ok(row(el, "src/chat/term.rs"), "the folders it was in stay open");
  assert.equal(row(el, "src/chat/term.rs").getAttribute("aria-selected"), "true");
});

test("a modified file's diff opens in Changes, and a file in Changes opens in Files", async () => {
  const { el, calls } = await open(tree({ tree_patch: "@@ -1,1 +1,1 @@\n-a\n+b\n" }));
  await filesTab(el);
  row(el, "src").dispatch("click");
  await settle();
  row(el, "src/chat").dispatch("click");
  await settle();
  row(el, "src/chat/term.rs").dispatch("click");
  await settle();
  el("viewer").querySelectorAll(".stat")[0].dispatch("click");
  await settle(40);
  assert.equal(el("pane-changes").hidden, false);
  assert.equal(el("pane-files").hidden, true);
  assert.deepEqual(calls.findLast(([name]) => name === "tree_patch")[1], {
    thread: recorded.thread.thread.id,
    scope: "unstaged",
    path: "src/chat/term.rs",
  });

  const reads = calls.filter(([name]) => name === "tree_read").length;
  el("files").querySelectorAll(".file")[0].querySelectorAll(".open")[0].dispatch("click");
  await settle(40);
  assert.equal(el("pane-files").hidden, false);
  assert.equal(calls.filter(([name]) => name === "tree_read").length, reads + 1);
  assert.equal(calls.findLast(([name]) => name === "tree_read")[1].path, "src/chat/term.rs");
});

test("a Markdown file reads rendered or as its source, and none of it becomes markup", async () => {
  const { el } = await open(tree());
  await filesTab(el);
  row(el, "README.md").dispatch("click");
  await settle();
  const viewer = el("viewer");
  assert.equal(viewer.querySelectorAll(".rendered").length, 1);
  assert.equal(viewer.querySelectorAll("b").length, 0, "the file's own tags stay text");
  assert.match(viewer.render(), /<b>body<\/b>/);
  viewer.querySelectorAll(".act").find((b) => b.textContent === "Source").dispatch("click");
  assert.equal(el("viewer").querySelectorAll(".code").length, 1);
});

test("a binary file is said to be one, and is not drawn as text", async () => {
  const { el } = await open(tree());
  await filesTab(el);
  row(el, "blob.bin").dispatch("click");
  await settle();
  assert.equal(el("viewer").querySelectorAll(".code").length, 0);
  assert.match(el("viewer").render(), /binary or not UTF-8 · 5 B/);
});

test("the search box finds files by name and lines by content, and a line opens where it is", async () => {
  const { el, calls } = await open(tree());
  await filesTab(el);
  const box = el("tree-search");
  box.value = "term";
  box.dispatch("input", { target: box });
  await settle(200);
  assert.deepEqual(calls.findLast(([name]) => name === "tree_find")[1], {
    thread: recorded.thread.thread.id,
    query: "term",
    by: "name",
    limit: 200,
  });
  assert.match(el("tree").render(), /div\.hit[\s\S]*src\/chat\/term\.rs/);

  el("tree-by").querySelectorAll("button").find((b) => b.dataset.by === "content").dispatch("click");
  await settle();
  assert.equal(calls.findLast(([name]) => name === "tree_find")[1].by, "content");
  const hit = el("tree").querySelectorAll(".hit")[0];
  assert.match(hit.render(), /src\/chat\/term\.rs:2[\s\S]*changed\(\)/);
  hit.dispatch("click");
  await settle();
  const lines = el("viewer").querySelectorAll(".line");
  assert.equal(lines[1].dataset.hit, "true", "the line that matched is marked");

  box.value = "";
  box.dispatch("input", { target: box });
  await settle(200);
  el("viewer").querySelectorAll(".back")[0].dispatch("click");
  assert.ok(row(el, "src"), "clearing the search brings the tree back");
});

test("outside a repository, content search says why it cannot", async () => {
  const { el } = await open(tree({ tree_refresh: { repository: false, partial: false, read_at: 1 } }));
  await filesTab(el);
  const content = el("tree-by").querySelectorAll("button").find((b) => b.dataset.by === "content");
  assert.equal(content.disabled, true);
  assert.match(content.title, /needs a git repository/);
});

test("a file goes into the next message as @path, and a line comment joins the review", async () => {
  const { el } = await open(tree(), { prompt: () => "why this call?" });
  await filesTab(el);
  row(el, "src").dispatch("click");
  await settle();
  row(el, "src/chat").dispatch("click");
  await settle();
  row(el, "src/chat/term.rs").dispatch("click");
  await settle();
  el("message").value = "look at";
  el("viewer").querySelectorAll(".act").find((b) => b.textContent === "Add to message").dispatch("click");
  assert.equal(el("message").value, "look at @src/chat/term.rs ");

  el("viewer").querySelectorAll(".no")[1].dispatch("click");
  assert.match(el("review-list").render(), /src\/chat\/term\.rs:2[\s\S]*why this call\?/);
  assert.match(el("viewer").render(), /1 comments waiting/);
});

test("a file a tool call was about opens in Files at its line; one outside the folder does not", async () => {
  const thread = structuredClone(recorded.thread);
  const base = thread.items.find((i) => i.kind === "agent_message");
  thread.items.splice(2, 0,
    { ...base, seq: 90, kind: "tool_call", text: "", status: "completed",
      data: { title: "Read", locations: [{ path: `${thread.thread.cwd}/src/chat/term.rs`, line: 2 }] } },
    { ...base, seq: 91, kind: "tool_call", text: "", status: "completed",
      data: { title: "Read", locations: [{ path: "/etc/hosts", line: null }] } },
  );
  const { el, calls } = await open(tree({ thread }));
  const links = el("timeline").querySelectorAll(".open-file");
  assert.equal(links.length, 1, "only a path inside the thread's folder is offered");
  assert.match(links[0].render(), /src\/chat\/term\.rs/);
  links[0].dispatch("click");
  await settle(40);
  assert.equal(el("pane-files").hidden, false);
  assert.equal(calls.findLast(([name]) => name === "tree_read")[1].path, "src/chat/term.rs");
  assert.equal(el("viewer").querySelectorAll(".line")[1].dataset.hit, "true");
});

test("the tree answers the keys a file list does, and Esc closes a file", async () => {
  const { el, calls } = await open(tree());
  await filesTab(el);
  const key = (name) => el("tree").dispatch("keydown", { key: name, target: el("tree") });
  key("ArrowDown");
  assert.equal(row(el, "src").getAttribute("aria-selected"), "true");
  key("ArrowRight");
  await settle();
  assert.equal(calls.findLast(([name]) => name === "tree_list")[1].dir, "src");
  key("ArrowLeft");
  await settle();
  assert.equal(row(el, "src/chat"), undefined);
  for (let i = 0; i < 6; i += 1) key("ArrowDown");
  key("Enter");
  await settle();
  assert.equal(el("viewer").hidden, false, "Enter opens the file under the cursor");
  el("viewer").dispatch("keydown", { key: "Escape", target: el("viewer") });
  assert.equal(el("viewer").hidden, true);
});

test("the tree is read again when a turn ends, and a file that moved offers itself rather than swapping", async () => {
  let current = structuredClone(recorded.thread);
  current.thread.status = "working";
  let modified = files.text.modified_at;
  const { el, calls, tick } = await open(
    tree({
      thread: () => current,
      // Whichever thread the window has open: the feed names the one that moved.
      changed: () => recorded.sidebar.flatMap((p) => p.threads.map((t) => t.id)),
      tree_read: () => ({ ...files.text, modified_at: modified }),
    }),
  );
  await filesTab(el);
  row(el, "src").dispatch("click");
  await settle();
  row(el, "src/chat").dispatch("click");
  await settle();
  row(el, "src/chat/term.rs").dispatch("click");
  await settle();
  const refreshes = () => calls.filter(([name]) => name === "tree_refresh").length;
  const before = refreshes();
  await tick();
  assert.equal(refreshes(), before, "a turn still working does not re-read the tree");

  current = structuredClone(current);
  current.thread.status = "unread";
  modified += 1000;
  await tick();
  assert.equal(refreshes(), before + 1, "a turn that ended does");
  assert.match(el("viewer").render(), /changed on disk/);
  assert.match(el("viewer").querySelectorAll(".line")[1].render(), /changed\(\)/);
  el("viewer").querySelectorAll(".act").find((b) => b.textContent === "Reload").dispatch("click");
  assert.doesNotMatch(el("viewer").render(), /changed on disk/);
});

test("the Files pane speaks the machine's language", async () => {
  const { el } = await open(tree(), { language: "ja-JP" });
  assert.equal(el("tree-search").getAttribute("placeholder"), "ファイルを検索");
  await filesTab(el);
  row(el, "README.md").dispatch("click");
  await settle();
  assert.ok(el("viewer").querySelectorAll(".act").some((b) => b.textContent === "エディタで開く"));
});

test("each panel's inner edge drags to a width this window remembers, and never over the conversation", async () => {
  const { el, window, localStorage } = await open({}, { stored: { widths: JSON.stringify({ left: 300 }) } });
  // What was dragged last time is where it opens; the other panel keeps the stylesheet's width.
  assert.equal(el("app").style["--left"], "300px");
  assert.equal(el("app").style["--right"], undefined);

  const drag = (handle, from, to) => {
    el(handle).dispatch("pointerdown", { button: 0, clientX: from, pointerId: 1 });
    assert.ok(document.body.classList.contains("resizing"));
    el(handle).dispatch("pointermove", { clientX: to });
    el(handle).dispatch("pointerup", {});
    assert.ok(!document.body.classList.contains("resizing"));
  };
  drag("sidebar-resizer", 300, 360);
  assert.equal(el("app").style["--left"], "360px");
  // The right pane is measured from the window's right edge.
  drag("side-resizer", 1400 - 320, 1400 - 500);
  assert.equal(el("app").style["--right"], "500px");
  assert.deepEqual(JSON.parse(localStorage.getItem("widths")), { left: 360, right: 500 });

  // Past its bounds a panel stops, and it stops before the conversation gets narrower than a
  // readable column: with the pane at 640, the sidebar gets 1400 - 640 - 360, under its cap.
  drag("sidebar-resizer", 360, 5);
  assert.equal(el("app").style["--left"], "180px");
  drag("side-resizer", 900, 0);
  assert.equal(el("app").style["--right"], "640px");
  drag("sidebar-resizer", 180, 1000);
  assert.equal(el("app").style["--left"], `${1400 - 640 - 360}px`);
  drag("sidebar-resizer", 400, 180);

  // The keyboard moves the edge the way it looks, and a double-click puts it back.
  el("sidebar-resizer").dispatch("keydown", { key: "ArrowRight" });
  assert.equal(el("app").style["--left"], "196px");
  el("side-resizer").dispatch("dblclick", {});
  assert.equal(el("app").style["--right"], undefined);
  assert.equal(JSON.parse(localStorage.getItem("widths")).right, null);

  // A window made narrower takes the room back from the panels, not from the conversation.
  el("sidebar-resizer").dispatch("keydown", { key: "ArrowRight", shiftKey: true });
  window.innerWidth = 900;
  window.listeners.get("resize")();
  assert.equal(el("app").style["--left"], `${900 - 320 - 360}px`);
});

test("a file read wide takes half the window for as long as it is open, and is never stored", async () => {
  const { el, localStorage } = await open(tree());
  await filesTab(el);
  row(el, "README.md").dispatch("click");
  await settle();
  el("viewer").querySelectorAll(".widen")[0].dispatch("click");
  assert.equal(el("app").style["--right"], "700px", "half of a 1400-pixel window");
  el("viewer").querySelectorAll(".back")[0].dispatch("click");
  assert.equal(el("app").style["--right"], undefined, "and the width it was dragged to comes back");
  assert.equal(localStorage.getItem("widths"), null);
});


function mediaThread(reply) {
  const thread = structuredClone(recorded.thread);
  let supplied = false;
  thread.items = thread.items.map((item) => {
    if (item.kind !== "agent_message") return item;
    const text = supplied ? "" : reply;
    supplied = true;
    return { ...item, text };
  });
  return thread;
}

test("screenshots from local links render inline and enlarge on click", async () => {
  const cwd = recorded.thread.thread.cwd;
  const { el, calls } = await open({
    thread: mediaThread(`![Screenshot](shot.png)\n[Same shot](<${cwd}/shot.png>)`),
    tree_media: { mime: "image/png", bytes: 12, data: "data:image/png;base64,AAAA" },
  });
  const images = el("timeline").querySelectorAll(".attachment-media");
  assert.equal(images.length, 2);
  assert.equal(images[0].getAttribute("src"), "data:image/png;base64,AAAA");
  assert.equal(images[0].getAttribute("alt"), "Screenshot");
  assert.equal(calls.filter(([name]) => name === "tree_media").length, 1, "streamed references share a bounded read");
  el("timeline").querySelectorAll(".attachment-image")[0].dispatch("click");
  assert.equal(el("app").querySelectorAll(".media-overlay").length, 1);
  el("app").querySelectorAll(".media-close")[0].dispatch("click");
  assert.equal(el("app").querySelectorAll(".media-overlay").length, 0);
});

test("localhost screenshots, video and audio use native previews without autoplay", async () => {
  const { el } = await open({ thread: mediaThread(
    "![Screenshot](http://localhost:8731/screenshot)\n[Video](http://127.0.0.1:8731/clip.mp4)\n[Audio](https://example.com/voice.mp3)"
  ) });
  const media = el("timeline").querySelectorAll(".attachment-media");
  assert.deepEqual(media.map((node) => node.tagName), ["IMG", "VIDEO", "AUDIO"]);
  for (const node of media.slice(1)) {
    assert.equal(node.getAttribute("controls"), "");
    assert.equal(node.getAttribute("autoplay"), null);
  }
});

test("unsafe image references stay text and missing previews keep a file action", async () => {
  const { el, calls } = await open({ thread: mediaThread(
    "![bad](javascript:alert)\n![bad](file:///etc/private.png)\n![bad](../secret.png)\n![bad](http://remote.example/shot.png)\n![bad](%2e%2e/secret.png)\n![missing](missing.png)\n[Report](report.pdf)"
  ), tree_media: new Error("not there") });
  assert.equal(el("timeline").querySelectorAll(".attachment-media").length, 0);
  assert.equal(calls.filter(([name]) => name === "tree_media").length, 1);
  assert.equal(el("timeline").querySelectorAll(".attachment-file").length, 2);
  assert.match(el("timeline").render(), /Preview unavailable/);
});

test("code examples do not load media", async () => {
  const { el, calls } = await open({ thread: mediaThread(
    "`![example](shot.png)`\n\n```md\n![example](other.png)\n```"
  ) });
  assert.equal(el("timeline").querySelectorAll(".attachment-media").length, 0);
  assert.equal(calls.filter(([name]) => name === "tree_media").length, 0);
});


test("file links open Files and preserve root-file line positions and encoded spaces", async () => {
  const { el, calls } = await open(tree({ thread: mediaThread(
    "[Source](README.md:7)\n[Report](<reports/my report.pdf>)\n[Encoded](reports/my%20report.pdf)"
  ) }));
  const links = el("timeline").querySelectorAll(".attachment-file");
  assert.equal(links.length, 3);
  links[0].dispatch("click");
  await settle();
  assert.equal(el("pane-files").hidden, false);
  assert.ok(calls.some(([name, args]) => name === "tree_read" && args.path === "README.md"));
  links[2].dispatch("click");
  await settle();
  assert.ok(calls.some(([name, args]) => name === "tree_read" && args.path === "reports/my report.pdf"));
});
