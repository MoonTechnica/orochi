# Desktop App Benchmark: Orca

Written: 2026-09-29.

**Status: survey.** The desktop app it is measured against is the one described in
[Desktop App Design](desktop-app-design.md), as it stands at commit `21bfa00`. What Orochi does
about it is designed in [Supervising Background Agents](supervision-design.md) (§7 for the
window, §8 for Orochi inside Orca); this document is the evidence that design cites.

## 0. What was read

| Source | What it gave |
|---|---|
| Orca 1.4.215, installed at `/Applications/Orca.app` | The main process (`app.asar` → `out/main/index.js`) for how status reaches the window; `orca --help` for its concepts |
| Orca's bundled Japanese dictionary (`out/renderer/assets/ja-*.js`), flattened: 14 737 labels keyed by component | What every screen offers, by component name (`auto.components.<area>.<Component>`). Labels are quoted in Japanese as shown, with an English gloss |
| Two screenshots of the user's Orca window (sidebar, a Claude Code pane, status bar) | What the labels look like assembled |
| Orochi's desktop app, `desktop/dist/preview.html` rendered against `desktop/tests/*.json` in Chrome | Orochi's side of every comparison |

Nothing was checked by running Orca's features; a label proves a screen exists, not how well it
works. Where a behavior is inferred from a label rather than seen, the row says so.

## 1. The two apps are different in kind

