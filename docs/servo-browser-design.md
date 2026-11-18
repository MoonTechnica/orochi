# Servo behind the window's browser tab, and the tab on a phone

Status: **built 2026-11-17, not run against a real phone or the user's own login site.**
§12 says what is built and what was run; the measurements are in
`docs/real-validation-20261117.md`. Claims are marked *built*, *measured* or
*unverified*, as in the other design documents. Extends `conversation-tabs-design.md` §3.5.

## 0. Decisions (the user, 2026-11-17)

| # | Question | Decision |
|---|---|---|
| D5 | What engine draws the window's centre-column browser tab | **Servo** (`servo` crate, MPL-2.0), embedded as a library. This supersedes D4 of `conversation-tabs-design.md` **for the person's own tab**; D4's reasons (trusted input, console, network, file upload for agents) are about the agents' tools, which this document does not move (§3.4) |
| D6 | Can the tab be seen from a phone | **Yes.** The same tab, over the network, in the phone's own browser (§6) |
| D7 | Is Chromium dropped | **No.** Chromium stays as the fallback (`browser.engine`, §3.4). Servo is the standard for the person's tab and, since 2026-11-18, for the agents' tools too (D8), until it has been run on the sites the user logs into (§9) |
| D8 | Which engine the agents' `browser_*` tools use | **Servo where an `orochi-servo` is found** (`browser.engine = "auto"`), on the user's instruction 2026-11-18 ("the embedded browser should be the standard"); Chrome where there is none, and for what Servo cannot do (§3.4) |

D7 was mine, not the user's: it is the cheapest way to keep the browser usable if Servo
cannot render a site the user needs, and it is easy to remove once §9's questions are
answered.

## 1. Requirements

