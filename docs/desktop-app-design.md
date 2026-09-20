# Desktop App Design: A Persistent Activity Store and a Desktop Client That Is a View Over It

Written: 2026-09-20.

**Status (as of 2026-09-20)**

| Phase | Status |
|---|---|
| P0 Store | **Implemented; automated tests only** (mock agents). `activity.sqlite3`, the recorder, the views, `orochi threads`, the telemetry changes of §5. The console, `orochi run`, `collaborate` and `serve` all write rows |
| P1 Control | **Implemented; automated tests only.** Prompts and controls through the store, `orochi host`, `chat --continue`, the mailbox move, the failed-check tail |
| P2 App | **Implemented; automated tests only.** A Tauri 2 window: sidebar, thread, composer with a folder picker, approval cards, Team and Changes panes |
| P3 Supervising | **Implemented; automated tests only.** Mission control, Agents, Insights (over the telemetry views of §5), the work-graph board and Settings |
| P4 The user in the room | **Implemented and validated against a real CLI** ([2026-09-20](real-validation-20260920.md)). Claude Sonnet read a note left mid-turn and acted on it; Haiku, told plainly to, did not. The mechanism is sound; whether it is read depends on the model, so "delivered when the agent next checks" stays the wording |

The window's **appearance** was checked on 2026-09-20 by rendering the same `app.js` and
`app.css` in a browser against the recorded view output (`desktop/dist/preview.html`), which
found four things and fixed them: tables stretched to the window's width, form controls left
in the browser's own style, a heading that named the conversation while showing a screen, and
`---`/`+++` diff lines coloured as changes. It has not been looked at inside the Tauri window
itself, which uses the same WebKit but its own chrome.

Still out of scope, as §6.5 says: staging, reverting, committing and opening a pull request.
The Changes pane reads the tree; it does not drive git.

Behavior against real agent CLIs was validated on [2026-09-20](real-validation-20260920.md): a
real turn on Claude, a real rate limit on Codex, and P4's mid-turn note. That run found one
defect (patch paths arrived absolute) and confirmed that the ACP fields `acp.rs` had been
dropping are ones real agents actually send. The automated tests still use the fixture agent
only, because a test must never spend the user's quota. The measurements §2 and §9 call for
have been taken and are asserted as bounds in `tests/activity.rs`.

The four open points were decided on 2026-09-20 (§10). The invariant text in `CLAUDE.md`, `README.md` and `docs/agent-mailbox.md` is updated as part of P0/P1.

## 0. Goal and starting point

Two goals, in this order:

1. **Revisit the SQLite design** so that what Orochi does — conversations, turns, the agents seated on a turn, their tool calls and diffs, the messages agents send each other, permission questions, collaboration waves — exists as rows, not only as terminal output and process memory.
2. **A desktop app** whose interaction model follows the Claude Code tab of Claude Desktop and the Codex app (project sidebar, thread timeline, composer, review pane), plus what only Orochi has: several agents on one turn, visible as a **group chat**, and routing that explains itself. The app should be close to *"render the views in SQLite"*: the core writes, the app reads, and the small amount the app writes is rows too.

Facts found by surveying the code as of 2026-09-20:

| # | Fact | Basis |
|---|---|---|
| 1 | `telemetry.sqlite3` holds `runs`, `runtime`, `sessions`, `quota_snapshots`, `classifications`, `metadata`. Every table is a key plus a JSON `record` blob; `user_version = 2`; a newer version makes `Store::open` refuse the file | `storage.rs:45-68` |
| 2 | The learning queries filter on `json_extract(record, …)` for purpose, language, framework, complexity, reasoning, mode and provider. Only `(agent, model, task_type, started_at)` is indexed, so `advice_cost` (by purpose alone) scans the table | `storage.rs` `advice_cost`, `learning_runs`, `tier_runs`, `pooled_runs` |
| 3 | `INSERT INTO runs VALUES (?1,…,?8)` names no columns, so any ordinary `ADD COLUMN` breaks every older binary that shares the file | `storage.rs:96` |
| 4 | Tests read the raw bytes of `telemetry.sqlite3` and assert that task text is absent | `tests/core.rs:353,776`, `tests/mailbox.rs:363,801,936` |
| 5 | `mailbox.sqlite3` holds `peers` and `messages`. A peer row is **deleted** when its session ends or its owner dies; messages are deleted after `mailbox.retention_secs` (default 1 day). Neither links to a run, a turn or a conversation | `mailbox.rs:82-127,223` |
| 6 | The conversation exists only in memory: `chat::Conversation::turns` (bounded), `Session::said`, `Session::queue`. Quitting the console loses it; an agent without `session/load` cannot be continued after a restart | `chat/mod.rs:290-312,2277-2300` |
| 7 | Everything a UI needs already flows through one channel: `acp::ExecutionEvent` (`Text`, `Permission(request, oneshot)`, `Finished`, `Progress`) with `Progress::{Thinking, Tool, Plan, Route, Unavailable, Note, Checking, Attempt}`. The terminal console is its only consumer | `acp.rs:118-176`, `scheduler/mod.rs:275-690` |
| 8 | `acp::progress` drops part of what ACP sends: tool `kind`, `locations`, `rawOutput`, `available_commands_update`, `current_mode_update` | `acp.rs:208-259` |
| 9 | A permission question is answered through a `oneshot` inside the process that asked it. Nothing outside that process can answer | `acp.rs:459-490` |
| 10 | One process serves one repository and one conversation: the interrupt latch is process-wide (`interrupt.rs`), and the mailbox membership is a process-wide `OnceLock` bound to one channel (`mailbox.rs:411`) | `interrupt.rs:9-20`, `mailbox.rs:399-411` |
| 11 | `collaborate` persists `report.json` — task text, every response, the plan, merge records — as its resumable state, and reports progress with `eprintln!`, not events | `collaboration/mod.rs:350-365` |
| 12 | `telemetry.sqlite3` is created `0644` inside a `0700` directory; `mailbox.sqlite3` is set to `0600` | `storage.rs:40-45`, `mailbox.rs:73-77`, observed in `~/.local/share/orochi/` |
| 13 | The repository is known to telemetry only as a salted hash. No table holds a path or a name a sidebar could show | `storage.rs:88-93` |

So the event stream a desktop app needs exists (fact 7); what is missing is (a) a place where it lands as rows, (b) identities that tie a message, a tool call and a mailbox message to the same turn, and (c) a way for a second process to answer a question or send the next message.

## 1. Requirements

| ID | Requirement | Reason |
|---|---|---|
| R1 | **The store is the interface.** Everything the app shows is a `SELECT` from a named view; everything it asks for is a row the core acts on | The stated goal; also makes every screen testable without a GUI |
| R2 | **The terminal and the desktop are equals.** A thread started in a terminal appears in the app while it runs; a permission question can be answered from either; a thread can be continued from either | One product, two surfaces. Falls out of R1 if the terminal console writes the same rows |
| R3 | **Telemetry stays content-free.** `telemetry.sqlite3` keeps every guarantee it has today, byte-level tests included | Fact 4; it is the file a user might share for debugging or benchmarking |
| R4 | **Content is kept on purpose, bounded, and deletable.** Separate file, `0600`, retention, real deletion, an off switch | Replaces "never persisted" with something a user can still reason about (§3) |
| R5 | **Old and new binaries can share the data directory.** The desktop app bundles an `orochi`; the user may have another on `PATH` | Facts 1 and 3: today a version bump locks the older binary out |
| R6 | **No new calls to any agent.** Titles, summaries and statuses come from what already flows | Cost, and the rule that nothing without a choice to make starts an agent |
| R7 | **Closing the window does not stop the work.** | Parity with Claude Code desktop and Codex background threads; also crash isolation |
| R8 | **Lossless enough to re-render.** A thread read back from the store renders the same timeline the live one did, including diffs and tool details | Otherwise the app needs a second, live-only data path and R1 is false |
| R9 | **Key and approval behavior follows Claude Code**, as the terminal console already does | Existing project convention |