Orca is **terminal-first**: a workspace is a git worktree, an agent is a CLI running in a
terminal pane, and Orca learns what the agent is doing from hooks it installs into each CLI, a
statusline, or an OSC 9999 report (see the console design's §5.1). On top of that it has grown a
structured "native chat" view of the same session, an Activity inbox, an Agent dashboard, usage
analytics, session history, issue trackers, automations, a browser, an emulator and a phone app.

Orochi's app is **conversation-first**: a thread is a row in `activity.sqlite3`, every turn is
routed on its own merits across agents and accounts, and the window reads views of that store.
It has no terminal and does not host a CLI.

So the benchmark is Orca's **supervision UX** — how it tells a person which of many sessions
needs them, what each is doing, what its helpers are doing, and what the accounts have left —
not its architecture. Orca's own orchestration (`orca orchestration`) overlaps Orochi's mailbox
and collaboration and is out of scope.

## 2. Area by area

Keys are abbreviated from `auto.components.…`; `ja` quotes Orca's own words.

### 2.1 Where sessions live (sidebar)

| Orca | Orochi today | Verdict |
|---|---|---|
| Projects → workspaces (worktrees) → agent sessions. Workspace card: primary badge (`プライマリ`, "primary"), branch, linked issue/PR, a note, created-by (`CLI が作成`, "created by the CLI"; `オートメーションによって作成`, "created by an automation"), child workspaces nested and foldable (`sidebar.WorktreeCard`, `WorktreeCardMeta`, `WorktreeList`) | Project → thread. One mark per thread, a seat count while working, a terminal glyph (`app.js` `drawSidebar`) | Orochi's thread-under-project is right for a conversation store; worktree is already a thread attribute (`v_sidebar.worktree`). **Adopt the row content, not the tree depth** (§3.1) |
| A session row shows **current activity and elapsed time** (`Agent API モデル - Bash: curl -sL… · 3m`) and **one child row per background agent** with its task (`Map generation request flow · now`) — screenshot; roster from `SubagentStart`/`background_tasks` | Title only; `v_roster.doing` exists but is not shown in the sidebar | **Adopt** (§3.1) |
| Project groups (`新規プロジェクトグループ`, "new project group"), project icons, rename, move to group | Sections ordered by name, pinned first, collapse remembered | Later; not what hurts today |
| Filter menu: hide sleeping (`スリープ中を非表示`), hide default branch, hide created-by-automation / by-CLI, by project, SSH (`sidebar.SidebarFilter`) | Filters by status designed (§6.2), not in the window | Adopt **status filters and unread-only**; the rest is Orca's worktree bookkeeping |
| Custom workspace status with icon and colour: in progress, in review, done, blocked, paused, alert, flag… (`sidebar.workspace.status`) | None; status is derived | **Do not adopt**: Orochi's status is what happened, not what someone typed |

### 2.2 What needs you (Activity, dashboard, tray)

| Orca | Orochi today | Verdict |
|---|---|---|
| **Activity** page: every agent thread across projects, grouped and filtered by agent / worktree / project / state, unread only, compact mode, search, mark read / unread, **clear completed (with undo)**, show child agents, jump to workspace (`activity.ActivityPrototypePage`) | No cross-thread screen in `app.js` (the menu has Agents, Insights, Settings). `desktop-app-design.md` §8 lists mission control under P3 as implemented; the current window does not reach one | **Adopt** as mission control's actual form (§3.2) |
| States, in its words: working, **monitoring background tasks**, blocked, **waiting for input**, failed, done, idle, **no recent update** (`unverifiable`), **needs attention** (`permission` → `要対応`) | `needs_you`, `working`, `queued`, `interrupted`, `failed`, `unread`, `idle` | **Adopt** `background` and `unreachable` (§3.2) |
| Agent dashboard pop-out: buckets **needs attention / working / idle / done**, a card per agent with elapsed time and open reviews, send a message to that agent from the card (`dashboardPopout`, `dashboard.DashboardAgentRow`) | Composer only inside a thread; Team pane can message the room | Adopt the four buckets inside §3.2; the per-card send is `say` already |
| Tray: `N 件のアクティビティが待機中` ("N activities waiting"), keep running in the tray; notification sounds to choose from (`tray`, `notification.sound`) | Designed in §6.9, not built | **Adopt** (§3.5) |

### 2.3 The conversation view

Orca's "native chat" (`components.native-chat`) renders a Claude Code or Codex session as a
structured conversation — the closest thing Orca has to Orochi's thread.

| Orca | Orochi today | Verdict |
|---|---|---|
| Turn status: `{{value0}} 処理中` ("working for …"), `{{value0}} で完了` ("done in …"), a toggle for turn details | `route` chip, `unverified` / outcome line | **Adopt** duration and done time on every turn (§3.3) |
| **Background tasks** block: header `{{value0}} バックグラウンドタスク`, counts by kind (agents, shells, monitors, workflows), state per task (running, waiting — *needs approval*, blocked — *failed*, done, stopped, unverifiable — *no contact*), output file, **stop one / stop all** | Seats appear as avatars in the timeline and the Team roster; nothing can be stopped from the window | **Adopt** (§3.3) — it is the desktop face of the console's background seats |
| Subagents: `{{value0}} サブ Agent をキックオフしました` ("kicked off N subagents"), later "ran N subagents", counts by state, tokens per subagent | Seats and their routes; no per-seat tokens in the timeline | Adopt the launch/finish pair and per-seat tokens (§3.3) |
| Task list with progress `{{total}} 件中 {{completed}} 件のタスクが完了しました` ("M of N tasks done") and per-task events | Plan tab (checklist) and the work-graph board | Adopt the progress line in the timeline; keep the Plan tab |
| Turn diff: `{{count}} がファイルを変更しました` ("N files changed"), partial diff, "edits recorded this turn" | Changes tab, *Last turn* scope | Adopt a one-line turn summary linking to Changes |
| Context usage: `コンテキスト {{used}} / {{window}} トークン、{{percent}}% 使用済み`, "estimated from the last response" | Tokens in the terminal footer only | Adopt, labeled as the agent's own report |
| Approval card: `{{value0}} を許可しますか?`, allow / deny / cancel, **reason**, **blocked path**, **ask rule**, plan file; receipts `{{device}} に回答しました` ("answered on …") | `Run a command` card with Allow once / Always allow / No | **Adopt reason and receipt** (§3.3) |
| Question flow with steps, "Other…", skip, next | Team dialog and permission cards | Later |
| Composer pills: **model, effort** (`最小`…`最大`, `ウルトラ`), fast mode, thinking; `既定` ("default") and `未報告` ("not reported"); commands and skills with scope (project / personal / built-in / plugin); dictation; `送信済み — 未確認` ("sent, not confirmed") | Folder picker only in the window; route / approvals / steps chips are designed (§6.6) and not built | **Adopt the pills as the route chip**, with *Auto* first (§3.4) |
| Goal mode: a measurable goal chip, pause / resume, pursuing / blocked / limited (`goal`) | None | Out of scope for now |

### 2.4 Accounts and usage

| Orca | Orochi today | Verdict |
|---|---|---|
| **Status bar**: one usage meter per provider (Claude, Codex, Gemini, Kimi, OpenCode Go, Grok, Cursor, Antigravity, MiniMax) with `5時間` / `週` ("5 hours" / "week") windows and reset time; click for details; refresh; an empty-state *connect an account* card (`status.bar.StatusBar`, `StatusBarUsageEmptyCta`) | Quota gauges exist but only inside **Agents**, a modal reached from the account menu | **Adopt a status bar** (§3.4). This is Orochi's strongest material — it routes *because* of these numbers — and it is hidden |
| Account switching per provider (`切り替え先`, "switch to"), restart sessions still on the old account, Codex rate-limit reset credits (`今すぐリセット`, "reset now") | Orochi follows each CLI's own login; cooldowns per agent | **Do not adopt switching**: Orochi does not own accounts. Show cooldown and "why this route" instead |
| **Usage analytics**: per provider — input / output / cache read / cache write tokens, cache reuse rate (defined as cache read ÷ (input + cache read)), turns, sessions, by model and by project, API-equivalent estimated cost, 7 / 30 / 90 days / all time; overview with active days and PRs created (`stats.*`) | **Insights**: route stats, success and cost per route and task type, calibration | Keep Insights; **add** the ranges and cache reuse from `Usage::weighted` data Orochi already stores |

### 2.5 History, search, jumping

| Orca | Orochi today | Verdict |
|---|---|---|
| Session history across agents (`AiVaultPanel`): search, scope to workspace / worktree, resume in terminal or **in a new chat**, copy the resume command, delete | Threads persist; `--continue`; no search in the window | Adopt **search** (the FTS5 table §6.2 already names) |
| Jump palette (`worktreeJumpPalette`): threads, PRs, issues, ports, automation runs; filter chips by host and project | None | Adopt a thread-only palette (⌘K) once search exists |

### 2.6 What Orca has that Orochi should not copy

Terminal panes, the embedded browser and iOS / Android emulators, Linear / Jira / GitHub / GitLab
task pages, automations, remote hosts over SSH, the phone app, pets, port scanning, and custom
workspace statuses. Two small ones are worth noting for later: **keep the Mac awake while an agent
works** (`CaffeinateStatusSegment`, *auto*), and a **resource manager** that lists and kills
orphaned terminals — Orochi already records process leases (`process.rs`) and could show them.

## 3. What was taken

| Verdict above | Designed in `supervision-design.md` |
|---|---|
| Thread row with activity, elapsed, child rows (§2.1) | §7.3, R10 |
| Status filters, unread only; Activity screen; `background` and `stalled` states (§2.1, §2.2) | §7.4, §5.3, R11, D8 |
| Tray count and notifications (§2.2) | §7.7, R13 |
| Turn duration, seats block with stop, files changed, plan progress, context use, approval reason and receipt (§2.3) | §7.5, §7.6, R6–R8, R12 |
| Composer pills (§2.3) | §7.8, R14 |
| Status bar with usage and reset times (§2.4) | §7.2, R9 |
| Search and ⌘K (§2.5) | §7.9, R15 |
| Not taken (§2.6, account switching, custom statuses) | — |
