# The Agent Organization: Design

Written: 2026-11-16. **Status: O0–O6 built and run against the mock agent only (automated tests); nothing has run on a real CLI (§9).** What is not built is listed at the end of §10. §10 lists the build
order and is updated as each step lands. Everything in §2 ("Where Orochi stands") was read
from the code and is cited.

One goal: **an agent team is shaped like a real organization**. The user gives work to a
lead; the lead decides how to divide it, hires the roles the work needs, and those roles may
lead teams of their own; every agent works on a branch of its manager's work; results flow
back up, each level integrating what its reports produced before handing it on; and
organizations in different projects can talk to each other. The user's words (2026-11-16):
the flow should be *user → lead → decomposition → subordinates → …*, with technical leads and
other leaders allowed below the lead; the structure should be flexible enough for an
autonomous organization at company scale ("究極、会社レベル"), though it is not operated at
that scale yet; and projects must be able to communicate.

## 1. Requirements

| ID | Requirement |
|---|---|
| R1 | **Orochi receives, the lead decides.** Orochi's own code takes the message, the decision model and the scorer choose which agent and model run the lead, and the lead is handed the whole request. Orochi does not divide the work itself |
| R2 | **The lead divides the work and delegates it**, to roles it names; small work it does itself |
| R3 | **Any depth.** A lead may hire leads, which hire their own reports. Depth, headcount and fan-out are settings, not constants |
| R4 | **Roles are data.** A role is a name, an instruction, whether it writes, whether it leads, and whom it may hire. The user defines them; a few ship |
| R5 | **Every agent works on a branch of its manager's work.** A report's worktree starts from its manager's worktree; its manager merges it there; only the root lead's merges reach the user's tree, and only as the manager design already allows |
| R6 | **Results flow up one level at a time.** A report's result goes to its manager, never past it; the user hears from the lead |
| R7 | **Projects can talk.** Leads of different projects exchange messages; the user decides which projects are linked |
| R8 | **Everything is bounded and visible.** Each limit is a setting with a refusal sentence an agent can repeat; the organization is a tree the console, `orochi threads team` and the window draw |
| R9 | **Off until asked.** Defaults keep today's behaviour; the capability is opened by configuration |

## 2. Where Orochi stands

| Fact | Where |
|---|---|
| The console lead is also the conversation's manager (plan, stop, merge trial, apply); this is the design of `manager-design.md` D1 | `docs/manager-design.md` §2 |
| **Orochi's code, not the lead, divides the work.** `workflow::steps` picks design → implement → review from the profiled complexity and `run_team` runs each phase as its own seat, handing the reply on. The lead is whoever has the phase | `src/chat/workflow.rs:9-20`, `src/chat/mod.rs:3770,3785` |
| **One level only.** `start_agent` and the manager tools are offered to the lead's session alone (`mailbox::delegate` guard); a helper has neither | `src/mailbox.rs:1360-1381`, `src/chat/mod.rs:4383-4393` |
| Gates for a helper: a live lead turn, a writing lead, `roles::MAX_SEATS` (6), `worth_seating`, worktrees on | `src/chat/mod.rs:5376-5470` |
| A writing helper's worktree is a detached worktree from a snapshot of **the user's tree** (`Workspaces::create_titled`); `Workspaces::source` is that tree | `src/workspaces.rs:354-405` |
| `merge::trial(source, workspace, …)` and `merge::apply(source, …)` already take the tree to merge into as a parameter | `src/merge.rs:80,224` |
| A helper's result returns as a completion turn of the lead | `src/chat/background.rs` `completion` |
| The mailbox scopes a room to a repository's git common dir; nothing crosses repositories | `src/mailbox.rs` (`channel_of`) |
| `manager-design.md` D8: merges across conversations are out of scope | `docs/manager-design.md` §2 |

