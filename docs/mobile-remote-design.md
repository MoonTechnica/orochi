# Mobile remote (design, 2026-11-17)

A phone app that reads and drives the conversations the desktop window shows, over Tailscale.
Activity is in. The Office was first left out and was then asked for ("Office, optimized for mobile"): see "Office on the phone" below.
Screenshots cited in the status sections are local run evidence under `mobile/e2e/screenshots/` and are not committed.

## Shape

```
phone (Expo)  ──HTTP + WebSocket over the tailnet──▶  orochi-desktop (remote server)  ──▶  view::Client ──▶ activity.sqlite3
```

The server lives in the **desktop crate** (`desktop/src-tauri/src/remote.rs`, a library module so
it is tested without a webview) and calls the same `view::Client` the Tauri commands call. It is
**off by default**, enabled in Settings → Orochi mobile app.

## Security (non-negotiable)

- Binds **only** to the machine's Tailscale address (100.64.0.0/10, found with `tailscale ip -4`,
  else the interface scan) — never `0.0.0.0`. No tailnet address → the server does not start and
  says so. `OROCHI_REMOTE_BIND` overrides the address for tests only.
- Traffic is WireGuard-encrypted by Tailscale, so plain HTTP is used; the app declares the
  cleartext exceptions (iOS `NSAllowsArbitraryLoads`, Android `usesCleartextTraffic`).
- Pairing: the desktop makes a **single-use code, valid 5 minutes**, shown as a QR
  `orochi://pair?host=<ip>&port=<port>&code=<code>&name=<machine>&id=<machine id>`. `POST /pair` trades it for a
  per-device **bearer token** (256-bit random). Only the SHA-256 of a token is stored
  (`<data>/remote/devices.json`, 0600). Devices are listed and revocable in Settings → Orochi mobile app.
  Wrong codes are rate-limited (5 failures → the pairing code is burned).
- Every route except `/pair` and `/health` needs `Authorization: Bearer <token>`; `/media` and
  `/file` also take `?token=` because `<Image>`/video players cannot always set headers.
  Constant-time comparison.
- `/rpc/<command>` is a **whitelist**, not a mirror of the Tauri commands. Never exposed: terminal
  (`term_*`), `settings`/`save_settings`, `sandbox*`, `setup*`, `decision_key`, `browser_*`,
  `open_*`, `places`/`set_place`, roster/memory writers, `delete_project`, `notify`, `badge`.
  Files are read through the same bounded, traversal-checked code as the window (`files.rs`).

## Protocol

JSON everywhere; errors are `{ "error": "text" }` with a 4xx/5xx status. Field names are exactly
the serialized `view`/`activity` structs (snake_case) — the TypeScript types in
`mobile/packages/protocol` mirror them, and the recorded fixtures in `desktop/tests/*.json` are the
test data on both sides.

| Route | Meaning |
|---|---|
| `GET /health` | `{ "ok": true, "name": "<machine>", "id": "<machine id>", "version": "…" }` (no auth) |
| `POST /pair` `{code, device}` | `{ token, name, id }`; 401 on a bad/expired/used code |
| `GET /me` | `{ device, paired_at, id, name }`; used to validate a stored token |
| `POST /rpc/<command>` body = the Tauri command's arguments as a JSON object (camelCase as the window passes them) | the command's result as JSON |
| `GET /events` (WebSocket upgrade) | server pushes `{"type":"changed","threads":[ids]}` when the store changes (one shared poller over `Client::changed`), plus `{"type":"hello"}` on connect and `{"type":"prompts"}` when an approval/question opens or closes |
| `GET /media/<thread>/<sha256>` | the `orochi-media` answer (`media::serve`), `Range` honoured |
| `GET /file?thread=<id>&path=<rel>` | a file of the thread's working folder (bounded 16 MiB, `Range` honoured, mime from bytes/extension) — for relative image/video links in messages |

Whitelisted RPC commands (read): `sidebar`, `thread`, `patch`, `room`, `chatter`, `members`,
`mentions`, `team`, `board`, `open_prompts`, `live_seats`, `live_tasks`, `search`, `agents`,
`insights`, `folders`, `git_overview`, `git_show`, `tree_files`, `tree_patch`, `tree_list`,
`tree_read`, `tree_media`, `tree_find`, `watch`, `agent_icons`.
(write): `send`, `say`, `answer`, `interrupt`, `stop_seat`, `seen`, `see_mentions`, `new_thread`,
`ensure_host`, `rename_thread`, `pin_thread`, `archive_thread`, `set_route`, `comment`.