| # | Requirement | Why |
|---|---|---|
| R1 | The tab renders real pages, runs their JavaScript, keeps cookies and storage, and takes mouse, wheel, keys and **Japanese IME** | The user logs in through it and looks at the result |
| R2 | A screenshot of the tab (and of an agent's page, where the engine is the same) can be taken | Said by the user |
| R3 | The user can sign in once and the session survives restarts of the broker, the window and the machine | Login is the reason a real browser is needed at all |
| R4 | Orochi's single-binary build is not slowed or made bigger: no SpiderMonkey in `cargo build` of the CLI, none in CI's default jobs | `CLAUDE.md`: every check runs here; the desktop crate is already its own workspace for the same reason |
| R5 | A crash, hang or memory growth in the engine never takes the window, the broker or an agent turn with it | Servo is 0.x and renders arbitrary pages |
| R6 | Nothing the page does reaches anything but the page: same host policy (`policy.rs`) as the Chromium tab, on a navigation **and** on where the tab is afterwards | Existing invariant: the repository and every web page are untrusted |
| R7 | The viewer (window or phone) is only a viewer: it sends input and receives frames, never a script, never a debugging connection | `conversation-tabs-design.md` R4, §3.5.2 |
| R8 | A phone can open the tab, see it update, tap, scroll, pinch and type (including with a soft keyboard and IME), with no app to install | D6 |
| R9 | Nothing reaches the phone from outside the machine unless the user turned it on, the phone proved it was paired, and the path is encrypted or the user said otherwise in words | Login pages are shown and passwords are typed over this link |
| R10 | Nothing changes the Mac's system configuration or asks for root | User instruction 2026-10-01 (`CLAUDE.md`, sandbox notes) |
| R11 | The person's tab and an agent's browser stay separate: separate profile, separate cookies, and an agent cannot reach the person's pages | `conversation-tabs-design.md` §3.5.5 |

## 2. What exists

* The broker (`orochi --internal-browser`, `src/browser/server.rs`, `broker.rs`) owns one
  headless Chrome over `--remote-debugging-pipe`. The window talks to it through
  `browser_call` (`desktop/src-tauri`) with the tools `view_open`, `view_navigate`,
  `view_back`, `view_forward`, `view_reload`, `view_resize`, `view_frame`, `view_input`,
  `view_dialog`, `view_preview`, `view_agents`, `view_unwatch`, `view_close`
  (`broker.rs` `view`, from line 356). A `user:<thread>:<tab>` scope is the person's; the
  broker refuses `view_*` from any other scope.
* The window draws the newest screencast frame on a `<canvas>` and sends mouse, wheel, key
  and IME events back (`desktop/src/app.js` from "A browser tab", about line 6650: `VK`,
  `modifiersOf`, `startBrowser`, the polling `view_frame` loop).
* `view_input` is **CDP-shaped** (`mouseMoved`, `keyDown` with `vk`, `ime` with `commit`);
  the viewer is already engine-neutral in what it *sends*, not in the field names.
* `policy.rs` is the one rule for where a tab may be; the broker applies it on navigation
  and, through `guard`, to where a tab is afterwards.
* Not present: any listener that is not a Unix socket or loopback, any WebSocket code, any
  notion of a second viewer of one tab.

## 3. Shape

### 3.1 Servo is a separate process, `orochi-servo`

```
 window (Tauri)            phone's browser
    │ browser_call              │ HTTPS/WSS (§6)
    ▼                           ▼
 ┌──────────── orochi --internal-browser (broker, one per machine) ─────────────┐
 │  policy · scopes · tab table · viewers · frame fan-out · pairing/remote (§6)  │
 │   ▲ CDP over a pipe (agent tools; tab when engine = chromium)                 │
 │   ▼ engine protocol over a pipe (tab when engine = servo)                     │
 └────┬──────────────────────────────────────────┬──────────────────────────────┘
      │                                          │
   Chrome (headless)                        orochi-servo  (one per person's tab)
                                             Servo + SoftwareRenderingContext
```

* **A separate binary, in its own workspace** (`browser-servo/`, the way `desktop/src-tauri`
  is), so the CLI's `cargo build`, `mise run check` and CI never build SpiderMonkey (R4).
  The broker finds it as it finds Chrome: `browser.servo` (a path), else
  `<data>/toolchain/servo/current/orochi-servo`, else the Servo tab is unavailable and the
  tab falls back to Chromium with a line saying so.
* **A separate process per tab** (R5, R11). Servo's API is `Rc`-based and single-threaded,
  so one process owns one `Servo` on its main thread; a crash or a runaway page ends that
  tab only, and the broker draws "the page crashed, reload". It also gives each tab its own
  data directory without Servo having to partition one.
* **Launched through `orochi --internal-supervise`**, like every agent and Chrome, so it dies
  with the broker (`process.rs`).
* **Distribution (not built).** A Servo build is hundreds of MiB
  unpacked (*unmeasured*). Built per platform by a release workflow, published with a
  SHA-256 pinned in source (as `ANTIGRAVITY_BUILDS` does), put in by `setup` on demand the
  first time a Servo tab is asked for (as `setup::sandbox_on_demand` does for Lima), never
  in the app bundle. Until that job exists: `mise run servo` builds it from `browser-servo/`
  into `<data>/toolchain/servo/`.

### 3.2 The engine protocol

Engine-neutral, on the child's stdin (commands) and stdout (events), one JSON object per
line; a `frame` event is followed by exactly `len` raw bytes of JPEG. stderr is the log and
is never parsed. **The input shape is the one `view_input` already has** (the window's
viewer sends it today and the Chromium backend maps it to `Input.dispatch*`), so neither the
window nor the phone changes when the engine does; the earlier idea of reshaping it was
dropped. Started as
`orochi-servo --profile <dir> --width <css px> --height <css px> --quality <1-100>`.

Commands (broker → engine):

| `cmd` | Fields |
|---|---|
| `open`, `navigate` | `url` (http, https or `about:blank` only) |
| `back`, `forward`, `reload`, `stop`, `show`, `hide`, `shutdown` | none |
| `resize` | `width`, `height` (CSS px), `ratio` (device pixel ratio, 1–3) |
| `zoom` | `factor` (page zoom) |
| `input` | exactly `view_input`'s fields: `type` = `mouse` (`kind` = `mouseMoved`/`mousePressed`/`mouseReleased`/`mouseWheel`, `x`, `y`, `button`, `buttons`, `clickCount`, `modifiers`, `deltaX`, `deltaY`) · `key` (`kind` = `keyDown`/`keyUp`/`rawKeyDown`/`char`, `key`, `code`, `vk`, `modifiers`, `text`) · `text` (`text`) · `ime` (`text`, `commit`) |
| `dialog` | `accept`, `text` (answers the open JavaScript dialog) |

`modifiers` is CDP's bit set: Alt 1, Ctrl 2, Meta 4, Shift 8.

Events (engine → broker):

| `event` | Fields |
|---|---|
| `ready` | — (the process is up, `open` may follow) |
| `url`, `title` | `url` / `title` |
| `history` | `back`, `forward` (booleans) |
| `loading` | `state`: `started` or `complete` |
| `cursor` | `cursor`: a CSS cursor name |
| `dialog` | `kind` (`alert`/`confirm`/`prompt`), `message`, `default` |
| `dialog_closed` | — |
| `popup` | `url` (a page asked for a new window; the broker decides) |
| `refused` | `url` (the engine refused a navigation that was not http, https or `about:blank`) |
| `console` | `level`, `text` (capped by the broker) |
| `crashed` | — (the engine's content process died; the process may still be up) |
| `frame` | `seq`, `width`, `height` (device pixels), `len`, then `len` bytes of JPEG |

The broker keeps the newest frame and the tab's state, and answers `view_frame` with them in
the JSON shape the Chromium path already uses (`url`, `title`, `dialog`, `frame` with
`seq`, `jpeg` as base64, `width`, `height`), so `view_*` stays a poll: the window and the
phone ask what changed since the last `seq` they drew.

### 3.3 Frames

* Servo paints into a `SoftwareRenderingContext` (CPU). On `notify_new_frame_ready` the
  engine calls `paint()`, reads the framebuffer and encodes it: **JPEG at quality 80 by
  default**, the same trade the Chromium screencast makes, and the same `since`/ack
  discipline `view_frame` already has, so a slow viewer gets fewer frames, never a queue.
* Only the **active** tab's engine paints for a viewer; a hidden tab is told to `hide()`
  (`WebView::hide`) and stops producing frames.
* The framebuffer read-back path, the achievable frame rate and the CPU cost of software
  rendering at 2× are **unmeasured** (§9, spike item 3). If software rendering is too slow,
  the fallback is `OffscreenRenderingContext` on a hidden GL window, which is a different
  context type with the same `WebView` API.

### 3.4 Agents' tools on Servo, and what stays on Chromium

* **The agents' `browser_*` tools** go to Servo where `browser.engine` is `auto` and an
  `orochi-servo` is found (or `servo`). A session gets **its own engine process with no
  profile** (`--frames 0`: no picture is drawn for a window that is not looking), so what it
  stores ends with it and nothing of the person's is in it. The session sticks to the engine
  it started on, like the person's tab; if Servo cannot start in `auto` the session uses
  Chrome. `engine/tools.rs` builds the tools from what the engine offers: `eval` (a script
  in the page, `WebView::evaluate_javascript`), `screenshot` (a PNG, optionally a clip) and
  the same `input` events the person's tab uses.
* **Snapshot** is a walk of the DOM in the page (`engine/snapshot.js`), not Chrome's
  accessibility tree: roles from tags and `role`, names from `aria-label`, labels, text,
  `alt`, `placeholder`; elements to act on get a ref (`e1`…), kept in a non-enumerable
  property of the page's window (the page can read it, as it can read anything in its own
  window). It is coarser than Chrome's.
* **A click or a type** scrolls the element into view, then clicks its middle with real
  mouse events; typing clicks first and waits until the page says the field has the
  keyboard, because the click and the keyboard reach the page by different ways and text
  sent at once landed in the field that had focus before (measured).
* **Policy** is the same: `browser.allow`/`deny` checked before a navigation and on where the
  tab is afterwards (a page that leaves is sent to `about:blank` and told so).
* **What Servo cannot do answers with that said**, and the way out is `engine = "chromium"`:
  `browser_upload` (no file chooser), `browser_network` (no request log), more than one tab
  per session, and a promise returned by `browser_evaluate` is not awaited.
* **Still Chrome's:** `view_preview`, `view_agents` and `view_unwatch` (watching an
  agent's page), which read what an agent's Chrome context holds. An agent on Servo cannot
  be watched yet.
* **Consequence the user should know:** the person's tab and an agent's page are in
  **different engines and different profiles**. An agent cannot use the person's logged-in
  session, and sharing one (`conversation-tabs-design.md` §3.5.6, "share with agent") is
  **not available** for a Servo tab.
* `browser.engine = "chromium" | "servo" | "auto"` (default `auto`). The window shows the
  engine in the tab's corner.

## 4. The tab, behaviour by behaviour

| Behaviour | With Servo | Status |
|---|---|---|
| Load, back, forward, reload, address | `WebView::load`, `go_back`, `go_forward`, `reload`; `notify_url_changed`, `notify_history_changed` | API read (v0.8.0), unrun |
| Mouse, wheel | `notify_input_event` with `MouseButtonEvent`, `MouseMoveEvent`, `WheelEvent` | API read, unrun |
| Keys | `InputEvent::Keyboard` with `keyboard_types` `Key`/`Code`/`Modifiers` | API read, unrun |
| **Japanese IME** | Servo has `CompositionEvent`/`CompositionState` and an `EmbedderControl::InputMethod`; the window keeps its textarea and sends `ime` events | **Unverified, and the riskiest item**: `components/servo/tests/input_methods.rs` exists upstream, but whether a composition underline and a commit reach a page's `<input>` and `contenteditable` is unproven here |
| Screenshot | `WebView::take_screenshot` (waits for load, fonts, images) for the tool; the frame stream for the window | API read, unrun |
| Cookies and storage | `SiteDataManager`, and a per-tab profile directory | Whether a profile persists across processes is **unverified** |
| JS dialogs | `WebViewDelegate` `SimpleDialog` (`AlertDialog`, `ConfirmDialog`, `PromptDialog`) → the window's DOM dialog, answered by `dialog` | API read |
| `<select>`, colour, file chooser | `EmbedderControl::{SelectElement, ColorPicker, FilePicker}` → DOM controls in the viewer | API read |
| Context menu | `ContextMenu` control → the viewer's menu | API read |
| Popups, `target=_blank` | `CreateNewWebViewRequest` → a tab inside the cap | API read |
| Downloads | Not seen in the API (`NetworkManager`, `WebResourceLoad` may serve); **unverified** | gap |
| Cursor shape | `notify_cursor_changed` → CSS cursor on the canvas (better than Chromium's screencast, which sends none) | API read |
| Console and network drawers | `ConsoleLogLevel` messages; network through `WebResourceLoad` | partial |
| Permissions (camera, location, notifications) | `PermissionRequest` → denied unless the origin is allowed | API read |
| Pinch zoom | `adjust_pinch_zoom`, `set_page_zoom` | API read |
| Policy (R6) | The engine is told `allow`/`deny` and refuses a navigation the broker would (`NavigationRequest` is the hook); the broker also checks `url` events and sends the tab to `about:blank` when it ends up outside, exactly as `guard` does | design |
| Web compatibility | **Unknown.** Servo does not render every site; sign-in flows that use WebAuthn, third-party iframes or heavy SPAs are the risk | §9, the deciding item |

## 5. Sessions and sign-in

* **A profile per project, not per tab:** `<data>/browser/servo/<repository-id>/` with
  Servo's own cookie and storage files, `0700`, so a sign-in survives the broker stopping
  after `browser.idle_secs` (R3). It is never shared with Chromium's profile and never read
  by an agent (R11).
* **Sign-in by the user's own hands:** the tab is the place. A login that needs a password
  manager, a passkey or the system's keychain does not work in any embedded engine; that
  limit is stated in the tab, and `Open in my browser` (the default browser, outside
  Orochi) is the documented way to such a site.
* **Orochi never types credentials into the person's tab**, and never reads its cookies.
  The existing rule for sign-in by test accounts in the agents' browser is unchanged.

## 6. The tab on a phone

### 6.1 What it is

A **web page the broker serves**, opened in the phone's browser, that shows the same tab and
sends the same input. No native app and no store (R8): Safari and Chrome on a phone, with
*Add to Home Screen* for a full-screen launch. It is the window's viewer code extracted
from `app.js` into one module (`browser-view.js`) used by both, so the two cannot drift: the
window's IME textarea, key mapping and frame loop are the phone's too.

Who sees what: the phone sees **the person's tabs of the threads on this machine** (a list,
then one tab), and may drive them (the person's own tab, the person's own hands). It can
also watch an agent's page read-only, as the window can. It cannot open a new origin the
policy refuses, run a script, or reach an agent's page for input.

### 6.2 Transport

* **Off by default.** `[browser.remote] enabled = false`. Turning it on (Settings → Browser →
  *Use from a phone*, or `orochi browser remote on`) starts a listener **inside the
  broker**; turning it off, or the broker stopping, closes it and every phone session.
* **One HTTP listener**, hand-written like `preview.rs`, plus **WebSocket** for the frame and
  input stream. This is the one new dependency the design needs: `tokio-tungstenite`
  or a hand-written RFC 6455 server — **the hand-written one was built** (`remote.rs`, with
  `sha1` and `base64` as its only new dependencies). A frame goes out as one binary message
  (`[u32 length][header JSON without the picture][JPEG]`, no base64); input comes in as text
  JSON, the `view_*` tool the window would call. HTTP fallback
  (long poll) is **not** built: a phone browser that cannot do WebSocket is not supported.
* **Backpressure per viewer:** each viewer acks the `seq` it drew; the broker keeps at most
  one frame in flight per viewer and drops older ones for that viewer. Quality and size
  adapt to the round-trip time (quality 80 → 50, scale 2× → 1×), so a mobile network
  degrades the picture, not the latency.
* **Bind address:** `browser.remote.bind`, default `127.0.0.1` (reachable only through a
  tunnel the user sets up, below). A non-loopback address is accepted only if it is in
  Tailscale's `100.64.0.0/10`, or `insecure_lan = true` is set, in which case the settings
  page and the first page the phone gets say, in words, that anyone on that network can see
  what is typed. Orochi never opens a firewall port, edits a route or runs `tailscale` (R10).
* **Reaching it from a phone, in the order Orochi recommends:**
  1. **Tailscale** (the user's own): bind to the machine's tailnet address, or leave
     loopback and run `tailscale serve` (the user's command) for HTTPS with a real
     certificate; WireGuard encrypts the link either way. *Unverified here: nothing in this
     repository runs or tests Tailscale.*
  2. **SSH port forward** from the phone's SSH app, to the loopback listener.
  3. **Same Wi-Fi, `insecure_lan = true`:** cleartext, for a trusted network only.
  There is **no Orochi-operated relay, no account and no cloud hop**: the page content and
  the keystrokes never leave the path the user chose.

### 6.3 Pairing and authentication

* The window shows a **URL carrying a one-time pairing code** in its fragment (128 bits, valid
  5 minutes, one use; **no QR code was built**, the page offers a copy button). The phone opens it, the broker redeems it for a **session cookie**
  (`HttpOnly`, `SameSite=Strict`, `Secure` when the request came over TLS), and the code is
  gone. A paired device has a name and a *Revoke* button; revoking closes its socket.
* Every request and the WebSocket upgrade need that cookie; the upgrade also checks `Origin`
  equals the listener's own origin (no cross-site WebSocket hijack) and rejects a mismatched
  `Host`. Failed redemptions are rate limited; the listener says nothing about what it is
  to an unpaired client.
* **Cookies are the credential, so the token is stored hashed** (SHA-256) in
  `<data>/browser/remote.json` (`0600`); the file holds names and last-seen times, never a
  token, a URL or a page.
* The pairing page is the only unauthenticated one, and it contains no browser content.

### 6.4 Input on a phone

| Gesture | Becomes |
|---|---|
| Tap | `mouse move` + `down` + `up`, one click |
| Double tap | two clicks (`clicks: 2`) |
| One-finger drag | `wheel` (scroll), momentum on the viewer; **text selection is by long-press then drag**, not by plain drag |
| Long press | right button down/up (a context menu), or selection on text |
| Two-finger pinch | `zoom` (page zoom, `adjust_pinch_zoom`), not the viewer's own browser zoom: the viewer's page is `touch-action: none` |
| Two-finger drag | scroll |
| Soft keyboard | the viewer's hidden `<textarea>` takes the keys; composition events become `ime` events exactly as in the window; a toolbar adds what a phone keyboard lacks (Esc, Tab, arrows, Ctrl) |
| Rotate / resize | `resize` to the new viewport |

`touch` events as such (`touch{…}`) are sent to the engine only if the engine has them
(`InputEvent::Touch` exists in Servo's API but its use in a page is **unverified**); until
then gestures are translated to the mouse and wheel events above, so a page that listens
for `touchstart` will not see one.

### 6.5 One tab, two viewers

A tab has **one viewport**, so one **driver** at a time. The driver is whichever viewer
touched it last; the page's `resize` follows the driver. The other viewer **watches**: it
shows the same frames letterboxed, sends no input (its input would fight the driver's), and
offers *Take control* — which makes it the driver and resizes the page to its viewport. The
window shows a small badge "driven from a phone". This is chosen over per-viewer viewports
because one engine process renders one page at one size; two sizes would need two pages.

Mobile emulation (a mobile user-agent and viewport meta handling, so a site serves its
phone layout to the phone) is a `mobile: true` flag on `resize`; whether Servo honours a
mobile viewport is **unverified**, and without it the phone shows the desktop layout at the
phone's width, which is what a narrow desktop window would show.

### 6.6 What the phone does not get

* Not a terminal, not a file, not the chat: only the browser tabs (and the Agents' browser
  watch view). A phone that can read the chat would be a different feature with a different
  threat model.
* Not a way to start the engine: if the tab's process is not running, the phone shows that
  and a *Start* that the broker may refuse by policy.
* Not offline: it is a live view.

## 7. Security notes

* **Same policy for everyone:** R6's check runs in the broker on every `navigate` and every
  `url` event whichever viewer or engine; a phone cannot widen it.
* **The engine process is the least trusted thing here** (it parses hostile pages with a
  0.x engine): no network access beyond what the page opens, no access to `<data>` beyond
  its own profile directory, no inherited environment (the supervisor launch clears it as
  for agents), and **no tool that lets it read the user's files** — a `file:` URL is
  refused by the policy and the engine is started with file access off if the API allows
  it (*unverified*).
* **Credentials typed on a phone** travel to the engine through the broker and the link of
  §6.2; the broker logs no `input` payload and the engine's `console` events are capped and
  never include input.
* **Frames are the page's content**: they are never written to disk or to telemetry, and a
  frame is dropped when no viewer wants it. A screenshot taken by a tool is a file the user
  asked for.
* **The pairing listener is a new attack surface on a machine that had none**, which is why
  it is off, loopback-bound, token-gated, `Origin`-checked and revocable, and why §6.2 does
  not run a relay.
* `THIRD_PARTY` notices carry MPL-2.0 for the `orochi-servo` binary; it is a separate
  executable, so nothing in the CLI's licence changes.

## 8. Tests

No test needs Servo, a phone or a network (`CLAUDE.md`: tests run here, offline).

| Test | Pins |
|---|---|
| `tests/browser.rs` over a **fake engine** (`tests/fixtures/fake_servo.py`, speaking §3.2 on stdin/stdout, as `fake_chrome.py` speaks CDP): each `view_*` tool sends the command it should; a crash event shows "crashed" and a reload starts a fresh process; the policy refuses a navigation and sends a tab that ended up outside to `about:blank` | §3, R5, R6 |
| `tests/browser.rs`: `view_input` in the new shape is mapped to `Input.dispatch*` for Chromium and passed through for the engine protocol | §3.2 |
| `tests/browser_remote.rs` (a Python WebSocket client in `tests/fixtures/`): refused without a cookie, refused with a wrong `Origin`, refused with a spent or expired pairing code, a revoked device is disconnected, the listener refuses a non-loopback, non-tailnet bind without `insecure_lan`, a second viewer watches until it takes control, one frame in flight per viewer, frames are dropped for a slow viewer | §6 |
| `tests/cli_e2e.rs`: `[browser.remote]` and `browser.engine` validate in `Config::validate`; `config show` redacts nothing it should not and prints no token | conventions |
| Desktop fake-DOM tests (`desktop/tests/`): the extracted `browser-view.js` maps touch gestures to the events of §6.4 and IME composition to `ime` | §6.4 |
| `browser-servo/tests/` **by hand, `#[ignore]`**, like `tests/browser_real.rs`: the real engine loads a local page, takes a screenshot, keeps a cookie across two processes, takes a composition | §9 |

## 9. The spike, and what must be true before Servo is the default

The spike (`browser-servo/`, one `main.rs`) loads a URL in a `SoftwareRenderingContext`,
waits for load and writes a screenshot. **Result (`docs/real-validation-20261117.md`):** items
1–3 below are answered for a cold single frame in a release build (it builds, it renders real
pages including CJK text, a few SVG icons are black squares); items 4–7 are **not run**, and
everything in §4 marked "API read" is still only that.

Order of questions, each a go/no-go for the next:

1. **Does it build** with Rust 1.96 on this Mac, and how long and how large? (Servo's own
   minimum is 1.88; `cmake` is not installed here.)
2. **Does it render** a real page and a local dev server's page, and is the screenshot right?
3. **How fast** is a frame: read-back, encode, size, CPU, at 1× and 2×?
4. **Does the user's site work** — the one they log into, end to end, including a redirect
   login. *This decides D5.*
5. **Does IME work** in an `<input>` and a `contenteditable`.
6. **Does a cookie survive** a second process on the same profile.
7. **Gestures and pinch** for §6.4, **`<select>`, dialogs, popups, downloads**.

If 4 or 5 fails, the tab stays on Chromium (`engine = "chromium"`) and this document's §6
(the phone) still stands unchanged, because the viewer and the transport do not depend on
the engine: it is built first for that reason (§10).

## 10. Order of work

1. **Spike** (§9 items 1–3). Cheap, and it decides whether anything below is Servo.
2. ~~Engine-neutral `view_input` and a shared viewer module~~ **Dropped:** the input shape
   was already neutral enough (§3.2), and the phone's viewer is its own small page
   (`src/browser/viewer.html`) that copies the window's key and IME handling rather than
   sharing a module with a 7,000-line `app.js`. The two can drift; that is the price.
3. **The remote listener, pairing and the phone viewer** (§6), over the same `view_*` tools,
   so it works with either engine.
4. **`orochi-servo` and the engine protocol** (§3), fake engine tests first, then the real
   one by hand (§9 items 4–7).
5. **Distribution** (`setup`, pinned builds), only if 4 passes.

## 11. Not decided

* Whether `engine = "servo"` becomes the default, and when (§9 item 4).
* A Windows story: the broker is Unix-only today (`#[cfg(unix)]`); Servo itself builds on
  Windows, the supervisor and the pipes do not yet.
* Agents on Servo (§3.4).
* An encrypted listener of Orochi's own (a self-signed certificate a phone must trust is a
  poor experience; Tailscale's HTTPS is the recommendation until the user wants more).

## 12. What is built, and what was run (2026-11-17)

| Part | State |
|---|---|
| `orochi-servo` (`browser-servo/`: `main.rs`, `input.rs`, `out.rs`, `tools/drive.py`) | **Built and run** against pages served from this machine: frames, mouse, wheel, keys, `text`, IME into an `<input>`, `alert()`, refusal of non-web addresses, a profile that keeps a cookie and `localStorage` across two processes, exit on `shutdown` and on stdin EOF. Numbers in `docs/real-validation-20261117.md`. **Not working:** typing and IME into a `contenteditable`; `<select>`, colour and file pickers and the context menu are dropped; a popup's address is empty; some SVG icons draw as black squares. **Not run:** back/forward/reload, `confirm`/`prompt`, touch, downloads, several tabs per process, a long session, the user's login site, `kill -9` (the profile is written only on a clean exit) |
| `src/browser/engine.rs`, `hub.rs`, `server.rs` (the broker without Chrome, engines, `remote_*` admin tools), `config.rs`, `policy.rs` | **Built**, tested by `tests/browser_engine.rs` (12) over `tests/fixtures/fake_servo.py`, including that nothing is sent before `ready`, that an engine is asked to exit and waited for, and that a restarted tab goes on counting frames; and by `tests/browser_servo_real.rs` (`#[ignore]`, run by hand) against the **real** engine: a page, typing and a click through the hub |
| `src/browser/remote.rs`, `viewer.html`, the window's Settings → *Use from a phone* | **Built**; `tests/browser_remote.rs` (20) covers pairing, cookie and `Origin` checks, tool and scope whitelists, one driver per tab, one frame in flight, revocation, hashed tokens, restart, and the whole chain phone → listener → hub → engine process. **Never run on a real phone**: touch, soft keyboard, IME, safe areas, `visualViewport` and iOS Safari are unverified. Tailscale and `tailscale serve` are unverified. The settings page was tested only against a faked admin answer |
| Agents' tools on Servo (`engine/tools.rs`, `snapshot.js`, the engine's `eval` and `screenshot`) | **Built.** `tests/browser_engine.rs` (3 more, over `fake_servo.py`: refs and actions, no profile and no frames, policy, read-only sessions) and `tests/browser_servo_real.rs` (`#[ignore]`, **run 2026-11-18**: snapshot, type into an `<input>` and a `contenteditable`, a click on a button 1500 px down, Enter, screenshot, a link, wait, back, evaluate). **Not run:** a real site, an agent (a model) driving it, the other tools on a long session |
| `engine = "auto"` as default | Chosen by the user's instruction, **before** item 4 of §9 was answered (their own login site); `engine = "chromium"` is the way back |

Known costs and limits, as the user should hear them:

* **The person's tab and an agent's browser are different engines with different profiles**
  with `engine = "servo"` (§3.4): the person's login cannot be handed to an agent.
* **A `kill -9` of the broker or the engine loses what the profile had not written**: cookies
  and storage reach disk only when the engine exits on its own. The broker asks every engine
  to exit and waits (5 s) on a signal, on idle and on a tab's close.
* **One profile for the whole machine** (`<data>/browser/servo/profile`), not one per project
  as §5 first said; two tabs of the person share cookies. Per-project profiles need the
  broker to know a thread's repository.
* **`contenteditable`** is typed into only because the engine turns `dom_exec_command_enabled`
  on (Servo's own preference, off by default in 0.8.0; measured 2026-11-18 on a plain
  `contenteditable` div, **not** on a real rich-text editor). An IME commit is delivered as
  one composition (start, update, end) because a `contenteditable` inserts every update
  instead of replacing the last.
* **The unix socket is the trust boundary** for `remote_*` and `view_*` alike: any process of
  this user can open it, as it could before this work. An agent's shell can therefore ask the
  broker for a pairing code. A sandboxed agent cannot reach it.
