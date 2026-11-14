# Looking at the window without building it

`preview.html` is generated from `index.html` by `../tests/make-preview.mjs`, so the page that
is looked at is the page that ships — a hand-copied one drifts, and a missing element takes the
whole script down with it. It renders the real `app.js` and `app.css` against the recorded view
output in `../tests/*.json`, with `window.__TAURI__` stubbed in `preview-data.js`, and the Files pane's per-folder answers in
`preview-files.js`, which the same script generates from `../tests/files.json`, and the team
channel in `preview-channel.js`, generated from `../tests/channel.json` (a lead, two members, one
continued across two seats, an assignment with its bar, an `@you` mention and an open trial;
`preview.html?thread` opens the first thread panel).

`preview.html?office` draws the home screen, the office, from `tests/office.json` (three floors, one team asking, sub-agents beside their seats).

```sh
node desktop/tests/make-preview.mjs
cd desktop/dist && python3 -m http.server 8731   # then open /preview.html
```

The window's *behavior* is checked by `../tests/ui.test.mjs` and the Rust tests beside it; this
is how its *appearance* is checked.

## Files in conversation messages

Screenshots can be returned as `![Screenshot](artifacts/screenshot.png)` or
`[Screenshot](artifacts/screenshot.png)`. Both show an inline image; clicking the image
opens a larger preview (Close or Escape dismisses it). Relative paths resolve from the
thread's working folder. Absolute paths and `file:///` URLs inside that folder also work;
use `<...>` around paths with spaces, or URL-encode them.

Video and audio links show native playback controls without autoplay. Other file links
open the Files pane, including optional `:line` or `#Lline` positions. Local previews use
a bounded Rust read (16 MiB per file) and reject traversal, `.git`, and symlinks outside
the working folder. Missing, oversized, or unsupported files keep their file action.

HTTPS media and HTTP media on `localhost`, `127.0.0.1`, or `[::1]` can also display.
Use image Markdown for URLs without an image extension. Playback formats depend on the
system webview. File contents are read from disk when shown and are not added to the store.

## The conversation as a team channel

A conversation reads like a Slack channel (`docs/team-channel-design.md`). The body holds the
person's messages, the lead's replies, one **assignment post per member** with a bar under it,
and what members posted to the channel (a reply sent "also to the channel", or a post that is
not in a thread). Everything else a member and the lead say to each other is a **reply in that
member's thread** and is not in the body; joined/left/status/route events and a member's own
working notes are not either, and a completion turn is one line, "reports from <names>", each
name a way into that thread.

The bar under an assignment shows the faces of whoever spoke in the thread, "N replies", "last
reply …" and where the member stands (working and what it is doing, asking, reported, applied,
patch kept, stopped, failed). Where something waits on the person (`needs`: asking, mentioned,
worktree open, trial not applied) the bar is highlighted and says what. Nothing is ever closed:
the header's **Threads (N)** lists the members that are working or have something waiting.

Clicking a bar opens the **thread panel** in the right pane in place of the tabs until × is
pressed. Top to bottom: the assignment, the replies and the member's work (folded, as in the
body) in time order with a divider where a later seat of the same member began, each seat's
last message tagged as its report, the member's worktree and trial as the Work pane draws one
lane, and the reply box ("Reply in thread…", with "also send to the channel"). It follows the
change feed like the body.

Typing `@` in either box lists the lead, the members and `@channel`: working members first,
then the most recently active, `@channel` last, filtered by a prefix of the name, agent or
role. Up/Down move, Tab or Enter take one (inserting `@name `), Esc closes the list and keeps
the text; Cmd-Enter still sends. A member that finished says "done · mention to continue". The
draft stays plain text; a sent post draws the names it mentions as chips. A post naming only
members is a note to them (`say`) and does not wake the lead; naming the lead or `@channel`, or
no one, is a turn for the lead as before.