## 3. Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | Who splits the work | **The lead.** `manager.drive = "lead"` hands the lead the whole message; `workflow::steps` returns nothing for `Auto`. `"phases"` (today) stays the default | R9. The phases remain what `/team` and `Steps::Team` ask for outright, and become role templates the lead can name |
| D2 | What a role is | `[[org.roles]]`: `name`, `instruction`, `writes`, `lead`, `may_hire` (names; empty = any), `max_reports` (optional). Built-ins: `researcher`, `designer`, `reviewer` (read only), `engineer` (writes), `lead` (writes, leads, hires any) | Flexible without a language of its own. A company is roles that may hire roles |
| D3 | Where depth lives | **In the member, not the code.** Each member row has `parent` (a name, or none for a report of the root lead) and its depth follows. `org.max_depth`, `org.max_members`, `org.max_reports` bound it; defaults 1, 6, 6 reproduce today | R3, R9 |
| D4 | Who may call `start_agent` | The root lead, and any member whose role has `lead = true`, while it runs. Orochi offers the tool to such a session (`mailbox::delegate` guard held for the helper's life) | One mechanism for every level |
| D5 | Hiring is checked, not trusted | `grant` reads the requester's member row: its role's `may_hire`, its live reports, the depth, the conversation's headcount. Each refusal is a sentence | The cap rule of the manager design, per lead |
| D6 | Branches | A report's worktree is created **from its manager's worktree** (`Workspaces::create_under`): the snapshot is of the manager's tree, `Workspace.parent` names the manager. The user's tree is the root's tree | R5. Git already does the rest: a worktree of a worktree, a base commit per member, a three-way merge per level |
| D7 | Integration | `merge_trial` / `apply_merge` / `finish_workspace` by a lead target **its own worktree** (`merge::trial`'s `source`). No user prompt and no repository lock: it writes a worktree only that lead's session and Orochi touch. The root lead's merge into the user's tree is unchanged | `merge.rs` is already parameterised by the target |
| D8 | Where a report goes | **To its manager's inbox as a mailbox message** (`Orochi` as sender), not as a completion turn. The root lead keeps the completion turn. A lead is told it must not finish while its reports run | A sub-lead is a background seat in one long turn; it waits with `read_messages`. Resuming it per report would pay a cold session each time |
| D9 | A lead that ends first | Its live reports are stopped and the manager's result says so; their worktrees close as patches under the existing rule that none outlives the conversation | No orphans |
| D10 | Cost and runaway | `org.max_members` is the hard cap; `worth_seating` applies at every level unless `org.cost_gate = false`; every refusal names the setting | Autonomy at scale is a budget question first. Measured with a real Codex lead (2026-11-16): on a three-file task `worth_seating` refused every hire, so a team that was asked for outright was not built; a user who runs a real organization turns the gate off |
| D11 | Projects | A **project** is a repository as the activity store knows it (`projects`). Organizations are per project. Projects talk through **liaison** tools — `list_projects`, `message_project`, `read_project_messages` — offered to leads only, addressed `project` or `project/member`, delivered to the other project's root lead | Departments talk through their heads, not through every member |
| D12 | Who allows a link | `[[org.links]] from, to` (project names or paths, `*` allowed on one side), written by the user. Without a link nothing is delivered. A delivery to a project with no running host is queued and shown in its next session; it never starts an agent by itself | The recipient of cross-project text is the user's to choose, as a cloud decision model is |
| D13 | What crosses | Message text only, bounded, marked untrusted, never a path, a diff or a file; never telemetry. Messages live in `mailbox.sqlite3` beside the room's and expire with the same retention | The content rule |

## 4. The model

### 4.1 Lead drives

```
user ─▶ Orochi (profile · decision model · scorer: which agent runs the lead)
          └─▶ lead  ── set_plan ─┬─ start_agent(role=engineer …)      ◀── hires
                                 ├─ start_agent(role=lead, name=api …)◀── a sub-lead
                                 │     └─ api: start_agent(role=engineer …) …
                                 └─ does the small parts itself
```

With `manager.drive = "lead"` the lead's prompt carries the organization brief (§4.4) and the
message goes to it as written. Work the lead can do alone costs one session, as a message
below `worth_seating` does today; the lead decides, and Orochi's gates still refuse a hire that
cannot pay for itself.

### 4.2 Roles

```toml
[org]
enabled = true
max_depth = 3            # levels below the root lead
max_members = 12         # live members in one conversation
max_reports = 4          # live direct reports of one lead

[[org.roles]]
name = "cto"
instruction = "You own the architecture. Divide the work between your leads and decide what is merged."
writes = true
lead = true
may_hire = ["engineering-manager", "reviewer"]

[[org.roles]]
name = "engineering-manager"
instruction = "You run a team on one area. Hire engineers, review and integrate their work."
writes = true
lead = true
may_hire = ["engineer", "reviewer", "tester"]
```

A role's `instruction` joins the member's prompt after Orochi's own; it never replaces the
rules about worktrees and merging. `may_hire` is the organization chart: a role not named
there cannot be hired by that role.

### 4.3 Branches

`Workspaces::create_under(name, title, parent)`:

| | root lead's report | sub-lead's report |
|---|---|---|
| snapshot taken of | the user's tree | the sub-lead's worktree |
| `Workspace.parent` | none | the sub-lead's name |
| merged by | root lead, trial then apply, into the user's tree | the sub-lead, trial then apply, into its own worktree |
| asks the user | `manager.apply` | never |
| closed | by the apply, `finish_workspace`, or the sweep | the same; the sweep closes children before parents |

A sub-lead's own worktree is itself a report's worktree, so it reaches the user's tree only
through its manager's merge. Conflicts are the manager design's: a trial reports them and a
resolution seat in the trial worktree resolves them.

### 4.4 What a lead is told

One block, written by Orochi from the rows, joined to the lead's note: its role and manager,
the roles it may hire (with their one-line instructions), its live reports, the limits that are
left (`depth 1 of 3`, `members 5 of 12`), the rule that a lead integrates its reports' work
before reporting up and does not finish while reports run, and, for a lead with links, the
linked projects.

### 4.5 Results and waiting

A report ends → Orochi posts its result to the manager as a mailbox message from `orochi`,
held and merged like the manager design's wake (`HOLD`), and the manager's `read_messages`
returns it. A sub-lead's prompt says: end your turn only after every report has finished and
you have integrated what you accept. If it ends early, §D9.

### 4.6 Projects

```
projects: billing, web
[[org.links]] from = "web", to = "billing"

web lead:     message_project(project="billing", body="Need the invoice API shape by tomorrow")
billing lead: (its next turn, or a wake while idle) sees "From project web: …" as an
              untrusted note, replies with message_project(project="web", …)
```

`list_projects` shows linked projects with their name and whether a host is running, never
their files or threads. A message is a row in `mailbox.sqlite3` addressed to a project's root
lead; the recipient reads it through the same wake the manager design gives a helper's
message to an idle lead, with the same budget so two projects cannot loop at each other: at
most `org.link_wakes` (3) wakes per link per conversation.

## 5. Store

Additive only, through `activity::added`.

| Object | Change |
|---|---|
| `seats` | `parent TEXT` — the manager's member name, none for the root lead's reports; `role TEXT` |
| `v_team`, `v_members` | `parent`, `role` and `depth` columns |
| `workspaces.json` | `Workspace.parent` (`#[serde(default)]`) |
| `agent_requests.payload` | `role`, `lead` as written |
| `messages` | `project TEXT` — a delivered cross-project message names the sending project |

## 6. Surfaces

- Console: the ↓ listing and `/team` draw the tree (indent by depth, role beside the name).
- `orochi threads team <id>` prints the tree; `--json` carries `parent`, `role`, `depth`.
- Window: the Work pane draws members under their managers; the Office view groups a lead's
  team under it.
- `orochi org [show|roles|links]` shows what is configured, as `roster` does for models.

## 7. Invariants

- A refused hire is a sentence naming the setting that refused it.
- No member is offered `start_agent` unless its role has `lead = true` (or it is the root lead).
- A worktree's parent is closed after its children; none outlives the conversation.
- Nothing a sub-lead merges reaches the user's tree except through the root lead's merge.
- A cross-project message needs a link, is text only, and is never written to telemetry.

## 8. Not decided

- Per-subtree token budgets (`max_members` is the only budget now).
- Whether a role may name the agent or model it runs on (today the scorer chooses, as for any helper).
- Sharing a worktree between two projects (out of scope; each project merges its own).

## 9. What is measured and what is not

Only the mock agent has run this (O1–O2): hiring through two levels, a report reaching its lead's inbox, and a merge into the lead's worktree and then the user's tree. No real CLI has. Until a real lead has hired a sub-lead that hired an engineer, and the
merge has gone up two levels, every claim above is design.

## 10. Build order

| Step | Content | Status |
|---|---|---|
| O0 | This document | written |
| O1 | `[org]` config, `org.rs` (roles, member registry, hire checks), `Workspace.parent` and `create_under` | built; `tests/org.rs` |
| O2 | `grant` reads the requester; sub-leads offered `start_agent`; reports routed to their manager's inbox | built; `a_lead_hires_a_lead_who_hires_an_engineer…` in `tests/cli_e2e.rs` (mock agent: lead → `api` → `eng`, merged up one level at a time) |
| O3 | Scoped team tools for a sub-lead | built in part: `list_work` and `stop_agent` over its own team (`Tier::team`, `tests/cli_e2e.rs`); `set_plan`, `merge_trial` and `apply_merge` stay the top lead's, a lead below integrates with `finish_workspace` (no checks run) |
| O4 | `manager.drive = "lead"` and the organization brief | built; `with_the_lead_driving_the_whole_request_goes_to_the_lead_instead_of_orochis_phases` |
| O5 | Store columns, `threads team` tree, console, window | built: `seats.manager` / `seats.org_role` in `v_roster`, `v_members`, `v_team`; `orochi threads team` and the window's team pane draw the tree (`desktop/tests/ui.test.mjs`); the console's ↓ listing and `/team` do **not** yet |
| O6 | Cross-project liaison tools and links | built: `list_projects` / `message_project`, `[[org.links]]`, `project_mail`; a message taken but not yet heard when a console ends is lost |
| O7 | A real run on both CLIs, recorded in a dated `real-validation-*.md` | — |

**Not built.** Per-subtree token budgets; `merge_trial` / `apply_merge` / `set_plan` for a lead below the top; the console's own tree (↓ listing, `/team`); `orochi org`; a project's lead being woken by a message while no console runs (a host is needed); a role naming the agent or model it runs on; and every real-CLI run (O7).
