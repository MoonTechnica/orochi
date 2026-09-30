# Supervising Background Agents: Design

Written: 2026-09-29.

**Status (2026-09-30): S0–S6 implemented; S7 partly verified.** A real Codex lead, inside a
real Orca pane, delegated to two helpers, answered at once and merged their reports, and Orca
recorded the pane's state ([real-validation-20260930.md](real-validation-20260930.md)). Not
verified: Orca's drawing of the helpers, a Claude lead, delegation nobody asked for, and the
window's notifications and dock badge. Decisions are in §2;
where the implementation departed from the design, §14 says how and why.

One goal, three surfaces: **a person running several agents at once can see which one needs
them, what each is doing, what its helpers are doing and what the accounts have left, and can
let helpers work without waiting on them.** The terminal console, the desktop app and — when
Orochi runs inside it — Orca's own sidebar all read it from the same rows.

## 0. Sources

| Source | Gives | Where |
|---|---|---|
| A screenshot of Claude Code 2.1.28x inside Orca, judged by the user "very good" | The console experience: helpers launched by the lead, the lead answering at once, a waiting row, a helper's permission prompt named by who asks | §0.1 |
| Claude Code 2.1.283's installed binary (`~/.local/share/claude/versions/2.1.283`) | Exact wording where it is static | §0.1 |
| Orca 1.4.215 (`/Applications/Orca.app`): its main process, its bundled UI dictionary, its hook scripts | The supervision UX benchmarked for the desktop app, and how a program reports itself to Orca | [Desktop App Benchmark: Orca](desktop-benchmark-orca.md), §8.1 |
| Orochi at `21bfa00`: `src/chat/`, `src/activity.rs`, `src/mailbox.rs`, `src/acp.rs`, `desktop/` rendered from `preview.html` | What exists | §3 |

### 0.1 The screenshot

```
● いいえ、今はフロントエンドからは変えられません。 …                 ← the lead's reply ("No, not from the front end today")
✻ Worked for 16s · done 11:54 PM                                   ← turn footer

❯ フロントエンドから変えられるようにしましょう。…                    ← the user ("Let's make it changeable from the front end")

● 2 background agents launched (↓ to manage)                       ← launch block
   ├ Explore (Map generation request flow)
   └ spec (Research Agents API model support)

● フロントエンドでモデルと推論の強さを選べるようにする作業を始めました。…  ← the lead answers now ("I've started…")

✻ Waiting for 2 background agents to finish                        ← the turn is over, the work is not
────────────────────────────────────────────
 Bash command · from the spec agent                                ← a helper asks
   curl -sL https://developers.openai.com/llms.txt | grep …
 Ask rule Bash(curl:*) overrides auto mode for this command.       ← why it asks
 Do you want to proceed?   ❯ 1. Yes   2. No
 Esc to cancel · Tab to amend
```

Read from the binary: `N background agents launched` + the chord hint `(↓ to manage)`, becoming
`N agents finished`; `Running in the background (↓ to manage)`; `No background agents running`;
`Press ctrl+x ctrl+k again to stop background agents`; `Background agent "…" was stopped by the
user.`; `Interrupted · What should Claude do instead?`; `Ask rule … overrides auto mode for this
…`. `Waiting for N background agents to finish`, `Worked for …` and `Tab to amend` are
assembled at runtime and were read **from the screenshot only**.

## 1. Requirements

| ID | Requirement | From | Surface |
|---|---|---|---|
| R1 | The lead may **start helpers** for separable sub-questions, each with a short title | screenshot | all |
| R2 | The start is **shown as one block**: count, then `name (title)` per helper | screenshot | console, desktop |
| R3 | The lead's turn **ends when the lead ends**; helpers keep working and the user may keep talking | screenshot | all |
| R4 | While helpers run and no turn does, that is **visible** | screenshot | all |
| R5 | A helper's result **returns to the lead**, which carries on by itself | screenshot | all |
| R6 | Helpers can be **listed, opened and stopped** — one or all | screenshot, Orca `backgroundTasks.stop/stopAll` | console, desktop |
| R7 | A helper's permission request is **asked of the user, named by who asks, with why** | screenshot, Orca `approval.reason/askRule` | all |
| R8 | Every turn says **how long and when** it ended | screenshot, Orca `status.workedFor` | console, desktop |
| R9 | **Accounts' remaining usage and reset times** are always visible | Orca status bar | console, desktop |
| R10 | A thread's row says **what it is doing now**, for how long, with **a child row per helper** | Orca sidebar | desktop, Orca |
| R11 | One screen answers **what needs me**: needs you / working / idle / done, across projects, with unread and clear-done | Orca Activity, dashboard | desktop |
| R12 | A thread's timeline shows **the work, not only the words**: files changed, plan progress, context use, who answered a prompt and where | Orca native chat | desktop |
| R13 | **Notifications** for a prompt, an unviewed end, a failure; a count in the dock | Orca tray, `desktop-app-design.md` §6.9 | desktop |
| R14 | The composer shows the **route as pills** (agent, model, effort), Auto first | Orca composer | desktop |
| R15 | Threads can be **searched** and jumped to | Orca session history, jump palette | desktop |
| R16 | Inside Orca, Orochi's pane is **reported correctly** — by Orochi, not by whichever of its sessions spoke last | Orca's hooks (§8.1) | Orca |
| R17 | Tool rows carry their **result line**; consecutive reads **fold** | screenshot | console |