Non-goals: cloud sync, multi-user, remote agents, a plugin system, an embedded editor or terminal emulator (the app links out to the user's editor and terminal), Windows in the first release (the supervisor and interrupts are Unix-only today).

## 2. Architecture: SQLite is the bus

```
┌────────────── Desktop app (Tauri 2) ──────────────┐
│  WebView UI  ── invoke ──▶  Rust shell            │
│  (renders v_* views)        links the orochi lib  │
└───────────────┬───────────────────────┬───────────┘
        SELECT v_*│            INSERT turn / answer / control
                 ▼                       ▼
        ┌──────────────── <data>/ ────────────────┐
        │ activity.sqlite3   threads, turns, seats, items,     │
        │                    patches, prompts, controls,       │
        │                    peers, messages  (content, 0600)  │
        │ telemetry.sqlite3  runs, runtime, quota (no content) │
        └───────▲──────────────────────▲──────────┘
      write rows │ poll controls        │ write rows
   ┌─────────────┴────────┐   ┌─────────┴──────────────┐
   │ orochi host --thread │   │ orochi (terminal chat, │
   │ (headless, one per   │   │  run, collaborate,     │
   │  active thread)      │   │  serve)                │
   └──────────┬───────────┘   └─────────┬──────────────┘
              └──── ACP stdio ──▶ agents ◀──┘
```

Three decisions:

**D1. Hosts write, the app reads, and the app's requests are rows.** Every process that runs agents (a *host*) tees `ExecutionEvent` into `activity.sqlite3`. The app never talks to a host directly — no socket, no second protocol. It inserts a queued turn, answers a prompt row, or inserts a control row; the host that owns the thread picks it up on its next poll. This is the pattern the mailbox already uses between processes, and it is what makes R2 free: a terminal console that polls the same tables is controllable from the app with no extra code path.

Rejected: *the app as an ACP client of `orochi serve`*. It gives streaming for free, but the app would then see only threads it started, the store would be a second copy of what the socket said, and the group chat, seats and routing have no ACP representation. Rejected: *running the scheduler inside the app process*. Fact 10 makes one process one repository and one conversation; lifting that is a refactor of `interrupt.rs` and `mailbox.rs` that buys nothing R7 does not already forbid.

**D2. One headless host per active thread** (`orochi host --thread <id>`, new). It is `chat::Session` with the terminal replaced by the store: it claims queued turns, runs them through the same `steps` / seats / phases logic, and exits after an idle period (default 10 min; the next queued turn makes the app start another). The existing supervisor leases (`process.rs`) already tie agents to their owner, so a host that dies takes its agents with it and a host that outlives the app window keeps working (R7).

**D3. The app's Rust side links the `orochi` library** for one reason: row types and SQL live in one crate. The WebView calls typed commands (`threads()`, `timeline(thread, after)`, `send(thread, text)`), never SQL. Tauri over Electron because the data layer is already Rust (`rusqlite`, bundled SQLite, `types.rs`); over SwiftUI because CI already covers Linux and macOS. The repository becomes a Cargo workspace: `orochi` stays at the root, `desktop/src-tauri` depends on it by path.

Latency budget: hosts flush streamed text at most every 80 ms (one transaction per flush); the app checks `PRAGMA data_version` every 200 ms and reads only what the change feed (§4.9) names. Worst case from token to pixel is therefore under 300 ms, which is about what a terminal reader perceives today.

**Measured** (2026-09-20, this machine, `tests/activity.rs`): a flush costs **157 µs**, so six seats streaming at once spend about 1.2 % of a second writing. A quiet poll — the `data_version` header read plus an empty feed query — costs **4.7 µs**, so polling at 5 Hz is free. A deliberately heavy turn (40 tool calls with their output and patches, plus 20 KiB of reply) leaves a **256 KiB** file, so the 30-day window is tens of MB for ordinary use rather than the hundreds §9 was guarding against. The tests assert bounds an order of magnitude above these, so they fail if the store ever becomes slow enough to feel.

## 3. The invariant this changes

Today: *"Task text, conversation, source, diffs, agent stderr and check output never reach SQLite … history and transcripts stay in memory only."* A desktop app that shows a conversation after a restart cannot coexist with that sentence. The proposal narrows it instead of dropping it:

| Stays exactly as is | Changes |
|---|---|
| `telemetry.sqlite3` never holds task text, conversation, source, diffs, stderr or check output. Existing byte-level tests stay untouched | A new file, `activity.sqlite3`, holds the conversation: user messages, agent replies, thinking, tool details, patches, mailbox messages |
| Router / judge / council payloads: attributes and ≤ 12 opaque candidates, nothing else | — |
| The classifier is the only adviser that sees task text, and caches only a salted hash | — |
| Repository identity in telemetry is a salted hash | `activity.sqlite3` stores the repository **path** and name, because a sidebar has to show them |
| Memory is written only by Orochi, from what the user said | Unchanged. The end-of-session distillation keeps reading `Session::said`, not the store |
| Nothing is transmitted | Unchanged. No adviser, agent or network call ever reads `activity.sqlite3`; the one exception is replaying a thread's **own** earlier turns to its **own** next agent, which the in-memory transcript already does |

Bounds on the new file (R4): mode `0600`; `PRAGMA secure_delete=ON` on every connection; `[activity] retention_days = 30` (0 keeps until deleted; pinned threads are exempt); deleting a thread deletes every row and attachment under it; `[activity] enabled = false` restores today's behavior exactly (the terminal works as now, the app shows telemetry and live peers only). Stored text is bounded per item (§4.4), and patches are stored as unified diffs, not file contents.

`report.json` already persists task text and responses (fact 11), so the narrowing is smaller than it reads: the content was on disk for `collaborate` all along.

Tests this adds: with `activity.enabled = false`, no file in `<data>` contains the task text; with it enabled, `telemetry.sqlite3` still does not; a deleted thread leaves no recoverable text in the file (`secure_delete`, checked on raw bytes after `VACUUM`-free deletion); retention removes unpinned threads only.

## 4. `activity.sqlite3`

Conventions: WAL, `foreign_keys=ON`, `secure_delete=ON`, `busy_timeout` 5 s, `synchronous=NORMAL`. IDs are UUIDs unless stated. **Timestamps are Unix milliseconds** (telemetry stays in seconds; a timeline needs sub-second order and elapsed time). JSON columns hold what is displayed whole and never filtered on. Every `INSERT` names its columns (the lesson of fact 3).

The hierarchy, with the code it mirrors:

```
project ─┬─ thread ─┬─ turn ─┬─ seat ─── attempt ─┬─ item ─── patch
         │          │        │                    └─ prompt
         │          │        └─ part (work graph)
         │          └─ control
         └─ peer ─── message            (the mailbox, moved here)
```

| Row | Is | Mirrors |
|---|---|---|
| project | one git repository (all its worktrees), or a plain directory | `mailbox::channel` |
| thread | a conversation: a console session until `/new`, one `orochi run`, one `collaborate`, one `serve` session | `chat::Session` + `Conversation` |
| turn | one user message and everything it caused | `chat::Message` → `Session::steps` |
| seat | a place at the table for that turn: the lead, a read-only perspective, a phase, a collaboration participant | `roles::seats`, `PHASES`, `collaboration::Participant` |
| attempt | one agent session trying to fill a seat; a failover is the next attempt | one pass of the `scheduler` retry loop; `collaboration::SessionResult` |
| item | one thing on the timeline | `ExecutionEvent` / ACP `session/update` |

### 4.1 Projects, threads, hosts

```sql
CREATE TABLE projects (
  id TEXT PRIMARY KEY,               -- mailbox channel: salted hash of the git common dir
  root TEXT NOT NULL,                -- main worktree, or the directory outside git
  name TEXT NOT NULL,                -- directory name until the user renames it
  pinned INTEGER NOT NULL DEFAULT 0,
  collapsed INTEGER NOT NULL DEFAULT 0,   -- the sidebar section's disclosure state (§6.2)
  hidden_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE threads (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  cwd TEXT NOT NULL,                 -- the worktree this thread works in
  branch TEXT,
  repository_id TEXT NOT NULL,       -- telemetry's salted hash of cwd: the join key to runs
  origin TEXT NOT NULL CHECK (origin IN ('console','run','collaborate','serve','desktop')),
  title TEXT NOT NULL DEFAULT '',    -- first line of the first message, bounded; user-editable (R6)
  overrides TEXT NOT NULL DEFAULT '{}',   -- types::Overrides
  permission TEXT NOT NULL DEFAULT 'ask', -- ask | allow | deny, or the agent's own mode id
  continuation TEXT,                 -- {agent, model, reasoning, mode, session_id, loadable, modes}
  pinned INTEGER NOT NULL DEFAULT 0,
  archived_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX threads_sidebar ON threads(project_id, archived_at, updated_at DESC);

CREATE TABLE hosts (
  id TEXT PRIMARY KEY,
  pid INTEGER NOT NULL, start INTEGER NOT NULL,   -- process::Identity, as peers already record
  kind TEXT NOT NULL CHECK (kind IN ('terminal','headless','serve')),
  thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
  version TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  heartbeat_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX hosts_thread ON hosts(thread_id);   -- one owner per thread
```

`continuation` is what `scheduler::Continuation` already carries; persisting it, plus the items, is what lets `orochi chat --continue` and the app resume a thread after a restart — by `session/load` when the agent offers it, otherwise by rebuilding the bounded transcript from `items` instead of from memory.

A host whose `(pid, start)` is gone is dead. The reader decides this the way `mailbox::prune` does; a turn still `running` under a dead host is shown as *interrupted*, and the next host to claim the thread writes that state down.

### 4.2 Turns

```sql
CREATE TABLE turns (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN
    ('queued','running','completed','failed','interrupted','withdrawn')),
  source TEXT NOT NULL CHECK (source IN ('terminal','desktop','stdin','acp')),
  steps TEXT NOT NULL DEFAULT 'auto',     -- what the user asked for: auto | team | solo
  shape TEXT,                             -- what it became: solo | seats | phases | team | discussion
  descriptor TEXT,                        -- the one TaskDescriptor this turn was classified as
  host_id TEXT,
  created_at INTEGER NOT NULL, started_at INTEGER, ended_at INTEGER,
  UNIQUE (thread_id, ordinal)
);
CREATE INDEX turns_queue ON turns(thread_id, state, ordinal);
```

**Sending a message is inserting a `queued` turn** plus its `user_message` item in one transaction. A host claims it with `UPDATE turns SET state='running', host_id=?, started_at=? WHERE id=? AND state='queued'`; the row count says who won. The console's own queue (Enter during a turn) becomes the same rows, so a message typed in the app during a terminal turn is simply next in line, and Up in the terminal taking the queue back is `state='withdrawn'`.

### 4.3 Seats and attempts

```sql
CREATE TABLE seats (
  id TEXT PRIMARY KEY,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL,               -- mailbox::Order place: 0 is the lead
  role TEXT NOT NULL,                     -- implementer, reviewer, architect, skeptic, design, coordinator…
  lead INTEGER NOT NULL DEFAULT 0,
  read_only INTEGER NOT NULL DEFAULT 0,
  phase TEXT,                             -- design | implement | review
  stage INTEGER, wave INTEGER, part_id TEXT,   -- collaboration only
  workspace TEXT,                         -- collaboration copy; NULL means the thread's cwd
  state TEXT NOT NULL CHECK (state IN
    ('choosing','starting','working','asking','checking','done','failed','cancelled')),
  started_at INTEGER NOT NULL, ended_at INTEGER
);

CREATE TABLE attempts (
  id TEXT PRIMARY KEY,
  seat_id TEXT NOT NULL REFERENCES seats(id) ON DELETE CASCADE,
  n INTEGER NOT NULL,
  agent TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
  reasoning TEXT, mode TEXT,
  resumed INTEGER NOT NULL DEFAULT 0,
  acp_session_id TEXT,
  peer_id TEXT,                           -- its mailbox identity (§4.7)
  considered TEXT,                        -- ≤ 5 runner-up candidates: agent, model, cost, p(success), reasons
  run_id TEXT,                            -- telemetry.runs.id, written when the run is recorded
  outcome TEXT, error_kind TEXT, error TEXT,   -- error: only what an ErrorKind::Other already shows
  checks TEXT,                            -- [types::CheckResult]
  usage TEXT,                             -- types::Usage, nullable fields stay null
  started_at INTEGER NOT NULL, ended_at INTEGER,
  UNIQUE (seat_id, n)
);
```

The link between the two files points one way: `attempts.run_id` names a telemetry run; telemetry never names a thread. Deleting a thread therefore leaves learning untouched, and sharing `telemetry.sqlite3` still shares no content (R3). `outcome`, `checks` and `usage` are copied onto the attempt so that the thread UI never needs the second file; they are labels and numbers, not content.

`considered` is the "why this route?" popover: what the scorer ranked next and the reasons it already produces (`ExecutionCandidate::reasons`).

### 4.4 Items

```sql
CREATE TABLE items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,   -- timeline order within a thread
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE CASCADE,  -- NULL: the user or Orochi itself
  kind TEXT NOT NULL,
  status TEXT,                            -- streaming | pending | in_progress | completed | failed
  key TEXT,                               -- ACP toolCallId, so an update finds its row
  text TEXT NOT NULL DEFAULT '',          -- what is read: message, thought, tool output, note
  data TEXT,                              -- kind-specific JSON, below
  truncated INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX items_timeline ON items(thread_id, id);
CREATE UNIQUE INDEX items_key ON items(attempt_id, key) WHERE key IS NOT NULL;
```

| `kind` | From | `text` | `data` |
|---|---|---|---|
| `user_message` | the composer / terminal input | the message | `{attachments:[{name, path, bytes, image}]}` |
| `agent_message` | `ExecutionEvent::Text`, appended while `status='streaming'` | the reply | — |
| `thought` | `Progress::Thinking` | reasoning text | — |
| `tool_call` | ACP `tool_call` / `tool_call_update`, upserted by `key` | output text | `{title, tool_kind, detail, locations, raw_input, raw_output}` |
| `plan` | ACP `plan`; one row per attempt, replaced in place | — | `[{status, content, priority}]` |
| `route` | `Progress::Route` | — | `{agent, provider, model, reasoning, resumed}` |
| `unavailable` | `Progress::Unavailable` | the classified error | `{agent}` |
| `note` | `Progress::Note`, `remembered:` lines | the line | — |
| `checks` | `Progress::Checking` → `Progress::Attempt` | — | `{outcome, checks, error, usage}`; a **failed** check also carries `tail`, its last 200 lines (§10.3) — the one place check output is kept, and never in telemetry |
| `mode` | ACP `current_mode_update`, Shift-Tab | — | `{mode}` |
| `handover` | a phase passing its reply on | — | `{from_seat, to_seat}` |
| `merge`, `apply` | `collaboration::MergeRecord`, `Application` | detail | the record |
| `context` | ACP `usage_update`; one row per attempt, replaced in place | — | `{used, size, cost}` — the context gauge; never copied into `types::Usage` |
| `commands` | ACP `available_commands_update`; one row per attempt, replaced | — | the agent's own slash commands, for the composer's `/` list |
| `acp` | any `sessionUpdate` kind this version does not know | — | the update as received, ≤ 16 KiB |

ACP schema v1 defines eleven `sessionUpdate` kinds ([schema](https://github.com/agentclientprotocol/agent-client-protocol/blob/main/schema/v1/schema.json), release `schema-v1.23.0`): the three chunk kinds, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update`, `usage_update`. All map to a row above except two that update a column instead: `session_info_update.title` fills `threads.title` when the user has not renamed the thread (a title with no call of Orochi's own — R6), and `config_option_update` refreshes `threads.continuation`. `user_message_chunk` is only ever a `session/load` replay and is not stored again. The `acp` kind is what keeps R8 true for kinds that do not exist yet.

Chunks carry an optional `messageId`; a new `agent_message` row starts when it changes, or, without one, when another kind interrupts the run of chunks. A `tool_call_update` replaces `content` whole and leaves omitted fields alone, which is exactly an upsert by `key`. Tool content of type `terminal` names a terminal the *client* owns, so its output exists only if Orochi captures it; `acp.rs` does not offer the terminal capability today, and this design does not add it.

A streamed message is one row that grows: the host appends to `text` at each flush, and a reader that already holds *n* characters asks for `substr(text, n+1)`. There is deliberately no separate append-only event log — it would double the stored text to serve a replay nobody needs once the row holds the result. Codex's `app-server` makes the same cut: deltas are transient and the item delivered by `item/completed` is authoritative (§6.0). What an event log would add — losslessness for fields and kinds this version does not know — is covered by keeping `raw_input` / `raw_output` and the `acp` kind.

Bounds: `text` ≤ 256 KiB per item, `raw_input` / `raw_output` ≤ 16 KiB each; beyond that the tail is kept and `truncated = 1`. `[activity] thinking = true` can be turned off to skip `thought` rows.

This requires `acp::progress` to stop dropping what fact 8 lists: `ToolUpdate` gains `kind`, `locations` and `raw_output`, and `current_mode_update` becomes a `Progress` variant. The terminal console ignores the additions.

### 4.5 Patches

```sql
CREATE TABLE patches (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  path TEXT NOT NULL,                     -- relative to the seat's workspace
  change TEXT NOT NULL CHECK (change IN ('add','modify','delete')),
  added INTEGER NOT NULL, removed INTEGER NOT NULL,
  patch TEXT NOT NULL,                    -- unified diff of acp::FileDiff old → new, ≤ 512 KiB
  truncated INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX patches_turn ON patches(turn_id, path);
```

ACP reports an edit as whole old and new file texts. Storing the unified diff keeps the review pane's input while holding far less source than the two texts would. These patches are **what the agent said it changed**; the review pane's authority is `git diff` in the thread's `cwd`, and the stored patch is what remains once the working tree has moved on (and the only record for an agent that edits through a shell).

### 4.6 Prompts and controls: what the app writes

```sql
CREATE TABLE prompts (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE CASCADE,
  item_id INTEGER REFERENCES items(id) ON DELETE SET NULL,   -- the tool call it is about
  kind TEXT NOT NULL CHECK (kind IN ('permission','team','apply')),
  request TEXT NOT NULL,                  -- ACP toolCall + options, or the proposed parts, as shown
  answer TEXT,                            -- an ACP optionId; 'yes' / 'no' for team and apply
  answered_by TEXT,                       -- terminal | desktop | policy | host_gone
  created_at INTEGER NOT NULL, answered_at INTEGER
);
CREATE INDEX prompts_open ON prompts(thread_id) WHERE answer IS NULL;

CREATE TABLE controls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('interrupt','set_mode','set_permission','reroute','new','stop')),
  payload TEXT,
  created_at INTEGER NOT NULL, consumed_at INTEGER
);
```

A permission question becomes a row the moment `acp.rs` raises `ExecutionEvent::Permission`. Whoever answers first wins: `UPDATE prompts SET answer=?, answered_by=? WHERE id=? AND answer IS NULL`. The terminal dialog and the app's card race on that one statement, and the loser's UI closes when it sees the row answered. The host resolves the `oneshot` from the row. Read-only seats and coordination tools are still decided inside `acp.rs` before any row exists, recorded with `answered_by='policy'` only when the refusal is something the user should see.

`interrupt` is what Esc does; the host raises its own interrupt latch, so every rule in `interrupt.rs` holds unchanged. `reroute` carries the feedback signal exactly as `/reroute` does today (`Store::feedback`, first signal only). Nothing here is a new capability — each control is an existing key or slash command arriving by another road.

The app's complete write surface: insert a thread; insert a queued turn with its `user_message`; answer a prompt; insert a control; rename / pin / archive / delete a thread or project; its own `ui_state`. All through typed functions in the `orochi` library, never SQL from the WebView.

### 4.7 The mailbox moves in

`peers` and `messages` move from `mailbox.sqlite3` into this file, because the group chat has to join a message to the seat that sent it, and a persistent view cannot span two files.

```sql
CREATE TABLE peers (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,               -- was `channel`
  name TEXT NOT NULL,
  attempt_id TEXT REFERENCES attempts(id) ON DELETE SET NULL,
  worktree TEXT NOT NULL, branch TEXT, route TEXT,
  status TEXT NOT NULL DEFAULT '',
  owner_pid INTEGER NOT NULL, owner_start INTEGER NOT NULL,
  last_read INTEGER NOT NULL,
  started_at INTEGER NOT NULL,
  left_at INTEGER                         -- set where the row used to be deleted
);
CREATE UNIQUE INDEX peers_live_name ON peers(project_id, lower(name)) WHERE left_at IS NULL;

CREATE TABLE messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id TEXT NOT NULL,
  thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,  -- the sender's thread, when it has one
  sender TEXT NOT NULL, sender_name TEXT NOT NULL,
  recipient TEXT NOT NULL, recipient_name TEXT NOT NULL,    -- '*' / 'all' for a broadcast
  via TEXT NOT NULL DEFAULT 'mailbox' CHECK (via IN ('mailbox','handoff','user')),
  body TEXT NOT NULL,
  sent_at INTEGER NOT NULL
);
CREATE INDEX messages_room ON messages(project_id, id);

CREATE TABLE peer_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  peer_id TEXT NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('joined','left','status','route')),
  text TEXT NOT NULL DEFAULT '',
  at INTEGER NOT NULL
);
```

What changes in behavior: nothing an agent can observe. "Running peers" becomes `left_at IS NULL` with a live owner; a session still starts deaf to earlier messages (`last_read = MAX(id)`); the hourly limit and body cap stay. What changes in lifetime: a message now lives as long as its thread (or `mailbox.retention_secs` when it belongs to none), because the group chat is part of the conversation. `peer_events` records what `chat::Feed` derives today by diffing two peer lists — joined, left, and each `set_status` — so that "claude-2 took `src/chat/`" is a line in the room rather than a value that was overwritten.

`via = 'handoff'` carries the messages `collaborate` routes between turns (`Report::messages`), so both kinds of agent-to-agent talk land in one room. `via = 'user'` is §6.4.

`mailbox.sqlite3` holds nothing worth migrating (it is a one-day window); the new binary stops opening it and deletes it once no live peer row remains in it. With `activity.enabled = false` the same two tables are created in `mailbox.sqlite3` as today, `attempt_id` and `thread_id` always NULL, and rows are pruned as now.

### 4.8 Work graph

```sql
CREATE TABLE parts (
  turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  id TEXT NOT NULL,                       -- the part's own id from {"parts": …}
  brief TEXT NOT NULL,
  paths TEXT NOT NULL DEFAULT '[]', after TEXT NOT NULL DEFAULT '[]',
  wave INTEGER,                           -- after graph.rs has gated and ordered it
  unit TEXT,                              -- parts fused into one session share a unit
  state TEXT NOT NULL DEFAULT 'waiting',  -- waiting | running | merged | failed
  strays INTEGER,                         -- files changed outside declared paths (MergeRecord::strays)
  PRIMARY KEY (turn_id, id)
);
```

`report.json` remains the resumable source of truth for `collaborate`; these rows are a projection written alongside it. Moving resumability into the store is possible later and is not part of this design.

### 4.9 Change feed

```sql
CREATE TABLE changes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  tbl TEXT NOT NULL, row TEXT NOT NULL, thread_id TEXT, at INTEGER NOT NULL
);
-- one AFTER INSERT and one AFTER UPDATE trigger per table above, each a single INSERT INTO changes
```

`sqlite3_update_hook` sees only its own connection, so cross-process change detection is `PRAGMA data_version` (did anything commit?) followed by `SELECT … FROM changes WHERE id > :cursor` (what?). Triggers rather than writer discipline, because there are several writers — hosts, the mailbox MCP subprocess, the app — and a forgotten notification is a UI that silently goes stale. Rows older than an hour are pruned; a reader whose cursor predates the oldest row reloads the open thread. The same feed drives OS notifications (a prompt opened, a turn ended) without diffing.

**One writing connection per process**, shared behind a mutex (`activity::Shared`). Found during P0, and the reason is worth keeping: a connection per writer looked cheaper and was not. SQLite serializes writers to a database anyway, so separate connections only contend; `busy_timeout` then *sleeps*, and a sleep on an async task stops every agent that task is driving. Closing one is worse — `sqlite3_close` of a WAL connection takes the VFS's shared-memory mutex, and opening and closing connections from several tasks at once deadlocked the seats of one collaboration against each other (observed, `sqlite3WalClose` → `unixEnterMutex`). With one connection: writes are short and local, the lock is never held across an await, and nothing is opened or closed while agents run.

**A reader between the agent and its consumer must be drained before the consumer stops listening.** The recorder forwards on a task of its own, so when a prompt future resolves, what the agent sent last may still be in flight. Every consumer that drains with `try_recv` after its future completes — `collaboration::turn::execute`, the gateway's `forward` — waits for the tee first, or it loses the end of the reply. This cost two real failures in P0 before it was understood.

Reader rules, from SQLite's own documentation ([WAL](https://www.sqlite.org/wal.html), [pragmas](https://www.sqlite.org/pragma.html)):

- `data_version` is comparable only on **one** connection over time and does not move for that connection's own commits. The app keeps one long-lived read connection (`PRAGMA query_only=ON`) for polling and reading, and a second, short-lived one for its few writes — whose effects it therefore sees through the feed like anyone else's.
- The read connection is opened read-write at the file level, not `mode=ro`: a read-only open of a WAL database fails when `-shm` / `-wal` do not exist yet, and `immutable=1` is wrong for a live file.
- A reader that is always inside a transaction stops checkpoints and the WAL grows without bound. Every poll is its own short transaction, no statement stays stepped across ticks, and a host runs `wal_checkpoint(TRUNCATE)` when a turn ends.
- `SQLITE_BUSY` still occurs in WAL (recovery, the last connection closing), so both sides keep `busy_timeout`.
- The data directory must be on a local disk; WAL needs shared memory and does not work over a network filesystem. The app says so if `<data>` is on one, rather than corrupting it.
- `PRAGMA application_id` marks the file as Orochi's; `user_version` follows the rule of §5.3, and only the core migrates — the app refuses a `view_api` it does not know and offers to update.

### 4.10 Views: the app's contract

The app reads **only** `v_*` views; base tables are free to change. `meta.view_api` is an integer the app checks at startup.

| View | Rows | Feeds |
|---|---|---|
| `v_sidebar` | thread × {project, title, `status`, last activity, working seats, open prompts, unread} | sidebar |
| `v_timeline` | items ∪ messages ∪ peer_events for a thread, in order, each with `lane` (seat), author label, provider | the thread |
| `v_room` | messages ∪ peer_events for a thread, or for a project | group chat |
| `v_roster` | seat × latest attempt × peer: role, agent, model, state, status line, tool in progress, elapsed, tokens | roster, mission control |
| `v_turn_files` | per turn and path: change, added, removed, latest patch id | review pane |
| `v_open_prompts` | unanswered prompts with their tool call | approval cards, badge, notifications |
| `v_board` | parts with wave, state and the seat running each | work-graph board |
| `v_route` | attempt with `considered`, prediction vs. outcome | "why this route?" |

`v_sidebar.status` is derived, never stored: `needs_you` (an open prompt) > `working` > `queued` > `interrupted` (running under a dead host, decided by the reader) > `failed` > `done_unread` > `idle`. `unread` compares `items.id` with `ui_state.last_seen_item`.

```sql
CREATE TABLE ui_state (thread_id TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
  last_seen_item INTEGER NOT NULL DEFAULT 0, draft TEXT);
```

`orochi threads [list|show <id>|export <id>|delete <id>]` reads the same views from the terminal. It is how P0 is tested, and how R1 stays honest: if a screen cannot be produced by `orochi threads show`, the view is missing something.

## 5. `telemetry.sqlite3`

The shape is sound for what it is — a learning log whose unit is a frozen JSON record — and the invariant that old records must keep deserializing argues against normalizing it. Four changes, all additive:

1. **Generated columns instead of `json_extract` in every query.** `ALTER TABLE runs ADD COLUMN purpose TEXT GENERATED ALWAYS AS (json_extract(record,'$.purpose')) VIRTUAL`, likewise `language`, `framework`, `complexity`, `provider`, `reasoning_level`, `mode`, `outcome`, `error_kind`, `feedback_signal`. Virtual generated columns may be added to an existing table, take no space, are not counted by an `INSERT … VALUES` without a column list (so older binaries keep writing — fact 3), and can be indexed: `(agent, model, task_type, purpose, started_at)` for `learning_runs` / `pooled_runs`, `(provider, task_type, complexity, started_at)` for `tier_runs`, `(purpose, started_at)` for `advice_cost`, which today reads every run to find the classification ones. The JSON stays the source of truth; nothing is migrated. Checked on 2026-09-20 with SQLite 3.51.0 against the current `runs` definition: after two such `ALTER TABLE`s an eight-value `INSERT INTO runs VALUES (…)` still succeeds, `json_set` on `record` (what `Store::feedback` does) updates the generated values, and the planner uses the new index once a query names the column instead of repeating the `json_extract`.
2. **Name the columns in every `INSERT`** from now on, so a later real column is possible.
3. **Do not bump `user_version` for additive changes.** Today `version <= 2` locks an older binary out the moment a newer one touches the file, which a bundled desktop binary would do to the user's CLI (R5). `user_version` moves only for a change an old binary cannot survive; additive changes are recorded in `metadata.schema_minor`.
4. **Set the file to `0600`**, as the mailbox already is (fact 12).

Views for the app's Insights screen, created by the core: `v_runs` (flat columns, no JSON), `v_route_stats` (per agent × model × task type: verified runs, success rate, mean tokens and duration, weak-signal counts kept apart as `calibrate` does), `v_runtime` (effective status and cooldown, computed from `runtime` at read time), `v_quota` (`quota_snapshots` unnested with `json_each`). An optional `quota_history(agent, bucket, observed_at, remaining)` makes a usage chart possible; it is numbers only.

`sessions`, `runtime`, `classifications` and `metadata` stay as they are.

## 6. The desktop app

### 6.0 What was surveyed (2026-09-20)

Read from the vendors' own documentation unless marked. Nothing here was checked by running the apps.

| Product | What it does | Taken here as |
|---|---|---|
| Claude Code in Claude Desktop ([docs](https://code.claude.com/docs/en/desktop)) | Session sidebar filterable by status / project / environment, groupable by project; composer selectors for environment, folder, model, permission mode; a worktree toggle beside the branch for parallel sessions; panes for chat, diff, plan, tasks, subagents; transcript modes *Normal* (tool calls collapsed), *Thinking*, *Verbose*; diff viewer with line comments submitted as one batch; OS notification when a session finishes unviewed; `/desktop` and `/resume` hand a session between CLI and app | Sidebar filters (§6.2), the three transcript modes (§6.3), batch diff comments (§6.5), the composer's selectors (§6.6), notifications (§6.9), terminal ↔ app handoff (`chat --continue`) |
| Claude Code's store ([docs](https://code.claude.com/docs/en/sessions)) | Append-only JSONL per session under `~/.claude/projects/`, kept 30 days; format declared internal | The 30-day default of `activity.retention_days` |
| Codex app ([projects](https://learn.chatgpt.com/docs/projects), [worktrees](https://learn.chatgpt.com/docs/environments/git-worktrees), [review](https://learn.chatgpt.com/docs/code-review), [subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)) | Threads under projects, pin and archive; each thread *Local* or *Worktree*; review pane with scopes Unstaged / Staged / Commit / Branch / **Last turn**, stage or revert per file or hunk, inline comments, filename opens the editor; a subagent panel with *Active* and *Done* lists, each child openable | Project → thread sidebar, the review scopes (§6.5), the roster's Active / Done split (§6.4) |
| Codex `app-server` ([docs](https://learn.chatgpt.com/docs/app-server)) | Thread → Turn → Item; `item/started`, deltas, then `item/completed` with the completed item authoritative; turn states `inProgress / completed / interrupted / failed`; approvals as requests answered accept / acceptForSession / decline / cancel; `turn/diff/updated` carries one aggregated diff per turn; `turn/steer`; a `collabToolCall` item for inter-agent calls | Confirms the shape of §4: thread / turn / item, one growing row per item instead of an event log, a per-turn file aggregate (`v_turn_files`) |
| Codex persistence (source: `codex-rs/state/migrations`; rollout details secondhand) | JSONL rollouts are the truth and a SQLite index is derived from them, with scan-and-repair on list | The opposite choice: **one** store. A derived index that can drift from its source is the failure mode §4.8 keeps small by declaring `report.json` the authority for the one place two copies exist |
| Zed ([docs](https://zed.dev/docs/ai/parallel-agents)), Conductor ([docs](https://www.conductor.build/docs/core/parallel-agents)), Cursor agents window (secondary source), Vibe Kanban ([repo](https://github.com/BloopAI/vibe-kanban)) | One list for every agent regardless of where it runs; a workspace is a worktree; several agents may share one workspace (one implements, one reviews); Rust + SQLite backend (Vibe Kanban) | A terminal thread and an app thread in one list; mission control (§6.7) |

None of the surveyed tools shows agent-to-agent messages as a conversation of its own; the nearest are Claude Desktop's cross-session messaging and Codex's `collabToolCall` items inside the parent timeline. The Team tab (§6.4) is where this app is not following anyone.

### 6.1 Layout

```
┌───────────────┬─────────────────────────────────────────┬──────────────────────────┐
│ + New thread  │ orochi · main ▸ Fix the flaky mailbox    │ Team │ Changes │ Plan │ Run│
│ ───────────── │ ─────────────────────────────────────── │ ─────────────────────── │
│ ▾ orochi    + │  You                                     │ ● implementer  claude    │
│  ◉ Fix flaky… │   Fix the flaky mailbox placement test   │   opus · high            │
│  ● Desktop d… │                                          │   "editing tests/mailbox"│
│  ○ Retire ta… │  ⟡ claude · opus · high   why?           │ ● reviewer     codex  RO │
│ ▸ scratch   + │  ▸ Thought for 8s                        │   gpt-5.6 · medium       │
│ ▾ web-app   + │  ▸ Read 3 files, ran 2 commands          │   "reading mailbox.rs"   │
│  ! Add login  │  ✎ tests/mailbox.rs  +12 −4              │ ─────────────────────── │
│  ○ Dark mode  │  The race is in …                        │ reviewer → implementer   │
│               │  ┌ Permission ───────────────────────┐   │  The sleep hides it; use │
│               │  │ $ cargo test --test mailbox       │   │  Order::place instead.   │
│               │  │ [Allow once] [Always] [Deny]      │   │ implementer → all        │
│               │  └───────────────────────────────────┘   │  Taking tests/mailbox.rs │
│ ───────────── │ ─────────────────────────────────────── │ ● reviewer joined        │
│ Agents  82%   │ ┌ Message…            @file  📎 ───────┐ │                          │
│ Insights      │ │ Auto ▾  Ask ▾  main ▾        ⏎ Send │ │ [ Message the team… ]    │
│ Settings      │ └──────────────────────────────────────┘ │                          │
└───────────────┴─────────────────────────────────────────┴──────────────────────────┘
```

A sidebar of projects and threads, the thread, and a collapsible side pane — the parts both surveyed apps are built from (§6.0), arranged so that nothing has to be learned. What is Orochi's own sits where those apps have nothing: the route chip at the head of a turn, the Team tab, and the Agents gauge.

### 6.2 Sidebar — `v_sidebar`

**Threads are grouped under the project they belong to**, the way Claude Code's Code tab groups sessions — asked for explicitly on 2026-09-20 and therefore a requirement, not a default that may be traded away:

- One **section per project**, headed by its name, with a disclosure chevron and a `+` that starts a thread in that project without going through the New-thread dialog. A project's section is shown even when every thread in it is archived, so the list of projects is stable between visits.
- **Sections are ordered by name**, case-insensitively, with pinned projects first — not by recency, so a project keeps its place in the list and can be found by eye. Threads *inside* a section are ordered by `updated_at` descending, pinned first.
- **Collapse is per project and persists** (`projects.collapsed`): the tree the user left behind is the tree they come back to. A section with a working thread or an open prompt shows a count on its header while collapsed, so folding a project never hides that something needs an answer.
- The list is **paged**: a fixed number of threads per section with *Load more* beneath, and a global *Load more sessions* at the foot, rather than every thread ever run.

One status mark per thread, from `v_sidebar.status`: `!` needs you, `◉` working, `●` finished and unread, `○` idle, `×` failed, `⏸` interrupted. A thread running in a terminal carries a small terminal glyph (`hosts.kind`); it is otherwise the same thread. Filters at the top: by status and *running in a terminal*; grouping can be turned off for one flat recency list, which is the only view where a project is not a heading. Search filters titles (content search is a later `FTS5` table over `items.text`, not part of this design). Context menu on a thread: rename, pin, archive, delete (R4: delete means gone), reveal in Finder, open in terminal (`orochi chat --continue <id>`); on a project: rename, pin, hide, new thread.

A project row exists as soon as Orochi runs anywhere in that repository, including from a terminal, so the sidebar is a map of the work rather than a list of what the app happened to start. `projects.name` starts as the directory name and is the user's to change; `root` is the main worktree, so every worktree of one repository lands in one section — the same identity the mailbox already uses for a room, which is what makes "the project" mean one thing in both the sidebar and the Team tab.

**New thread** asks for what `orochi` takes from its working directory and flags: project (recent roots or a folder picker), where it works — *Local* (the project's checkout) or *Worktree* (an existing worktree, or a new one the app creates with `git worktree add` under `<data>/worktrees/` when the user turns the toggle on; the two-mode choice is Codex's and Claude Code's alike) — and optionally a route override. A worktree is what lets two threads of one project write at the same time without `scheduler.shared_workspace`: each has its own workspace lock, and they still share one project room, because the mailbox channel is the git common dir. The app never deletes a worktree it did not create, and asks before deleting one it did. Nothing else — R1 of the parallel-execution design still holds: the user hands over a prompt.

### 6.3 Thread — `v_timeline`

One column, in `items.id` order, grouped by turn. Three levels of detail, Claude Code's and under its shortcut (Ctrl+O): **Normal** — work collapsed; **Thinking** — work collapsed, reasoning open; **Verbose** — every item open, second seats included. The level is a rendering choice over the same rows.

- **Turn head:** the user's message, then the route chip `agent · model · reasoning` (`route` item). *why?* opens `v_route`: the runners-up with cost and predicted success, the reasons the scorer gave, and after the turn the prediction beside the outcome. `unavailable` and `note` items are muted single lines, as in the terminal.
- **Work, collapsed by default:** consecutive `thought` and `tool_call` items fold into one line ("Thought for 8 s", "Read 3 files, ran 2 commands"); expanding shows each call with its `detail`, status, output and locations. An `edit` tool call shows its patches inline with `+n −m` and opens the Changes tab at that file.
- **Reply:** `agent_message`, rendered as Markdown while it streams.
- **Plan:** the attempt's `plan` item pinned above the composer as a checklist while the turn runs, folded into the timeline when it ends.
- **Checks:** one card per `checks` item — each check's name, pass / fail, duration — and the outcome in Orochi's own words: *verified*, *unverified* (`PartialSuccess` — never shown as success), *failed*, *stopped*.
- **Phases:** a `design → implement → review` turn draws a divider per seat with its own route chip; a `handover` item says what was passed on. A `VERDICT: fix` review that adds a pass reads as another divider, not as a new turn.
- **Failover:** a second attempt in the same seat is a muted line ("codex hit a rate limit — continuing on claude") and a new route chip. The error is the classified kind, never the provider's text, as the invariant requires.
- **Prompts:** a `permission` prompt is a card at the point in the timeline where the tool call is, with the agent's own options (allow once / always / reject). `team` is the plan-approval dialog: the proposed parts as a small graph (`v_board`), *Run as a team* / *Keep the design*. Esc on either does what it does in the terminal.
- **Second seats stay out of the timeline**, as they stay out of the transcript today: what a read-only seat has to say arrives as messages, shown inline as compact chat bubbles and in full in the Team tab. A lane filter ("show reviewer's work") reveals its items for the curious.

### 6.4 Team — `v_room`, `v_roster`

The part no other client has. Top: the **roster** — one row per seat: role, agent and model, a provider-colored avatar, `RO` for read-only, state, the agent's own `set_status` line, the tool it is in, elapsed time, tokens, split into *Active* and *Done* as Codex's subagent panel is; clicking a seat filters the timeline to its lane. Below: the **room** — mailbox messages as chat bubbles colored by sender (the same name-derived colors as the terminal), `→ name` or `→ all` on each, with `joined`, `left` and status changes as system lines between them. A collaboration's handoff messages appear in the same room, marked as relayed by Orochi.

Two scopes, one switch: *this thread* (messages from and to this thread's peers, plus broadcasts while its turns ran) and *this repository* (everything in the project's channel, across threads and terminals — what `orochi peers --messages` shows).

**The user in the room** (`via = 'user'`): a message box that posts to one seat or to all. It is delivered the way every mailbox message is — when the agent next calls `read_messages` — so it is a note left on the table, not an interruption, and the UI says so ("delivered when the agent next checks"). It lifts a documented limitation (*"a way for a person to send messages"*) without changing the agent-facing protocol: the host registers one peer named `user` per thread. Whether agents act on such a note mid-turn is **unverified** and must be measured before this is presented as steering. (Codex has `turn/steer` for this; ACP v1 has no equivalent, so the mailbox is the only road that works for every agent.)

### 6.5 Changes — `v_turn_files` + `git diff`

File list with `+n −m`, unified or split diff beside it. Scopes follow Codex's review pane: **Last turn** (stored patches — the only scope that survives the working tree moving on), **Unstaged**, **Staged**, **Branch** (against the merge base) — the last three are `git diff` in the thread's `cwd`, which is the authority. Clicking a filename opens the user's editor at the line.

**Comments become the next message.** Hovering a line offers a comment; comments collect, and *Send* turns them into one queued turn whose text lists `path:line — comment` (the batch submission both surveyed apps use). No new mechanism: it is a user message, routed like any other, and it continues the thread's agent.

Stage, revert, commit and PR are out of the first release; the app surfaces the user's git, it does not drive it. For a collaboration thread the tab shows the final workspace against the user's tree and an **Apply** button that is exactly `--apply`: it appears only when real checks passed, inserts an `apply` prompt, and the host performs the staged, re-verified write the invariant describes.

### 6.6 Composer

Multiline input; **⌘Enter sends, Enter breaks the line** (a message here is often several lines of thinking, and a stray Enter would send half of one; decided 2026-09-21); `@` completes paths in the thread's `cwd`; pasted or dropped files become attachments (a pasted image is written under `<data>/attachments/<thread>/` and deleted with the thread); `/` lists the console's commands. Three selectors, each a persistent chip: **route** (*Auto*, or pin agent / model / reasoning — `threads.overrides`), **approvals** (the agent's own session modes when it has them, else ask / always / never — the Shift-Tab cycle), **steps** (*Auto* / *Solo* / *Team*). While a turn runs, Enter queues (the queue is shown above the input and can be taken back), Esc interrupts. All of it is the terminal console's behavior reached with a mouse; where they could differ, Claude Code's behavior decides (R9).

**New thread** starts one where the work already happens — the open thread's folder, else the
one worked in last — and only asks when nowhere has been worked in yet, through the system's
own folder picker. It does not open the composer's folder menu: that menu belongs beside the
chip that owns it, a screen away from this button, and the click that opened it from here
closed it again on the way out (fixed 2026-09-21). `desktop/tests/dom.mjs` now carries events
to the document and builds the markup's real nesting, so a handler that opens something and a
document handler that closes what was not clicked meet in the tests the way they meet in the
window.

### 6.7 Mission control — `v_roster` across threads

One screen of every seat that is working right now, across projects: a card per seat grouped by thread, columns by state (choosing → working → asking → checking). Open prompts float to the top with their buttons, so ten threads can be supervised without opening any. This is also the menu-bar popover.

### 6.8 Agents and Insights — telemetry views

**Agents:** per configured agent — discovered models, runtime status and cooldown with its reset time (`v_runtime`), quota windows as gauges (`v_quota`), the last probe, adapter version. *Refresh* runs `orochi quota refresh`. **Insights:** `v_route_stats` as a table and small multiples — success rate and cost per route and task type, verified and weak-signal evidence kept visibly apart, calibration (predicted vs. observed) as `orochi calibrate` reports it. Read-only; the numbers are Orochi's own estimates and the screen says so.

### 6.9 Notifications and lifecycle

OS notifications from the change feed: a prompt opened (with Allow / Deny actions), a turn ended while the window was unfocused, a turn failed. Dock badge = open prompts. Quitting the app leaves hosts running (R7) and says so once; *Stop all* is explicit. On launch the app reaps nothing — reaping dead hosts' leases stays the core's job (`cli::execute`).

### 6.10 Settings

A form over `config.toml` (`orochi config show` / validated writes through `Config::validate`; `agent.env` values stay redacted), the `[activity]` section (retention, thinking, off switch, *Delete all history*), the memory files as editable Markdown (they are the user's text), and which `orochi` binary the app uses, with its version beside the one on `PATH`.

## 7. Changes in the core

| Where | Change |
|---|---|
| `activity.rs` (new) | The store: schema, triggers, views, typed reads and writes, retention, `secure_delete`. The only module that knows the SQL |
| `activity/recorder.rs` (new) | An `EventSink` tee: consumes `ExecutionEvent`, coalesces text (≤ 80 ms), upserts items and patches, opens prompts and resolves their `oneshot` from the row, forwards everything unchanged to the console's own sink. A failure to write never fails the turn — it degrades to today's behavior with one `note` |
| `acp.rs` | `ToolUpdate` gains `kind`, `locations`, `raw_output`; `Progress` gains `Mode`, `Context` (`usage_update`), `Commands`, `Info` (`session_info_update`) and `Other(Value)` for kinds it does not know (fact 8, §4.4); `Text` carries the chunk's `messageId`. The terminal console ignores all of them |
| `scheduler/mod.rs` | `RunOptions` gains `seat: Option<activity::SeatRef>`; the retry loop opens an attempt per pass and writes `run_id` where it records the run. No routing logic changes |
| `chat/mod.rs` | Thread and turn rows at the points where `Conversation` and `queue` change today; the queue becomes queued turns; `/new` starts a thread; prompts and controls are polled beside the keyboard. `--continue [<id>]` resumes from the store |
| `chat/host.rs` (new) | `orochi host --thread <id>`: `Session` with no terminal — `View` and `Keyboard` behind the trait they already nearly are |
| `collaboration/` | Progress through `acp::Progress` instead of `eprintln!` (fact 11, already noted in the parallel-execution design); seats, attempts, parts, `merge` / `apply` items and handoff messages projected beside `report.json` |
| `gateway.rs` | A `serve` session is a thread with `origin='serve'`; its history string stays as is |
| `mailbox.rs` | Tables move (§4.7); `unregister` sets `left_at`; `peer_events` written on join, leave, `set_status`, `set_route` |
| `storage.rs` | §5 |
| `cli.rs` | `orochi threads …`, `orochi host` (hidden), `chat --continue` |
| `config.rs` | `[activity] enabled, retention_days, thinking, host_idle_secs` with `Default` entries and `validate` checks |

## 8. Phases

| Phase | Delivers | Proven by |
|---|---|---|
| **P0 Store** ✅ | `activity.sqlite3`, the recorder, views, `orochi threads`, the telemetry changes of §5. Console, `run`, `collaborate` and `serve` all write rows. No behavior change a user can see except that history exists | Fixture tests with the mock agent: a thread read back through `v_timeline` equals the live event sequence (R8); byte-level privacy tests of §3; an old-schema `telemetry.sqlite3` still opens and an `INSERT` without column names still succeeds against the new columns (R5); measured flush and poll latency |
| **P1 Control** ✅ | Prompts and controls through the store; `orochi host`; `chat --continue`; the mailbox move; the failed-check tail (`evaluator` hands it to the recorder, `CheckResult` and `RunRecord` stay as they are) | Terminal test: a second process answers a permission question and interrupts a turn the terminal owns; a queued turn inserted from outside runs next; a killed host leaves a thread that reads *interrupted* and resumes |
| **P2 App, observing and conversing** ✅ | Tauri shell, sidebar, thread, composer, folder picker, prompts, Team tab read-only (notifications remain) | The app's Rust commands tested against fixture databases; the WebView tested against recorded view output. No agent account needed, as everywhere else |
| **P3 Review and supervise** ✅ | Changes with its git scopes and line comments, mission control, Agents, Insights, work-graph board, Settings | Client tests over fixture stores (including a real `git init` for the scopes); UI tests over recorded view output |
| **P4 The user in the room** ✅ | `via='user'` messages, from the terminal and the window | Library test: a note reaches a peer's `read_messages`. **Against a real CLI (2026-09-20): Sonnet read one mid-turn and acted on it; Haiku did not** |

P0 and P1 are worth having with no app at all: persistent history, `--continue`, and answering a prompt from another terminal.

## 9. Risks

| Risk | Handling |
|---|---|
| **Write amplification while streaming** — several seats, each flushing every 80 ms, plus triggers | WAL with `synchronous=NORMAL`; one transaction per flush. **Measured with six seats**: 157 µs a flush, about 1.2 % of a second's wall clock. If it ever bites: widen the flush interval per seat count before anything cleverer |
| **Checkpoint starvation** — a reader holding a transaction open keeps the WAL growing | The app opens short read transactions per poll and never holds a statement across ticks; a test asserts the WAL shrinks after a checkpoint with the app's reader attached |
| **Two writers claim one thread** | `hosts_thread` is unique; a second host exits with "this thread is running in another process (pid …)". The workspace lock already prevents two runs in one directory unless `shared_workspace` |
| **Version skew between the app's `orochi` and the CLI's** | §5.3; `activity.sqlite3` follows the same rule from its first version; the app reads views only and checks `view_api`; Settings shows both versions |
| **Disk growth** | Per-item and per-patch caps, retention, thinking off switch. **Measured**: a deliberately heavy turn costs 256 KiB, so a busy day is tens of MB |
| **A user who relied on "nothing is kept"** | The change is announced in the release notes and `README`; `activity.enabled = false` is one line; `orochi threads delete --all` exists from P0 |
| **The store becomes a second source of truth for `collaborate`** | It is declared a projection (§4.8); `report.json` decides on resume, and a mismatch is resolved in its favor |
| **macOS App Sandbox** — a sandboxed app cannot read `~/.local/share/orochi`, and even with a user-selected file the sibling `-wal` / `-shm` files are a known failure ([Apple forum](https://developer.apple.com/forums/thread/670503)) | Ship outside the sandbox: Developer ID signing and notarization, no Mac App Store. The app spawns CLIs the user installed; a sandbox would forbid that too |
| **WebView differences** (WebKit on macOS, WebKitGTK on Linux) and large diffs | The diff and Markdown renderers are the two components chosen for it (CodeMirror merge view); lists are virtualized; patches are capped (§4.5). No embedded terminal |
| **Polling cost when idle** | `data_version` is a header read; 1 Hz when nothing runs, and the app stops polling when hidden with no working thread |

## 10. Decisions (made 2026-09-20)

| # | Question | Decision |
|---|---|---|
| 1 | §3 — may the conversation be persisted in a separate, bounded, deletable file? | **Yes, on by default.** `activity.sqlite3`, `0600`, `secure_delete`, 30-day retention with pinned threads exempt, `activity.enabled = false` as the off switch. `telemetry.sqlite3` keeps every guarantee it has |
| 2 | §4.7 — fold the mailbox into `activity.sqlite3`, or keep two files and `ATTACH`? | **Fold.** The group chat is conversation; one retention rule instead of two. `mailbox.sqlite3` remains only when `activity.enabled = false` |
| 3 | Check output, never stored today | **Store the last 200 lines of a failed check** on the `checks` item (`data.checks[].tail`), bounded like any item text, only in `activity.sqlite3`, dropped with `activity.enabled = false`. Passing checks store nothing. Ships in P1, not P0 |
| 4 | D3 — the shell | **Tauri 2 with a React + TypeScript WebView**, outside the macOS sandbox. The front-end framework is not load-bearing: the WebView only renders typed view rows |

P0 and P1 shipped on 2026-09-20; P2 onwards is the app itself.
