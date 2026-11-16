# Conversation Tabs and Media: Terminal, Browser and File Tabs, and Images and Video in the Chat

Written: 2026-11-14 (browser rewritten the same day for Chromium, D4).

**Status (2026-11-15): partly built. Automated tests, plus the browser and the viewers' frame run against a real Chrome / Chromium; nothing run inside the Tauri window.**

| Phase | State |
|---|---|
| **T0** tab strip, Chat tab kept alive, persistence, limits, keys, file tabs, quick open | built (`app.js` "Tabs", `ui.test.mjs`) |
| **M1** media core: `src/media.rs`, `media` table and `v_media`, ACP image/audio blocks, files a tool or reply names, caps, sweep, `orochi threads media` | built (`tests/media.rs`); the ACP blocks are tested as parsed `ToolUpdate`s, **not against a real agent** |
| **M2** media in the window: `orochi-media://` scheme with `Range`, timeline figures, media tab with previous/next, CSP | built for images, video and audio (`desktop/src-tauri/tests/media.rs`, `ui.test.mjs`); **the gallery is not**; WKWebView playback over the scheme is *unverified* |
| **T1** terminal tab: `portable-pty` + vendored xterm.js 6.0.0 | built (`desktop/src-tauri/tests/term.rs` over a real PTY, `ui.test.mjs` with a fake xterm); **never drawn by the real xterm.js in a window**; Windows (ConPTY) untested |
| **B1** browser broker in the core: pipe CDP client, per-session contexts, host policy, the `browser_*` MCP tools, `orochi --internal-browser` / `--internal-browser-mcp`, `[browser]` config | built (`tests/browser.rs`, 10 tests over `fake_chrome.py`, including the whole chain through the real binary). **Never run against a real Chrome**, and whether an agent CLI forwards an MCP `image` result to ACP (so a screenshot reaches the chat) is *unverified*. Departures from §3.5: sessions are scoped **per agent session**, not per thread; a new origin is **refused** (naming `browser.allow`) rather than asked about; Chrome is one the machine has (or `browser.chrome`), **not** a pinned Chrome for Testing download; `orochi browser` (the CLI) and `browser_drag`/`browser_select` are not built; **off by default**; **On by default since 2026-11-15** (when a Chrome or Chromium is found): every agent that can be reached is told in its prompt to use Orochi's browser rather than another browser tool, and the browser counts as its `browser` capability, so a task that needs one is no longer handed to another agent. **Run with a real agent (Claude, 2026-11-15):** asked to open a local page, read its heading, click a button and show a screenshot, it used `browser_navigate`, `browser_snapshot`, `browser_click` and `browser_screenshot` on its own and answered correctly; the screenshot the tool returned reached Orochi as an image and was kept as media, which answers, for Claude's adapter, whether an agent CLI forwards an MCP image result over ACP. **Not measured:** Codex and the other agents (Codex was out of quota), and whether an agent prefers Orochi's browser when it has its own and the task does not say |
| **B2** the browser tab in the window | **built, partly checked.** The broker has a person's-tab API (`view_*` tools, scope `user:<thread>:<tab>`, its own context, any web address, not held to the agents' host list): a screencast frame the window polls for (acknowledged so Chrome keeps sending, sent only when new), resize, mouse, wheel, keys, IME composition, text paste, JS dialogs, history. The window draws it on a canvas with an address bar; input methods go through a textarea in the window. **Checked against real Chrome:** JPEG frames arrive at the requested size, a click through `view_input` reaches the page, the title is read. **Not checked:** the window's canvas, key and IME code in a real window (fake-DOM tests only; the IME path is the riskiest), file choosers, downloads, popups, a context menu, native `<select>` popups, the cursor shape, cookies persisting per project, an agent's pointer, take-over. The frames are polled (about 14 a second at most), not pushed over a `Channel` |
| **Watching an agent's page** (the "watch what the agent does" part of §3.5.6) | **built; the broker half checked against real Chrome, the window half in fake-DOM tests only (not seen in the window).** A session's name now begins with its peer's name (`implementer-1a2b3c4d`); the broker lists the agents that have a page open (`view_agents`, nothing about the page beyond its title and address) and the person's tab can ask for one's frames (`view_frame` with `watch`) — never a page of the person's own, never opening one, and **never driving it**: input, navigation, resize, dialogs and close are refused, and no script is run in a watched page. The window has an *Agents' browser* tab (`+` menu) with a chooser and a canvas that sends nothing back. **Real Chrome:** an agent opened a page, the person's side listed it, received a JPEG of it, and the agent's own tools kept working on the same page. **Not done:** take-over ("Hand back"), the agent's pointer drawn on the frame, the agent's tab as a tab of the conversation's own strip, narrowing the list to this conversation's agents (it lists every agent session on this machine) |
| **B3** sharing a tab with agents, raw CDP proxy, DevTools | **not built** |
| **D1** Markdown preview (existed), CSV/TSV tables (sort, filter, 500 rows at a time), JSON tree, JSONL as a table | built, in the Files pane's viewer and in file tabs (`ui.test.mjs`). **Not done from §4.7.2:** Shift_JIS detection (the file is read as UTF-8 by `tree_read`), document *cards* in the timeline, true row virtualization |
| **D2** the sandboxed viewer frame, PDF | **built, partly checked.** `viewer.html` + `viewer.js` run pdf.js 6.5.438 in `<iframe sandbox="allow-scripts">`; the window passes the library source and the bytes by `postMessage`, the frame imports it from a blob. **Checked in a real Chromium** (a page served with the app's CSP): the frame opened a 2-page PDF and drew it. **Not checked:** inside Tauri's WKWebView (custom-scheme frame, whether IPC is refused from an opaque origin, `script-src blob:` taking effect), PDFs with non-embedded CJK fonts (no CMaps shipped), text selection, find, outline, thumbnails. The window's CSP gained `script-src 'self' blob:` and `worker-src blob:` for this |
| **D4 (Word, `.xlsx`, `.pptx`)** `.docx`, `.xlsx`, `.pptx` | **built, partly checked.** docx-preview 0.4.1 + JSZip 3.10.1 as classic scripts run from blobs in the same frame. **Checked in a real Chromium** with a hand-made one-page file (heading, Japanese text): it rendered. **Not checked:** real-world documents (tables, images, headers, footnotes, fonts), Tauri's WebKit. `.xlsx`: SheetJS CE 0.20.3 (mini build) in the same frame, one table per sheet with a sheet switcher, cached values only, first 500 rows and 60 columns; **checked in a real Chromium** with a hand-made two-sheet file (Japanese sheet name and text, a formula's cached value), **not** with real workbooks or in Tauri. `.xls`, `.ods`, hover formulas, column sort and filter: **not built**. `.pptx`: **@aiden0z/pptx-renderer 1.4.0** (standalone browser ESM build, imported from a blob) in the same frame, as a scrolling list with windowed rendering and `RECOMMENDED_ZIP_LIMITS`, no PDF fallback; **checked in a real Chromium** on one real 17-slide, 10 MB Japanese deck from this machine (gradients, images, text rendered; the deck was not kept), **not** on charts, SmartArt or EMF, and **not** against Pagus, so §4.7.2's "decided by rendering a fixture set in both" has not happened. The bundle includes MPL-2.0 code (mtx-decompressor, unmodified, notice shipped). ECharts uses `new Function` somewhere, which the frame's CSP (no `unsafe-eval`) refuses, so a chart path that relies on it would fail inside the frame, not in the window. No thumbnail strip, slide-by-slide mode or notes. |
| **D3** HTML preview | **built, checked against real Chrome.** `view_preview` serves a conversation's folder on loopback under a random token (`GET`/`HEAD` only, never through `..`, `.git` or a symlink out) and opens it in a tab whose every request outside that origin is refused by `Fetch.requestPaused` (confinement is switched on before the first navigation). On the real Chrome a page's own CSS and script ran and its `fetch` to another local server failed with that server seeing 0 requests (`tests/browser_real.rs`). **Gap:** `Fetch` does not intercept WebSockets, so a preview page could still open one; no live reload, no thumbnail |
| **D5** 3D models | **built, partly checked.** three.js 0.170.0 with the glTF/OBJ/STL/PLY loaders and OrbitControls, bundled once with esbuild 0.24.0 (recipe in `desktop/src/vendor/three/entry.js`) and run in the same frame, whose CSP now allows `connect-src blob: data:` (no network) so embedded textures load. Rendered a hand-made GLB cube and an ASCII STL in real Chromium (headless, software WebGL). **Not done:** a `.gltf` with external buffers or textures, animations, Draco/Meshopt, a still for the timeline, a limit on live WebGL contexts; real models and Tauri's WebKit untested |
| **Lottie** | **not built** |
| **Split view** (the user's request 2026-11-15: *each split section has tabs of its own*; the "dock beside the chat" of §3.6 and two tools side by side) | **built; seen in the real window except the last fix.** The screen is one or two **sides** (`groups`), each with its own row of tabs and the tab it shows; `focus` is the side the keys go to (a line on top of its row), and every tab is on exactly one side, the chat included. The split button (right end of the last row, `⌘\\`) sends the tab in view to a side of its own, or asks what to open when there is nothing to send; with two sides it joins them. A `+` opens on its own side; a tab goes to the other side with its `⇥` button or by dragging it onto the other row or body; a side left with no tab is gone. The edge is dragged, moved with the arrows (20–80 %) or reset by double-click; sides, focus and width are remembered per conversation, and the older one-row state opens as one side. **Seen in the window:** chat | terminal each with its own row, the old state migrated, the edge dragged, a tab switched on one side. **Found there and fixed, not re-seen:** pressing a side redrew its row, so the click that followed on a tab of the other side was lost (the fix: only marks change; fake-DOM test checks the button survives). **Not done:** more than two sides, a vertical split, dropping a tab on a screen edge to split, a tab shown on both sides |
| **W4** user attachments in the timeline, sandbox terminal | **not built** |

Claims about what Tauri, WKWebView,
Chrome or the ACP adapters do are marked *unverified* where no one has measured them; what
other products do is cited to their own documentation as read on 2026-11-14.

Today a conversation's screen is one column (the chat) plus a fixed side pane
(`Team | Work | Changes | Plan | Files`). The user asked for two things:

1. **Tabs inside a conversation**: open a terminal, a browser or a file next to the chat.
2. **Media in the chat**: see screenshots, generated images, video and the like where the
   agent produced them.

## 0. Decisions already made (2026-11-14, the user)

| # | Question | Decision |
|---|---|---|
| D1 | Where do the tabs go | **A new tab strip over the centre column.** `Chat` is the first tab and cannot be closed; terminal, browser, file and media tabs sit beside it. The right-hand side pane stays as it is |
| D2 | What is a terminal tab | **A real shell inside the app**: a PTY in the desktop crate, drawn by a vendored xterm.js, started in the thread's `cwd` |
| D3 | What is a browser tab | ~~A Tauri child webview~~ (superseded the same day by D4) |
| D4 | Can agents use the browser tab, and on what engine | **Yes, and the engine is Chromium**, driven over the Chrome DevTools Protocol, so the user and the agents work in one browser and the agent gets trusted input, console, network and file upload. Chosen over a WKWebView driven by injected script after the survey in §3.5.1 |

## 1. Sources and what is already there

| Have | Where | Consequence |
|---|---|---|
| Resizable column beside the chat inside `#split` | `#member-col`, `#member-resizer` in `desktop/src/index.html` | The precedent for docking a tab beside the chat (§3.5) |
| A file tree, a bounded read-only viewer, `@path` into the composer | `docs/files-pane-design.md`, `view.rs` `tree_list` / `tree_read`, `app.js` `openFile` | The **file tab is this viewer given a tab**, not a second one |
| `open_terminal`, which opens Terminal.app with a command | `desktop/src-tauri/src/main.rs` | Stays as "Open in Terminal.app"; the new tab is a different thing |
| A tool call carries `kind`, `locations`, `raw_output`, `diffs` | `acp::ToolUpdate`, `src/acp.rs` | Media paths can be found from `locations` (§4.2) |
| **Tool-call image content is dropped.** `tool_update` reads only `content.content.text` | `src/acp.rs` (`tool_update`) | Screenshots a tool returns as an ACP `image` block never reach the store. This is the one core change media needs |
| `items.kind` is free text; `items.data` is JSON | `src/activity.rs` schema | A `media` item needs no schema change, only a sibling table for the bytes (§4.3) |
| **The timeline already previews media** in markdown links: images inline with an enlarge overlay, `<video>` and `<audio>`, from files in `cwd` (`tree_media`, `files::media`, 16 MiB, as a data URL) and from remote `https` / loopback URLs | `desktop/src-tauri/src/files.rs` (`media`, `MEDIA_LIMIT`), `app.js` (`localMedia`, `mediaKind`, `remoteMedia`, `enlargeImage`), commit `e8f7673` | §4 **extends** this rather than replacing it: what is missing is media the agent returns as ACP blocks, files outside `cwd` (a `/tmp` screenshot), keeping them, and large files without base64 |
| CSP allows `img-src`/`media-src` `'self' data: https: localhost`, no `frame-src`, no `script-src` beyond `default-src 'self'` | `desktop/src-tauri/tauri.conf.json` | A vendored script in `desktop/src/` is allowed; an iframe is not; a custom scheme must be added to `img-src`/`media-src` |
| A per-machine helper process dispatched from `main.rs`, supervised, stopped when idle | `orochi --internal-decision-server` (`decision/local.rs`) | The shape of the browser broker (§3.5.2) |
| A stdio MCP server attached by `acp.rs` to every non-adviser session, withheld from sandboxed launches | `orochi --internal-mailbox` (`mailbox.rs`) | The shape of the browser tools (§3.5.4) |
| A pinned download checked against a SHA-256 in source before unpacking, on demand | `setup.rs` (Node.js, `ANTIGRAVITY_BUILDS`, `sandbox_on_demand`) | How Chrome for Testing is installed |
| Approval through the console's `Ask` queue or a `prompts` row under a host | `chat::Ask`, `recorder::Answerer::Store` | How a new origin is approved (§3.5.6) |
| The window reads only `v_*` views; `orochi threads` reads the same | `docs/desktop-app-design.md` §4.10 | Media gets a `v_media` view and an `orochi threads media` listing (§4.5) |

## 2. Requirements

| ID | Requirement | Reason |
|---|---|---|
| R1 | Tabs are **per conversation**; switching threads switches tab sets | The terminal's `cwd` and the browser's page belong to the work in that thread |
| R2 | The chat tab is always present and never loses its state (scroll, draft, queued messages) when another tab is shown | Switching to a terminal must not cost the user their half-typed message |
| R3 | **Nothing a tab shows enters telemetry; a terminal's bytes and a browser's pages are never stored.** Only the *list* of tabs is remembered, in the page's `localStorage` (as `open`, `cleared`, `sound` already are) | `CLAUDE.md`: telemetry is content-free, and the window's per-viewer state is not the store's |
| R4 | **An agent cannot reach a terminal tab, and reaches a browser tab only through Orochi's broker** (§3.5), under the thread's domain policy, the seat's role and the user's approval. No agent ever holds the browser's own debugging connection unless the user turns that on for the thread | The terminal runs with the user's privileges; the repository, the agent and every web page are untrusted (`CLAUDE.md`, invariants) |
| R5 | Media shown in the chat is **kept** (it is conversation content, like text and diffs), in the same retention and deletion as the thread | A screenshot that vanishes after a restart is worse than none; and it must not outlive its thread |
| R6 | Bytes never go through SQLite or through base64 over IPC when large | A video is tens of MiB; `items.data` is a bounded JSON blob |
| R7 | Media is decoded by the webview only as `<img>`, `<video>`, `<audio>`: never as markup, never as a frame | An SVG or HTML an agent produced must not run script in the window that holds every command |
| R8 | Tested without a GUI and without an agent: Rust tests (PTY over `sh`, media store over a fixture, `mock_acp.py` emitting an image block), page tests over recorded command output | The rule every pane follows |
| R9 | Labels in `WORDS.ja` as well as English | `app.js` already speaks both |

Non-goals: letting an agent drive the terminal tab (a later design, because it needs an
approval model of its own); editing files (`files-pane-design.md` non-goal stands); a tab for
the sandbox's shell (§3.6 says why it is later); driving the user's everyday Chrome (that is
what Claude in Chrome and similar extensions are for, and they keep working beside this);
browser extensions; transcoding or thumbnailing video.

## 3. Tabs

### 3.1 Layout

```
┌ header ───────────────────────────────────────────────────────────────┐
│ [ Chat ] [ ⌘ zsh ● ] [ ◎ localhost:5173 ] [ ▤ term.rs M ] [ ▣ shot.png ] [ + ] │
├───────────────────────────────────────────────────────────────────────┤
│  the active tab's content (the chat, with its composer, or …)         │
│                                                                       │
└───────────────────────────────────────────────────────────────────────┘
```

`#split` becomes the **Chat tab's panel**. A new `#workbench` wraps a `#doc-tabs` strip and one
panel per tab; inactive panels are `hidden`, not destroyed (R2). The member column, the
composer and every existing id stay where they are inside the chat panel, so none of the
existing code that finds them by id changes.

`+` opens a menu: *Terminal*, *Browser*, *File…* (a quick-open over `tree_find` by name),
*Media gallery*. Tabs reorder by drag; each shows an icon for its kind, a title, a dot for
*running / has unread output / exited*, and `×`.

### 3.2 Tab model

```js
// per thread, in localStorage "tabs:<thread id>"; never in the store (R3)
{ order: ["chat","t1","b1","f1"], active: "t1",
  tabs: { t1: { kind: "terminal", title: "zsh" },
          b1: { kind: "browser",  url: "http://localhost:5173/" },
          f1: { kind: "file",     path: "src/chat/term.rs", line: 120, pinned: false },
          m1: { kind: "media",    sha: "9c1f…" } } }
```

* **Restore** brings back every tab; a terminal comes back as a **fresh shell** with one muted
  line, "restored — the shell was restarted", because a process cannot be restored. A browser
  reloads its URL. A file re-reads the file. A media tab whose bytes were reclaimed says so.
* **Limits**: 12 tabs and 6 terminals per thread, 8 terminals in the window. At the limit `+`
  says why instead of opening.
* **Preview tabs** (as VS Code): opening a file from the tree with one click reuses one italic
  preview tab; double-click or editing the tab pins it.
* **Keys**: `⌘T` the `+` menu, `⌘W` close, `⌘1…9` select, `⌃Tab` next, `⌘P` quick-open a file.
  These are handled on `window` in the capture phase so a focused terminal does not swallow
  them; every other key goes to xterm. `⌘C`/`⌘V` in a terminal copy a selection and paste, and
  `⌃C` is sent to the shell.

### 3.3 File tab

`openFile(path, line)` already exists for the Files pane. It gains a second destination: the
tab. The viewer's markup is reused; only its container changes (a tab panel rather than the
side pane's `#viewer`). The same entry points apply and open tabs:

* a file in the Files pane (the side pane's `⤢` becomes **Open in tab**),
* a file path in the timeline, a diff header in Changes, a `locations` entry of a tool call,
* `⌘P`.

Files that are images, video or audio route to a **media tab** instead of the text viewer, so
the `image` data-URL path of `tree_read` (2 MiB cap) is no longer the only way to see a
picture in the working tree. Everything `files-pane-design.md` says about confining paths to
`cwd` (R6 there) applies unchanged: the tab asks `tree_read`, never the disk.

### 3.4 Terminal tab

**Rust (`desktop/src-tauri/src/term.rs`, new)**

```rust
pub struct Terminals { sessions: HashMap<String, Session> }   // in Open state, behind its mutex

// commands (each one call from main.rs)
fn term_open(thread, cols, rows, on_data: Channel<Vec<u8>>) -> Result<String>;   // id
fn term_write(id, data: Vec<u8>) -> Result<()>;
fn term_resize(id, cols, rows) -> Result<()>;
fn term_close(id) -> Result<()>;                // SIGHUP to the session's process group
```

* **PTY**: `portable-pty` (one new dependency of the desktop crate; the CLI is untouched).
  The shell is the user's `$SHELL` (else `/bin/zsh`, `/bin/sh`), started as a login shell, in
  the thread's `cwd` as `Client::cwd` returns it, after the same existence check `tree_*`
  does. The environment is the window's own plus `TERM=xterm-256color`, `COLORTERM=truecolor`
  and `TERM_PROGRAM=orochi`. Nothing is read from the repository to build it.
* **Output** goes to the page through a `tauri::ipc::Channel` (binary, ordered), not events,
  with a 64 KiB read buffer and a coalescing tick of 8 ms so a `yes`-style flood cannot stall
  the page. The page drops to `Pause` (stop reading the channel) past 1 MiB unflushed, and the
  reader thread blocks on the PTY, which is how a real terminal applies back-pressure.
* **Scrollback** lives in xterm (10 000 lines) and nowhere else (R3). Closing the tab or
  quitting the window discards it.
* **Lifetime**: a terminal lives until its tab is closed, the shell exits (the tab shows
  *exited 0 / signal* and `Enter` restarts), the thread is deleted, or the window quits. It
  survives switching threads, because killing a dev server by looking at another
  conversation is the surprise. The session is a process-group leader and is killed as a
  group, so nothing is left behind (the rule `process.rs` follows for agents). Quitting with a
  foreground child running asks first.
* **Windows**: `portable-pty` uses ConPTY there. *Unverified*; the desktop crate's Windows
  build is not part of the hand-run CI jobs today, so the first release of this tab is
  macOS and Linux, and the Windows menu entry says so.

**Page**: xterm.js and its fit and web-links addons, **vendored** under
`desktop/src/vendor/xterm/` (the window has no bundler and its CSP names no CDN), pinned to an
exact version with the SHA-256 of each file recorded in `desktop/src/README.md` and the MIT
licence text beside them. `ResizeObserver` → fit → `term_resize`. Theme follows the window's
light/dark tokens. Web links open in a **browser tab** (or the system browser with `⌘`-click).

### 3.5 Browser tab: one Chromium the user and the agents share

#### 3.5.1 What others do (surveyed 2026-11-14)

| Product | Engine and how the agent reaches it | What it does about safety and visibility |
|---|---|---|
| Cursor ([docs](https://cursor.com/docs/agent/browser)) | A browser in an editor pane, driven through MCP tools: navigate, click, type, scroll, screenshot, console, network | Approval by default; an origin allowlist outside which the tools stop; storage isolated per workspace; console logs written to files the agent can grep |
| VS Code Copilot browser tools, GA 2026-07-01 ([changelog](https://github.blog/changelog/2026-07-01-browser-tools-for-github-copilot-in-vs-code-are-generally-available/)) | Built-in tools: navigate, click, type, hover, drag, dialogs, read the page, console errors, screenshots | **The user's tabs are private until "Share with Agent"**, revocable; pages the agent opens get a fresh session with no cookies; domain allow/deny lists with wildcards, deny first; camera, microphone, location and clipboard reads never granted by an agent |
| Orca ([docs](https://orca.dev/docs/browser/overview)) | A Chromium pane per worktree; agents use the `orca` CLI (`orca snapshot`, `orca click`, `orca fill`) | The agent's cursor and an activity badge on the page; profiles with their own cookie jar; cookie import from Chrome |
| Antigravity ([docs](https://antigravity.google/docs/commands/browser/)) | A separate Chrome profile with its extension, driven through Chrome DevTools MCP | A blue frame on the tab under the agent's control; site allow/deny lists |
| Claude in Chrome ([docs](https://code.claude.com/docs/en/chrome)) | The user's real Chrome through an extension and Native Messaging | Site permissions in the extension |
| Devin ([docs](https://docs.devin.ai/work-with-devin/devin-session-tools)) | A browser in its cloud VM, shown live in a Browser tab, which the user can take over | — |

What follows from it: the products whose agent gets full browser powers are on **Chromium and
CDP** (Cursor, Orca, Antigravity); macOS's WKWebView has neither CDP nor a WebDriver
([Tauri](https://v2.tauri.app/develop/tests/webdriver/): "no WKWebView driver tool"), so the
products on it drive the page with injected script and synthetic events. Hence D4. And every
one of them, whatever the engine, has the same four rules, which this design takes as they are:
**a separate session for the agent, a domain policy, approval by default, and a visible sign
that the agent is acting.**

#### 3.5.2 Shape

```
            ┌──────────────── orochi --internal-browser (one per machine) ────────────────┐
            │  broker: owns Chrome over --remote-debugging-pipe, policy, sessions, tabs   │
            │      ▲ unix socket <data>/browser/broker.sock (dir 0700)                   │
            └──────┼─────────────────────────────┬─────────────────────────┬──────────────┘
                   │                             │                         │
     desktop window: screencast + input   orochi --internal-browser-mcp   (opt-in) CDP proxy
     (a viewer, like every other pane)    per agent session (stdio MCP)   ws://127.0.0.1:<p>/<token>
                   │                             │                         │
                 user                      Orochi's browser tools      the agent's own tools
                                                                        (chrome-devtools-mcp,
                                                                         Playwright MCP, …)
                   └──────────────► Chrome for Testing, --headless=new ◄───┘
```

* **The broker is in the core binary, not the window.** `orochi --internal-browser` is
  dispatched in `main.rs` like `--internal-decision-server`: one per machine, its own process
  group, recorded in `<data>/browser/server.json`, stopped with Chrome when no client and no
  agent session has used it for `browser.idle_secs` (default 600). The window is only a
  viewer of it, which is the rule the whole desktop follows (`desktop-app-design.md` R1): a
  console session in a terminal and a headless host get the same browser and the same tools,
  and closing the window does not take the agent's browser away mid-turn.
* **Chrome for Testing**, not the user's Chrome: a pinned version installed by
  `setup` on demand (the first time a browser tab or a browser tool is asked for, as
  `setup::sandbox_on_demand` does for Lima), under `<data>/toolchain/chrome/<version>/` with
  `current` swapped by rename. Chrome for Testing's JSON endpoints give the download URLs but
  no checksums, so **the SHA-256 of each platform's archive is pinned in source** as
  `ANTIGRAVITY_BUILDS` does, and an archive that does not match is refused before it is
  unpacked. Size is hundreds of MiB unpacked (*unmeasured*), which is why it is on demand and
  never part of the app bundle.
* **Started with `--headless=new --remote-debugging-pipe`**: a pipe, never a port, so no other
  process on the machine can attach to it. `--user-data-dir` is Orochi's own (§3.5.5), plus
  `--no-first-run`, `--no-default-browser-check`, `--disable-sync` and the CfT defaults. It is
  launched through `orochi --internal-supervise`, so it dies with its owner (`process.rs`).

#### 3.5.3 What the user sees: a screencast with real input

The tab is a `<canvas>` plus ordinary DOM chrome: address bar, back, forward, reload, a
loading bar, the tab's owner (*you* / an agent's name) and its sharing state.

* **Frames**: the broker runs `Page.startScreencast` (JPEG, quality 80, `maxWidth/maxHeight`
  = the panel's size × `devicePixelRatio`) on the page shown in the **active** tab only, acks
  each frame, and stops it when the tab is hidden; frames reach the window as bytes over a
  `tauri::ipc::Channel`. Chrome sends no frame while nothing changes, which costs nothing and
  means the canvas keeps the last one. The page's viewport follows the panel through
  `Emulation.setDeviceMetricsOverride` (debounced while the user resizes).
* **Mouse, wheel, keys**: `Input.dispatchMouseEvent` / `dispatchKeyEvent`, which Chrome treats
  as trusted input (`isTrusted` true). `⌘`-shortcuts the window owns (§3.2) are not sent.
* **Japanese input**: CDP's `Input.insertText` fires no composition events
  ([Playwright #5777](https://github.com/microsoft/playwright/issues/5777)), so composition
  is done **in the window**, where the system IME works: a transparent `<textarea>` over the
  caret takes the keys; `compositionupdate` is sent as `Input.imeSetComposition` (so the page
  sees the underlined candidate) and `compositionend` as `Input.imeCommitComposition`. The
  caret's place comes from the focused element's box (`DOM.getBoxModel`), which is approximate
  in a rich editor. *Unverified, and the riskiest part of the tab for this user.*
* **What headless has no UI for, the window draws**: JavaScript dialogs
  (`Page.javascriptDialogOpening` → a DOM dialog), file choosers
  (`Page.setInterceptFileChooserDialog` → the system's open dialog → `DOM.setFileInputFiles`),
  downloads (`Browser.setDownloadBehavior` into `<data>/browser/downloads/`, listed in the tab
  and handed to the Files pane's confined `open`), permission requests (denied unless the user
  allows the origin, §3.5.6), and a context menu with *Copy*, *Copy link*, *Open in new tab*,
  *Inspect*.
* **Popups** a page opens (`Target.targetCreated` with an opener) become tabs, inside the cap.
* **DevTools**: *Inspect* opens Chrome's own DevTools frontend for that target in a second
  browser tab, served by the broker. (Later phase; until then console and network are in the
  tab's own drawer, from the same buffers the agent reads.)
* **Cursor shape and native `<select>` popups** are the known gaps of a screencast:
  the canvas shows an arrow, and whether headless renders a `<select>`'s popup into the
  frame is *unverified*; if it does not, the broker answers a click on a `<select>` with a DOM
  menu built from its options.

Later, when Tauri's CEF runtime is stable (alpha in Tauri 3 as of 2026-09,
[Tauri docs](https://v3.tauri.app/develop/cef/)), the canvas can be replaced by a native
Chromium view with the system IME and cursor. **Nothing on the agent's side changes then**,
because the agent talks to the broker, and the broker talks CDP to whichever Chromium it is.

#### 3.5.4 What the agent gets: Orochi's browser tools, and raw CDP only if asked

**Default: MCP tools Orochi implements over CDP.** `acp.rs` attaches
`orochi --internal-browser-mcp --thread <id> --seat <id>` to every non-adviser `session/new`
the way it attaches the mailbox (after it in the list), when `browser.enabled` and the agent
is not sandboxed. The server holds no browser; it relays to the broker over the socket. Tools,
named and shaped after Playwright MCP's so an agent's habits carry over:

| Tool | CDP underneath | Read-only seat |
|---|---|---|
| `browser_tabs` (list, select, open, close) | `Target.*` within the thread's contexts | list, select |
| `browser_navigate` (url), `browser_back`, `browser_reload` | `Page.navigate` / history | no |
| `browser_snapshot` — the page as an accessibility tree with `ref`s | `Accessibility.getFullAXTree`, `DOM.describeNode` | yes |
| `browser_click`, `browser_hover`, `browser_drag` (ref) | box model → `Input.dispatchMouseEvent` | no |
| `browser_type` (ref, text, submit?), `browser_press` (key) | focus → `Input.insertText` / `dispatchKeyEvent` | no |
| `browser_select` (ref, values), `browser_upload` (ref, paths under `cwd`) | `DOM.setFileInputFiles` | no |
| `browser_screenshot` (full page?, ref?) | `Page.captureScreenshot` → an MCP image result, which §4 then keeps as media | yes |
| `browser_console`, `browser_network` (since, filter) | buffered `Runtime.consoleAPICalled`, `Log.entryAdded`, `Network.*` (no bodies) | yes |
| `browser_wait` (text / ref / load) | events | yes |
| `browser_evaluate` (expression) | `Runtime.evaluate` in the page | **off unless** `browser.agent_evaluate = true` |

Every result that carries page text is prefixed with one line saying it is untrusted page
content, because a page can contain instructions aimed at the agent; that is all a tool can
do about prompt injection, and the domain policy is the real bound.

The same tools are also an `orochi browser <tool> …` CLI (Orca's shape), for an agent that
prefers its shell and for the user's scripts; it talks to the same broker under the thread
named by `OROCHI_THREAD`, and the same policy applies.

**Opt-in: raw CDP for the agent's own tools.** With `[browser] agent_cdp = true` (per thread
from the tab's menu, off by default), the broker opens a loopback WebSocket per agent session,
`ws://127.0.0.1:<port>/devtools/browser/<token>` with a 256-bit token, and the session's
prompt note names it, so `chrome-devtools-mcp --wsEndpoint`, `@playwright/mcp --cdp-endpoint`
or `agent-browser --cdp` can attach. The proxy shows that session only the targets of its own
browser context and refuses `Browser.close`, `Target.createBrowserContext`/`disposeBrowserContext`,
`Storage.*` and `Network.getAllCookies`. **This is a weaker boundary and the setting says so**:
over raw CDP an agent can evaluate script that fetches any address, so the domain policy is
advisory there. It exists because "existing tools just work" is half the reason for Chromium.

#### 3.5.5 Sessions: whose tab, whose cookies

Following VS Code's model:

* **A tab the user opened is the user's, and private.** The agent's tools do not list it until
  the user presses **Share with agents** in the tab (revocable; the tab shows a band while it
  is shared). Its cookies are the **project's profile**: one `user-data-dir` per project under
  `<data>/browser/profiles/<project id>/`, so a login lasts across threads of that project and
  never reaches another project.
* **A tab an agent opened** lives in a **fresh browser context** per thread
  (`Target.createBrowserContext`, nothing persisted, gone with the thread or the broker), so an
  agent never inherits the user's logins by default. The tab appears in the thread's strip with
  the agent's name, and the user may use it like any other.
* **"Let agents use my sign-ins in this project"** (off by default) puts agent tabs in the
  project profile instead, for the case where the work is behind a login the user has made.
* **Clear browsing data** in the tab's menu deletes the project profile.

#### 3.5.6 Policy, approval, visibility

* **Domains**: `[browser] allow` defaults to `localhost`, `127.0.0.1`, `[::1]`, `*.localhost`
  (the dev-server and sandbox-gateway case); `deny` takes precedence; both take `*.` wildcards,
  as VS Code's do. A `browser_navigate` (or a click that navigates the top frame) to an origin
  outside `allow` **asks the user** once per origin per thread — the console's `Ask` queue in a
  terminal, a `prompts` row of kind `permission` with `request.browser = {origin}` under a host
  — and is refused with nobody to ask. An agent's tools stop working on a page that has
  navigated outside the allowed set until the user allows it, as Cursor's do.
* **Schemes**: `http`, `https` only for navigation by anyone; `file:`, `chrome:`, `javascript:`,
  `data:` top-level navigations are refused by the broker (`Fetch`/`Page.frameRequestedNavigation`).
* **Approval mode**: the thread's approval mode (ask / always / never) applies to the write
  tools (`navigate`, `click`, `type`, `select`, `upload`, `press`) as it does to an agent's
  edits; read tools never ask. A read-only seat gets the read tools only (§3.5.4 table).
* **Site permissions** (camera, microphone, geolocation, notifications, clipboard read) are
  denied for every origin unless the user allows one in the tab; an agent can never allow one.
* **Visible**: while an agent tool runs on a tab, the tab shows the agent's name and a frame,
  the canvas draws the agent's pointer at the clicked point, and the tab's strip entry pulses.
  **Take over** (or any user input in the tab) pauses the agent's write tools on that tab — they
  answer "the user is using this tab" — until the user presses **Hand back**. Esc in the chat
  still stops the whole turn.
* **Recorded**: each tool call is an ordinary tool item in the timeline (title, URL, outcome);
  screenshots are media (§4). No page content reaches telemetry; nothing about the browser
  does, beyond the tool call's existence in the run's counts.

#### 3.5.7 Edges

* **Local dev servers**: a URL on loopback that appears in a tool call's `detail` or output is
  offered as an **Open in browser tab** chip on that tool row; never opened automatically.
* **Sandbox**: an agent launched in a sandbox gets no stdio MCP server from this machine
  (`AgentConfig::sandboxed`), so it gets no browser tools in v1. Services inside the VM are
  reachable through the `*.localhost` gateway, which is loopback here, so the *user's* tab can
  open them. Giving sandboxed agents the tools goes with `sandbox/runner.rs`'s MCP server and
  is later. Nothing here touches the Mac's system configuration.
* **Windows**: the same design; Chrome for Testing has Windows builds. *Unverified*, as for the
  terminal tab.
* **The agent's own browser tools** (Claude in Chrome, a configured Playwright MCP) keep
  working; the prompt note says that the Orochi tab is the one the user is watching.

#### 3.6 Dock beside the chat (phase W4)

Watching a dev server while talking to the agent needs both on screen. Each non-chat tab has a
**Dock right** button that moves its panel into a resizable column inside `#split`, using the
same `.resizer` and width memory as `#member-col`. The chat keeps its composer. Only one tab is
docked per thread. (A free split layout is not proposed: it is the point where this becomes an
IDE, which `desktop-app-design.md` lists as a non-goal.)

A **terminal in the sandbox** is not in v1: it needs `Placement::launch`-style wrapping and
`ops::ensure_vm`, and "never start the VM from something that only looks" has to be answered
for a tab that is opened on purpose. Until then a sandboxed thread's terminal tab opens on the
host and says so in its tab title (`zsh (host)`).

## 4. Media

### 4.1 What counts as media, and where it comes from

| Source | How Orochi sees it | New work |
|---|---|---|
| **ACP content blocks**: `image` and `audio` in an `agent_message_chunk`, and `{"type":"content","content":{"type":"image"…}}` inside a tool call's `content` (what a browser or screenshot MCP tool returns) | `data` (base64) + `mimeType`, sometimes `uri` | `acp.rs` reads them (§4.2). **Today dropped.** ACP has no video block |
| **Files an agent wrote or named**: a tool call's `locations` or `rawOutput` path, a markdown image `![](path)` or a bare path with a media extension in the reply (`/tmp/shot.png`, `out/demo.mp4`) | A path | Detected after the item is written, **copied** when seen (a `/tmp` file is gone by tomorrow) |
| **`resource_link` blocks** (ACP's own way to name a file, in an agent message or a tool result) and **`share_file`**, an Orochi mailbox tool the agent calls with a path and a caption | A name, which is kept only if it lies in the project folder, the temporary directory or Orochi's data (the same check as a path in a reply) | **Built 2026-11-15.** Chosen after reading what the agents' own UIs do: MCP and ACP both carry `image`/`audio`/`resource_link` blocks, but Claude Code is reported (user-filed issues, not documentation) to hand an MCP tool's image to the model without drawing it for the person, so Orochi does not depend on a CLI forwarding bytes; `share_file` runs in Orochi's own server, which knows exactly what was shared, and the prompt note tells the agent to use it. Not measured with a real agent: whether an agent *chooses* to call it |
| **Attachments the user sent** (pasted or `@path` images) | `types::Attachment` already on the turn | Shown above the user's message, same store (phase M3) |
| **Remote URLs** in a markdown link or image | A URL | **Already drawn today** for `https`, loopback `http` and `data:image` (`remoteMedia` in `app.js`, commit `e8f7673`), loaded by the webview and not kept. This design keeps that behaviour; it does not copy remote media into the store. Loading one tells that host the user's address and when they read the reply, so a setting `media.remote = "show" \| "click"` (default `show`, as today) lets the user require a click |

Allowed types, by **magic bytes**, never by the agent's `mimeType` alone: png, jpeg, gif,
webp, avif, svg (as an image only, §4.6); mp4, webm, mov; mp3, wav, ogg, m4a. Anything else is
a file link, not media. Documents — PDF, HTML, Markdown, tables, Office, 3D and Lottie — have
viewers of their own (§4.7).

### 4.2 Core changes (`src/`)

* `acp::ToolUpdate` gains `media: Vec<MediaRef>`, and `Progress` gains `Media(MediaRef)` for a
  message chunk. `MediaRef { name, mime, source: Inline(Vec<u8>) | Path(PathBuf), caption }`.
  `tool_update` reads `content.content` blocks of type `image` / `audio` (and `resource_link`
  with a media `mimeType`) instead of ignoring all but `text`.
* A **path finder** (`src/media.rs`, new) turns text and `locations` into `Path` refs: absolute
  paths, and relative paths resolved against the attempt's `cwd`, kept only if the file exists,
  is regular (not a symlink out of an allowed root), has a media magic number, and lies under
  the thread's `cwd`, the OS temp directory, or Orochi's data directory. A path outside those is
  a plain link. This is the `R6` rule of the Files pane applied to the agent's words.
* `recorder::Tee` turns a ref into a **`media` item** (`kind = 'media'`, `data` =
  `{sha, mime, bytes, name, width?, height?, source: "inline"|"path", from?}`) and stores the
  bytes (§4.3). `orochi serve` and the console keep working: the console prints one muted line,
  `[image: shot.png · image/png · 184 KiB]`, and does not try to draw it in the terminal.

### 4.3 The media store

Bytes live in **files**, indexed in the activity store, never in a row (R6):

```
<data>/activity-media/<thread id>/<sha256>.<ext>        0600, the directory 0700
```

```sql
CREATE TABLE IF NOT EXISTS media (
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  sha TEXT NOT NULL, mime TEXT NOT NULL, bytes INTEGER NOT NULL, name TEXT NOT NULL,
  width INTEGER, height INTEGER, kept INTEGER NOT NULL DEFAULT 1,   -- 0: dropped for size
  derived_from TEXT, derived_as TEXT,           -- a thumbnail / conversion of another sha (§4.7.3)
  created_at INTEGER NOT NULL, PRIMARY KEY (thread_id, sha));
```

* **Content-addressed**: the same screenshot taken twice is one file.
* **Caps** (Orochi's own, uncalibrated, in `[activity]`): 25 MiB per file, 500 MiB per thread.
  A file over the cap is not copied; its item says *too large to keep* and, for a `Path` source
  inside `cwd`, still points at the original. Past the thread cap the oldest unpinned media is
  dropped (`kept = 0`, file removed, item stays and says *no longer kept*).
* **Deletion follows the thread**, in the three places a thread already ends: `threads delete`,
  retention inside `Activity::open`, and `reclaim::reclaim`, which gains "media directories
  with no thread" as a fourth thing a deleted thread leaves. `ON DELETE CASCADE` removes the
  rows; the files go with the directory. `activity.enabled = false` stores nothing, as for
  every other content table.
* **Telemetry never sees any of it** (byte-level test, as `tests/core.rs` does for task text).
  A telemetry row cannot name a thread, so it cannot name media either.

### 4.4 Serving it to the window: a custom scheme, not base64

The window registers `orochi-media://` (`register_asynchronous_uri_scheme_protocol`). A request
`orochi-media://localhost/<thread>/<sha>` is answered from `<data>/activity-media/` **only**:
both segments are validated as an id and 64 hex characters, so there is no path to traverse, the
answer carries the stored `Content-Type` plus `X-Content-Type-Options: nosniff`, and it supports
`Range` with `206` replies because WKWebView will not play a `<video>` without them
(*unverified* here). `img-src` and `media-src` in the CSP gain `orochi-media:` (and on Windows
and Android Tauri's `http://orochi-media.localhost` form, *unverified*). Nothing else about the
CSP loosens; there is still no `frame-src` and no inline script.

### 4.5 The view and the terminal's side

```sql
CREATE VIEW v_media AS SELECT i.id AS item_id, i.thread_id, i.turn_id, i.created_at,
  m.sha, m.mime, m.bytes, m.name, m.width, m.height, m.kept
FROM items i JOIN media m ON m.thread_id = i.thread_id AND m.sha = json_extract(i.data,'$.sha')
WHERE i.kind = 'media';
```

`orochi threads media <id>` lists them and `--save <dir>` writes them out, which keeps the rule
that a terminal user can see everything the window can. A view the CLI cannot read is a view
that is missing something.

### 4.6 Drawing it

* **In the timeline**: a media item is a `<figure>` in the turn, under the tool row or message
  that produced it: `<img loading="lazy">` capped at 360 px high and the column's width, with
  name, size and dimensions as a caption; `<video controls preload="metadata">` (no autoplay,
  muted by default); `<audio controls>`. Consecutive images of one tool call form a row.
  `kept = 0` draws a placeholder with the caption and why.
* **SVG is an `<img>` only**, never inlined, never an `<object>`/`<iframe>` (R7): an `<img>`
  executes no script. The same holds for any file whose bytes sniff as markup.
* **Click** opens a **media tab**: fit / 100 % / zoom and pan for images, the native controls
  for video and audio, previous/next through the thread's media in order, and actions *Copy
  image*, *Save as…* (a dialog, via the dialog plugin already granted), *Reveal in Finder*
  (the confined `open`), *Attach to the next message* (puts it in the composer as the console's
  `@path` does, by saving to the thread's attachment path and writing the reference).
* **Media gallery** (`+` menu): the thread's media as a grid, newest first, filtered by kind.
* **Accessibility**: `alt` is the caption or the agent's own description; focus order is the
  caption, then the figure.

### 4.7 Documents: PDF, HTML, Markdown, tables, Office, 3D and animation

**View only.** None of these viewers edits, saves back or converts in place (the user,
2026-11-14: *editing is not needed*). Each one is a **card** in the timeline (icon, name, size,
and a page/sheet/slide count or a thumbnail where one can be had cheaply) and a **viewer tab**
when the card is clicked. They come from the same sources as §4.1 (paths an agent wrote or
named, attachments, files opened from the Files pane or `⌘P`), classified by magic bytes and
structure, never by extension alone.

#### 4.7.1 Where a viewer runs: a sandboxed frame for every third-party parser

The window holds every Tauri command. A PDF, an Office file or a glTF model is untrusted input
parsed by a large JavaScript library, and a parser bug in the window's own context would be a
bug with the window's privileges. So:

* **Orochi's own renderers stay in the window**: Markdown (the existing `markdown()`, which
  builds DOM nodes and emits no markup) and the CSV/TSV/JSON table, which are a few hundred
  lines of Orochi's code with no third-party parser.
* **Every third-party parser runs in `viewer.html`**, a page of the app's own loaded in
  `<iframe sandbox="allow-scripts">` — **no `allow-same-origin`**, so it has an opaque origin,
  no storage, no access to the parent and no Tauri IPC. The window hands it the bytes with
  `postMessage` (a transferred `ArrayBuffer`) and receives only `{pages, title, thumbnail}`
  back. That Tauri refuses IPC from an opaque-origin frame is *unverified* and is a test (§6).
  The CSP gains `frame-src 'self'` and nothing else; the frame's own CSP forbids network
  (`connect-src 'none'`, `img-src blob: data:`), so a document cannot fetch anything.
* **HTML runs in Chromium** (§3.5), out of the window's process altogether.

The libraries are **vendored** under `desktop/src/vendor/`, pinned to an exact version with
the SHA-256 of each file and the licence beside it (as xterm.js, §3.4), and **loaded on first
use** inside the frame, so none of them costs the window anything at start.

#### 4.7.2 Each kind

| Kind | Detected by | Card in the chat | Viewer tab | How |
|---|---|---|---|---|
| **PDF** | `%PDF-` | First page as a thumbnail, page count | Pages, zoom, fit, page jump, find, text selection and copy, outline | **pdf.js** (Apache-2.0) in the frame with `isEvalSupported: false` and `enableScripting: false`, so a PDF's JavaScript never runs; annotations drawn, forms read-only. Newer pdf.js decodes some image formats in WebAssembly, which would need `'wasm-unsafe-eval'` in the frame's CSP only (*unverified* which build needs it) |
| **HTML** (`.html`, `.htm`) | Extension plus a text sniff | Title from `<title>`, a screenshot thumbnail once the broker has rendered it | A **browser tab** on a preview origin | The broker serves the file's directory, confined to the thread's `cwd`, at `http://127.0.0.1:<port>/<token>/…` so relative CSS, scripts and images work; the page runs in a **fresh browser context with every request outside that preview origin blocked** (`Fetch.enable` + fail), so an agent's page cannot call out. A live-reload toggle re-renders when the file changes on disk. Without Chrome installed: the card only, with *Open in system browser* through the confined `open` |
| **Markdown** (`.md`, `.markdown`) | Extension, text | First heading and two lines | *Preview* / *Source* toggle in the file tab; relative images and links resolved inside `cwd` through `tree_media` / `openFile` | The existing `markdown()`; tables, task lists and fenced code as it already draws them. Mermaid and math are not drawn in v1 (they would mean a third-party renderer in the window) |
| **CSV / TSV** | Extension, delimiter sniffed on the first 64 KiB | The first 5 rows as a small table, row and column counts | A table with a sticky header, column sort, a filter box, row numbers, virtualized rows | Orochi's own parser (RFC 4180 quoting, UTF-8 / UTF-8-BOM / Shift_JIS by detection), bounded at 50 000 rows or 20 MiB, past which the header says *first N rows* |
| **JSON / JSONL** | Parses as JSON (and is not Lottie, below) | Top-level shape (`object · 12 keys`, `array · 3 400`) | A collapsible tree; JSONL as a table when the rows share keys | Orochi's own; same bounds |
| **Spreadsheets** (`.xlsx`, `.xls`, `.ods`) | ZIP with `[Content_Types].xml` / OLE / ODF manifest | Sheet names, first sheet's first rows | The CSV table, with a sheet switcher; formulas shown as their cached values, a cell's formula on hover | **SheetJS** Community Edition (Apache-2.0) in the frame. Charts and images inside sheets are not drawn |
| **Word** (`.docx`) | ZIP with `[Content_Types].xml` naming a WordprocessingML part | First page as a thumbnail (below), first heading | Pages as Word lays them out approximately: page size and margins, headings, lists, tables, images, headers and footers, footnotes | **docx-preview** (docxjs, Apache-2.0, a few hundred thousand downloads a week by [Snyk](https://security.snyk.io/package/npm/docx-preview) and [Depscope](https://depscope.dev/pkg/npm/docx-preview), which disagree) in the frame, rendering HTML with the document's own styles. The frame is the boundary (§4.7.1), so its HTML is not rebuilt; a link click is posted to the window, which opens it under the browser policy. Fonts the machine lacks fall back; fields, tracked changes and comments show as their current text. `.doc` (binary Word) and `.odt` get the card and a Quick Look thumbnail only |
| **PowerPoint** (`.pptx`) | As above, PresentationML | First slide as a thumbnail, slide count | Slides one by one or as a scroll, thumbnails down the side, zoom, presenter notes hidden | **@aiden0z/pptx-renderer** (Apache-2.0, 1.4.0 read 2026-11-14, [pkgstats](https://www.pkgstats.com/pkg:@aiden0z/pptx-renderer)) in the frame: HTML/SVG with shapes, text, images, groups, gradients, tables with their styles, charts through ECharts, equations; SmartArt drawn from its fallback picture; no animations or transitions; EMF/WMF only where a PDF preview is embedded. **Pagus** (MIT, SVG per slide, embedded fonts via `@font-face`, [site](https://pagus-kit.github.io/Pagus/)) is the alternative; which one ships is decided by rendering a fixture set of real decks in both (§6), not by their READMEs. `.ppt` and `.odp`: card and thumbnail only |
| **3D models** (`.glb`, `.gltf`, `.obj`, `.stl`, `.ply`) | `glTF` magic / JSON with `asset.version` / text or binary STL | A still rendered once when scrolled into view | Orbit, pan, zoom, wireframe toggle, a grid, the model's size and triangle count, animations listed and playable | **three.js** (MIT) with its glTF / OBJ / STL / PLY loaders in the frame. A `.gltf`'s external buffers and textures are fetched by the window through `tree_media` (inside `cwd` only) and handed over; Draco / Meshopt-compressed models need those decoders' WebAssembly (*later*). At most **2 live WebGL contexts** in the timeline; the rest are stills, since browsers cap the number of contexts |
| **Lottie** (`.json` with `v`, `fr`, `ip`, `op`, `layers`; `.lottie`) | Structure | First frame as a still, duration | Play, pause, scrub, loop, background toggle | **lottie-web**'s *light* build (MIT), which has **no expression engine** — expressions are JavaScript `eval`, which the frame's CSP forbids anyway, so an animation that relies on them draws without them and the viewer says so. `.lottie` (dotLottie, a ZIP) is unpacked with **fflate** (MIT) |

Animated GIF, APNG and animated WebP are already images (§4.1) and animate as `<img>` does.

#### 4.7.3 Derived files

**Thumbnails for cards** come from the viewer itself (the frame renders page or slide one at
card size and posts back a PNG), and on macOS first from **Quick Look** (`qlmanage -t -s 800
-o <tmp> <file>`, run in its own process group with a 15 s timeout), which is the system's own
renderer for every Office and iWork type and costs nothing to install — that is also what
gives `.doc`, `.ppt`, `.odt`, `.odp`, `.key`, `.pages` and `.numbers` a picture at all. Quick
Look's full preview (`QLPreviewView`) is not used: it is a native view and would have the
browser tab's problem of painting over the DOM, and Apple says an embedded preview controller
may show only a thumbnail.


A thumbnail, an HTML screenshot or a Quick Look image is a **derived** media
file: stored as §4.3 stores media, keyed by the source's sha plus what it was derived as
(`media.derived_from`, `media.derived_as`), counted against the same caps, deleted with the
thread, and made again on demand if it was dropped. A source over the per-file cap is still
viewed from where it lies when it lies inside `cwd`; it is just not kept.

#### 4.7.4 What is deliberately not done

Editing or saving any of these (the user's decision); running a PDF's or an Office file's
scripts and macros; fetching a document's remote resources; Mermaid, math and syntax
highlighting in the window (each is a renderer in the privileged context — a later decision);
CAD formats (STEP, DWG) and USDZ; LibreOffice or any other converter the user would have to install; cloud viewers.

## 5. Security notes, in one place

1. **Terminal**: same privileges as the app and as the user's own Terminal; only the user's
   keystrokes reach it (R4). The session's `cwd` is validated; no repository file contributes
   to its environment or command line.
2. **Browser**: Chrome is reached only over a pipe the broker holds, never a port; pages are
   untrusted content that cannot reach the window (they are pixels on a canvas, with no
   bridge back); agents act only through the broker's tools under the domain policy, the
   approval mode and the seat's role; an agent's tab gets a fresh context, never the user's
   sign-ins unless the user says so; site permissions are never granted by an agent. Raw CDP
   for agents is opt-in per thread and documented as the weaker boundary it is.
3. **Media**: bytes served only by id and hash from one directory; type by magic bytes;
   rendered only through passive elements, or parsed in a sandboxed frame with no IPC and no
   network (§4.7.1); remote URLs not copied into the store; paths from an agent's text
   confined to `cwd`, the temp directory and Orochi's data directory, then copied.
4. **Outward transmission.** The terminal and the document viewers send nothing anywhere; the
   timeline loads a remote image or video an agent linked, as it already does today, unless
   `media.remote = "click"`. The browser loads
   what the user navigates to, and what an agent navigates to **only inside `browser.allow`**
   or after the user approved that origin; by default that is loopback only.

## 6. Tests

Following the rules each pane already uses (`desktop/src-tauri/tests`, `desktop/tests`):

| Test | What it pins |
|---|---|
| `term.rs`: spawn `sh -c 'printf hi; exit 3'` over a PTY, read `hi`, see exit 3; write then echo; resize changes `stty size`; closing kills a `sleep` grandchild (process group) | R4 and the lifetime rules, with no GUI |
| `term.rs`: `cwd` outside an existing directory is refused; the repository's `.envrc`/`.zshrc` is never read | The repository is not trusted |
| `acp` + `mock_acp.py`: an `image` block in a tool call and in a message chunk reaches the store as a `media` item and a file; a block with a lying `mimeType` is classified by its bytes; an HTML payload is not media | §4.2, R7 |
| `src/media.rs`: path finder accepts `cwd`/tmp files, rejects `/etc/…`, a symlink out, a missing file, a non-media file | §4.2 |
| `tests/core.rs`: a media byte string never appears in `telemetry.sqlite3` | R3 / telemetry invariant |
| `tests/workspaces.rs`-style: deleting a thread, retention, and `reclaim` remove its media directory and nothing else | R5, §4.3 |
| `desktop/src-tauri/tests`: the scheme handler answers a stored file with `Range`, 404s `..`, a short hash and another thread's sha | §4.4 |
| `desktop/tests/ui.test.mjs`: tab strip open/close/restore from `localStorage`, limits, preview-tab replacement, chat panel keeps its draft and scroll when another tab is shown, media figure drawn from `thread.json` | R1, R2, §3.2 |
| `tests/browser.rs` over `tests/fixtures/fake_chrome.py` (a CDP peer on fds 3/4, NUL-delimited, as `--remote-debugging-pipe` speaks; the same idea as `fake_incus.py`): the broker starts, one per machine, stops when idle; each tool sends the CDP it should; a read-only seat is refused the write tools; navigation outside `allow` asks, and is refused with nobody to ask; `deny` wins over `allow`; `file:`/`chrome:` refused; an agent's tab is in a new browser context and a user's unshared tab is not listed; *Take over* makes write tools answer busy | §3.5.4–3.5.6 |
| `tests/browser.rs`: the raw-CDP proxy rejects a wrong token, hides other contexts' targets, refuses `Browser.close` and `Storage.*`; it is not opened unless `agent_cdp` is set | §3.5.4 |
| `tests/setup.rs`-style: Chrome for Testing archive with a wrong SHA-256 is refused before unpacking (local `http.server`, as Node's is) | §3.5.2 |
| `desktop/tests/ui.test.mjs`: every third-party viewer is created as an `<iframe sandbox="allow-scripts">` without `allow-same-origin`; the frame's CSP has `connect-src 'none'`; pdf.js is called with `enableScripting: false` and `isEvalSupported: false`; a `<script>`, an `onerror` and a `javascript:` link inside a `.docx` do not run in the frame and a link is opened only through the window's policy | §4.7.1 |
| `desktop/tests/ui.test.mjs` over fixtures: a CSV with quoted newlines, a Shift_JIS CSV, a 60 000-row CSV (truncated and said so), a Lottie JSON told from plain JSON, a `.gltf` whose buffer path tries `../` (refused) | §4.7.2 |
| `tests/browser.rs`: an HTML preview's request to another origin fails; its directory server refuses a path out of `cwd` | §4.7.2 HTML |
| A fake `qlmanage` on `PATH`: a thumbnail is a derived file; a hung one is killed with its group; off macOS it is never called | §4.7.3 |
| A fixture set of real documents (a text-heavy `.docx`, a `.docx` with tables and images, a `.pptx` with charts, tables, SmartArt and Japanese text, an `.xlsx` with several sheets), each rendered and screenshotted in the frame and kept as reference images reviewed by eye once; the pptx renderer is chosen on them | §4.7.2 |
| `desktop/tests/ui.test.mjs`: the browser tab draws frames from a recorded stream, maps a click on the canvas to page coordinates at a non-1 `devicePixelRatio`, and turns a composition into `imeSetComposition` / `imeCommitComposition` | §3.5.3 |

What these tests **cannot** show, and which therefore stay *unverified* until the window is
run by hand: Japanese input through `imeSetComposition` in real pages (a plain `<input>`,
a `contenteditable` editor, a React-controlled field), screencast latency and frame size on a
Retina display, whether headless draws `<select>` popups, WKWebView video playback over the
custom scheme, ConPTY on Windows, and xterm's key handling against `⌘` shortcuts. A real
run is a separate dated `real-validation-*.md`.

## 7. Phases

| Phase | Content | New dependency |
|---|---|---|
| **T0** | Tab framework: strip, chat panel kept alive, persistence, limits, keys; **file tabs** from the existing viewer; Files pane `⤢` → tab | none |
| **M1** | Media core: ACP image/audio blocks, `src/media.rs`, `media` table, store files, `v_media`, `orochi threads media`, reclaim/retention, console line | none |
| **M2** | Media in the window: scheme handler, timeline figures, media tab, gallery, CSP change | none |
| **T1** | Terminal tab (Rust PTY + vendored xterm.js) | `portable-pty`, xterm.js (vendored, pinned) |
| **B1** | Browser broker in the core (`src/browser.rs`, `src/browser/{broker,cdp,tools,policy}.rs`): Chrome for Testing on demand, pipe CDP (hand-rolled, as the mailbox's MCP is), contexts, policy, the MCP tools and `orochi browser` CLI. Usable from the terminal console before the window draws anything | Chrome for Testing (downloaded, pinned by SHA-256; no crate) |
| **B2** | Browser tab in the window: screencast canvas, input, IME, dialogs, file chooser, downloads, sharing, agent pointer and take-over | none |
| **B3** | Raw CDP proxy (opt-in), DevTools frontend, dev-server chips | none |
| **D1** | Document viewers with no dependency: Markdown preview, CSV/TSV/JSON tables, document cards | none |
| **D2** | `viewer.html` sandboxed frame; PDF | pdf.js (vendored) |
| **D3** | HTML preview in a browser tab on a closed preview origin (after B1) | none |
| **D4** | Spreadsheets, Word, PowerPoint; card thumbnails, Quick Look on macOS | SheetJS, docx-preview, @aiden0z/pptx-renderer or Pagus (vendored) |
| **D5** | 3D models and Lottie | three.js, lottie-web light, fflate (vendored) |
| **W4** | Dock a tab beside the chat; user attachments in the timeline (M3); sandbox terminal | none |

T0 → M1 → M2 first, because they need no new dependency and deliver "see a screenshot" and
"open a file" on their own; B1 before B2 because the agent's side is where the value is and
it is testable without a GUI (a `browser_screenshot` already shows in the chat through M2);
T1 is independent and can go at any point.

## 8. Risks and open points

| Risk | Note |
|---|---|
| **Japanese input in a screencast** | The whole of §3.5.3's IME path is *unverified*; if `imeSetComposition` misbehaves in common editors the fallback is commit-only (`insertText` at `compositionend`, the candidate drawn by the window), which works but shows no inline underline in the page |
| **Screencast feel** | JPEG frames, latency, an arrow cursor, no native context menu. Acceptable for checking a dev server and watching an agent; not a daily browser. The path out is Tauri's CEF runtime (§3.5.3), alpha today, and nothing on the agent's side would change |
| **Download size** | Chrome for Testing is hundreds of MiB (*unmeasured*); on demand only, and a setting points at an existing Chrome/Chromium binary instead |
| **Prompt injection from pages** | A page can address the agent. Bounded by the loopback-only default, approval of new origins, fresh contexts and no agent-granted permissions; not eliminated |
| **Raw CDP** | Opt-in because it can step around the domain policy; the proxy's filtering is a convenience, not a sandbox |
| **WKWebView and `<video>`** | Needs `Range` support; unverified against the custom scheme |
| **Terminal output flood and key handling** | Back-pressure and the `⌘`-shortcut capture order need a real run |
| **Disk growth** | 500 MiB per thread is a guess; the caps are config so they can be corrected after use |
| **Agents that print huge base64 images** | An inline block over the per-file cap is dropped with a note; the block is not stored in `items.data` either way |
| **Viewer frame and IPC** | The whole safety of §4.7.1 rests on an opaque-origin frame getting no Tauri IPC; *unverified*, first thing to test by hand. If it does get IPC, viewers move to the Chromium broker (render there, hand back pixels), which is slower but out of process |
| **Vendored library weight** | pdf.js, SheetJS, three.js and lottie-web together are several MiB in the app bundle (*unmeasured*); loaded lazily, so they cost disk, not start-up. Each is pinned and updated by hand, which also means security fixes are by hand |
| **Office fidelity** | Browser renderers approximate: fonts the machine lacks, SmartArt, EMF/WMF art and animations are where they differ from PowerPoint and Word. Good enough to read what an agent produced; not a proof of how it prints. Said in the viewer's footer, not hidden. Cloud viewers (Microsoft's Office Online viewer, Google's) render better but **upload the file to that company**, which Orochi does not do with the user's content, so they are not offered |
| **New dependencies** | One Rust crate (`portable-pty`) and one vendored JS library (xterm.js), both in the desktop crate only; the core's CDP client is hand-rolled and Chrome is a pinned download, so the CLI gains no crate |

## 9. What to update when built

`CLAUDE.md` (a note per new module: the desktop's `term.rs`, the core's `browser.rs` and
`browser/`, `media.rs`; and the new invariant that an agent reaches the browser only through
the broker), `README.md` "Layout" table for `src/media.rs` and `src/browser*`, `docs/desktop-app-design.md` §6.1 (the layout gains a tab strip),
`desktop/src/README.md` (vendored files and their hashes), and `docs/files-pane-design.md`
§4 (the viewer can live in a tab). A real-use measurement goes in a new dated
`docs/real-validation-*.md`, not into this file.