## 2. Decisions (2026-09-29)

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | Who starts a background seat | **The lead, through `start_agent`; Orochi gates and routes it.** Orochi's own companion seats (`roles::seats`) stay as they are | The one who read the request divides it; Orochi still owns every constraint (§10) |
| D2 | A read-only seat's `execute` / `other` request | **Asked of the user, always** — even under *always allow*. `edit` / `delete` / `move` stay refused at the source | Research needs `curl`, `rg`, `git log`; a read-only seat still cannot write without a yes to that command |
| D3 | How results return | **One completion turn per result, coalesced while the lead is busy** | Progress is visible; a burst of finishes costs one resumed session |
| D4 | Leaving with helpers running | **Ask once**: a second Ctrl-D stops them and exits. `/new` stops them with a line. A headless host stays until they end | A helper whose lead is gone has no one to report to |
| D5 | Helper cap | **`roles::MAX_SEATS` (6) live seats per thread**, companions included; no new setting | Nothing measured yet to set a different number from |
| D6 | What Orca is told | **State, route, running tool, the permission text, seats with titles; never the prompt or a reply** | Orca stores what it receives and pairs with a phone; a title is already on screen |
| D7 | Orca's hook variables in launched agents | **Neutralized** (§8.2) | Otherwise a classifier's session can mark the pane done mid-turn |
| D8 | Where the desktop's "what needs me" lives | **An Activity screen in the left rail**, replacing mission control's intended form | Orca's Activity is the benchmark; the window has no cross-thread screen today (§3) |

## 3. Where Orochi stands

| Area | Today | Gap |
|---|---|---|
| Seats | Derived from the descriptor (`roles::seats`), started with the lead, all given the user's text; the lead is told "never start another agent" (`chat/mod.rs` `alongside`) | R1: no delegation, no titles |
| Seat lifetime | The turn's `select!` loop owns them; when the lead ends they get `mailbox::closing_note` and `· stopped, the work is done` | R3, R5 |
| Seat permissions | `read_only` refuses all but read/search/fetch/think in `acp.rs`; any request that still reaches the console is answered `None` (`chat/mod.rs` `aside`) | R7 |
| Status row | Only during a turn (`Screen::set_status`) | R4, R9 |
| Footer | `✻ 16s · agent · model · ↑… ↓… · cache …%` (`Screen::done`) | R8: no clock |
| ↓ | History forward (`press`) | R6 |
| Store | `seats` has no title; `turns.source` ∈ terminal/desktop/stdin/acp; `controls.kind` includes `stop`; `prompts.answered_by` exists; `items` has `context` rows from ACP `usage_update` | Additive columns only (§5) |
| Desktop | Sidebar of project → thread with one mark; timeline; Team / Changes / Plan; Agents (quota gauges), Insights and Settings behind the account menu (`app.js` `PANELS`). No cross-thread screen, although `desktop-app-design.md` §8 lists mission control as implemented under P3 | R9–R15 |
| Orca | Every Claude session Orochi opens runs Orca's hooks with the pane's key (§8.1 O9) | R16 |

## 4. The model

### 4.1 A background seat

A seat whose life is not the lead's turn. It has a **name** (`[a-z0-9-]{1,24}`, unique among the
room's live peers, a `-2` suffix otherwise), a **title** (≤ 80 characters), its **own task**
(≤ 8 KiB), and a **result**: its final reply, bounded to 4 000 characters, which is what returns
to the lead. It is always `read_only: true`, `verify: false`, joined to the mailbox, and routed
by the scheduler like any seat — so `ScoringContext::busy` still spreads it across accounts.

Routing: the **turn's descriptor** (so the learning strata are the task's, not the helper's) at
`difficulty = complexity.eased()` — a helper is asked for a part of the task, which is what
`ScoringContext::difficulty` exists for. No classifier call is made for it.

Its prompt, prepended to the lead's `task`:

> You are `{name}`, working in the background for `{lead}` on one part of what the user asked.
> You can read everything and change nothing: write tools are refused, and a shell command
> runs only if the user approves it. `{lead}` is not waiting on you turn by turn; your final
> reply is what it receives, so end with the result — what you found, where, and what it means
> — in a few short paragraphs. Use `send_message` only for something `{lead}` needs before you
> finish. Never start another agent or another Orochi.