## Desktop side

Tauri commands for Settings → Orochi mobile app (not remotely callable): `remote_status`, `remote_enable`,
`remote_disable`, `remote_pair` (returns `{ svg, code, expires_at, url }`, QR drawn in Rust),
`remote_devices`, `remote_revoke`. Section added to the window's Settings (hand-written JS in
`desktop/src/app.js`, English + Japanese strings, same conventions), covered by
`desktop/tests/ui.test.mjs`.

## Mobile side (`mobile/`, npm workspaces, TypeScript)

```
mobile/
  apps/app/            Expo (SDK current stable), expo-router
  packages/protocol/   types mirroring the structs above + RemoteClient (fetch + WebSocket, reconnect)
  packages/core/       pure logic, no React Native imports: timeline grouping (items → messages),
                       thread/member folding as the desktop does it (docs/team-channel-design.md),
                       markdown media-link resolution, @mention parsing, i18n strings (en, ja)
```

Screens: pairing (QR scan with `expo-camera`, plus paste-a-link fallback) → project/conversation
list (projects, pinned, status marks, unread/needs-you) → conversation (Slack-like: messages,
assignment posts with member thread bars, thread panel, composer with `@` completion, approval
prompts, interrupt) → Activity (live seats and tasks, mentions, chatter, route stats) → Settings
(machine, unpair, language). **Media in the chat is a first-class feature**: images inline with
pinch-zoom full-screen viewer, save/share; video and audio with native players; other files open
a file viewer (text/code/diff). Office documents (docx/xlsx/pptx) are out of scope. The token
lives in `expo-secure-store`. Mobile layout is its own design (bottom tabs, large touch targets),
not a shrunk desktop window.

## Status (desktop side, 2026-11-17)

Implemented, with automated tests only (`desktop/src-tauri/tests/remote.rs`, driving the server on
loopback through `OROCHI_REMOTE_BIND`; `desktop/tests/ui.test.mjs` for the Settings page):
`desktop/src-tauri/src/remote.rs` (axum; pairing with single-use 5-minute codes burned after 5
wrong tries; SHA-256 device tokens in `<data>/remote/devices.json` at 0600 compared in constant
time; the `/rpc` whitelist; `/events` over one shared poller; `/media` and `/file` with `Range`;
`/health`, `/me`), the `remote_*` Tauri commands, and Settings → Orochi mobile app (English and Japanese).

Decisions the design left open:

- The poller reads the change feed through a connection and cursor **of its own**; the `/rpc` calls
  share the window's `Mutex<Client>`. Sharing the window's cursor would take the changes the window
  is waiting for.
- `new_thread` over `/rpc` is refused for a folder that is not already one of the desktop's
  projects (a phone adds one with `add_project` / `clone_project`, below). `delete_thread`,
  `open_*` and everything else on the never-list are not whitelisted.
- The on/off flag is the file `<data>/remote/enabled`; the port is 47821, or any free port on the
  same address when that one is taken (the QR carries whatever was bound).
- `/file` serves what `files::within` allows (no `..`, no absolute path, no `.git`, no symlink out),
  up to 16 MiB, with `nosniff` and a sandboxing CSP.
- The Settings page is built by `app.js` alone; `index.html` needed no change.

Added 2026-11-17 (same day, second pass): the headless `orochi-remote` binary and the seeded demo
store (section below), "Copy link" and the host/port/code as plain text in Settings → Orochi mobile app
(`Pairing` now also carries `host`, `port`, `name`; Japanese strings and `ui.test.mjs` updated), and
`tests/remote.rs` starts the real binary on `127.0.0.1:0` with demo data, pairs with the code it
prints, reads the sidebar, room, members and thread over `/rpc`, fetches media and a folder file
with `Range`, and issues a second code through stdin. Not built: `--keep-pairing` is only
exercised by hand; no "send to phone" form (not needed).

