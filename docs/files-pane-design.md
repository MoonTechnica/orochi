# Files Pane: Browsing the Working Tree from the Window

Written: 2026-09-30. **Status (2026-10-01): implemented; automated tests only.** F0–F3 are built
(§10 says where the build departed from this text). The pane was looked at in a browser through
`desktop/dist/preview.html`, not inside the Tauri window; every claim about cost in §9 is still
unmeasured.

The desktop app shows a conversation and what it changed. It does not show the repository the
conversation is *about*: to read a file an agent mentioned, or to see what is in a directory
before asking for something, the user leaves the window. This design adds a fourth tab to the
side pane — **Files** — with a tree of the thread's working tree, git state on every row, a
read-only text viewer, and search by name or by content.

## 0. Sources

| Source | Used for |
|---|---|
| A screenshot of Orca's file panel over this repository (2026-09-30, the user's window) | The benchmark. Read from it: a header with the project name and *refresh* / *more* actions; a search box (`ファイルを検索`, "search files"); two tabs, `名前` / `内容` (*name* / *content*); a tree with a chevron per directory, an icon per entry, dotfiles listed; git state as colour and a trailing badge — `desktop`, `src`, `tests` in the modified colour with `M`, `target` italic with `⊘` (ignored); files sorted after directories, both case-insensitively |
| [`desktop-app-design.md`](desktop-app-design.md) §1 (R1, R2, R4), §6.1, §6.5; its non-goals | What the pane may and may not be: a non-goal of that design is *an embedded editor* (the app links out to the user's editor), so this is a **viewer** |
| [`desktop-benchmark-orca.md`](desktop-benchmark-orca.md) §2.6 | The list of what not to copy from Orca. A file tree is not on it |
| `desktop/src-tauri/src/view.rs` (`tree_files`, `tree_patch`, `scope_args`) | The precedent: the Changes pane's *Unstaged* / *Staged* / *Branch* scopes already read the working tree live through `git diff`, outside the store |
| `src/context.rs` (`changed_files`, `tree_fingerprint`) | How the core already lists a tree: `git ls-files --cached --others --exclude-standard`, else a bounded `walkdir` skipping `.git`, `node_modules`, `target`, giving up past 50 000 files |
| `src/chat/mod.rs` (`resolve_references`, `attach`) | What `@path` means in a message: an attachment the agent receives as an embedded resource |
| `CLAUDE.md` invariants | *The target repository is never trusted*; *telemetry is content-free; content lives in one file*; patches are stored as diffs, **never file contents** |

## 1. Requirements

| ID | Requirement | Reason |
|---|---|---|
| R1 | **A tree of the thread's `cwd`**, one directory level per request, expanded on demand | A worktree is the thread's own view of the project (`threads.cwd`); a whole-tree walk on open is what `tree_fingerprint` refuses to do past 50 000 files |
| R2 | **Git state on every row**, directories included: modified, added, untracked, deleted, ignored, with a directory carrying the state of what is under it | The screenshot's `M` on `desktop`, `src`, `tests` and `⊘` on `target` is what lets a user see *where* the agent worked without opening anything |
| R3 | **A read-only viewer**: the file's text with line numbers, bounded in size, binary and image files said to be what they are | The non-goal stands: no editing. What the user wants is to *read* what an agent read or wrote |
| R4 | **Search by name and by content**, the screenshot's two tabs, each bounded in results | Finding a file is most of what a tree is for once the repository is bigger than a screen |
| R5 | **Nothing from the tree enters the store.** No row in `activity.sqlite3`, nothing in telemetry; the page keeps only its own view state | The desktop design's R3/R4 and the *content-free telemetry* invariant. Patches are diffs; file contents were deliberately never persisted, and a pane that reads the disk has no reason to start |
| R6 | **Every path is confined to `cwd`.** No `..`, no absolute path, no symlink that resolves outside, never `.git`, and nothing from the repository is executed or opened as configuration | *The target repository is never trusted* — and a window that reads any path it is handed is a window an agent's tool-call `locations` could point anywhere |
| R7 | **It connects to what is already there**: a file mentioned in the timeline opens here; a modified file here opens its diff in Changes; a file here can be put into the next message as `@path`; a line here can become a review comment | Otherwise it is a second file browser beside the one in the user's editor, which is what the non-goal warns against |
| R8 | **Tested without a GUI, without an agent**: Rust tests over a `git init` fixture, page tests over recorded command output | The rule every other pane follows (`desktop/src-tauri/tests`, `desktop/tests/ui.test.mjs`) |
| R9 | **Labels in the page's languages**, the Japanese ones as the benchmark shows them | `app.js` already speaks `WORDS.ja`; the screenshot supplies the exact words |

Non-goals: editing, creating, renaming or deleting files; a terminal; syntax highlighting in
the first release (a monospace `<pre>` reads fine, and a highlighter is the one dependency this
adds that the CSP would have to allow); watching the disk with a file-system watcher (§6.5
says when the pane re-reads instead).

## 2. Decisions (2026-09-30)

| # | Question | Decision |
|---|---|---|
| D1 | Is this a screen over the store, as R1 of the desktop design demands? | **No, and on purpose.** The store holds what was *said and done*; the working tree is the user's own disk. The Changes pane already crosses this line for its git scopes, and this pane extends the same exception by the same road: Rust commands in `view.rs` that run `git` in the thread's `cwd`, never SQL, never a row. `orochi threads` cannot show a file tree, and that is right: a terminal user has `ls` |
| D2 | Where does the viewer go — in the 320 px side pane, in the `#panel` dialog, or in the conversation column? | **In the side pane, in place of the tree**, with a breadcrumb and a back arrow; and the side pane becomes **resizable** by dragging its left edge (width kept in `localStorage`, floor 280 px). Rejected: the `#panel` dialog, because it is modal and a file is something read *beside* a conversation, not instead of it; the conversation column, because a file open there hides the thing the window is for. Resizing also helps Changes' diffs, which were cramped at 320 px |
| D3 | One `git status` for the whole tree, or one per expanded directory? | **One for the whole tree**, `git status --porcelain=v1 -z --ignored=matching --untracked-files=all`, parsed into a map and folded up to every ancestor; refreshed when the pane opens, on the refresh button, and when the feed says a turn in this thread ended (§6.5). Rejected: per directory, because ancestors need the sum anyway. Bounded: past 20 000 status lines the map keeps only the first 20 000 and the header says *partial* |
| D4 | How is a file put into a message? | **As `@path` text in the composer**, the console's own syntax, so the host attaches it the way the terminal does. Checked 2026-10-01: it did not — a queued turn reached `chat::press` as `Key::Queued` and was sent with no attachments, so the agent got the bare words. `resolve_references` and `attach` now work on a `Prompt` rather than the terminal, and a queued turn's text goes through them as typed text does (`a_queued_message_attaches_the_files_it_names`, `tests/cli_e2e.rs`) |
| D5 | Open in the editor: a Tauri plugin or a spawned `open`? | **Spawn `/usr/bin/open` (macOS) / `xdg-open` (Linux) from `view.rs`**, after the path has passed R6. The check is Orochi's rule and belongs where it is tested; `agent_icons` already spawns `sips` the same way. `open` is given a **path**, never a URL |

## 3. Where Orochi stands

| Have | Where |
|---|---|
| A thread knows its working tree | `threads.cwd`; `Client::cwd` |
| Running `git` in it, read-only, from the window | `Client::tree_files` / `tree_patch` via `orochi::context::git` |
| A bounded way to list a tree without git | `context::tree_fingerprint`'s `walkdir` branch |
| A file's diff in every scope, and comments that become a message | Changes pane, `state.comments`, `Client::comment` |
| A side pane with tabs, and a pane that scrolls on its own | `#side`, `#tabs`, `showPane`, `.pane` |
| Per-viewer view state | `remembered` / `remember` over `localStorage` (`open`, `cleared`, `sound`) |
| Drawn icons and a Markdown renderer that emits no markup | `drawn`, `ICONS`, `markdown` |
| Japanese labels | `WORDS.ja`, `t()` |

Missing: listing a directory, reading a file, the status map, search, opening in the editor,
a resizable pane, and the links of R7.

## 4. Layout

```
┌── side pane ──────────────────────┐   ┌── side pane ──────────────────────┐
│ Team │ Changes │ Plan │ Files     │   │ Team │ Changes │ Plan │ Files     │
│ orochi · titabash/ui-ux-review  ⟳ │   │ ← src / chat / term.rs      M  ⤢ │
│ [ Search files…               ]   │   │ 1 208 lines · 41 KiB · +12 −4     │
│ ( Name ) ( Content )              │   │ Open in editor · Reveal · @ Add   │
│ ▸ .claude                         │   │ ──────────────────────────────── │
│ ▸ .github                         │   │   1 │ //! The one screen the     │
│ ▸ desktop                      M  │   │   2 │ //! pinned input and the   │
│ ▸ docs                            │   │   3 │                            │
│ ▾ src                          M  │   │   4 │ use std::io::{self, Write};│
│   ▾ chat                       M  │   │   … │                            │
│       background.rs               │   │                                   │
│       host.rs                     │   │                                   │
│       mod.rs                   M  │   │                                   │
│       term.rs                  M  │   │                                   │
│   ▸ collaboration                 │   │                                   │
│     activity.rs                M  │   │                                   │
│ ▸ target                       ⊘  │   │                                   │
│ ▸ tests                        M  │   │                                   │
│   .gitignore                      │   │                                   │
│   Cargo.lock                      │   │                                   │
│   README.md                       │   │                                   │
└───────────────────────────────────┘   └───────────────────────────────────┘
        the tree                              a file, in the tree's place
```

The **Files** tab joins the three that exist. Its header names the project and the branch
(`thread.project · thread.branch`, as `#thread-where` does) and carries one refresh button.
Below it the search box, the *Name* / *Content* toggle, and the tree. Choosing a file replaces
the tree with the viewer; `←` and Esc bring the tree back where it was, scrolled to that file.
`⤢` widens the side pane to half the window for as long as the file is open.

## 5. Commands

All in `desktop/src-tauri/src/view.rs`, each one call from `main.rs`, none touching
`Activity` beyond `Client::cwd`. Every path in and out is **relative to `cwd`, with `/`**, as
`patches.path` and `FileRow.path` already are.

```rust
/// One directory level. Directories first, then files, each case-insensitively by name; `.git`
/// (directory or worktree file) is never listed. A symlink is listed as one and never expanded.
pub fn tree_list(&self, thread: &str, dir: &str) -> Result<Vec<Entry>>;

pub struct Entry {
    pub name: String,
    pub path: String,            // relative, `/`-separated
    pub kind: Kind,              // Dir | File | Symlink
    pub size: Option<u64>,       // files only
    pub status: Option<Status>,  // this entry's own git state
    pub within: Option<Status>,  // a directory: the strongest state under it (D3)
}

/// One byte of `git status --porcelain`, read as the user reads it.
pub enum Status { Modified, Added, Deleted, Renamed, Untracked, Ignored, Conflict }

/// The whole tree's git state, folded up to every ancestor. Cached in the `Client` with the
/// time it was read; `tree_list` reads it, and `tree_refresh` throws it away.
pub fn tree_refresh(&mut self, thread: &str) -> Result<TreeState>;   // { partial: bool, read_at }

/// A file's text, bounded. `text` is the first `LIMIT` bytes as lossy UTF-8; `binary` is a NUL
/// in the first 8 KiB; `image` is a data URL for png / jpg / gif / webp / svg under 2 MiB,
/// which the CSP's `img-src data:` already allows.
pub fn tree_read(&self, thread: &str, path: &str) -> Result<FileText>;

pub struct FileText {
    pub path: String, pub bytes: u64, pub modified_at: i64,
    pub text: String, pub lines: u64, pub truncated: bool,
    pub binary: bool, pub image: Option<String>,
    pub status: Option<Status>,
}

/// Files by name, or lines by content. Name: `git ls-files -z --cached --others
/// --exclude-standard` (or the `walkdir` listing outside git) filtered as a case-insensitive
/// subsequence, basename matches ranked first, `limit` at most 200. Content: `git grep -n -I -i
/// --untracked -e <query>`, `limit` lines at most 200; outside git the answer is `None` and
/// the page says content search needs a repository. A query under two characters answers empty.
pub fn tree_find(&self, thread: &str, query: &str, by: By, limit: usize) -> Result<Option<Vec<Hit>>>;

pub struct Hit { pub path: String, pub line: Option<u64>, pub text: Option<String> }

/// `open <path>` / `open -R <path>` (macOS), `xdg-open` (Linux). A path only, never a URL.
pub fn open_path(&self, thread: &str, path: &str, reveal: bool) -> Result<()>;
```

**Confinement (R6)** is one function every command goes through:

```rust
/// The absolute path a relative one names inside this thread's tree, or why it is refused:
/// absolute, empty, a `..` component, `.git` anywhere in it, or a symlink that resolves
/// outside `cwd` (both sides canonicalized, then `starts_with`). The canonical path is what
/// is then opened, so the check and the read see one file.
fn within(cwd: &Path, relative: &str) -> Result<PathBuf>;
```

`LIMIT` is 512 KiB, the cap patches already have; past it the head is kept and `truncated` set,
and the viewer says *n KiB more · Open in editor*. Reading is `File::open` on the canonical path,
`take(LIMIT + 1)`, no `read_to_string` on an unbounded file.

The status map (D3): `git status --porcelain=v1 -z --ignored=matching --untracked-files=all`.
`!!` marks ignored (a whole directory comes back as `target/`); `??` untracked; the two
columns `XY` read as the user's status where either is set, `UU`/`AA`/`DD` as conflict. Folding:
a directory's `within` is the strongest of its descendants in the order *conflict > modified >
added > deleted > renamed > untracked*; ignored never folds upward (an ignored `target` does not
mark `.` ignored). The screenshot's `M` on a directory is this fold.

## 6. The page

### 6.1 Tree

One `div.entry` per row: chevron (directories), icon (`drawn("folder")` / `drawn("file")`,
plus `drawn("link")` for a symlink), name, badge. State on the row as `data-status`, coloured as
the screenshot: modified in the brand yellow, untracked in the ok green, deleted struck through
in the danger colour, ignored muted and italic with `⊘`. A directory's badge is its `within`.
Expanding calls `tree_list` for that directory only and remembers the set of expanded paths per
thread (`remember("tree:<thread>")`), so the tree the user left is the tree they come back to,
as the sidebar's projects are. Keyboard: Up / Down move, Right expands or enters, Left collapses
or goes to the parent, Enter opens, `/` focuses the search box.

### 6.2 Viewer

Header: breadcrumb (each segment expands the tree there and goes back to it), the status badge,
`n lines · n KiB`, and for a modified file `+n −m` from the Changes pane's *Unstaged* numbers,
which opens that diff. Actions: **Open in editor**, **Reveal**, **@ Add to message** (D4),
**Copy path**. Body: a `<pre>` with line numbers from a CSS counter, tab size 4, wrapping off
with a toggle; clicking a line number selects the line and offers *Comment*, which goes into the
same `state.comments` list the Changes pane sends as one message (R7 — a review comment on a file
the agent did not touch is still a comment). A Markdown file gets a *Rendered* / *Source*
toggle using the existing `markdown()` renderer. An image is an `<img>`. A binary file is one
line: *binary or not UTF-8 · n KiB · Open in editor*.

### 6.3 Search

The box filters as typed, debounced 150 ms, at most one call in flight; a newer query discards an
older answer. *Name* lists files as rows that open in the viewer; *Content* lists `path:line`
rows with the matched line, opening the viewer scrolled to and highlighting that line. A result
list replaces the tree while the box has text and the tree returns when it is cleared. Outside a
repository the *Content* tab is disabled with the reason as its title.

### 6.4 Links (R7)

| From | To |
|---|---|
| A `tool_call` item's `locations[].path` or `rawInput.file_path` in the timeline | A small file icon on the folded work line; clicking opens Files at that path, line if given |
| A `.file` row in Changes | *Open file* beside the stat opens Files at that path |
| A modified file in Files | Its `+n −m` opens Changes, *Unstaged*, expanded at that file |
| A file in Files | `@path` appended to the composer, with a space |
| A line in Files | A comment in `state.comments` |

### 6.5 When the pane re-reads

No file-system watcher. The tree's status map is read when the pane is shown, on the refresh
button, and when `changed()` names this thread **and** a turn in it has just ended (the same
tick the Changes pane would need); an open file is re-read at those moments too, and if its
`modified_at` moved while a turn was running the viewer shows a *changed on disk · Reload* bar
rather than swapping the text under the reader. Nothing polls git while nothing is happening.

### 6.6 Words

`WORDS.ja`: *Files* → `ファイル`, *Search files…* → `ファイルを検索`, *Name* → `名前`,
*Content* → `内容`, *Open in editor* → `エディタで開く`, *Reveal* → `Finder で表示`,
*Add to message* → `メッセージに追加`, *ignored* → `無視`, *binary* → `バイナリ`,
*changed on disk* → `ディスク上で変更されました`, *Reload* → `再読み込み`.

## 7. Tests

**Rust**, `desktop/src-tauri/tests/files.rs`, over a fixture made with `git init` in a temp
directory (as `views.rs` does for the scopes): a committed file then modified, an untracked
file, a `.gitignore` naming `target/` with a file under it, a nested `src/chat/` with one
modified file, a symlink to a file outside the directory, a symlink to a directory inside it, a
file with a NUL byte, a 600 KiB file, and a `.git` file standing in for a worktree.

- `tree_list(".")` lists directories first, case-insensitively, hides `.git`, lists the
  dotfile, marks `target` ignored and `src` modified by fold, and marks the symlinks as such.
- `tree_list("src/chat")` carries the file's own `M`; `tree_list("src")` carries `within`.
- `within` refuses `/etc/passwd`, `../x`, `src/../../x`, `.git/config`, `link-out`; allows
  `link-in/file` only if it resolves inside.
- `tree_read` on the NUL file says `binary`; on the 600 KiB file returns 512 KiB and
  `truncated`; on a png returns `image` and empty `text`.
- `tree_find` by name ranks a basename match first and stops at `limit`; by content returns
  `path:line`; in a directory that is not a repository name search still lists and content
  search is `None`.
- After a file is modified on disk, `tree_refresh` moves its `status` and its ancestors' `within`.
- The status map for a tree with more than 20 000 lines reports `partial` (fixture generated,
  marked `#[ignore]` if it proves slow to build).

**Page**, `desktop/tests/ui.test.mjs`, with a recorded `desktop/tests/files.json` (the output
of `tree_list` for `.` and `src`, `tree_read` for one text, one Markdown, one binary file, and
`tree_find` for both tabs), added to `make-preview.mjs` so `preview.html` shows the pane:

- the Files tab draws the tree with directories first and the badges the fixture carries;
- expanding a directory asks for that directory only;
- a file opens with numbered lines and a breadcrumb, and `←` returns to the tree expanded as it was;
- a modified file's `+n −m` opens Changes at that file; *Open file* in Changes opens Files;
- a Markdown file offers *Rendered* / *Source* and renders no markup;
- a binary file is one line and no `<pre>`;
- the search box switches between name and content, and a content hit opens the file at the line;
- *@ Add* puts `@src/chat/term.rs ` into the composer;
- a comment on a line lands in the review list beside the Changes comments;
- the tool-call file icon in the timeline opens Files at the path;
- the pane reads its labels in Japanese when the machine does.

## 8. Phases

| Phase | Delivers | Proven by |
|---|---|---|
| **F0 Commands** | `within`, `tree_list`, `tree_refresh`, `tree_read`, `tree_find`, `open_path`; the `main.rs` wrappers | The Rust tests of §7 |
| **F1 Tab, tree, viewer** | The Files tab, the tree with badges, the viewer with line numbers, the resizable pane (D2), the words of §6.6 | The page tests through *binary file* |
| **F2 Search** | Name and content tabs, results replacing the tree | The search tests |
| **F3 Links** | §6.4, the `@path` check of D4, Markdown and image display, §6.5 re-reads | The remaining page tests; `chat/host.rs` read for D4 and the result written into this document |

F0 is worth having alone: the Rust side is what carries the confinement rule, and a later
pane cannot loosen it without failing a test.

## 9. Risks and what is unverified

| Risk | Handling |
|---|---|
| `git status --ignored --untracked-files=all` on a large repository is slow, and it runs on every refresh | Unmeasured. D3's cap bounds the parse, not git; the first implementation measures it on this repository and on one with `node_modules`, and if it costs more than ~200 ms the ignored scan is dropped (`⊘` then only from `check-ignore` on visible rows) |
| A 512 KiB `<pre>` in WebKit | Unmeasured; the cap equals the patch cap the Changes pane already renders. If it scrolls badly, virtualize by line before lowering the cap |
| `@path` in a queued turn is not resolved by the host | D4: checked in F3 before the button ships; the button is hidden until it is |
| Symlink race between `within` and the read | The canonical path is what is opened, so a link swapped after the check points the read at the checked file, not the new target |
| Rendering something the repository wrote — an SVG as an image, a Markdown file with HTML | SVG goes through the `<img>` element, where scripts do not run; `markdown()` already emits no markup (`ui.test.mjs`, "none of it becomes markup"). Nothing else from the tree is rendered as anything but text |
| UTF-16 or Latin-1 files read as binary | Said as *binary or not UTF-8*, with *Open in editor* beside it, rather than guessed at |
| A worktree thread and the main checkout are different trees | The tree is `threads.cwd`, the worktree; the sidebar already says which checkout a thread works in |

## 10. Where the implementation departed (2026-10-01)

| Point | What was built, and why |
|---|---|
| `git` is run with `-c core.fsmonitor=false` (and no untracked cache, no colour) | A repository's own config may name a `core.fsmonitor` command, and `git status` starts it: measured here, a hook the repository named ran on a plain `git status` and did not with the override. The pane reads an untrusted tree, so it never lets the tree run anything (`a_repository_config_cannot_make_the_pane_run_anything`). The Changes pane's `git diff` scopes go through `context::git`, which has no such override; that is outside this change and is noted rather than altered |
| A file deleted from the tree is listed, struck through, and not opened | The status map knows it and the tree would otherwise silently lose it; `Entry::missing` says so |
| `[hidden] { display: none !important; }` added to `app.css` | `#pane-team` sets `display: flex`, which outranks the browser's own rule for `hidden`, so the Team pane stayed on screen under whichever tab was chosen. Found while adding a second flex pane |
| The preview gets `dist/preview-files.js`, generated by `tests/make-preview.mjs` from `tests/files.json` | The preview's stub answers one fixed value per command, and the tree answers per folder. Generating it from the recording keeps what is looked at and what is tested one set of rows |
| `tests/files.json` is compared with what the commands serialize (`the_recorded_pane_output_has_the_fields_the_commands_send`) | The rule `ui.test.mjs` already applies to seats and items: a recorded field the core never sends would let the page read it for ever with nothing failing |
| The side pane's width is `main`'s resizer, not one of this pane's own | D2 asked for a draggable pane; `main` gained one for both panels while this was built ("Let both side panels be dragged…"), so the merge keeps that one. *Widen* sets the pane to half the window through it, bounded so the conversation keeps a readable column, and never stores the widening — closing the file brings back the dragged width |
| The comment bar in the viewer sends the review from there | The review list lives in Changes; a comment left on a file would otherwise wait on a tab nobody is looking at |