Companion seats (Orochi's `roles::seats`) keep their lifecycle and prompts; they only take the
new rendering (§6.1).

### 4.2 `start_agent`

A fifth tool in the mailbox MCP server (`mailbox::tools`):

```json
{"name": "start_agent",
 "description": "Start a helper that works in the background on one part of this task and reports back to you when it is done. It can read everything and change nothing. Use it for independent investigation you would otherwise do yourself in sequence.",
 "inputSchema": {"type": "object",
   "properties": {"name":  {"type": "string", "pattern": "^[a-z0-9-]{1,24}$"},
                  "title": {"type": "string", "maxLength": 80},
                  "task":  {"type": "string", "maxLength": 8192}},
   "required": ["name", "title", "task"], "additionalProperties": false}}
```

The mailbox server is a separate process (`orochi --internal-mailbox`), so the request goes
through the store it already writes (`activity.sqlite3`, or `mailbox.sqlite3` with
`activity.enabled = false`):

1. The server inserts an `agent_requests` row (§5.2) naming the requesting peer and waits on it,
   polling as `read_messages` waits, for at most **30 s**.
2. The console (or headless host) reads open requests on the tick it already runs for the feed
   (`mail_ticker`, 1 s), decides, starts the seat, and writes `state` and `reason`.
3. The server returns `{"started": "<name>"}` or `{"refused": "<reason>"}`; a timeout returns
   `{"refused": "Orochi did not answer; do the work yourself"}` and marks the row `expired`.

Gates, in order — each refusal is a sentence the lead can repeat to the user:

| Gate | Refusal |
|---|---|
| The requester is the **lead** of a live console or host turn | `only the agent leading a conversation can start helpers` |
| The turn is not read-only (a discussion) | `this conversation changes nothing, so it has no helpers` |
| Live seats in the thread < `roles::MAX_SEATS` | `already {n} agents on this conversation` |
| `roles::worth_seating` for the turn's descriptor | `work like this has cost less than a helper session costs here` |
| A candidate route exists (scheduler) | the scheduler's own unavailability summary, classified kinds only |

`start_agent` is not offered to one-shot runs, `serve`, `collaborate`, advisers or the classifier:
there is nobody to report to. `mailbox::prompt_note` gains one line for the lead describing it;
`alongside`'s "never start another agent" becomes "start helpers only with `start_agent`".

### 4.3 Lifecycle

```
lead turn:   start_agent ×2 ──► reply ──► end_turn ──► footer
                                                      │
background:  explorer ═════════════ done ─┐           │  ✻ Waiting for 2 background agents…
             spec     ══════════════════════════ done ┤
                                                      ▼
completion turn (lead):  "Background agent `explorer` … finished:" + "`spec` … finished:" ──► reply
```

- **The lead's turn ends when the lead ends.** Footer, record and outcome as today.
- **Background seats are owned by the session, not the turn.** `chat::Session` holds a
  `Background` set: the seats' futures (`FuturesUnordered`, borrowing only the session's `'a`
  references and an owned policy `Registry`) and their event receivers. Both the idle loop
  (`Session::serve`) and the turn loop poll it, so a seat progresses whether or not a turn is
  running. It takes **no workspace lock**: it writes nothing, and a lock held between turns
  would lock the user's own next turn out.
- **The user may keep talking.** A user turn while helpers run carries one line after the
  language line: ``Background agents still running: `spec` (Research Agents API model support).
  Their results arrive as their own messages; do not start them again.``
- **A result returns as a completion turn**, text written by Orochi:

  ```
  Background agent `spec` (Research Agents API model support) finished after 3m 12s:
  <result, bounded to 4 000 characters>
  ```

  or `… was stopped by the user.` / `… failed: rate limit` (the classified kind only, as the
  invariant requires). It is queued **behind** a running turn and behind messages the user
  queued; results that arrive while it waits join the same turn. It resumes the lead's session
  like any follow-up, and is recorded with `turns.origin = 'background'`.
- **Esc / Ctrl-C** interrupt the lead's turn only. A helper is stopped explicitly (§6.5) or by
  D4.
- **The console dies:** the supervisor kills every agent it leads (`process.rs`), seats
  included; their rows read *interrupted* from the host's liveness as threads already do.

### 4.4 Learning

| Run | Treated as |
|---|---|
| A background seat | An execution run, read-only, never verified: `PartialSuccess` at best, like today's second seat. Its descriptor is the turn's |
| A completion turn | An ordinary turn of the lead, routed by the conversation's continuation. It **never writes `feedback`** on the turn before it — only what the user does is a verdict — and what the user does after it judges the completion turn, not the helpers |
| An `agent_requests` refusal | Nothing; it is not a run |

## 5. Store

### 5.1 Additive migrations for `activity.sqlite3`

`activity.rs` promises that additive changes do not move `user_version`, but applies `SCHEMA`
only when the version moves. It gains the step telemetry already has (`storage::minor_migrations`):
`activity::minor_migrations`, run on every `open` before `VIEWS`, inside `BEGIN IMMEDIATE`, reading
`pragma_table_xinfo` *inside* the transaction (two opens at once must not both add a column), and
recording what it applied in a one-row table `schema_minor(applied INTEGER)`. `USER_VERSION` stays
2, `VIEW_API` stays 2 — every change below adds a column, a table or a view and removes nothing.

### 5.2 Changes