Not verified: no real tailnet address, no real phone, no run inside the packaged window. The
Tailscale lookup (`tailscale ip -4`, the macOS app bundle's binary, then an interface scan) has not
been exercised against a real Tailscale install, and Windows is not compiled (the interface scan is
`cfg(unix)`; the hand-run CI would have to check it).

## Headless server and demo data (2026-11-17)

`orochi-remote` is a second binary of the desktop crate (`desktop/src-tauri/src/bin/orochi-remote.rs`,
links the library only, no webview). It is the way to use the phone with no window.

```sh
cd desktop/src-tauri && cargo build --locked --bin orochi-remote   # -> target/debug/orochi-remote

# the real data directory, on the Tailscale address, with a pairing QR on the terminal
orochi-remote --pair
# a simulator or a test: loopback, a seeded store, one JSON line {"url","host","port","code","name","expires_at","base"}
orochi-remote --demo-data /tmp/orochi-demo --bind 127.0.0.1:47821 --json
```

Options: `--data DIR`, `--bind ADDR[:PORT]` (same as `OROCHI_REMOTE_BIND`; port 0 picks one),
`--pair` (print the link, the host/port/code as text and the QR), `--keep-pairing` (a fresh code
whenever the last was used or ran out; typing `pair` + Enter on stdin issues one any time),
`--json` (one JSON object per pairing on stdout, nothing else; logs go to stderr), `--no-qr`,
`--demo-data DIR`, `--seed-only` (write the store, print where, exit). The iOS simulator reaches
the Mac at `127.0.0.1`, so the pairing link above works in it as printed; read `base` for the
HTTP address.

`--demo-data DIR` writes `DIR/demo-repo` (png/jpg/mp4/m4a/txt/diff/md files, a git repository)
and `DIR/activity.sqlite3` through the real `orochi::activity` calls, then serves it. Threads:
`Show me what you made` (png, jpg, mp4, m4a items), `Write up with files` (markdown with
relative image/video/audio/file links into the folder, a tool call with a diff), `Split the work`
(a team channel: members alice and bob, two assignments, replies, an `@you` mention and an
`@channel` post), `Run the tests` (an open approval), `Refactor the parser` (a working seat).
The media are real, tiny files (embedded from `desktop/src-tauri/assets/demo/`, made with
ffmpeg: 6 s mp4 with sound, 6 s AAC m4a, png, jpg). Live seats are hosted by the process that
seeded them, so they read as live only while `orochi-remote --demo-data` keeps running; with
`--seed-only` the approval and the team's mention remain but nothing is live.

## Several desktops on one phone (2026-11-17)

Each desktop has a stable random `machine_id` (16 bytes hex, `<data>/remote/machine.json`, 0600,
made once). It is in `/health`, `/pair`, `/me`, the pairing link (`&id=`), `Status.id` /
`Pairing.id`, the `orochi-remote --json` line, and Settings → Orochi mobile app ("Machine id"). The phone
keeps a list of machines and matches a pairing by that id, or by host:port when either side has none.

Implemented and tested: the server side (`tests/remote.rs`, the machine id test; fmt, clippy
`-D warnings` and the whole crate's tests pass) and the phone's machine store, switcher, Home
dashboard, per-machine scoping, pairing dedup and re-pair, the right-hand drawer, run on the web
build against two real `orochi-remote` instances (see `mobile/README.md`).

Not verified: iOS/Android, gestures, notifications naming the desktop, two real tailnet hosts.
Decisions: only the active desktop has a live socket (the rest are polled every 20 s in the
foreground). Re-pairing replaced nothing at first; see "Device store and name" below.

## iOS simulator verification (2026-11-17)

The development build was run on the iPhone 17 simulator (iOS 27.2) against two real
`orochi-remote --demo-data` instances on loopback, driven by maestro (`mobile/e2e/run-ios.sh`,
`mobile/e2e/flows/`, 51 screenshots in `mobile/e2e/screenshots/ios-*`). Verified there: pairing by
deep link, cleartext HTTP to 127.0.0.1, secure-store persistence across relaunch, Home, the
conversation, inline and full-screen media, audio and video playback (H.264/AAC MP4 only), the
drawer gestures, files, git, team channel and thread panel, approvals, the composer with the
keyboard, activity, rename/pin/archive, two desktops with the switcher, revocation, pairing again,
removal and the notification permission prompt. No server change was needed; the demo mp4/m4a were
lengthened to 6 s so playback can be observed. Still unverified: camera scanning, a physical
device, a real tailnet, Android, notification delivery and anything in the background, the photo
library and share sheet, pinch zoom. Details and the fixes are in `mobile/README.md`.

## Office on the phone (2026-11-17)

No server change: the Office reads `sidebar`, `live_seats`, `live_tasks`, `chatter` and `agent_icons`, all
already whitelisted, and Stop seat is `stop_seat`. The rules are the desktop's (`app.js`, "Office ---"), ported to
`mobile/packages/core/src/office.ts` and tested on `desktop/tests/office.json`: a room per conversation with a
busy status or a live member, floors by project ordered by what needs the person, the org chart under the lead,
per-person state (working / asking / mentioned / reported / idle / leaving), sub-agents beside their seat, the
arrival / leaving / done / closing tracking (`stepOffice`). The layout is its own: rooms stack vertically, people
flow in a grid, and the cross-room wires are replaced by message lines inside the rooms (an intentional
simplification). Verified on the web build and on the iPhone 17 simulator; see `mobile/README.md` for what is not.

### Office at scale (2026-11-18)

The first Office drew every room and person at once and was only seen with demo data (3 rooms, 8 people). It now
scales to 100+ people and 30+ rooms: a per-room cap with a "+N" tile and a list sheet, one-line compact rows opened as an
accordion, a flat virtualized list, motion only for what is on screen (paused when crowded, backgrounded or reduced),
and All / Needs you / Working filters shared with the Strip tiles. The rules are pure functions in `packages/core/src/officeScale.ts`
with named thresholds; the screen only draws them. Decisions: the drawn people keep the room's order and membership is
chosen by state, so a state change moves one person in or out and shuffles nobody; compaction is by the unfiltered floor
size so narrowing a filter does not redraw the rooms that stay; a one-line row's faces never animate and do not count
toward the 24-person limit. Measured and not measured: see the README ("Verified here, and not").

## Device store and name (2026-11-17, third pass)

`devices.json` is the source of truth. The server re-reads it when its `(modified, length)` changes
(checked on every authenticated request, the socket's revocation tick, `devices()`, `status()`,
`revoke` and `/pair`), so a row removed or the file emptied or deleted while the server runs revokes
that phone on its next request; a file that does not parse changes nothing; the server's own writes
are remembered and do not reload. `/pair` takes an optional `device_id` (8-64 characters of
`[A-Za-z0-9-]`, otherwise ignored), kept in the row: pairing again with the same one replaces that
phone's old row, and so its old token. Without an id nothing is guessed from the name (two phones
called "iPhone" must not replace each other). The app makes one id per install (`orochi.install` in
the secure store). Tests: `tests/remote.rs` (`the_device_file_is_the_truth...`, `pairing_again_from_the_same_install...`).

The name a phone shows is now `machine.json`'s `name` (Settings → Orochi mobile app → "Name shown on the phone",
`remote_set_name`, at most 40 characters, empty = default), else the host name **without its domain**
(`mac.example.net` -> `mac`). `/health`, `/me`, `/pair`, the pairing link and `Status` carry it
(`Status.default_name` is the placeholder). English and Japanese strings and `ui.test.mjs` updated.

