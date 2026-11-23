# Resident Agents: Design

Written: 2026-11-21. **Status (2026-11-21): built against the mock agent and the mock OneSignal; nothing
has run on a real agent CLI, a real phone or a real OneSignal app. §22 says what was built, where it
differs from the design, and what is not built.** Everything in §2 ("Where Orochi stands") was read from the code at `dd1c665` and is cited.
Every number marked *Orochi's* is a starting value chosen here, not a measurement.

One goal: **an agent that is there when nobody is asking** — it wakes on a schedule, on its
own request or on a change it was told to watch, does the work, keeps its own notes, asks the
user only for what it may not do alone, and reaches the user wherever they are. The user's
words (2026-11-21): a feature like OpenAI's Dots; Meta's Muse and OpenClaw are the same idea;
"organise the requirements properly and design it". Then, the same day: the recommendations
in §20 are accepted; phone notifications go through **OneSignal**; and the **desktop app**
must be where the settings are managed and where the state of the work and the notifications
are seen (§14).

The working name is **resident** (the agent) and **routine** (one reason it wakes). "Dot" is
OpenAI's product name and is not used for anything in Orochi.

## 0. Sources

| Source | Gives | Where |
|---|---|---|
| [Getting started with your dot](https://help.openai.com/en/articles/20001530-getting-started-with-your-dot) (OpenAI Help; only search snippets were read) | Scheduled and recurring tasks under *Scheduled*, memory, pause, reset | §1 |
| [TechCrunch, 2026-09-29](https://techcrunch.com/2026/09/29/openai-launches-dots-its-bubbly-agentic-avatar/), [SEJ](https://www.searchenginejournal.com/openai-dots-read-only-proactive-research/591565/), [redreamality](https://redreamality.com/blog/openai-dots-always-on-agent-engineering/), [BankInfoSecurity](https://www.bankinfosecurity.com/openai-dots-pushes-always-on-agents-into-enterprise-a-32970) | Dots: always on, own cloud computer and browser, apps, Slack/Teams, approvals and custom rules, proactive read-only research | §1 |
| [lobsterland: Dots vs OpenClaw](https://lobsterland.ing/blog/openclaw-internals/openai-dots-vs-openclaw/), [OpenClaw (Wikipedia)](https://en.wikipedia.org/wiki/OpenClaw), [OpenClaw memory](https://docs.openclaw.ai/concepts/memory) | OpenClaw: self-hosted, any model, heartbeat, messengers, textual memory; its exposed-instance record | §1 |
| [Yahoo Finance on Dots vs Muse](https://finance.yahoo.com/technology/article/openai-debuts-dots-ai-agents-in-challenge-to-metas-popular-muse-agent-174616593.html) | Muse: Meta's consumer errand agent, own app and WhatsApp | §1 |
| `src/chat/host.rs`, `src/activity.rs`, `src/chat/mod.rs`, `src/activity/recorder.rs`, `src/mailbox.rs`, `src/memory.rs`, `desktop/src-tauri/src/{view,remote,main}.rs`, `mobile/` at `dd1c665` | What exists | §2 |
| [manager-design.md](manager-design.md) D1, [real-validation-20260930.md](real-validation-20260930.md) | A cold session costs 20–24k weighted tokens; a completion turn that only said "still waiting" cost 30,355 | §10 |
| [mobile-remote-design.md](mobile-remote-design.md), [personalization-design.md](personalization-design.md), [sandbox-design.md](sandbox-design.md) §15 | The phone link, memory's rules, the "no root, no system change" rule | §2, §13 |
| OneSignal: [Create notification](https://documentation.onesignal.com/reference/create-notification), [Push notification](https://documentation.onesignal.com/reference/push-notification), [Aliases / External ID](https://documentation.onesignal.com/docs/en/aliases.md), [Expo SDK setup](https://documentation.onesignal.com/docs/react-native-expo-sdk-setup), [Mobile SDK reference](https://documentation.onesignal.com/docs/en/mobile-sdk-reference) (search snippets read, full pages not yet) | `POST https://api.onesignal.com/notifications` with `Authorization: Key <REST API key>`; `include_aliases: {"external_id": [...]}` with `target_channel: "push"`, ≤ 2,000 per call; an alias must be registered on the user first (`invalid_aliases` otherwise); the phone sets it with `OneSignal.login(externalId)`; Expo needs a development build, `onesignal-expo-plugin` + `react-native-onesignal`, SDK 53+ with the New Architecture | §13 |
| `desktop/src/app.js` at `dd1c665` | `SETTINGS_PAGES` (`app.js:5110`), the Notifications page (sound only, `app.js:5725`), the window's own `notify()` (`app.js:3143`), the Activity page (`app.js:3716`) | §14 |

The product descriptions are secondary sources from the launch period; OpenAI's own
announcement was not read. They shape the requirements and are cited for nothing else.

## 1. What the three products are

| | Dots (OpenAI, 2026-09-29) | Muse (Meta, 2026-09-08) | OpenClaw (open source) |
|---|---|---|---|
| Where it runs | OpenAI's cloud: a computer and browser per dot | Meta's VM, with a separate guard agent | The user's machine or server |
| Model | OpenAI's own | Meta's own | Any (Claude, GPT, DeepSeek, …) |
| What wakes it | Schedules the user or the dot sets, proactive review of connected data, "pause and wake up to continue" | Errands the user gives | A heartbeat, cron, messages |
| Where you talk to it | ChatGPT, Slack, Teams | Its app, WhatsApp | Messengers (Telegram, WhatsApp, Slack, …) |
| Memory | Its own, from conversations and connected apps; reset deletes everything | — | Plain-text `USER.md` / `MEMORY.md` / daily notes |
| Acting safely | App permissions, custom rules, approvals, auto-review; money and passwords stay with the user | Separate guard agent | Weak: many instances exposed to the internet |

What Orochi takes from them: an agent that wakes itself, schedules its own next wake-up,
remembers in text the user can read, keeps a computer and browser of its own, asks before it
acts outward, and reaches the user on the phone.

What Orochi does not take: a cloud that runs while this machine is off, a model of its own, a
4,000-app catalogue, or anything listening on the internet.

## 2. Where Orochi stands

| Need | Today | Where |
|---|---|---|
| A turn with nobody at a terminal | **Exists.** `orochi host --thread` is the console fed from the store | `src/chat/host.rs` |
| A computer of its own | **Exists.** Incus sandbox per project, stops itself when unused | `src/sandbox.rs` |
| A browser of its own | **Exists.** Servo engine per session; sign-ins handed to the person with `browser_handoff` | `src/browser/engine/` |
| Outside apps | **Exists.** MCP servers, with Orochi holding the OAuth token | `src/mcp.rs`, `src/mcp/oauth.rs` |
| Talk and approve from a phone | **Exists, foreground only.** `send`, `answer`, `ensure_host` over Tailscale | `desktop/src-tauri/src/remote.rs:43-89` |
| A thread outside a repository | **Exists.** `project_for` falls back to the folder when `git rev-parse` fails | `src/activity.rs:1477-1488` |
| A timed wake-up | **Only inside a running host.** `holds.wake` resumes a quota-held request; `mail_ticker` wakes the lead on mail | `src/activity.rs:1878`, `src/chat/mod.rs:680-690, 8549` |
| A turn Orochi writes itself | **Exists.** `turns.origin` = `background`/`merge`/`mail`, never `feedback` | `src/chat/mod.rs:2822`, `src/chat/manager.rs:760` |
| Something that starts a host on a clock | **Missing.** Only the window and the phone start hosts (`view::start_host`); the CLI does not, though `threads send --help` says "otherwise start one" | `desktop/src-tauri/src/view.rs:16`, `src/cli.rs:508` |
| A process alive with no window | **Missing.** No daemon, login item, tray or LaunchAgent anywhere | — |
| Telling a user whose app is closed | **Missing.** Desktop notifications come from the open page; the phone raises local notifications only while it runs ("a killed app receives none") | `desktop/src-tauri/src/main.rs:710-731`, `mobile/apps/app/src/state/notify.tsx` |
| Approval with nobody there | **Waits forever.** An `Ask` in a host opens a `prompts` row and polls every 150 ms with no timeout | `src/activity/recorder.rs:713-745` |
| Facts about the user beyond coding | **Missing.** `USER.md` holds preferences, 1,500 characters | `src/memory.rs`, `src/config.rs:1091-1103` |

Three things found while reading, to be settled before anything here is built (§18):
1. `threads send` promises a host it never starts.
2. A host's stderr is a pipe the **window** drains (`view.rs:28-30`: "the first line written
   after this end went away would kill it"). A host started by a window may therefore die
   when the window closes. Not verified.
3. The host's idle check (`host.rs:186-207`) counts background seats and holds but not an open
   prompt. Not verified whether a turn waiting on a prompt can outlive its host's idle limit.

## 3. Requirements

| ID | Requirement |
|---|---|
| R1 | **It wakes without being asked**: at a time, repeatedly, or once, without a window or a terminal open |
| R2 | **It can wake itself.** During a turn the agent may ask to be woken later ("check again at 15:00") and may set up a recurring task the user asked for in words |
| R3 | **It can watch.** A change in something the user named (a CI run, a feed, a file) wakes it, and an unchanged one costs no agent session |
| R4 | **It remembers in text the user can read and edit**, across wakes, without growing a session without bound |
| R5 | **It has a place of its own**: a folder, and optionally the sandbox and the browser, apart from the user's repositories |
| R6 | **It can also work in a project**: a routine may target an existing project thread ("every morning, look at last night's CI on orochi") |
| R7 | **Unattended means safe by default.** With nobody present it reads; it writes only in its own place or a worktree; anything that leaves the machine or changes the user's tree is asked, and an unanswered question is declined, never waited on for ever |
| R8 | **It reaches the user** when it has something to say or needs an answer, with the window and the phone app closed, through a channel the user chose |
| R9 | **The user can reach it** from the window, the console, `orochi`, the phone, and later from a messenger |
| R10 | **It is bounded.** A daily token budget, a floor on how often it may wake, a cap on routines, and it yields to the user's own work when accounts run low |
| R11 | **Everything it did is visible**: each wake, what it cost, why it was skipped, what it scheduled, what it wrote to its notes |
| R12 | **No new trust.** Telemetry stays content-free; no port is opened; nothing needs root; the routing, learning and memory invariants hold unchanged |

## 4. Non-goals

- Running while this Mac is asleep or off. A wake missed while asleep runs once on waking or is
  skipped (§8.3); a cloud host is a separate project.
- A push service of Orochi's own. Push goes through OneSignal (the user's decision), which
  holds the APNs/FCM credentials; what Orochi needs beyond that is in §13.3.
- A catalogue of integrations. Outside apps are MCP servers, as today.
- Agents of Orochi's own. A resident is whichever agent and model the scheduler picks.
- Teams of residents talking to each other. Each resident is one thread; `message_project`
  (org-design) is the path if this is wanted later.

## 5. Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | What a resident is | **A thread with a home folder and standing instructions**, recorded in a `residents` row. Not a new kind of process | The host already is the console with nobody at it; everything routing, seats, approvals and the window draw comes for free |
| D2 | What wakes it | **A routine**: a schedule, a one-shot time, or a watch, each a row in `activity.sqlite3`. A routine targets any thread, a resident's or a project's | R6 without a second mechanism; routines are content (their instruction is text) so they live beside conversations, not in telemetry |
| D3 | Who keeps time | **One keeper per machine**, `orochi --internal-keeper`, the same shape as the decision-model server and the browser broker: own process group, lock file, exits when no routine is active | A host lives per thread and idles out; a timer has to outlive all of them. One per machine so two windows never fire one routine twice |
| D4 | What a wake is | **A queued turn with `origin = 'routine'`**, written by the keeper with `queue_turn`, which then makes sure a host is running for the thread | The host already claims queued turns; the keeper needs no ACP, no routing and no seats of its own |
| D5 | Session per wake | **A fresh session each wake**, opened with a standing brief (§12.2): the routine, the notes, the last reports. Messages the user types in the resident's thread resume the session as usual | A session resumed for weeks grows without bound and each wake pays for all of it. The brief costs a known, bounded number of characters |
| D6 | Classification | **Once, when the routine is made.** The descriptor is stored with it and passed as `RunOptions::descriptor` on every wake | The same instruction gets the same answer every time; asking the decision model or the classifier 24 times a day about one sentence is waste |
| D7 | Default access | **`read`**: a read-only seat. `home` allows edits inside the resident's folder only. `worktree` allows writes in a thread worktree of a project. Writing to the user's own tree is not an unattended option | R7. Dots, Muse and OpenClaw all go wrong by acting; reading is where the value starts |
| D8 | An unanswered question | **Declined after `routines.ask_secs`**, recorded as such, and the user told. The turn goes on with the refusal | The user is asleep; a question waiting for ever holds a host, a session and possibly a quota slot |
| D9 | Memory | **The resident's own `NOTES.md` in its folder**, agent-written, user-editable, shown to that resident only. `memory.rs` is unchanged | Agent output must not become Orochi's memory (personalization invariant). A file in the agent's own folder is working material, like a repository's content, and a diff of it goes in every report |
| D10 | Agents scheduling | **Through a mailbox tool that is a request Orochi decides**, as `start_agent` is. An agent may make schedules and one-shots, never a watch command; access never above the routine it runs in | A watch runs a command with nobody present: only the user writes one. A request lets Orochi apply the cap, the floor and the budget |
| D11 | Telling the user | **One notifier in the keeper, two sinks**: the Mac's Notification Center (on by default) and **OneSignal push to the paired phones** (the user's choice, 2026-11-21; off until a key is given). Every notice is a row the window and the phone list. OneSignal gets the title only unless the user raises the level | R8 with the phone app closed. OneSignal is a recipient of content the user chose, as a cloud decision model is, so what it sees is the user's setting and starts at the least |
| D14 | Where it is managed | **The desktop app** (the user's requirement, 2026-11-21): residents, routines, the keeper, budgets and notifications are all set there, and the state of every routine and every notice is shown there. The CLI and the phone read the same views | The window is where the user already manages agents, storage and the phone; a setting that only exists in a TOML file is one nobody finds |
| D12 | Waking for a watch | **The change alone wakes**, and optionally the local decision model is asked first whether it needs attention | An OpenClaw-style heartbeat is a cold session each beat (§10); a hash comparison costs nothing |
| D13 | Learning | **A wake is an ordinary execution with `RunRecord::trigger = "routine"`.** It labels learning the same way any run does; nothing is relabeled from the user's reaction in v1 | A routine's report has no checks to verify it, so it is `PartialSuccess` and labels nothing strong; separating it lets `calibrate` report it apart |

## 6. Model

```
resident ──1:1── thread (cwd = <data>/residents/<name>/)
   │                 │
   │                 ├── turns (origin NULL: the user typed; 'routine': a wake)
   │                 └── prompts (expires_at set for a wake)
   └── routines ──1:n── firings ──0:1── turn
          ▲
project thread ──1:n── routines (R6)
```

- **Resident**: a name (`[a-z0-9-]`, ≤ 32), a home folder, a standing instruction ("You are my
  assistant for …"), its access ceiling, its sandbox placement, whether its browser is on, and
  a pause switch. Making one makes its thread. Deleting one deletes the thread (so `reclaim`
  takes the rest), its routines and, after a question, its folder.
- **Routine**: what to do (instruction text), when (§8) or what to watch (§9), the access it
  runs with (≤ its resident's ceiling), its notify policy, who made it (`user` / `agent`), and
  the stored descriptor.
- **Firing**: one row per due time, whether it ran or not: due, started, the turn it queued,
  and a `skipped` reason when it did not run (`paused`, `overlap`, `budget`, `quota`, `late`,
  `gate`, `unchanged`). A skipped firing costs nothing and is still a row, so R11 holds.

## 7. The keeper

### 7.1 Lifecycle

- `orochi --internal-keeper <data>`, dispatched in `main.rs` before clap, like the other
  internal servers. It takes `<data>/keeper/keeper.lock` (exclusive `flock`) and writes
  `keeper.pid`; a second one exits at once.
- **It is started** by `keeper::ensure` wherever Orochi already runs: the desktop on launch,
  `cli::execute` (cheap: one indexed query for an active routine, then the lock), the phone
  server, and the command that makes a routine. It starts in its own session (`setsid`), stdin
  null, stdout/stderr to `<data>/keeper/keeper.log` (rotated at 1 MiB, *Orochi's*), never a
  pipe someone has to keep reading.
- **It ends** when no routine is active and no firing is in flight.
- **Surviving a reboot or logout** is a user LaunchAgent (Q1, decided 2026-11-21), opt-in:
  `~/Library/LaunchAgents/dev.orochi.keeper.plist` running `orochi --internal-keeper <data>`
  at login with `KeepAlive` off (the keeper exits on its own when nothing is active, and the
  next `keeper::ensure` starts it). Written and loaded with `launchctl bootstrap gui/<uid>` by
  *Start at login* or `orochi keeper login on`; `bootout` and deleting the file undo it. No root,
  nothing outside the user's Library. Without it, routines run while some Orochi process has
  run since login.
- **Windows** has no LaunchAgent; *Start at login* is not offered there in this design, and the
  keeper runs while Orochi has been started (compiled by the hand-run CI only).

### 7.2 Loop

1. Read every active routine's `next_at`; sleep until the earliest, at most 30 s, or until
   `PRAGMA data_version` moves (a routine was added or changed).
2. On waking, compare the wall clock with when it expected to wake. A gap larger than 120 s is
   a sleep or a clock jump: apply §8.3 to what fell due inside it.
3. For each routine due: write the firing, run the gates (§10.2) in order, and either record the
   skip or `queue_turn(thread, brief, [], steps, "routine")`, set `turns.origin = 'routine'`, and
   start a host for the thread unless one holds it (`hosts` row with a live heartbeat).
4. Compute the next `next_at` and store it in the same transaction, so a crash between them
   cannot fire a routine twice.
5. Every 5 s, look at the `changes` feed for finished routine turns, open prompts on routine
   turns, and new `@you` mentions on resident threads, and hand them to the notifier (§13).

Starting a host moves from `desktop/src-tauri/src/view.rs` into the CLI crate
(`chat::host::spawn`) so the keeper, the window, the phone and `threads send` share one: in
its own session, stderr to a per-host log file instead of a pipe (fixes §2 items 1 and 2).

## 8. Schedules

### 8.1 Grammar

Written by people and by agents, so it reads like speech and parses strictly:

| Form | Example |
|---|---|
| `at <time>` (once, today or tomorrow) | `at 15:00` |
| `on <date> at <time>` (once) | `on 2026-11-24 at 09:30` |
| `in <n><unit>` (once) | `in 90m`, `in 2h` |
| `daily at <time>[, <time>…]` | `daily at 08:00, 18:00` |
| `weekdays at <time>` / `weekends at <time>` | `weekdays at 09:00` |
| `every <day>[,<day>…] at <time>` | `every mon,thu at 10:00` |
| `every <n><unit>[ from <time> to <time>]` | `every 2h from 09:00 to 19:00` |
| `cron <5 fields>` | `cron 0 8 * * 1-5` |

Times are the machine's local time. The parsed form is stored next to the text, and the
next due time is computed with `libc::localtime_r` / `mktime` (no new dependency): a time
skipped by a DST change runs at the next valid minute, a repeated one runs once. On Windows the
same computation goes through `GetDynamicTimeZoneInformation`; this is compiled only by the
hand-run CI, so it needs care.

### 8.2 Floors

- `routines.min_interval_secs` = 900 (*Orochi's*): no repeating routine more often than every
  15 minutes. A user may lower it to 300 in config. *(Superseded, §23: an agent's routine is a
  proposal the person allows, so the same floor holds for both.)*
- `routines.max_per_thread` = 10 (*Orochi's*); `routines.max_proposals` = 5 proposals waiting per
  thread (*Orochi's*, §23).
- A proposal nobody answers is dropped after `routines.proposal_days` = 7 (*Orochi's*, §23); an
  allowed one does not expire.

### 8.3 Missed while asleep

Each routine has `catch_up` = `once` (default) or `skip`. On waking from a gap, everything that
fell due inside it collapses to **one** firing if the latest of them is within
`routines.max_late_secs` = 21,600 (6 h, *Orochi's*); otherwise each is recorded `late` and the
next one is scheduled normally. A morning brief missed at 08:00 because the lid was shut runs
at 09:10 when it opens; it does not run three times.

## 9. Watches

A watch is a command the **user** wrote, run every `every` (≥ the floor) with nobody present:

```toml
[[routines]]               # or `orochi routines add --watch`
thread = "resident:ci"
watch  = "gh run list --repo MoonTechnica/orochi --limit 5 --json status,conclusion,name"
every  = "15m"
instruction = "A CI run changed. Say what failed and whether it is the same failure as before."
```

- Run as checks are: in the thread's folder, `CI=true`, stdin closed, through the process
  supervisor, timeout `routines.watch_timeout_secs` = 60, output capped at 64 KiB.
- The output's SHA-256 is compared with the last one. The first run records a baseline and
  fires nothing. Unchanged: a firing row `unchanged`, no turn, no tokens.
- Changed: the output goes into the turn's text (content, so `activity.sqlite3` only; the hash
  is all the firing row keeps).
- A command that fails three times in a row pauses the routine and tells the user.
- Resolved through `discovery::locate` and the search path, never a relative `PATH` entry,
  exactly as agents are.

## 10. Cost

### 10.1 Why no heartbeat

A cold session is 20–24k weighted tokens before it does anything (manager-design D1), and a
turn that only said it was still waiting cost 30,355 (real-validation-20260930). A half-hourly
heartbeat is 48 sessions, about **1M weighted tokens a day** spent mostly on saying nothing.
Routines therefore wake only for a time the user or agent chose or for a watch that changed.

### 10.2 Gates, in order

| Gate | Skips with | Rule |
|---|---|---|
| Paused | `paused` | The routine or its resident is paused |
| Overlap | `overlap` | The routine's previous turn has not finished |
| Budget | `budget` | Today's routine runs (telemetry, `trigger = "routine"`, `Usage::weighted()`) reached `routines.daily_tokens` = 300,000 (*Orochi's*). Reaching 80% notifies once |
| Quota | `quota` | Every agent the routine could route to has a stored quota snapshot under `routines.min_quota_left` = 0.2 (*Orochi's*). Snapshots are read, never refreshed, since a probe costs. With no snapshot it runs |
| Decision | `gate` | Watches only, and only with `routines.gate = "decision"` and a decision model chosen: one `noul` question ("does this change need attention, given the instruction?") with the probability at or above `routines.gate_threshold` = 0.7 (*Orochi's*, unmeasured) |

A routine's turn is priced like any run, and the scorer's busy factor already steers it off an
account the user is working on (`mailbox::busy_agents`). A user turn queued on the same thread
goes first: the keeper queues behind, it never interrupts.

## 11. Acting with nobody present

### 11.1 Access levels

| Access | Edits | `execute` / `other` / MCP tools | Where |
|---|---|---|---|
| `read` (default) | Refused at the source (`read_only`) | Asked (§11.2) unless allow-listed | Anywhere the thread is |
| `home` | Allowed only where every `toolCall.locations` path is inside the resident's folder; refused otherwise | Asked unless allow-listed | Resident threads only |
| `worktree` | Allowed, in a thread worktree of the project made for the routine (`worktrees.rs`); the result stays on its branch | Asked unless allow-listed | Project threads only |

Writing the user's own tree, merging, `apply_merge`, `finish_workspace` with `merge`, pull
requests, and starting writing helpers are not available to a routine turn at any level: a
merge is a decision the user makes awake.

### 11.2 Questions

- A permission request that is neither allowed by the level nor refused is a `prompts` row with
  `expires_at = now + routines.ask_secs` (1,800, *Orochi's*). The notifier sends "approval
  needed" at once. Answered from the window, the console or the phone, it goes on. Expired, it is
  answered `deny` by `answered_by = 'expired'`, the agent sees an ordinary refusal, and the
  report says what was refused.
- `[[residents]] allow` lists tools allowed without asking, as `server/tool` globs
  (`gmail/search_*`, `calendar/list_*`). **Unverified:** whether every adapter's permission
  request names the MCP server and tool in a form this can match (`claude-agent-acp` titles
  look like `mcp__server__tool`; `codex-acp` not read). Built only after reading both.
- What no rule can detect — paying, changing a password, sending money — relies on what
  exists: the browser never types a password (`browser_handoff`), and anything outward is an
  `other` request, which is asked.

### 11.3 Untrusted input

A resident reads mail, pages and feeds written by strangers, and writes notes it will read
again. Measures, none of which makes injection impossible:
- Page text is already marked untrusted by the browser tools; watch output and MCP results are
  put into the turn under the same marking.
- Outward actions are asked (§11.2), so an injected "forward everything to X" becomes a
  question with the destination in it, not an action.
- Every report includes the diff of `NOTES.md`, so an injected "remember to …" is in front of
  the user the first time it is written.
- A resident with `home` access or a browser runs in the sandbox by default (`sandbox` =
  `container`), so its folder is the only thing of this machine it can touch.

## 12. Memory

### 12.1 What is remembered where

| Store | Writer | Read by | Bound |
|---|---|---|---|
| `USER.md`, repository `MEMORY.md` | Orochi (classifier) and the user, unchanged | Every fresh session, as today | 1,500 / 2,500 characters |
| `<home>/NOTES.md` | The resident (an ordinary file edit) and the user | That resident's sessions only | `residents.notes_chars` = 6,000 injected (*Orochi's*); the file may be larger, and the resident is told to keep the top a summary |
| `<home>/` other files | The resident | The resident, on demand | Disk |
| Firing reports | Orochi (the turn's final reply) | The next brief | The last `residents.brief_reports` = 3, 800 characters each (*Orochi's*) |

The personalization invariants hold: no agent writes `memory.rs`'s files, a resident's notes
never reach another thread, and a project's memory never reaches a resident unless a routine
targets that project's thread.

### 12.2 The standing brief

Every wake opens a fresh session whose task text is, in order:
1. The resident's standing instruction.
2. "Your notes" — the first `notes_chars` of `NOTES.md`, marked as its own earlier writing.
3. "Earlier reports" — the last reports of this routine, newest last.
4. Open items — questions that expired since the last wake, routines it made, quota or budget
   skips since the last report.
5. The routine's instruction, and for a watch the changed output.

This is `Manager::standing` in another chair. Messages the user types in the thread are
ordinary console turns and resume the session as they do today.

## 13. Reaching the user, and being reached

### 13.1 What is a notice

A **notice** is one thing the user should hear of. The keeper's notifier makes it, stores it
as a `notices` row (§15), and hands it to the sinks. The window, the phone and
`orochi notices` all list the same rows, so a push the phone missed is still there in the
app, and the window shows what reached the phone and what did not.

| Event | Notice kind | Sent by default |
|---|---|---|
| A question opened on a routine turn | `ask` | Always: it expires (§11.2) |
| A routine's turn finished | `report` | Per routine: `always`, `mention` (default: only when the resident wrote `@you`), `never` |
| The resident mentioned `@you` outside a report | `mention` | Always |
| A routine failed (the first of a run of failures; §23), or paused itself (its turn or a watch's command failing three times in a row, an agent-made routine expiring) | `trouble` | Always |
| An agent made a routine (§14.4 shows it) | `scheduled` | Always, once |
| Budget at 80%, budget reached; keeper could not start | `budget` / `keeper` | Once a day each |

The window's own `notify()` (`app.js:3143`) keeps announcing interactive turns as it does
today. To avoid saying one thing twice, the keeper's notices cover what happens on
**routine turns and resident threads**, and the window skips those in its own pass.

### 13.2 Sinks and when each is used

| Sink | Reaches | Used when |
|---|---|---|
| **Mac** — macOS Notification Center | This Mac | No Orochi window is running (the running window shows its own); off in Settings if not wanted |
| **Phone** — OneSignal push | Every paired phone that registered for push | The user has not used this Mac's Orochi window for `notify.away_secs` = 300 (*Orochi's*), or the window is not running, or the kind is `ask` (an expiring question goes to the phone at once) |
| **In-app** — the `notices` row | The window's Activity page and the phone's list | Always |

- *Away* is read from `ui_state.active_at`, which the window writes while it has focus and the
  user is typing or clicking (at most once a minute). No idle-time API of macOS is used.
- **Quiet hours** (`notify.quiet = "23:00-07:00"`, off by default): no push and no Mac
  notification inside them except `ask` if the user ticks *Wake me for approvals*; held
  notices go out as one summary when quiet hours end. An `ask` held through quiet hours
  expires as any other (§11.2) and the summary says so.
- **Grouping**: at most one push per thread per `notify.coalesce_secs` = 60 (*Orochi's*);
  later ones in the window replace the text with "3 updates from …".
- The Mac sink runs `osascript -e 'display notification …'` with the text passed as an
  argument to a fixed script, never interpolated into it. **Unverified:** whether macOS shows a
  notification from a process without an app bundle under the user's settings; if not, the
  keeper asks the desktop app's binary to post it (`Orochi.app --notify`), which has the
  bundle and the permission the window already asked for.

### 13.3 OneSignal

**Who owns what.** A OneSignal *app* is tied to one mobile app build: its **App ID** is built
into the Orochi phone app (`app.json` `extra`) and is public; its **REST API key** can push to
every phone of that app, so it is a secret and is never built into anything.

**Sending** (keeper, `src/notify/onesignal.rs`, through the `reqwest` already in the crate):

```
POST https://api.onesignal.com/notifications
Authorization: Key <REST API key>
{
  "app_id": "<App ID>",
  "target_channel": "push",
  "include_aliases": { "external_id": ["<push id of each paired phone>"] },
  "headings": { "en": "Approval needed" },
  "contents": { "en": "assistant — wants to send an email" },
  "data": { "machine": "<machine_id>", "thread": "<thread id>", "notice": <notice id>, "kind": "ask" }
}
```

- The **push id** is a random 128-bit id the **phone** makes once, registers with
  `OneSignal.login(pushId)`, and hands to each desktop when it pairs (`/pair` gains a
  `push_id` field; `/me` updates it). One id per phone, not per machine, because OneSignal
  binds a device to one external id and a phone may be paired with several Macs. It names
  nobody: never the user's email, name or Apple ID.
- Unpairing a phone (Settings → Orochi mobile app → remove) deletes its push id here, so this
  Mac pushes to it no more. The phone calls `OneSignal.logout()` when the last machine is
  removed.
- `data` carries only ids, so tapping the push opens that machine's thread
  (`mobile/apps/app/src/state/open.ts`) without the push holding any content.
- **What OneSignal sees** follows `notify.level`: `title` (default) — the kind and the
  resident's or project's name ("Approval needed — assistant"); `summary` — plus the first 120
  characters; `full` — the text, up to OneSignal's limit. The level is set in the window,
  with a sentence saying OneSignal and Apple/Google carry it.
- A response naming `invalid_aliases` marks that phone *not registered* (it has not opened the
  app since installing it, or notifications are off on it); other failures retry three times
  over five minutes, then the notice row records `failed` with the error kind (never the
  response body, which can carry ids).
- The key is kept as the decision key is (`key.rs`): `ONESIGNAL_REST_API_KEY` if set, then the
  keychain (service `orochi-notify`, account `onesignal`, written through `security -i` on
  stdin), else a `0600` file. Tests set `notify.keychain = false` and point
  `notify.onesignal_url` at a loopback stand-in.

**On the phone** (`mobile/apps/app`):
- Add `onesignal-expo-plugin` and `react-native-onesignal` (a development build exists already;
  the docs require Expo SDK 53+ with the New Architecture — the app is on SDK 58 and React
  Native 0.88, and whether the current `react-native-onesignal` supports 0.88 has to be read
  before it is added).
- `OneSignal.initialize(appId)` at start; `OneSignal.login(pushId)` after the first pairing;
  the notification permission is asked where the app asks today (`state/notify.tsx`).
- While the app runs, its own local notifications (`expo-notifications`) skip any notice that
  arrives as a push (matched by `data.notice`), so nothing is shown twice. **Unverified:**
  whether `expo-notifications` and OneSignal can share the iOS notification delegate; if they
  cannot, local notifications move onto OneSignal's in-app handler.
- iOS needs an APNs key uploaded to the OneSignal app, and push can only be checked on a
  physical device.

**Distribution.** With the key in the user's own Mac, the arrangement works for the person who
owns the OneSignal app — today, the developer. Shipping the phone app to others means either
each user brings a OneSignal app and a phone build of their own (impractical), or a small relay
holds the key and accepts a push only for a phone id that this machine paired with, signed by
the machine. The relay is not built here; it is open decision Q5 (§20). Nothing in the keeper
changes between the two: `notify.onesignal_url` points at the relay instead of OneSignal.

### 13.4 Being reached (later phase)

A messenger can also be a source: Telegram `getUpdates` long-polling or Slack Socket Mode,
both outbound connections, so no port is opened. Only messages from a configured sender id are
taken; each is `queue_turn` on the resident's thread with source `telegram`/`slack`, and the
final reply goes back on the same messenger at the notify level. Approvals are never taken from
a messenger: a "yes" from a chat app is not tied to the person the way the paired phone is.

## 14. Surfaces

### 14.1 The desktop app

The desktop is where all of this is managed and watched (D14). Every screen below reads the
same views the CLI does (`v_residents`, `v_routines`, `v_firings`, `v_notices`), and every
change goes through the same library calls as `orochi routines` / `orochi resident`, as the
Agents screen and `orochi roster` already share one implementation.

**Sidebar.** A *Residents* group above the projects. Each resident is a row with its state —
`idle`, `working`, `asking` (with the time the question expires), `paused`, `trouble` — and its
next wake ("08:00"). Selecting it opens its thread, which is an ordinary conversation.

**A resident's thread** has a strip above the transcript:
- next wake and the routine it belongs to; *Run now*; *Pause* / *Resume*;
- today's tokens against its share of the budget;
- *Routines*, *Notes* and *Settings* tabs beside the transcript: Notes shows `NOTES.md` with an
  edit button and the last change's diff; Settings holds the standing instruction, access
  ceiling, sandbox, browser and the `allow` list.
Each wake in the transcript opens with a `routine` marker — "⏰ morning brief · 08:00 (ran
09:10, after sleep)" — so the user can tell what they typed from what the clock did.

**Routines screen** (a top-level panel beside Activity and Office) lists every routine on this
machine, resident or project:

| Column | Shows |
|---|---|
| Routine | Name; target (resident or project · thread); `agent` badge for one an agent made, with "expires in 12 days · Keep" |
| When | The schedule in words ("weekdays at 09:00", "watch · every 15 min") |
| Next | The next due time, or why there is none (paused, expired) |
| Last | Result of the last firing: done / asked / failed / skipped with its reason (`budget`, `quota`, `late`, `unchanged`, …) and how long ago |
| Today | Tokens today |
| | Run now · Pause · Edit · Delete |

A row opens its last 20 firings, each linking to the turn it ran. A header line shows the
keeper (running since, or the reason it is not) and the day's tokens against
`routines.daily_tokens`.

**New / edit routine** dialog: target; *When* as text with the next three due times worked
out under it as the user types ("Mon 24 Nov 08:00, Tue 25 Nov 08:00, …"), or *Watch* with
the command, interval and a *Try it* button that runs it once and shows the output; the
instruction; access; notify policy; catch-up. Watch is offered only here and in the CLI, never
to an agent (D10).

**Activity page** (`app.js:3716`) gains a *Notifications* filter: every notice with its kind,
where it went — Mac ✓, phone ✓ / *not registered* / *failed* / *held (quiet hours)* — and the
thread it opens. Reading a notice here marks it read on the phone too.

**Approvals** keep today's prompt UI, with a countdown on a routine's question ("declined in
24 min").

**Office.** A resident at work is a room like any conversation; a resident asleep is not
drawn.

### 14.2 Settings pages

`SETTINGS_PAGES` (`app.js:5110`) gains one page and extends one:

**Residents and routines** (new):
- *Keeper*: state (running since, pid; or stopped and why), *Open log*.
- *Start at login* toggle: installs or removes the LaunchAgent (§7.1, Q1), with the sentence
  "macOS will start Orochi's keeper when you log in, so routines run without the app open.
  Nothing outside your user's Library is changed." Off by default.
- *Budget*: tokens per day; leave accounts with at least N% for your own work; shortest
  interval.
- *Defaults for new residents*: access, sandbox, browser.
- *Agent-made routines*: allowed or not, how many, days before they expire.

**Notifications** (extended; today it holds the sound only, `app.js:5725`):
- *On this Mac*: on/off, sound (the existing toggle).
- *On your phone (OneSignal)*: App ID (filled from the build when known); REST API key (a
  password field; stored in the keychain and shown afterwards as "set · Replace · Remove");
  *Send test*; the paired phones with their state — registered, not registered (open the app
  once), last delivery.
- *What to send*: the kinds of §13.1 as checkboxes; how much OneSignal is shown (title /
  summary / full) with the sentence about who carries it.
- *When*: "Only when I have been away from this Mac for N minutes"; quiet hours with
  *Wake me for approvals*.

### 14.3 Phone

- Push from OneSignal (§13.3); tapping opens the thread.
- *Residents* on Home with the same states as the sidebar; a resident's thread as any thread.
- *Notifications* list (`v_notices`), read state shared with the window.
- Pause / Resume / Run now on a routine: `routine_action` added to the remote whitelist
  (`remote.rs:43-89`). Creating or editing routines stays in the window and the CLI.
- Approvals as today, with the countdown.

### 14.4 CLI, console and mailbox

| Where | What |
|---|---|
| CLI | `orochi resident new <name> [--instruction …] [--access read\|home] [--sandbox …]`, `resident list/show/pause/resume/rm`; `orochi routines add <thread> --when "<schedule>" \| --watch "<cmd>" --every 15m -- "<instruction>"`, `routines list [--thread]`, `routines show <id>`, `routines run/pause/resume/keep/rm <id>`; `orochi notices [--unread]`; `orochi notify test`; `orochi keeper status` |
| Console | `/routines` lists the thread's routines; `/schedule <when> <instruction>` adds one; `⎿ scheduled: daily at 08:00 — morning brief` whenever an agent's request was granted, as `remembered:` is |
| Mailbox (resident and project leads) | `schedule {when, instruction, notify?}`, `list_schedules`, `cancel_schedule {id}`, `pause_schedule {id}`, `resume_schedule {id}` (§23), after the existing tools, each an `agent_requests` row with a `kind` decided on the mail tick |

## 15. Data

All in `activity.sqlite3` (content), added through `ADDED` columns and new tables, so no
existing `CHECK` constraint needs a rebuild:

```sql
CREATE TABLE IF NOT EXISTS residents (
  thread_id TEXT PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
  name TEXT NOT NULL UNIQUE, home TEXT NOT NULL,
  instruction TEXT NOT NULL, access TEXT NOT NULL CHECK (access IN ('read','home')),
  paused INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS routines (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  name TEXT NOT NULL, instruction TEXT NOT NULL,
  schedule TEXT, schedule_parsed TEXT,           -- one of schedule / watch
  watch TEXT, every_secs INTEGER, watch_hash TEXT, watch_failures INTEGER NOT NULL DEFAULT 0,
  access TEXT NOT NULL CHECK (access IN ('read','home','worktree')),
  notify TEXT NOT NULL DEFAULT 'mention' CHECK (notify IN ('always','mention','never')),
  catch_up TEXT NOT NULL DEFAULT 'once' CHECK (catch_up IN ('once','skip')),
  descriptor TEXT NOT NULL,                      -- JSON labels, as classified once
  made_by TEXT NOT NULL CHECK (made_by IN ('user','agent')),
  expires_at INTEGER, paused INTEGER NOT NULL DEFAULT 0,
  next_at INTEGER, last_at INTEGER, created_at INTEGER NOT NULL,
  CHECK ((schedule IS NULL) <> (watch IS NULL)));
CREATE INDEX IF NOT EXISTS routines_due ON routines(next_at) WHERE paused = 0;

CREATE TABLE IF NOT EXISTS firings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  due_at INTEGER NOT NULL, started_at INTEGER,
  turn_id TEXT REFERENCES turns(id) ON DELETE SET NULL,
  skipped TEXT CHECK (skipped IN
    ('paused','overlap','budget','quota','late','gate','unchanged','failed')));

CREATE TABLE IF NOT EXISTS notices (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK (kind IN
    ('ask','report','mention','trouble','scheduled','budget','keeper')),
  thread_id TEXT REFERENCES threads(id) ON DELETE CASCADE,
  routine_id TEXT REFERENCES routines(id) ON DELETE SET NULL,
  prompt_id TEXT REFERENCES prompts(id) ON DELETE SET NULL,
  title TEXT NOT NULL, body TEXT,              -- what Orochi shows; what a sink got follows its level
  mac TEXT,                                    -- sent | skipped_window | held | off
  phone TEXT,                                  -- JSON: {push_id: sent|not_registered|failed|held}
  created_at INTEGER NOT NULL, read_at INTEGER);
CREATE INDEX IF NOT EXISTS notices_unread ON notices(created_at) WHERE read_at IS NULL;
```

- `prompts` gains `expires_at INTEGER` (`ADDED`); `answered_by = 'expired'` needs no schema.
- `turns.origin = 'routine'` (no `CHECK` on that column).
- `ui_state.active_at`, written by the window (§13.2).
- The phone's push id is kept with its pairing in `<data>/remote/` beside the device token
  (`desktop/src-tauri/src/remote.rs`), not in `activity.sqlite3`, and goes when the pairing does.
- Views `v_residents`, `v_routines`, `v_firings` and `v_notices` for the window, the phone and
  the CLI, which read the same ones.
- Telemetry: `RunRecord::trigger: Option<String>` with `#[serde(default)]`, `"routine"` or
  absent. No instruction, schedule text or watch output ever reaches it.
- Retention: firings older than `activity.retention_days` go with the rest; a resident's thread
  is pinned when made, so retention never deletes a resident.

## 16. Config

```toml
[routines]
enabled = true
min_interval_secs = 900
daily_tokens = 300000
min_quota_left = 0.2
ask_secs = 1800
max_late_secs = 21600
max_per_thread = 10
max_proposals = 5
proposal_days = 7
watch_timeout_secs = 60
gate = "off"              # off | decision
gate_threshold = 0.7

[residents]
root = "{data}/residents"   # template like workspaces.root; places.rs gains it
notes_chars = 6000
brief_reports = 3

[[residents.allow]]         # per resident, by name
resident = "assistant"
tools = ["gmail/search_*", "calendar/list_*"]

[notify]                    # §13; every field is set from Settings → Notifications
mac = true
level = "title"             # title | summary | full — what OneSignal is given
away_secs = 300
coalesce_secs = 60
quiet = ""                  # e.g. "23:00-07:00"; empty is off
wake_for_approvals = false
kinds = ["ask", "report", "mention", "trouble", "scheduled", "budget", "keeper"]
keychain = true

[notify.onesignal]
enabled = false             # true once a key is set
app_id = ""                 # public; taken from the phone build when known
url = "https://api.onesignal.com/notifications"   # or a relay (Q5)
api_key_env = ""            # else ONESIGNAL_REST_API_KEY, then the keychain
```

Each field gets its `Default` entry and a `Config::validate` check (ranges, the template,
the quiet-hours form, an `https` or loopback `url`, the floor the user may lower to). The
window writes these through `Config::save`, as the Agents and Storage pages do.

## 17. Invariants

Kept unchanged: content-free telemetry; memory only Orochi's or the user's; the repository never
trusted; constraints bound every smart layer; a verified outcome only from real checks; no
helper worktree outlives its conversation; nothing needs root or changes the system.

New, each with a test:
- **A routine fires at most once per due time**, across keeper restarts and crashes (the firing
  row and the next `next_at` in one transaction).
- **A routine turn writes only what `residents.access` allows** (2026-11-23, the user's decision: align with OpenClaw and Hermes Agent, which reach the machine broadly and rely on approvals). `read` (default) changes nothing; `home` and `full` leave the seats their write tools, each change goes through the thread's approval as in a conversation, and a question nobody answers is declined after `routines.ask_secs`. Routine turns use the conversation's team as it is (roles, `start_agent`); a writing helper on a routine's turn works **in the lead's own folder with no worktree** (`Post::in_place`; an assistant's folder is not a repository, and the user asked for it — 2026-11-23), so the lead is told to divide the work. `routines.approval` answers a routine's questions: `ask` (default, declined after `ask_secs`), `off` (allowed at once; OpenClaw `security=full`, Hermes `off`) and `smart` (allowed when it is a change whose paths all lie inside the resident's folder, asked otherwise; Hermes' `smart` asks a model, this is a rule). **Not built:** a model judging for `smart`; per-routine access; reads wider than the agent's own, with secrets left out.
- **No question on a routine turn outlives `ask_secs`.**
- **Only the user writes a watch command.** An agent's `schedule` request carrying one is
  refused.
- **OneSignal receives nothing without a key the user gave**, never more than `notify.level`
  allows, never a name of the user, and only ids in `data`; a removed pairing receives nothing.
- **Every notice is a row before it is sent**, so what failed to arrive can still be read in the
  window and the app.
- **A resident's notes reach that resident's sessions only.**
- **A skipped firing costs no tokens and is still recorded.**

## 18. Fixes first

| # | Fix | Test |
|---|---|---|
| F1 | `chat::host::spawn` in the CLI crate; `threads send` starts a host when none holds the thread, as its help says | `cli_e2e`: `threads new` + `send` with no window runs the turn against `mock_acp.py` |
| F2 | A spawned host is in its own session and logs to a file, not a pipe | Kill the parent after spawning; the host finishes its turn |
| F3 | An open prompt keeps its host up; expiry closes it | Host idle 60 s, prompt open 90 s: the host stays, then answers `expired` |

## 19. Build order

| Step | Contents | Done when |
|---|---|---|
| K0 | F1–F3 | Tests above pass |
| K1 | Schema, schedule grammar and next-due computation (DST, catch-up), `orochi routines` CLI on project threads, keeper with lock, gates without quota/decision | `tests/routines.rs`: a fake clock (`OROCHI_KEEPER_CLOCK`) drives due times, sleep gaps, DST days and two keepers racing; a firing runs a turn on the mock agent |
| K2 | Residents: folder, thread, brief, `NOTES.md`, `home` access by path, sandbox default | A resident wakes twice and the second brief carries the first report and the notes diff |
| K3 | Expiring questions, `[[residents.allow]]` after the adapters' permission shapes are read | Expired prompt answered `deny`; an allow-listed MCP call is not asked |
| K4 | Notices and the notifier: `notices` table, Mac sink, OneSignal sender (key in the keychain, retry, `invalid_aliases`), away / quiet / coalesce rules; budget and quota gates | `tests/notify.rs` against `tests/fixtures/mock_onesignal.py` on loopback: the request's shape, `level` honoured, nothing sent without a key or to an unpaired id, quiet hours held and summarised |
| K5 | Desktop (§14.1–14.2): sidebar Residents, resident strip and tabs, Routines screen and dialog, Activity's Notifications filter, the Residents-and-routines and Notifications settings pages, `ui_state.active_at` | `desktop/tests` for the new views' shapes; the window run against a mock agent with a routine created, run, paused and its notices listed (screenshots in the PR) |
| K6 | Phone (§13.3, §14.3): OneSignal SDK, push id at pairing, deep link from `data`, Residents and Notifications lists, `routine_action` | `tests/remote.rs` for `push_id` and the whitelist; the app's tests for dedupe by `data.notice`; push itself on a physical iPhone |
| K7 | Mailbox `schedule` tools, agent-made caps and expiry, console `/routines` and `scheduled:` line | `mock_acp.py` asks for a schedule; it is granted, capped, refused below 3,600 s, refused with a watch |
| K8 | Watches with the decision gate | Unchanged output fires nothing; a change with the gate off fires once |
| K9 | Messenger as a source (Telegram first) | Sender not allow-listed is ignored |
| M | Real run: one resident, a morning brief and a CI watch, on Claude and Codex, for three days, pushed to a physical iPhone through OneSignal | A dated `real-validation-*.md` with tokens per wake, skips, push delivery and anything that went wrong |

`mise run quick` per step; the full check before pushing to `main`.

## 20. Decisions taken and still open

Taken by the user on 2026-11-21:

| # | Question | Decision |
|---|---|---|
| Q1 | What restarts the keeper after a reboot or logout | **A user LaunchAgent** (`~/Library/LaunchAgents/…plist`, no root), installed only when the user turns on *Start at login* (Settings → Residents and routines) or runs `orochi keeper login on`, removed by turning it off or `--no-keeper`. Off by default |
| Q2 | How the phone hears of something with the app closed | **OneSignal push**, title only by default; the Mac's Notification Center beside it (§13) |
| Q3 | The name | "resident" and "routine" |
| Q4 | The first job, for the M run | A morning brief and a CI watch |
| — | Where it is managed | **The desktop app**: settings, the state of every routine, notifications (§14) |

Still open:

| # | Question | Options | Recommendation |
|---|---|---|---|
| Q5 | Who holds the OneSignal REST API key once the phone app is used by anyone but its developer | (a) The developer's key on the developer's own Mac only (enough for now); (b) a small relay of Moon Technica's that holds the key and accepts a push only for a phone id the sending machine paired with, signed by that machine; (c) each user brings their own OneSignal app and phone build | **(a) now, (b) before the phone app is given to anyone else.** (c) is not something a user can do |
| Q6 | Which OneSignal app and Apple push key | The developer's OneSignal account, with an APNs key from the Apple developer account the phone app is built under | Needed before K6 can be tried on a device |

## 21. Risks and what is not known

- **Sleep.** A Mac laptop is asleep much of the day; the product is only as always-on as the
  machine. Scheduled wake (`pmset`) needs root and is excluded.
- **Cost.** The 300k daily budget is a guess. A morning brief with mail and calendar MCP calls
  has not been measured; the M run is what sets it.
- **Permission shapes.** §11.2's allow-list depends on how each adapter names MCP tools in a
  permission request, which has not been read.
- **Injection.** §11.3 lowers the harm; it does not prevent a stranger's text steering what the
  resident reads, writes in its notes or asks the user to approve.
- **Notification delivery** from a bare process on macOS, and on the phone through OneSignal,
  has not been tried. Whether `react-native-onesignal` supports React Native 0.88 / Expo SDK 58
  and can share the iOS notification delegate with `expo-notifications` has not been read.
- **OneSignal is a recipient.** At `title` it learns when the user's agents ask and finish and
  the residents' names; at `summary`/`full` it learns content. Apple or Google carry the same.
- **The key.** Anyone holding the REST API key can push anything to every phone of the app;
  until Q5 (b) exists it must stay on the developer's Mac.
- **Quota.** Routines spending a subscription at night may leave it empty in the morning; the
  quota gate reads snapshots that may be stale.

## 22. What was built, and how it differs (2026-11-21)

Built, with tests against `tests/fixtures/mock_acp.py` and a loopback stand-in for OneSignal
(`cli_e2e.rs`, `notify.rs`, `routines.rs`, the desktop's `remote.rs`/`ui.test.mjs`, the phone's
vitest suites):

| Step | Built |
|---|---|
| K0 | `chat::host::spawn` (own process group, stderr to `<data>/hosts/<thread>.log`); `threads send` starts a host (`--no-host` to only queue); an open question keeps its host up |
| K1 | Schedule grammar and next-due (`routines.rs`, local time through the C library, UTC elsewhere), `routines`/`firings` tables and views, the keeper (`keeper.rs`: lock, firing-and-next in one transaction, collapse of what a sleep skipped, overlap/paused/budget gates), `orochi routines add/list/show/run/pause/resume/keep/rm/preview`, `orochi keeper status` |
| K2 | Residents: `orochi resident new/list/show/pause/resume/rm`, a folder and a pinned thread, the standing brief (instruction, notes, earlier reports) |
| K3 | A routine's turn runs one agent, reading only *(superseded, §23: a read-only team where the work calls for one)*, with approval forced to `Ask`; its question is declined after `routines.ask_secs` (`prompts.expires_at`, `answered_by = 'expired'`) |
| K4 | `notices` table, the notifier (Mac notification through `osascript` with the text as an argument; OneSignal push with the request shape of §13.3), levels, quiet hours with release, coalescing, the key in the keychain/file, `orochi notices`, `orochi notify test/phones/phone-add/phone-rm/key`, the daily token budget (`firings.tokens`) |
| K5 | Desktop: a **Routines** page (tabs Routines · Residents · Notices; keeper strip with today's tokens; make a routine with the next three times shown while typing; run now, pause, resume, delete, keep; firings per routine; notes editor; mark read; send a test), settings pages *Residents and routines* and *Notifications* (OneSignal key, level, away, quiet hours, kinds, phones, start-at-login), the window tells the store when a person is at it |
| K6 | Phone: `POST /push` and `push_id` at pairing on the desktop's server (registered with the pairing, replaced, removed when the phone is revoked), the phone's *Routines* screen (notices, routines with run/pause/resume, residents), the push provider written against the OneSignal SDK, the plugin and `extra.oneSignalAppId` in `app.json`, ids and tap parsing in `@orochi/core` |
| K7 | Mailbox tools `schedule`, `list_schedules`, `cancel_schedule` for a resident and for a routine's own turn; Orochi decides (floor, count, expiry), writes a `scheduled` notice, cancels only what the agent made |
| K8 | Watches: `orochi routines add --watch <cmd> --every <n>`, output hashed, baseline on the first look, an unchanged look costs no turn, three failures pause it and say so, the changed output goes into the turn marked untrusted; *Try it* in the window |

Where the build differs from the design:

- **Notes are kept by Orochi, not written by the agent (D9).** A resident runs reading only, so it
  ends a reply with `<notes>…</notes>` and Orochi writes `NOTES.md` (the previous file beside it as
  `NOTES.md.prev`, a line count in the report). `home` and `worktree` access are **not built**:
  every routine turn reads only, and `routines.access` is always `read`.
- **The stored descriptor (D6) is written and not yet read.** The profile is made once when a
  routine is made, but the turn is routed by the console as any queued turn is, so it is classified
  again. Passing it through (`RunOptions::descriptor`) needs a column on the turn.
- **The quota gate and the decision-model gate of §10.2 are not built.** The paused, overlap and
  budget gates are.
- **The Windows keeper is not built** (the commands, the views and `keeper login` compile there;
  the process does not run). The non-Unix side was written to the rules of the repository and not
  compiled, which only the hand-run CI can do.
- **Not built:** the sidebar's *Residents* group and a resident thread's strip and tabs (the Routines
  page holds them), the console's `/routines` and `/schedule`, the Activity page's notices filter
  (notices are on the Routines page), a messenger as a source (K9), the relay of Q5.
- **`orochi keeper login on|off|status`** replaces `orochi setup --keeper`.
- **A started host is in its own process group, not its own session**, which keeps an interrupt
  aimed at its starter away from it without needing `unsafe`.

Not verified, and not claimed:

- Any of it on a real agent CLI: the routines, the briefs, the schedule tools and the read-only
  refusal were run against the mock agent only.
- OneSignal itself. The request is checked against a loopback server for its shape; no real app,
  key, APNs key or device was used, so delivery, `invalid_aliases` in the real response and the
  token's permissions are unknown.
- The phone's push path on a device: `react-native-onesignal` 5.5.17 and `onesignal-expo-plugin`
  2.8.1 are added and the iOS bundle builds, but `oneSignalAppId` is empty (push stays off until it
  is set), a development build has not been made, and whether the SDK and `expo-notifications`
  share the iOS notification delegate, and whether a push and the app's own local notification say
  one thing twice, are unknown. In the foreground the push is suppressed.
- The Mac notification (`osascript`, from a process with no app bundle) and the LaunchAgent
  (`launchctl bootstrap`) were not run: no test raises a notification, and only the property list's text
  is tested, so neither effect is known.
- The desktop page was looked at in the preview with recorded answers, not in the real window.
- The costs: nothing was measured, so §10.2's numbers are still Orochi's own starting values.

## 23. The assistant is a conversation; an agent proposes and the person allows (2026-11-21)

The Routines page of §22 made routines something the person writes. Nobody does: a person asks in
a conversation ("every weekday at 8, tell me how CI did") and expects that to be enough, as with
OpenAI's Dot, whose phone app is one conversation with its assistant and nothing that looks like a
scheduler. So the user's decision (2026-11-21): the phone's main place is the conversation with a
resident, and a routine is made by asking for it there.

**An agent's `schedule` only proposes.** The routine is written with `proposed = 1` and paused, so
neither the keeper nor *Run now* nor *Resume* starts it; an `ask` notice tells the person (and so
pushes as an approval does); the agent is told it waits for an answer. *Allow* (`orochi routines
approve`, the window, the phone's card) clears it, works out the next due time from now and removes
its expiry; *Decline* deletes it; one nobody answers is dropped after `routines.proposal_days`. With
the person in the loop, the agent's own floor (`agent_min_interval_secs`), its count of routines
made (`max_agent_made`) and the expiry of what it made (`agent_expiry_days`, *Keep*) went: the
person's floor holds for both, and `max_proposals` caps only what waits. **An agent may propose a
watch** (`watch` + `every`): the command is shown whole on the card before it can run, which is
what "only the person writes a command run with nobody present" was protecting.

**The phone's Assistant tab** (`app/(tabs)/assistant.tsx`, `@orochi/core` `chat.ts`): the
resident's conversation drawn as a messenger — the person's bubbles and the assistant's, a long
reply folded behind *Show more*, what it read or fetched for an answer listed under it as links
(from the turn's `read`/`fetch` tool calls), a routine's wake as a quiet line naming the routine,
proposals as cards with *Allow* / *Decline*, open questions as the usual card. A push about a
notice opens it there. With no resident yet, it offers to make one (`resident_add`, now allowed to
a phone). The menu holds the pause switch (below) and the full conversation view; the Home card
for routines is gone. Schedules are shown in the
person's language (`scheduleWords`; the grammar stays English, and the agent writes it).

**The desktop's Assistant page** (`desktop/src/assistant.js`, the sidebar's first button) is the
same conversation in the conversation's place: bubbles, folded long replies, sources, wakes, the
proposal card (*Allow* / *Don't allow*) and the window's own question card, a composer, a picker
when there is more than one resident, the pause switch and the way to the full conversation; the
sidebar's button counts what waits on the person (proposals and questions). With no resident it offers to make one. A source opens through the
window: a file through `open_path`, held to the resident's folder, and a page through
`open_source`, which opens only an address one of this conversation's own tool calls fetched, so a
link in a reply cannot send the window anywhere. The card says a schedule in
Japanese in a Japanese window (`routines.js` `scheduleWords`, the phone's rules). "Decline" was
already 反映しない (a merge's word), so a proposal's refusal is *Don't allow* / 許可しない on the desktop.

**There is no Routines page** (the user's decision, 2026-11-21: Dot has none). A routine is
started by asking and stopped by asking: `cancel_schedule` removes any routine on the conversation
during a turn the person started (`turns.origin` null), and only what an agent made during a
routine's own turn, which nobody asked for — the agent finds it with `list_schedules` and says in
one line what it stopped. When asking is not enough there is one switch, not a list: *Pause* on the
assistant (the desktop page's head, the phone's menu, `resident_action` — the phone may pause and
resume, never delete) stops everything the resident has scheduled, since the keeper skips a paused
resident's routines and watches as `paused`. Today's spending moved to Settings → Residents and
routines, beside the budget; the desktop's Routines page and dialog, the phone's routines screen and
their list-only helpers are gone. `orochi routines` stays the way to list, add, run or remove one
from a terminal.

**Held, not removed, and a failure is one incident** (2026-11-21, after reading how OpenClaw,
OpenDots, Hermes Agent and NanoClaw do it — each can pause one job, not only all of them, and the
three that retry stop a job that keeps failing). `pause_schedule {id}` and `resume_schedule {id}`
follow `cancel_schedule` in the mailbox: the agent pauses rather than removes when the person asks
to hold one for a while; pausing follows removing's rule (any routine on a turn the person started,
only an agent's on a routine's own turn), and **resuming is the person's alone**, so a routine's turn
cannot wake what was put to rest. Resuming starts from now (`routines::api::resume`, which also
clears a watch's failure count). A routine's failed turn is told once (`trouble`, "did not finish"),
the failures after it are not, and the third in a row (`keeper::FAILURES_TO_PAUSE`, *Orochi's*,
the same as a watch's command) pauses the routine with a second notice saying how to resume it; a
turn that finishes ends the count, and an interrupted one (the Mac slept, the host was stopped) is
told but not counted. A failed or interrupted turn is never run again on its own — the next due
time is a new run — which is what OpenDots' *Retry after review* protects: work a run already did
is not done twice. Tested end to end (`a_routine_whose_turn_fails_three_times_in_a_row_is_told_once_and_then_paused`,
and the pause, resume and refusals in `a_resident_proposes_routines_that_run_only_once_the_person_allows_them`).
**A routine's turn is a team, reading only** (the user's decision, 2026-11-21: working as a team
is what Orochi is for; all four products compared run a scheduled job as one agent). The seats are
the ones a message would get (`roles::seats` from the instruction, then `worth_seating`), every
one of them read-only, the lead included, and told so (`alongside` / `beside_seat` with
`unattended`): nobody is at the keyboard to answer for a change. A routine is never split into
design → implement → review phases, which would be read-only sessions in a row; its team sits in
one turn, so the keeper's report, the budget it counts and the question's expiry all hold as for
one agent. The routine budget is checked before the turn starts, so a team can overrun it by what
that one turn costs. Tested end to end (`a_routine_whose_work_calls_for_a_team_runs_one_that_only_reads`);
not with a real agent.

**A routine's lead may start helpers, and they only read** (`start_agent`, forced read-only by
`grant`'s `unattended`; a worktree is never made). A helper's result comes back as a later turn,
which would have left the keeper reporting "I started it". So a turn that started a helper marks
its firing `waiting` (`Activity::hold_firing`): `unsettled` skips it, the turn the helper's result
wakes is a `routine` turn like the first (read-only, questions expiring), and it takes the firing
over (`continue_firing`), so the notice is made from the answer. A firing still waiting after an
hour (the host went away) is settled as it stands. The prompt tells the lead to say only that it
waits. Tested (`a_routine_may_start_a_helper_that_only_reads_and_its_report_waits_for_the_answer`);
not with a real agent.

**The assistant does not show how it is scheduled** (2026-11-23, the user's decision: OpenClaw, Hermes Agent and Orca Bots talk like a colleague, and none of them puts "next wakes at 14:33", "woke for Morning brief" or a list of routines in front of the person). The list of routines on the desktop and the phone, the status line with the next wake, and the line that said a routine woke the assistant are gone; what a routine produced is the assistant's own message. Schedules are asked about, paused and stopped by talking to the assistant (`list_schedules`, `pause_schedule`, `cancel_schedule`), and `orochi routines` lists them from a terminal. Notifications never announce a wake: only a report, something that needs the person, a mention or a failure.

Not done: a real agent deciding between pausing and removing.

Not built: a character or voice. Checked with the unit and end-to-end tests above, the window's UI
tests (`ui.test.mjs`) and `views.rs`, and by hand in the web build against
`mobile/tools/mock-server.mjs` (`mock-residents.mjs`) and the desktop's `preview.html`; not on a
device, not in the real desktop window, the first-run screen of the desktop page was not looked at,
and not with a real agent deciding when to propose or what to stop. The phone's pause switch was
not looked at in the web build.

**The assistant is an organization** (2026-11-23, the user's decision): the single-assistant reading of this page is superseded by `docs/resident-organization-design.md` — a team of members hired by role, released when unused, each reachable by the person and by each other, a routine run by the member it is assigned to, and authority per member. Nothing in that document is built yet; the `residents.access` and `routines.approval` settings above are its interim.

**On the phone too** (2026-11-23): the Assistant tab is the phone's own conversation screen (`ui/Conversation.tsx`, the same one `/thread/[id]` renders), with the pause entry in its menu and the proposals as cards at the bottom; it has no gallery, changes, files, git or work. Looked at in the web build against `mobile/tools/mock-server.mjs`; not on a device.