| Object | Change |
|---|---|
| `seats` | `title TEXT` (≤ 80), `background INTEGER NOT NULL DEFAULT 0`, `origin TEXT` (`'orochi'` \| `'lead'`), `result TEXT` (the bounded result, content, so here and not in telemetry) |
| `turns` | `origin TEXT` — `NULL` for a turn someone wrote, `'background'` for a completion turn. `source` is left alone: its `CHECK` cannot grow without a table rebuild |
| `agent_requests` (new; also in `mailbox.sqlite3`) | `id, project_id, thread_id, requester (peer id), name, title, task, state CHECK IN ('open','started','refused','expired'), reason, seat_id, created_at, answered_at` |
| `controls` | No schema change: `stop` with payload `{"seat": "<id>"}` stops one seat, `{"background": "all"}` all of a thread's background seats. A payload-less `stop` keeps its meaning |
| `prompts` | No schema change: `answered_by` is written as `terminal`, `desktop` or `headless` from every path, which the receipt reads |
| `v_roster` | Adds `title, background, origin, result IS NOT NULL AS reported`, and `elapsed_ms` |
| `v_live_seats` (new) | Live seats across threads, for the sidebar's child rows and Activity: `seat_id, thread_id, project_id, ordinal, role, title, background, state, started_at, agent, model, doing, usage, host_pid, host_start` |
| `v_sidebar` | Adds `doing` (the lead's in-progress tool, as `v_roster.doing` computes it), `background` (live background seats), `heartbeat_at` |
| `v_timeline` | Adds `turn_origin` so a completion turn is drawn as Orochi's |
| `v_open_prompts` | Adds `read_only` of the asking seat, so a card can say why it is asking |

Retention and deletion need nothing new: every new row hangs off a thread and cascades.
`agent_requests.task` is content; it lives only in these two files, never in telemetry.

### 5.3 Status

`Activity::sidebar`'s derivation gains two words, in this order:

| Status | When |
|---|---|
| `needs_you` | an open prompt (unchanged) |
| `working` | a running turn under a live host (unchanged) |
| **`stalled`** | a running turn under a live host whose `heartbeat_at` is older than 30 s — Orca's *no recent update* |
| **`background`** | no running turn, live background seats, live host |
| `queued`, `interrupted`, `failed`, `unread`, `idle` | unchanged |

An older window that does not know a word draws its default mark; nothing breaks.

## 6. The console

### 6.1 Launch block and finish lines (R2)

```
● 2 background agents launched (↓ to manage)
   ├ explorer (Map generation request flow)
   └ spec (Research Agents API model support)
```

Printed when the first `start_agent` of a burst is granted; requests granted within 2 s join the
same block (they arrive one tool call at a time). Rows that have scrolled cannot be rewritten
(`chat/term.rs`), so the block is never updated in place. A later grant prints its own block.
Each end is its own muted line, carrying the route — what Orochi chose:

```
  ⎿ explorer finished · codex · gpt-6-mini · 1m 48s · 18k tokens
  ⎿ spec stopped by the user · claude · opus-5.5 · 2m 02s
```

When the last one ends: `● 2 agents finished`. Companion seats use the same block headed
`● 1 agent beside claude` and end with the turn as today.

### 6.2 Status and waiting rows (R4)

During a turn the status row gains ` · 2 in background`. With no turn running and background
seats live, the row above the input reads:

```
✻ Waiting for 2 background agents to finish · ↓ to manage
```

It is pinned, not transcript: `chat/term.rs` frees and clears it when the count reaches zero,
under the rules the pinned area already follows.

### 6.3 Footer (R8)

```
✻ Worked for 16s · done 23:54 · claude · opus-5.5 · ↑ 12k ↓ 2.1k · cache 93%
```

Local time, 24-hour unless the locale's `LC_TIME` asks for 12-hour.

### 6.4 Accounts (R9)

At an idle prompt with nothing else to show, the status row carries the freshest quota snapshot
per configured agent (`storage::quota_snapshots`, only while `valid_until` holds), its highest
window and reset:

```
claude 13% · 1h 11m │ codex 99% · 21h 1m │ gemini cooling 4m
```

≥ 90 % is drawn in `WARN`; a cooldown from `RuntimeState` replaces the meter. The console never
probes for this; an agent with no fresh snapshot is left out, not shown as zero.

### 6.5 Manager (R6)

**↓** opens it when history has nothing newer to recall and a background seat is live; otherwise
↓ is what it is today. An overlay in the permission panel's rows:

```
────────────────────────────────────────────
 Background agents
 ❯ spec      working · 3m 02s · Fetch developers.openai.com/…   claude · opus-5.5
   explorer  done    · 1m 48s                                    codex · gpt-6-mini
 Enter to view · x to stop · ↑↓ to navigate · Esc to close
```

- **Enter** shows the seat's lane — its tool rows and reply as the transcript would draw them,
  replayed from `v_timeline` by `seat_id`, live while it runs. Esc returns. The lane never enters
  the transcript.
- **x** stops one: `Background agent "spec" was stopped by the user.`
- **Ctrl-X Ctrl-K** twice stops all (`Press ctrl+x ctrl+k again to stop background agents`;
  `No background agents running`).

### 6.6 Permission queue (R7)

Every seat's prompts share one FIFO; one overlay at a time, the lead's first when both wait.

```
────────────────────────────────────────────
 Bash command · from the spec agent
   curl -sL https://developers.openai.com/llms.txt | grep -i -E "agents-api"
 spec is read only · this command runs in your working tree
 ❯ 1. Yes
   2. Yes, and don't ask again this session
   3. No, and say what to do instead (esc)
```

- Reason line, Orochi's words: `approvals are set to ask (shift+tab)`, `<seat> is read only ·
  this command runs in your working tree`, or the agent's own mode name.
- For a read-only seat, `acp.rs` forwards `execute` / `other` as a `Permission` event carrying
  `read_only: true` instead of refusing it, **regardless of the approval mode** (D2); the
  console's `aside` sends it to the queue instead of answering `None`.
- *Don't ask again* is scoped to that seat's session. **No** refuses the call and hands the typed
  text to **that seat**; the lead's turn is untouched. `Interrupted · What should Claude do
  instead?` stays the lead's line.

### 6.7 Tool rows (R17)

A finished row gains a `⎿` result line where the agent reported output (`ToolState::output`,
one line, fitted: `Received 3.7 KB (200 OK)`). Consecutive finished `read` / `search` rows fold
into `Searched for 1 pattern, read 3 files` — only while the rows are still the last printed,
for the reason a running row is replaced only while it is last.

### 6.8 Keys

| Key | Idle, helpers live | During a turn |
|---|---|---|
| ↓ | Manager (when history has nothing newer) | unchanged |
| Esc / Ctrl-C | unchanged (never touch helpers) | interrupt the lead only |
| Ctrl-X Ctrl-K ×2 | stop all helpers | stop all helpers |
| Ctrl-D ×2 | first press: `2 background agents are running · press Ctrl-D again to stop them and exit` | unchanged |

### 6.9 Headless host

`orochi host` maps the new controls as it maps the others: `stop {"seat"}` / `stop
{"background":"all"}` → the manager's actions; a host counts live background seats as not idle.
Helper prompts become `prompts` rows (`Answerer::Store`), attributed through `attempt → seat`.

## 7. The desktop app

### 7.1 Layout

```
┌──────────────────────┬──────────────────────────────────────────┬──────────────────────┐
│ ◎ Activity        2  │ game-sns ▸ Agent API モデル               │ Team │Changes│ Plan  │
│ + New thread         │ ──────────────────────────────────────── │ ● lead   claude      │
│ ▾ game-sns        2  │  you  Make the model selectable…          │ ◌ spec   codex  RO   │
│  ◉ Agent API モデル 3m│  ⟡ claude · opus-5.5 · high               │   Fetch developers…  │
│    Bash: curl -sL…   │  ┌ 2 background agents ───────── Stop all┐│ ◌ explorer codex RO  │
│    ├ ◌ spec  Resea… now│ │ ◌ spec      working 3m · 12k   Stop  ││                      │
│    └ ◌ explorer  … now│ │ ✓ explorer  done 1m 48s · 18k  Open  ││                      │
│  ✓ Fix flaky… 22h    │  └──────────────────────────────────────┘│                      │
│ ▸ ficchat            │  I've started two investigations…         │                      │
│                      │  ✻ Worked for 16s · done 23:54            │                      │
│                      │  ┌ Bash · from spec ────────────────────┐ │                      │
│                      │  │ curl -sL https://developers.openai… │ │                      │
│                      │  │ spec is read only · runs in your tree│ │                      │
│                      │  │ [Allow once] [Always] [No]           │ │                      │
│                      │  └──────────────────────────────────────┘ │                      │
│                      │ [claude ▾][opus-5.5 ▾][high ▾]  Message…  │                      │
├──────────────────────┴──────────────────────────────────────────┴──────────────────────┤
│ claude ▰▰▱▱ 13% · 1h 11m   codex ▰▰▰▰ 99% · 21h 1m   ⟳          1 working · 1 needs you │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 7.2 Status bar (R9)

A new bottom bar across the window, always visible. Left: per configured agent, its highest
quota window as a meter with percent and reset (`v_quota`), a cooldown in its place when
`v_runtime` has one, ≥ 90 % in the warning colour; a refresh button running `orochi quota
refresh`. Right: counts from the sidebar rows — `n working · n needs you · n in background` —
each opening Activity filtered to it. Clicking an agent opens today's Agents screen. Tauri
command `status_bar` returns both halves in one call, polled with `changed` like everything else.

### 7.3 Thread row (R10)

Second line `doing` while working; right edge the time in the current state; a child row per
live seat from `v_live_seats` — mark, name, title, elapsed — folded beyond three
(`+2 more`). Marks for the new statuses: `background` → a hollow spinner, `stalled` → a clock
with a slash. The project header's count keeps its meaning (`!n` needs you, else working).

### 7.4 Activity (R11, D8)

A left-rail entry above *New thread*, with the needs-you count. Four buckets — **Needs you /
Working / Background / Done** (idle and done together, most recent first) — each thread a row as
in §7.3 plus its project, answerable in place when it has an open prompt (the card of §7.6).
Filters: project, agent, status, unread only. *Mark all read* (writes `ui_state.last_seen_item`
through `seen`), *Clear done* — hides done threads from Activity until they change, kept in
`localStorage` as a per-viewer convenience, with an Undo toast. Read entirely from `sidebar`,
`live_seats` and `open_prompts`; no new store object.

### 7.5 Timeline (R12)

| Element | From | Draw |
|---|---|---|
| Turn footer | turn `started_at`/`ended_at`, route item, usage | `✻ Worked for 16s · done 23:54 · claude · opus-5.5 · ↑12k ↓2.1k` |
| Seats block | `v_live_seats` / `v_roster` for the turn's background seats | Heading `2 background agents` with **Stop all**; a row per seat: mark, name, title, state, elapsed, tokens, **Stop** / **Open**. Open shows the seat's lane in the Team pane |
| Completion turn | `turn_origin = 'background'` | Headed `from the helpers` in the muted voice, not `you` |
| Files changed | `v_turn_files` for the turn | `3 files changed` → Changes, *Last turn* |
| Plan progress | the attempt's `plan` item | `2 of 5 done`; the checklist stays in Plan |
| Context | the latest `context` item | `context 81k / 200k (40%)`, labeled as the agent's report; absent when the agent sends none |

### 7.6 Approval card (R7)

Header `<tool> · from <role>` (lead or seat); body the command or diff as today; a reason line
(`read only · runs in your working tree`, `approvals: ask`); buttons as today. Once answered, the
card collapses to a receipt: `Allowed once · in the terminal` / `Denied · here`
(`prompts.answered_by`).

### 7.7 Notifications (R13)

From the change feed while the window is unfocused: a prompt opened (actions Allow / Deny), a
turn ended unviewed, a turn failed, a background seat finished. Dock badge = open prompts. A
sound, off by default, chosen in Settings. Tauri's notification plugin; nothing new in the store.

### 7.8 Composer pills (R14)

`[Auto ▾]` by default; choosing pins agent, then model, then effort, each a pill that reads
`default` or `not reported` where the agent did not say — the values `threads.overrides` already
holds and the Shift-Tab approvals chip beside them. This is §6.6 of the desktop design, built.

### 7.9 Search and ⌘K (R15)

An FTS5 table over `items.text` (kinds `user`, `agent_message`), kept by triggers, added by
`minor_migrations`, deleted with its thread. ⌘K opens a palette of threads by title, then by
content, with project chips. Last, because it is the only item that adds an index that grows.

### 7.10 Tauri commands

| Command | New / changed |
|---|---|
| `status_bar` | new: quota + runtime + counts |
| `live_seats` | new: `v_live_seats` |
| `stop_seat` | new: writes a `stop` control with `{"seat"}` or `{"background":"all"}` |
| `search` | new (last phase) |
| `sidebar`, `thread`, `open_prompts` | read the added columns |

## 8. Orochi inside Orca: the host

### 8.1 How Orca learns what a pane is doing

Read on 2026-09-29 from Orca 1.4.215; nothing here was checked by running Orca against Orochi.

| # | Fact | Where |
|---|---|---|
| O1 | Orca registers a command hook on 13 events and the `statusLine` in the user's `~/.claude/settings.json` | that file |
| O2 | The hook posts to `127.0.0.1:$ORCA_AGENT_HOOK_PORT/hook/claude` with a token, keyed by `ORCA_PANE_KEY`; with port, token or pane key empty it exits (spooling beside `$ORCA_AGENT_HOOK_ENDPOINT` when readable) | `~/.orca/agent-hooks/claude-hook.sh` |
| O3 | A pane's child rows come from `SubagentStart`/`SubagentStop` and a `background_tasks` inventory | main: `claudeSubagentRosterByPaneKey` |
| O4 | The usage footer comes from the statusline's `rate_limits`; Codex accounts are Orca's own (`CODEX_HOME` = Orca's runtime home, with its own `hooks.json`) | `claude-statusline.sh`, pane environment |
| O5 | Hook routes exist for a fixed list of 21 agents; Orochi is not one | main |
| O6 | **Any program in an Orca terminal can report itself** with `ESC ] 9999 ; <json> BEL`, stripped from the display | main: prefix `\x1B]9999;` |
| O7 | Payload: `state` ∈ working/blocked/waiting/done (required), `agentType` ≤40, `model` ≤120, `toolName` ≤60, `toolInput` ≤160, `interactivePrompt` ≤16 000, `lastAssistantMessage` ≤8 000, `interrupted`, `turnCompletedAt`, `workingMode: "monitoring"`, `subagents` ≤32 × `{id ≤64, state ∈ working/blocked/waiting/idle/unverifiable, startedAt, agentType, model, description ≤160}`; ≤4 096 structural tokens, depth ≤16 | main: `fre`, `lre`, `sre` |
| O8 | While a pane's hook-derived status is Claude with working subagents, OSC reports are ignored | main: `ingestTerminalStatus` |
| O9 | `claude-agent-acp` 0.77.0 loads `settingSources: ["user","project","local"]`, so Orca's hooks run in every Claude session Orochi opens; Orochi passes its environment through unchanged | adapter `dist/acp-agent.js:6066`; no `env_remove` in `src/` |

So today every Claude session Orochi opens in an Orca pane — lead, seats, classifier, advisers —
reports itself as *the* Claude of that pane, and a classifier's `Stop` can mark the pane done
mid-turn (inferred from the code, not observed).

### 8.2 Neutralizing the hooks (R16, D7)

Every agent launch sets `ORCA_AGENT_HOOK_PORT`, `ORCA_AGENT_HOOK_TOKEN` and
`ORCA_AGENT_HOOK_ENDPOINT` to the **empty string** when they are set in Orochi's environment.
Empty rather than removed, because it goes through the one place every launch passes —
`AcpAgentConfig::envs` in `acp::Client` — and O2's scripts test `-z`, so empty exits them
without posting or spooling. `CODEX_HOME` and the rest of `ORCA_*` stay: the account Orca gave
the pane is the user's choice, and an agent running `orca` commands is not a status report.
Adviser and classifier sessions (`start_isolated`) get the same treatment.

### 8.3 Reporting over OSC 9999 (R16)

A new module, `src/pane.rs` (the report and the hook silencing, used by `acp.rs` and `quota_sources.rs` too), driven by the console's `View`. It writes one sequence when the
state it would report **changes**, at most every 500 ms, never per streamed token, and only when
`ORCA_PANE_KEY` is set, stdout is a terminal, and `console.host_status` is on (a new
`[console]` section, default `true`, `deny_unknown_fields`, a `Default` entry and a
`Config::validate` check for nothing — it is a bool).

| Field | Value |
|---|---|
| `state` | `working` during a turn; `working` + `workingMode: "monitoring"` when only background seats run; `blocked` while a permission prompt waits; `waiting` while the console waits on a question it asked (team dialog, MCP approval); `done` at an idle prompt |
| `agentType` | `"orochi"` |
| `model` | `agent · model` of the lead |
| `toolName` / `toolInput` | the lead's running tool, as the transcript labels it |
| `interactivePrompt` | the permission panel's text, fitted |
| `subagents` | every live seat: `id` = name, `state` (`working`; `blocked` while its prompt waits; `idle` when done and not yet reported), `startedAt` ms, `agentType` = its agent, `model`, `description` = title |
| `turnCompletedAt`, `interrupted` | at a turn's end |

Never sent (D6): `prompt`, `lastAssistantMessage`. `orochi host`, `serve`, pipes and tests never
emit it. A manual check, in any Orca terminal:

```sh
printf '\033]9999;{"state":"working","agentType":"orochi","model":"claude · opus-5.5","toolName":"Bash","toolInput":"cargo test","subagents":[{"id":"spec","state":"working","startedAt":%s000,"agentType":"codex","description":"Research Agents API model support"}]}\007' "$(date +%s)"
```

Orochi does not adopt Orca's orchestration (`orca orchestration`); the two coexist, and an Orochi
seat may run `orca` commands like any shell command, under the same permission rules. Orca's
usage footer will not show Orochi's accounts (O4); the console's own accounts row (§6.4) covers
that inside Orca too.

## 9. Configuration

| Key | Default | Meaning |
|---|---|---|
| `console.host_status` | `true` | §8.3 |

Nothing else: the cap is `roles::MAX_SEATS` (D5), the timeouts are constants
(`start_agent` 30 s, stalled 30 s, OSC 500 ms), and every surface reads the same rows.

## 10. Invariants

| Invariant | How it holds |
|---|---|
| Telemetry is content-free | Titles, tasks and results live in `activity.sqlite3` (or `mailbox.sqlite3`) only; telemetry records each seat as any seat |
| Constraints bound every smart layer | `start_agent` is a request Orochi gates and routes; the lead cannot choose the model, pass a gate, or exceed the cap |
| A second seat cannot write | Edits refused at the source, as today. A command runs only on the user's yes to that command, under every approval mode (D2), said on the card |
| Feedback is the user's | A completion turn never writes `feedback`; background seats are never verified |
| `end_turn` alone is not success | Unchanged |
| Nothing in the stores is transmitted | The OSC report is written from the live console, not read from a store, and carries what the screen shows (D6), to the terminal the user opened |
| The target repository is never trusted | Nothing here reads the repository as configuration |
| Records written by older versions deserialize | Every column is nullable or defaulted; `USER_VERSION` and `VIEW_API` unchanged (§5.1) |

## 11. Tests

| Area | Test |
|---|---|
| Store | `tests/core.rs`: `minor_migrations` adds each column once under two concurrent opens; an older schema opens; byte-level check that a title, task and result never reach `telemetry.sqlite3` |
| `start_agent` | `tests/mailbox.rs`: each gate's refusal; a granted request starts a seat; a 30 s timeout returns `expired` |
| Lifecycle | `tests/cli_e2e.rs` with mock agents: a lead that calls `start_agent` twice and ends; seats finishing out of order; one completion turn when both finish during a lead turn, two when apart; stop one, stop all, Ctrl-D twice |
| Console rendering | `tests/test_chat_terminal.py` through `tests/screen.py`: launch block, finish lines, waiting row appearing and freeing its row, manager overlay, a helper's prompt queued behind the lead's, footer clock |
| Permissions | `acp.rs` unit: a read-only seat's `execute` is forwarded with `read_only`, `edit` still refused, under `Allow` too |
| Learning | `tests/adaptive.rs`: a completion turn writes no feedback; a background seat's record carries the turn's descriptor |
| Orca | unit: the payload's limits (O7) and that `prompt` / `lastAssistantMessage` never appear; the sequence is written only with `ORCA_PANE_KEY` and a terminal; launched agents see the three variables empty. One manual run in Orca, recorded in a dated `real-validation-*.md` |
| Desktop | Rust commands against fixture stores; `desktop/tests/ui.test.mjs` over recorded view output for the status bar, child rows, Activity buckets, seats block, receipt; a look at `preview.html` |

## 12. Phases

| Phase | Delivers | Depends on |
|---|---|---|
| **S0 Orca** ✅ | §8.2 and §8.3 with today's seats | — (fixes a wrong report that exists now) |
| **S1 Store** ✅ | §5: additive migration, columns, `agent_requests`, views, status words | — |
| **S2 Console rendering** ✅ | Launch block for companion seats, finish lines, footer clock, accounts row. Tool result lines already existed | S1 for titles |
| **S3 Desktop, reading** ✅ | Status bar, thread rows with `doing` and child rows, Activity, turn footer, files-changed line, context line. Receipts not built | S1 |
| **S4 Background seats** ✅ | §4 whole: `start_agent`, session-owned seats, completion turns, waiting row, the prompt changes | S1 |
| **S5 Control** ✅ | Manager, stop one / all (console, host controls and `stop_seat`), shared permission queue with D2, seats block with Stop in the window, the card naming the seat | S4 |
| **S6 Desktop, rest** ✅ | Notifications and dock badge, composer pills (with `/model` in the console), search and ⌘K, lookup folding in the console | S3 |
| **S7 Validation** | Real runs on both adapters: does a real lead delegate sensibly, do completion turns read well, does Orca draw the pane — recorded in `real-validation-*.md`; nothing above is claimed verified until then | S4, S0 |

## 13. Risks and what is unverified

| Risk | Handling |
|---|---|
| A lead delegates what it should do itself, or delegates too much | The gates (§4.2), the cap, and the tool's description; measured in S7 before any default changes |
| Completion turns cost a resumed session each | Coalescing (D3); `Usage::weighted` of completion turns visible in Insights under their own trigger |
| A read-only seat's approved command writes | Said on the card; the user decides per command; never auto-allowed |
| Helpers read a tree another Orochi process is writing (no lock between turns) | They are read-only and report what they saw; the lead verifies in its own turn |
| Orca draws `agentType: "orochi"` without an icon, or O8 still suppresses OSC | **Unverified**; the manual check in §8.3 and S7 |
| Whether `codex-acp` runs Codex hooks from Orca's `CODEX_HOME` | **Unverified**; neutralizing the variables covers it either way |
| `Waiting for…`, `Worked for…` wording | Taken from the screenshot, not from Claude Code's binary |

## 14. Where the implementation departed (2026-09-30)

| Designed | Built | Why |
|---|---|---|
| `turns.trigger` | `turns.origin` | `TRIGGER` is an SQL keyword |
| `activity::minor_migrations` recording into `schema_minor` | `activity::added`: reads `pragma_table_xinfo`, then again inside `BEGIN IMMEDIATE`, and adds what is missing | Nothing needed recording: an up-to-date store costs one read and no write lock |
| A `stalled` status from `hosts.heartbeat_at` | Not built | Only a headless host beats; a terminal's thread would read *stalled* on every long tool call. A dead host already reads *interrupted* |
| `start_agent` gated per request by "is the requester the lead" | Only the lead's session is **offered** the tool (`mailbox::delegate` guard → `SessionPeer::delegate` → a `delegate` argument to its mailbox server), and a session not offered it cannot call it | A tool a seat cannot use is not listed to it at all |
| `src/chat/pane.rs` | `src/pane.rs` | The hook silencing is used by `acp.rs` and `quota_sources.rs`, not only the console |
| A route line per helper when it is routed | The route goes on its finish line only; grants within 1.5 s share one launch block | The lead asks one call at a time, so a route line per grant split the block Claude Code shows as one |
| Every helper result returns as a completion turn | A helper **the user stopped** returns nothing on its own: its notice rides with the next message (user's or another completion) | Telling the lead "it was stopped" is not worth a session of its own |
| Manager Enter: the seat's lane replayed from `v_timeline` | What the helper has said so far, from memory | The lane view needs the timeline reader in the console; the reply is what a person looks for there |
| No on a helper's question hands the typed text to that seat | No refuses the one call | Typed-instead text for a non-lead seat has no path yet |
| Footer clock follows `LC_TIME` | 24-hour always | The existing mailbox clock is 24-hour; one clock in one transcript |
| Composer pills write `threads.overrides` directly | The window writes the overrides row **and** a `reroute` control carrying the route, which the host runs as `/model` — a console command added for it, after Claude Code's | `controls.kind` cannot grow without a table rebuild, and the running session holds its route in memory; a command is the one path both a person and a client take |
| Effort pill in the agent's own words | The canonical rungs (`low` … `xhigh`), cycled | The window never discovers an agent; `effort.rs` translates a canonical rung for the agent it reaches |
| Notifications through the page's own permission | A `notify` command in Rust (`tauri-plugin-notification`), and `set_badge_count` for the dock | The window declares no capabilities; a call from Rust needs none |
| Search over `items.text` | An FTS5 table over the user's messages and **finished** replies, kept by triggers | A reply indexed on every streamed chunk would rewrite the index dozens of times a second |
| D3: every report returns as its own completion turn when the lead is idle | A report is held while other helpers still run, up to `background::HOLD` (5 minutes) | Measured 2026-09-30: a real lead spent a whole turn (30,355 tokens) saying it was still waiting for the other helper ([real-validation-20260930.md](real-validation-20260930.md)) |
| Grants within 1.5 s share a launch block | The block is printed when the lead starts to answer; 20 s is only a fallback | A real lead's calls came seconds apart, each with its own review |
| A read-only seat's `execute`/`other` request is asked about | Also a request with no `kind` | codex-acp asks by call id alone; a missing kind was being taken for a read |
| Closed input with helpers running | The console keeps going until they end and their results have been handed back, then leaves | A piped session would otherwise drop what the lead asked for |