A mention of `@you` (`mentions()`) produces an OS notification, "<who> mentioned you" with the
first line of the post, once, whether or not the window is in front when the conversation is not
the one being looked at; in the conversation in view and in front it is only marked seen
(`see_mentions`, also after an answer). The dock badge counts open questions plus unseen
mentions, and the sidebar row carries an `@` before its other marks until the mention is seen.

## The office (home)

The window opens on the **Office**, not on a conversation: a project is a floor, a conversation a
room, an agent someone at a desk. Working agents type and say what they are doing in a bubble, a
conversation that asks has its lead raise a hand, a sub-agent its CLI runs (`live_tasks`) is an
assistant beside its seat, and quiet conversations wait in "Off duty". It reads only what the
sidebar and `live_seats` already carry, redraws when that data changes, and only the clocks tick
each second. The Office button in the sidebar returns to it; choosing a conversation leaves it,
and the office does not mark the conversation behind it as read.

## Vendored libraries

The window has no bundler and its CSP names no CDN, so a library it uses is a file here, pinned
to an exact version, with its licence beside it and its SHA-256 recorded. A file changed
by hand fails this list. Read from the npm packages of the same version on 2026-11-14.

| Library | Version | Licence | Used for | Files (SHA-256) |
|---|---|---|---|---|
| xterm.js (`@xterm/xterm`) | 6.0.0 | MIT | the terminal tab | `ba3ea256ce0620a0…` addon-fit.js<br>`b569f629d00f2626…` LICENSE<br>`854a7c0fb70e8b1a…` xterm.css<br>`14903579ff54664c…` xterm.js
| `@xterm/addon-fit` | 0.11.0 | MIT | sizing the terminal to its tab | (above, `addon-fit.js`) |

| pdf.js (`pdfjs-dist`) | 6.5.438 | Apache-2.0 | the PDF viewer, inside `viewer.html`'s sandboxed frame | `3a88402882cfa3e7…` pdf.min.mjs<br>`1e64c8240e54735e…` pdf.worker.min.mjs<br>`0d542e0c8804e39a…` LICENSE |
| docx-preview | 0.4.1 | Apache-2.0 | `.docx`, inside the same frame | `c4a133c65a112799…` docx-preview.min.js<br>`8668bf4417d161e4…` LICENSE |
| JSZip | 3.10.1 | MIT (offered as MIT or GPL-3.0-or-later; taken as MIT) | docx-preview's one dependency | `acc7e41455a80765…` jszip.min.js<br>`566c953c6090b1218…` LICENSE |
| SheetJS Community Edition (`xlsx.mini.min.js`) | 0.20.3 | Apache-2.0 | `.xlsx`, inside the same frame | `0cb353f830d7288385492c83d277b058ddeac664ca51cf1393aa1fd3e2b70939` xlsx.mini.min.js<br>`4d2a38ac35cda06a555c84074a819d413339cd3691b822cae50f8f322fe01f64` LICENSE |
| `@aiden0z/pptx-renderer` (standalone browser build) | 1.4.0 | Apache-2.0; bundles JSZip (MIT), ECharts (Apache-2.0) and **mtx-decompressor 1.4.2 (MPL-2.0, unmodified; notice and licence copy kept in `vendor/pptx-renderer/`)** | `.pptx`, inside the same frame | `eed96db79a8300ef…` aiden0z-pptx-renderer.browser.es.js<br>`c71d239df91726fc…` LICENSE<br>`4e49f2e111bff02e…` THIRD_PARTY_NOTICES.md<br>`1f256ecad192880b…` licenses/mtx-decompressor-MPL-2.0.txt |

SheetJS's newer releases are not on npm (the npm `xlsx` is 0.18.5, which has known security fixes it lacks), so this one was read from `https://cdn.sheetjs.com/xlsx-0.20.3/` on 2026-11-15 and is pinned by hash.

Full hashes are in `dist/vendor/<library>/SHA256SUMS`. Only the two minified builds are kept: the
CMaps (for non-embedded CJK fonts), standard fonts and WebAssembly decoders are not shipped yet.
