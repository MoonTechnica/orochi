# Looking at the window without building it

`preview.html` is generated from `index.html` by `../tests/make-preview.mjs`, so the page that
is looked at is the page that ships — a hand-copied one drifts, and a missing element takes the
whole script down with it. It renders the real `app.js` and `app.css` against the recorded view
output in `../tests/*.json`, with `window.__TAURI__` stubbed in `preview-data.js`, and the Files pane's per-folder answers in
`preview-files.js`, which the same script generates from `../tests/files.json`, and the team
channel in `preview-channel.js`, generated from `../tests/channel.json` (a lead, two members, one
continued across two seats, an assignment with its bar, an `@you` mention and an open trial;
`preview.html?thread` opens the first thread panel).

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