On the phone: Home shows the needs-you cards first; Reply focuses the composer addressed to the
sender; Dismiss hides one mention and calls `see_mentions` (per thread) only when none of that thread
is left; the nest bars are **conversations touched** (`updated_at`), because no RPC gives a history of
agent activity (`live_seats`/`live_tasks` are only what is live now) and the caption and the
accessibility label say so; a desktop that needs pairing again no longer replaces the tab bar; text
colours are checked for WCAG AA in both modes; double tap zooms the image viewer. Not verified:
physical device, a real tailnet, Android, camera scanning, notification delivery, pinch on a screen.

## Look of the phone app (2026-11-17)

The phone's UI follows its **icon** (black, white serpent, red eyes), not the desktop window, which
keeps its amber on purpose: the two apps share the data, not the palette. Near black and white
with greys; the main action and what waits for the user are the **inversion** (white fill with
black text in dark, black fill with white text in light, a thick border and a halo for what waits),
and **red (`#e0160e`, the eyes' red) means error, failure or a destructive action only**. A desktop
that must pair again is not an error: white border and an inverted Re-pair button. The brand red
exists as a token for a few pixels but is unused, since a red spot read as an error. All colour is
in `mobile/apps/app/src/ui/theme.ts`; `apps/app/test/theme.test.ts` checks the palette, where the
red tokens may be read, that there are no hex colours in screens and 4.5:1 for text.

Exception (2026-11-17): the Office's people and their provider badge are drawn in colour (the phone's own minimal, flat, startup-style vector people, not the desktop's busts; vendor colour or the `agent_icons` image on the badge). It is confined to `apps/app/src/ui/office/avatarPalette.ts`, each avatar sits inside a solid white (light: black) rim, and the ring states, bubbles and text stay monochrome, so red still means error.

## Faces in the conversation (2026-11-18)

The conversation side drew a letter in a circle for every speaker. It now draws the Office's characters, so one agent is one face
on both sides of the app: the key is the Office's (`seedOf({ role, agent })`, with the lead's kind from the thread's lead role),
reused from `packages/core`, not copied. Decisions: (1) **you** are a neutral character from the same set chosen by a per-install
seed (not an initial: a face keeps the list readable and the seed makes two phones differ), told from agents by a gapped ring and the
absence of the provider badge, never by colour; (2) message faces are static (`Character still`, no animation hooks at all) and
memoized, and only a thread bar of a working member and the thread panel head use the Office's moving `Avatar`, which stops under
reduced motion; (3) the expression comes from the member's state through `personStateOf` and `faceOf`, messages use the smile;
(4) places where the name is the point (chatter, usage, the prompts of Home, Settings, Files, Git) stay text; (5) no new colour
literal and no change to the palette test's allow-lists. A mention names a role, not an agent, so `NeedItem.from` is the thread's
agent for that role, filled by `buildHome`. Measurements and the list of places: `mobile/README.md`, "Faces in the conversation".

## Threads on the phone (2026-11-18)

The conversation's member threads follow Slack's thread design within the existing model: a footer under the assignment post (faces,
"N replies", last reply, unread, "Needs you") instead of a bordered bar, a thread screen with the parent, a reply count and a sticky
composer with an "Also send to the channel" checkbox, a Threads list (pill, drawer entry), day dividers, long-press actions and the
"Replied to a thread" caption for `broadcast` rows. Unread is local to the phone (per machine, conversation and member). No server or
whitelist change: `say` (with `member`), `send`, `see_mentions` as before. Starting a thread on an arbitrary message is deliberately not
offered, the server having no such operation. Details and what was verified: `mobile/README.md`, "Threads like Slack's".

## New projects from the phone (2026-11-18)

The phone's "New conversation" sheet can add a project the desktop does not have yet, by two
whitelisted writes and one read, and then starts the first thread in it:

- `browse {path?}` lists the visible sub-folders of `path` (the clone folder, else home, when
  none is given; marks git checkouts; names the parent only while it is still allowed).
- `add_project {root}` registers an existing folder (`view::Client::add_project`).
- `clone_project {url, name?}` runs `git clone` into the **clone folder** and registers it.

Where a phone may reach is fixed by the desktop, never by the phone: `browse` and `add_project`
accept only a folder inside the user's home or the clone folder (symlinks resolved first), and
`clone_project` writes only a new folder directly in the clone folder. Settings → Orochi mobile
app has the **clone folder** row (system folder picker, Clear); it lives in
`<data>/remote/projects.json`, and until it is chosen a phone cannot clone at all
(`remote_set_clone_dir` is a window command, not on the whitelist).

A clone address is checked before git sees it: `https://`, `ssh://` or `user@host:path` only, no
whitespace or control characters, nothing that begins with `-`, a host that begins alphanumerically
(no `file://`, local path or `ext::` transport; `protocol.ext.allow=never` as well). The folder
name is the address's last segment (or `name`), letters, digits, `.`, `_`, `-` only, and a name
already taken is refused. Git runs with `GIT_TERMINAL_PROMPT=0`, no stdin and (unless the user set
`GIT_SSH_COMMAND`) `ssh -o BatchMode=yes`, so a repository that needs a password fails with git's
message rather than hanging; it is stopped after 15 minutes and a failed clone removes what it made.
The clone does not hold the window's connection (other phone calls go on), and the phone waits up
to 16 minutes for it. Credentials come from what the desktop's git already has (SSH keys, a
credential helper); the phone never sends any.

Tests: `desktop/src-tauri/tests/remote.rs` (browse and add inside and outside the allowed places;
a clone through a local repository standing in for a host, and every refused address) and
`desktop/tests/ui.test.mjs` (the Settings row). Not run: a real phone, a clone from a real host.
