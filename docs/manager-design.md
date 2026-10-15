# The Lead as Manager: Design

Written: 2026-10-16. **Status (2026-10-16): M0–M5 implemented, automated tests only; M6 not
run.** Everything in §3 ("Where Orochi stands") was read from the code at `8d7dead` and is
cited; §14 says where the implementation departed from the design and why.

One goal: **the agents Orochi starts for one conversation work as one team.** One of them — the
lead — holds the plan, divides the work, starts and stops the others, answers their questions
while they run, judges whether what each produced may be merged, and has Orochi merge it. The
user talks to the lead; the team talks to each other; Orochi owns every constraint.

The user's words (2026-10-16): a manager that supervises the agents is essential; it should be
resident rather than per-stage; it should judge whether parallel worktrees' work may be merged;
and the agents should genuinely work as a team, communicating as they go.

## 0. Sources

| Source | Gives | Where |
|---|---|---|
| [Supervising Background Agents](supervision-design.md) (2026-09-29, S0–S6 built) | Helpers started by the lead through `start_agent`, their lifecycle, completion turns, the `agent_requests` queue | §3.1 |
| [Session workspaces](session-workspaces.md) (2026-10-05) | Writable helpers in conversation-owned Git worktrees; retained, never merged implicitly | §3.2 |
| [Session collaboration](session-collaboration.md), [Parallel execution](parallel-execution-design.md) | The coordinator's management state, three-way merge, stale-write checks, resolution turns, `apply` | §3.3 |
| [Agent mailbox](agent-mailbox.md) | Peer-to-peer messages, `set_status`, the prompt note | §3.4 |
| `src/chat/`, `src/chat/background.rs`, `src/mailbox.rs`, `src/workspaces.rs`, `src/collaboration/`, `src/activity.rs` at `8d7dead` | What exists | §3 |
| [real-validation-20260930.md](real-validation-20260930.md) | A real Codex lead delegated to two helpers; a completion turn cost 30,355 tokens when the lead only said it was still waiting | §4.4, §13 |

## 1. Requirements

| ID | Requirement | Surface |
|---|---|---|
| R1 | **One manager per conversation, resident.** The lead keeps its plan, its team and its view of the work across turns, across a host restart, and across an agent that cannot `session/load` | all |
| R2 | **The manager hears its team.** A helper that is blocked on the lead gets an answer without waiting for the user's next message | console, host |
| R3 | **The manager can stop a helper**, not only start one | all |
| R4 | **The manager sees the work in one listing**: every helper, its state, its worktree, what it changed, what a merge of it would do | all |
| R5 | **The manager judges a merge and Orochi performs it.** A helper's worktree is merged into the user's tree only through Orochi: a trial first (merge, checks, conflicts reported), then an apply that re-verifies and never overwrites a tree that moved | all |
| R6 | **A conflict is resolved by an agent in a scratch tree**, never in the user's tree, and the manager decides which result stands | all |
| R7 | **Every lever is gated and bounded**: seat cap, cost gate, the user's approval where a write to their tree is at stake, and a budget on how often the team may wake the manager | all |
| R8 | **The user sees the team's state** — plan, who is doing what, which results are waiting, which merges are proposed — in the console, the desktop and `orochi threads` | all |
| R9 | **Nothing here changes `collaborate`**, telemetry's content rule, or the invariant that a verified outcome comes from real checks | — |

## 2. Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | Who the manager is | **The lead's own session.** No separate manager seat above it | A seat costs a cold session (20–24k weighted tokens measured) per turn even for a one-line request; a manager between the user and the lead re-tells the request it did not read first. The lead already divides the work (`start_agent`), receives results and resumes; it lacks persistence and levers, not a chair |
| D2 | What "resident" means | **The manager's state is Orochi's rows, not the agent's context.** Plan, team, workspaces and merge trials are stored; a fresh session is told where things stand | An ACP session is the agent's conversation with its provider, not the owner of the work (session-workspaces §1). A loadable session is a cache of that state, not its home |
| D3 | How the manager acts | **Through mailbox tools that are requests Orochi decides** (`agent_requests`), as `start_agent` already is | The lead cannot pick a model, pass a gate, exceed the cap or write the user's tree; it asks, Orochi answers, each refusal a sentence it can repeat |
| D4 | Who merges | **Orochi, in two steps: `merge_trial` then `apply_merge`.** The lead never runs `git merge` in the user's tree | A merge is the one action here that writes the user's tree; it gets the same shape `collaborate --apply` has: trial, verify, stale check, exclusive lock |
| D5 | Who approves an apply | **The user, through the existing `apply` prompt, by default.** `manager.apply = "auto"` lets a trial that passed real checks with no conflicts apply on the lead's word alone | The team question after a design asks first because it writes back; so does this. Auto is offered because the user asked for the manager's judgment to count; it is measured before it becomes a default (§13) |
| D6 | Who resolves a conflict | **A resolution seat in the trial worktree**, started by Orochi on the lead's `resolve: true`, the integrator's `untangle` turn in another chair | The user's tree is never the place conflict markers appear; the trial worktree is already the merged tree |
| D7 | When the manager wakes | **On a finished helper (today), on a finished trial or apply (new), and on a helper's message to it while idle (new), coalesced and budgeted** | A helper waiting on the lead with `read_messages` waits for nothing when the lead is idle; a wake is a resumed session and costs like one, so it is held, merged with others, and capped per helper |
| D8 | Merges across conversations | **Out of scope.** The manager lists sibling worktrees of other conversations, read-only, and merges only its own | Two managers over one tree are the race this design exists to prevent; a thread that wants another thread's result asks its user |
| D9 | The plan's shape | **The coordinator's `Coordination` JSON** (`plan`, `decisions`, `open_questions`, `next_step`), written by the lead with `set_plan`, stored on the thread | One shape for management state in both places; `collaborate` keeps parsing it from a trailing object, the console takes it from a tool because a reply is shown to the user as written |

## 3. Where Orochi stands

### 3.1 Helpers (`supervision-design.md`, built)

- Only the lead's session is offered `start_agent`: the console holds a `mailbox::delegate` guard for the lead's turn, `SessionPeer::delegate` marks it, its mailbox server gets a `delegate` argument (`src/mailbox.rs:1034-1036`, `1205`). Companions, helpers, guests and discussions are never delegates (`src/chat/mod.rs:2769-2775`).
- The request is an `agent_requests` row; the mailbox server polls it for `REQUEST_WAIT` = 30 s; the console decides on its 1 s mail tick in `grant` (`src/chat/mod.rs:3523-3637`): a descriptor (a live lead turn), a writing lead, `roles::MAX_SEATS` (6), `roles::worth_seating`. When idle, every request is refused with "only the agent leading a conversation can start helpers" (`grant_idle`, `:1110-1126`).
- A helper is a seat owned by `chat::Session` (`background::Background`), polled from the idle loop and the turn loop, `background: true` so Esc never reaches it, routed by the turn's descriptor at `complexity.eased()`, never resumed (`src/chat/background.rs:134-225`).
- Its final reply, bounded to 4 000 characters, returns as a completion turn (`turns.origin = 'background'`) that resumes the lead; reports are held while other helpers run, up to `HOLD` = 300 s, because a real lead spent a 30k-token turn saying it was still waiting (`real-validation-20260930.md`).
- Stopping: ↓ manager overlay (`x`), Ctrl-X Ctrl-K twice, `/new`, a `stop` control with `{"seat"}` or `{"background":"all"}` from the desktop or `orochi threads stop`. **The lead itself has no way to stop a helper.**

### 3.2 Writable helpers (`session-workspaces.md`, built)

- `start_agent` with `write: true` gives the helper a detached Git worktree under `.orochi/sessions/<key>/<name>`, based on a snapshot commit of the user's tree as it is at launch, dirty state included, the user's HEAD, branch and index untouched (`Workspaces::create`, `src/workspaces.rs:66-149`). The manifest `workspaces.json` records `{name, root, base}`.
- It writes only there; it takes the repository lock as a writer (`src/chat/background.rs:186-194`); its completion says `workspace: <path> (changes retained; integrate explicitly)`.
- **Nothing merges it back.** `src/workspaces.rs:1`: "Never merge or remove them implicitly." The only merge primitive in the tree is `git merge-file` inside a collaboration run; no `git merge`, `rebase` or `cherry-pick` is invoked anywhere. The lead, which may write the user's tree, is left to run git there itself, outside every gate and check.
- `/workspaces` lists the manifest. It runs no `git worktree list`.

### 3.3 Collaboration (`session-collaboration.md`, built)

- The coordinator is a fresh session per stage, read-only, replying with `Coordination { plan, decisions, open_questions, next_step, messages, parts }` (`src/collaboration/turn.rs:53-66`). The struct replaces `report.management` and is re-injected into later prompts; nothing reads `next_step` to control flow; `parts` are accepted once at stage 0 (`adopt`, `src/collaboration/mod.rs:449-481`).
- Merging: `workspace::three_way(base, ours, theirs)` over copied trees, `git merge-file -p --diff3` per text file outside any repository, conflicts as `Conflict { path, sources, reason, markers }`; `workspace::commit` writes staged temp files only after every target still matches its `expected` entry (`src/collaboration/workspace.rs:215-357`).
- `untangle`: a resolution turn in the integrator's seat inside the merged tree, `git_conflict_markers` the only check, `Blocked` if markers remain (`src/collaboration/turn.rs:304-329`).
- `apply.rs`: only if `--apply` and the integrator's outcome is `Success`; exclusive `workspace_lock`; `Pending → Resolving → Resolved → Applied`, at most four rounds; a user edit that conflicts with the resolution blocks rather than being overwritten (`src/collaboration/apply.rs:153-270`).
- Participants are mailbox peers in the repository's room; none is a delegate; the prompt says "Do not spawn subagents."

### 3.4 Mailbox (`agent-mailbox.md`, built)

- `list_peers`, `send_message`, `read_messages` (wait ≤ 120 s), `set_status`; `start_agent` with `delegate` (`src/mailbox.rs:1123-1146`). One peer per agent session, scoped to the git common dir, so worktrees share a room. The prompt note asks peers to claim a part with `set_status` before changing anything and to hand over what a peer waits on as soon as it holds.
- **A message to the lead while no turn runs is read at the lead's next turn**, whenever the user next writes. The idle loop shows it to the user (`Feed`) but wakes nobody.

### 3.5 Persistence (`src/activity.rs`, `src/chat/`)

- `close_turn` writes `threads.continuation` (`{agent, model, reasoning, mode, session_id, loadable, modes}`); **nothing reads it**: `Activity::continuation()` has no callers, and `orochi host` starts with `resume: None` and an empty transcript (`src/chat/host.rs:85`). A restarted host therefore re-opens the thread's agent cold. The "rebuild the transcript from `items`" step in `desktop-app-design.md` §5 is not implemented.
- The in-memory transcript for agents without `session/load` is `Conversation.turns`, bounded to 32 KiB, process-local (`src/chat/mod.rs:4129-4163`).
- `prompts.kind` already allows `'apply'`; `agent_requests` already has the request/decision shape (`state`, `reason`, `seat_id`); `seats` already carry `title`, `background`, `origin`, `result`; `parts` and `v_board` exist for the collaboration board.
- Telemetry is content-free; titles, tasks, results and conflict paths live in `activity.sqlite3` only.

### 3.6 Gap summary

| Requirement | Gap |
|---|---|
| R1 | Continuation never read; transcript process-local; no stored plan |
| R2 | No wake on a helper's message |
| R3 | No `stop_agent` |
| R4 | `/workspaces` lists names; no diff, no merge preview, no sibling worktrees |
| R5, R6 | No merge path at all for session worktrees |
| R7 | Gates exist for `start_agent` only; no wake budget |
| R8 | `v_board` is for collaboration parts; the console has no team view beyond the ↓ overlay |

## 4. The model

### 4.1 The manager

The lead seat of a console or headless-host conversation, while it writes (`seats[0].writes`)
and is not a guest. Nothing else is a manager: a discussion lead, a companion, a helper, a
collaboration participant, `serve` and one-shot runs keep what they have.

What the manager holds, all as rows (D2):

| State | Where | Written by |
|---|---|---|
| Plan | `threads.management` (new, `Coordination` JSON ≤ 16 KiB) | `set_plan` |
| Team | `seats` (`background`, `title`, `state`, `result`) and `peers` | Orochi |
| Workspaces | `workspaces.json` as today, mirrored into `workspaces` (new table) so a client can read them | Orochi |
| Merges | `merges` (new table) | Orochi |
| Session | `threads.continuation`, now read back | Orochi |

### 4.2 What a fresh manager session is told

When the lead's next turn cannot `resume` — a host restart, an agent without `session/load`, or
`/reroute` — the task text gets, after the memory note and before the mailbox note, one block
Orochi writes from the rows:

```
Where this conversation stands (written by Orochi from its records):
Plan: <management.plan as a numbered list>
Decisions: <…>  Open questions: <…>  Next step: <…>
Team: `backend` (Implement request validation) · working 4m · worktree .orochi/sessions/…/backend
      `spec` (Research Agents API) · finished · result delivered in turn 7
Merges: `backend` · trial 2 clean · checks passed · awaiting your apply_merge
Recent turns:
<the bounded transcript, rebuilt from `items` of kind user and agent_message, newest last,
 the same USER_CHARS / AGENT_CHARS / TRANSCRIPT_BYTES bounds as the in-memory one>
```

The block is bounded to `TRANSCRIPT_BYTES` + 4 KiB, the plan first and the transcript cut
first. It is skipped on `resume`, exactly as the memory note is (`src/scheduler/mod.rs:829-858`),
because a resumed session heard all of it. The transcript part is the thread replaying **its
own** turns to **its own** next agent, the one exception the content rule already allows.

### 4.3 Levers

Five tools join the mailbox server for the delegate only (D3). Each is a row in
`agent_requests` with a new `kind`; the console decides on the mail tick it already runs;
`list_work` and `set_plan` are answered by the mailbox server process itself from the store,
because they need no decision.

| Tool | Arguments | Answer | Gates (each a sentence) |
|---|---|---|---|
| `list_work` | — | The team, the workspaces with `git diff --stat` against their base, the merges and their state, and the sibling worktrees of other conversations (name, branch, route, status; read-only, D8) | none; it reads |
| `set_plan` | `plan[]`, `decisions[]`, `open_questions[]`, `next_step` | `{"saved": true}` | ≤ 16 KiB, else "plan too long; keep it to the points" |
| `stop_agent` | `name` | `{"stopped": name}` | the name is one of this conversation's live helpers, else "no helper named …" |
| `merge_trial` | `workspace`, `resolve` (bool, default false) | `{"started": "<trial id>"}`; the outcome returns as a turn (§4.5) | the workspace is this conversation's and its helper has ended ("`backend` is still working; stop it or wait"); no trial of it is running |
| `apply_merge` | `trial` | `{"requested": "<trial id>"}`; the outcome returns as a turn | the trial is `clean`; under `apply = "ask"` the user is asked; under `"auto"` the trial's checks passed as real checks, else it is asked anyway |

`start_agent` is unchanged. The lead's prompt note gains one sentence per tool, and `alongside`'s
"never start another agent or another Orochi yourself" becomes "start helpers only with
`start_agent`" — the sentence the supervision design planned and the build left as it was
(`src/chat/mod.rs:3811`).

### 4.4 Waking the manager (D7)

Today one event gives the lead a turn without the user: a helper's result, held up to `HOLD`.
Two more join it, through the same `completion_message` path and the same hold:

| Event | Text Orochi writes | Hold |
|---|---|---|
| Helper finished / stopped / failed | as today | as today |
| Trial or apply finished | ``Merge trial of `backend` (trial 2) finished: clean · 5 files · checks passed (cargo test) · 1 file both sides changed (src/api.rs).`` / ``… 2 conflicts (src/api.rs, src/lib.rs); resolve with merge_trial(resolve=true)`` / ``Applied `backend` (trial 2): 5 files written.`` / ``Apply of `backend` declined by the user.`` | with the others |
| A helper's message to the lead while idle | ``Message from `backend` while you were idle:\n<body>`` | **60 s**, so a helper that writes three lines in a row costs one turn |

Budget: a helper may wake the manager **three times**; its later messages wait for the next
turn and the helper is told so in the `send_message` reply (`{"sent": true, "lead_reads": "at
its next turn"}`). A trial or apply result always wakes, because the lead asked for it. The hold
and the cap are constants, like `HOLD` and `REQUEST_WAIT`; measured before any of them becomes a
setting (§13).

Each wake is a turn with `turns.origin` = `'background'`, `'merge'` or `'mail'`, routed by the
conversation's continuation, never writing `feedback`, never classified again.

### 4.5 Merging

A worktree and the user's tree are both Git trees, so the merge is Git's.

```
merge_trial(backend)
  ours   = snapshot(user tree)            ← Workspaces::create's private-index commit, factored out
  theirs = snapshot(backend worktree)
  base   = workspaces.json: backend.base
  git merge-tree --write-tree --messages ours theirs      (merge base: `base`)
        ──► merged tree, or conflicted paths + the tree with markers
  git worktree add --detach .orochi/sessions/<key>/trials/backend-2 <merged>
  checks:  evaluator::evaluate(config.evaluator, trial root)        when no conflicts
           git_conflict_markers only                                 when conflicts
  row:     merges { state clean | conflicts | failed, files, overlap, conflicts, checks }
```

- `git merge-tree --write-tree` needs Git ≥ 2.38 (2022). Where it is older, the trial falls back
  to `collaboration::workspace::three_way` over `git archive`d trees, with its copy budget; the
  trial row says which. `merge-tree` is preferred because it detects renames and copies nothing.
- `overlap` is the paths both sides changed since `base`: the one thing a clean merge cannot
  say about itself, and what the manager reads to judge a semantic conflict.
- The trial worktree is a seat's workspace like any other: inside the conversation's sandbox
  when one is configured, checks run where the project's checks run.
- `resolve: true` on a `conflicts` result starts a **resolution seat** in the trial worktree:
  `read_only: false`, confined to that worktree as a writable helper is, the instruction
  `untangle` uses ("keep what each part meant, remove every marker, change nothing else"), then
  the full checks. It is a helper under the cap and the cost gate; its finish re-runs the
  trial's checks and the trial becomes `clean` or stays `conflicts` with the markers it left.
- A trial is stale the moment the user's tree moves. `apply_merge` snapshots again; if the tree
  differs from the trial's `ours`, the trial is re-run once, and only a trial on the current tree
  is applied.

```
apply_merge(trial 2)
  gate (D5): clean; ask the user (prompts.kind = 'apply', the team dialog's chair) — or auto
  lock:  workspace_lock(data, repository_id, exclusive) — waits for the lead's turn to end
  ours' = snapshot(user tree); ours' == trial.ours or re-trial
  diff   = merged tree ∖ ours' as collaboration Changes with `expected` entries
  workspace::commit(root, changes)        ← staged temp files, every target checked, then renamed
  row:   state applied, files; the workspace keeps its row and its worktree
```

The apply writes files. It never commits, never touches HEAD, the branch or the index, and
never deletes the worktree: what a merge did is visible in `git status` as the helper's edits
would have been had the lead typed them, and the retained worktree is the record of what was
merged. `/workspaces` says `applied` beside it.

An apply request that arrives while the lead's turn runs is answered `{"requested"}` and runs
when the turn ends, because the turn holds the repository lock; the result then wakes the lead.

### 4.6 A team turn, end to end

```
you      Add request validation to the API and document it.
lead     set_plan(plan: [validate, document, review], next_step: "start both")
         start_agent(backend, write=true, "Implement validation in src/api.rs; run cargo test")
         start_agent(docs, write=true, "Document the validation rules in docs/api.md")
● 2 background agents launched
lead     I've started `backend` and `docs`; I'll merge them when they report.      ← turn ends
backend  set_status("taking src/api.rs")  … send_message(lead, "error type: ValidationError
         in src/error.rs — ok?")                                        ← lead idle: wake (60 s)
lead     (mail turn) send_message(backend, "yes; keep it in api.rs")                ← turn ends
docs     read_messages → sees backend's status; writes docs/api.md; finishes
backend  finishes: "5 files; cargo test passes"
● (completion turn, both results, held until both ended)
lead     merge_trial(backend)  merge_trial(docs)                                    ← turn ends
● Merge trial of `backend` finished: clean · checks passed; `docs`: clean · 1 overlap (docs/api.md)
lead     apply_merge(backend)  → user: "Apply `backend` to your tree? 5 files · cargo test passed"
you      y
● Applied `backend`: 5 files written.
lead     apply_merge(docs)  …
lead     Both merged; validation is in src/api.rs with tests, rules in docs/api.md.
```

Six resumed turns of the lead for two helpers and two merges; the holds keep it at that.

## 5. Store

Additive only, through `activity::added` (`ADDED`, `ADDED_TABLES`), `USER_VERSION` and
`VIEW_API` unchanged, as the supervision design's columns were added.

| Object | Change |
|---|---|
| `threads` | `management TEXT` (the plan JSON) |
| `agent_requests` | `kind TEXT NOT NULL DEFAULT 'start'` (`start` \| `stop` \| `trial` \| `apply`), `payload TEXT` (the tool's arguments as written). `name`, `title`, `task` stay for `start` |
| `workspaces` (new) | `thread_id, name, root, base, seat_id, created_at, state CHECK IN ('open','applied','abandoned')` — the manifest mirrored so `v_*` can show it; `workspaces.json` stays the source for the console |
| `merges` (new) | `id, thread_id, workspace, n, state CHECK IN ('running','clean','conflicts','failed','applying','applied','declined','stale'), method ('merge-tree' \| 'three-way'), ours, theirs, base, files, overlap, conflicts, checks, requested_by (seat id), resolution_seat, created_at, ended_at, applied_at` |
| `turns.origin` | gains the words `'merge'` and `'mail'` (a plain `TEXT`, no CHECK) |
| `messages` | `woke INTEGER NOT NULL DEFAULT 0` — this message woke the lead; what the budget counts |
| `v_team` (new) | One row per helper, workspace and merge of a thread for the console's `/team`, `orochi threads team <id>` and the desktop's Team pane |
| `v_sidebar` | `merges_waiting` (clean trials not applied) beside `background` |
| `v_open_prompts` | an `apply` prompt carries the trial's files, overlap and checks in `request` |

Retention needs nothing new: every row hangs off a thread. `merges.conflicts` and `overlap` are
paths, `payload` is the lead's words — content, in this file only, never in telemetry.

## 6. The console

| Surface | Change |
|---|---|
| `/team` (new) and `/workspaces` | `v_team` as a table: name, title, state, elapsed, route, worktree, last trial, `applied`. `/workspaces` keeps its lines and adds the trial column |
| ↓ manager overlay | Rows for trials and applies (`trial 2 · clean · checks passed · a to apply`), `a` asks the apply question from the overlay too |
| Apply question | The team dialog's chair: `Apply `backend` to your working tree? 5 files · cargo test passed · 1 file you also changed (src/api.rs)` → `Yes` / `No` (Esc), `Yes` only after a trial; `No` wakes the lead with "declined" |
| Launch block, finish lines | unchanged; a trial start prints `⎿ merge trial of backend started`, its end a finish line |
| Status row | `· 1 merge waiting` after `· 2 in background` |
| Keys | unchanged; Esc still interrupts the lead only; a mail turn is interruptible like any turn |

## 7. The headless host and clients

- `orochi host` reads `threads.continuation` on start and resumes the session when `loadable`;
  else it gets the §4.2 block. This is a fix with its own value and comes first (§12, M0).
- An `apply` prompt under a host is a `prompts` row (`Answerer::Store`), answered from the
  desktop's card or `orochi threads answer <prompt> yes|no` (new; today there is no CLI answer
  path, which also leaves helpers' command questions unanswerable from a terminal).
- `orochi threads team <id>` prints `v_team`, which is what keeps the view honest.
- Desktop: the Team pane draws `v_team`; the apply card shows files, overlap and checks; the
  sidebar's row gains the merges-waiting count.
- A host with a running trial or apply is not idle.

## 8. Configuration

| Key | Default | Meaning |
|---|---|---|
| `manager.apply` | `"ask"` | `"ask"` \| `"auto"` (D5). Auto applies only a clean trial whose checks passed as real checks; everything else still asks |
| `manager.enabled` | `true` | Off: the lead keeps today's tools, nothing wakes it but a finished helper |

Nothing else: the wake hold (60 s), the per-helper wake cap (3), the plan bound (16 KiB) and the
re-trial count (1) are constants until measured.

## 9. Invariants

| Invariant | How it holds |
|---|---|
| Constraints bound every smart layer | Every lever is an `agent_requests` row Orochi decides; the lead cannot write the user's tree, pick a model, pass a gate or exceed the cap |
| `collaborate` leaves the tree alone without `--apply`, and applies only verified results | Untouched. The console's apply has its own chair: the user's yes, or `auto` with real checks |
| A verified outcome is never relabeled; feedback is the user's | Mail, merge and completion turns never write `feedback`; a trial's checks label the trial row, never a run record |
| Telemetry is content-free | Plans, payloads, paths, overlap and results live in `activity.sqlite3`; a trial is not a run and writes no `RunRecord`; the resolution seat's record carries kinds and usage only |
| A read-only seat cannot write | Unchanged; a resolution seat writes only in its trial worktree, as a writable helper does |
| Nothing in the stores is transmitted | The §4.2 block is a thread's own rows to its own next agent, the exception the memory design already names; sibling worktrees are listed by name, branch, route and status, as `list_peers` lists them today |
| The target repository is never trusted | Trials run the project's configured or detected checks, as every turn in the tree already does; nothing is read from the repository as configuration |
| The user's HEAD, branch and index are unchanged | Apply writes files only; snapshots use a private index; worktrees are detached |
| Records written by older versions read back | Every column defaulted or nullable; `USER_VERSION` and `VIEW_API` unchanged |

## 10. Tests

| Area | Test |
|---|---|
| Store | `tests/core.rs`: `added` adds each column and table once under two concurrent opens; an older store opens; byte-level check that a plan, a payload and a conflict path never reach `telemetry.sqlite3` |
| Levers | `tests/mailbox.rs`: each gate's refusal; `list_work` on an empty and a full thread; `set_plan` bound; `stop_agent` ends the seat as `Cancelled`; a non-delegate cannot call any of them |
| Merge | new `tests/merge.rs` with a scratch repository: clean trial; overlap reported; conflicts reported with markers in the trial worktree; `resolve` with a mock agent that clears markers and one that does not; stale tree re-trialed once then refused; apply writes exactly the diff, leaves HEAD, branch and index unchanged, and refuses a target that moved between snapshot and write; the `three_way` fallback on a forced old-git path |
| Wake-ups | `tests/cli_e2e.rs` with mock agents: a helper's message while idle wakes the lead once within the hold, three messages make one turn, a fourth helper message after the cap waits; a trial result wakes; an apply during a turn runs after it |
| Resume | `tests/cli_e2e.rs`: a host restarted mid-conversation resumes the loadable session; with a non-loadable agent its first turn carries the §4.2 block with the plan and the rebuilt transcript |
| Console | `tests/test_chat_terminal.py`: `/team` table, the apply dialog with Esc meaning No, the overlay rows |
| Learning | `tests/adaptive.rs`: mail and merge turns write no feedback and are routed by the continuation |

## 11. What is reused

| Piece | From | Used for |
|---|---|---|
| `agent_requests` + `grant` on the mail tick | supervision | every lever |
| `Workspaces::create`'s snapshot commit | session workspaces | `snapshot()` for both sides of a trial |
| `Coordination` | collaboration | the plan |
| `three_way`, `Change`, `commit` with `expected` | collaboration | the fallback merge and every apply write |
| `untangle`'s instruction and marker check | collaboration | the resolution seat |
| `apply` prompt kind, the team dialog | console | the apply question |
| `completion_message`, `HOLD` | supervision | merge and mail turns |
| Memory-note injection order | memory | the §4.2 block |

## 12. Phases

| Phase | Delivers | Depends on |
|---|---|---|
| **M0 Resume** | `threads.continuation` read by the host; transcript rebuilt from `items` for a non-loadable agent; `alongside` names `start_agent` | — (fixes a loss that exists now) |
| **M1 State and levers** | §5 columns and tables; `set_plan`, `list_work`, `stop_agent`; the §4.2 block; `/team`, `orochi threads team` | M0 |
| **M2 Wake on mail** | §4.4 mail turns, hold and cap; `messages.woke` | M1 |
| **M3 Trial** | `snapshot()`, `merge_trial` with `merge-tree` and the fallback, trial worktrees, checks, the resolution seat, merge turns | M1 |
| **M4 Apply** | `apply_merge` under `ask`; the apply dialog, card and `threads answer`; `commit` with stale check under the exclusive lock | M3 |
| **M5 Auto** | `manager.apply = "auto"` | M4, M6 numbers |
| **M6 Validation** | Real leads on both adapters: does a lead plan, delegate, answer a helper, trial and apply sensibly; wake cost per turn; agreement between the lead's apply judgment and the user's answer — recorded in a dated `real-validation-*.md`. Nothing above is claimed verified until then | M2, M4 |

## 13. Risks and what is unverified

| Risk | Handling |
|---|---|
| A resident manager costs a resumed session per wake | Holds and the per-helper cap (§4.4); `Usage::weighted` of mail/merge turns shown under their own origin in Insights; measured in M6 before any constant becomes a setting |
| A lead applies something half done because the checks are thin | `ask` is the default; the dialog says what the checks were, or that there were none; `auto` needs real checks |
| A lead's plan drifts from what the team did | The §4.2 block is Orochi's rows first, the lead's plan second; `list_work` reads rows, never the plan |
| Two apply requests race | One `merges` row may be `applying` per thread; the second is answered "an apply is running" |
| The trial reads a tree the lead is editing mid-turn | A trial is advisory; apply re-snapshots under the exclusive lock, after the turn |
| `git merge-tree --write-tree` absent | The `three_way` fallback, with its copy budget, and the method named on the row |
| A helper wakes the lead for nothing | The cap, the hold, and the helper told in the reply when its message will be read; measured in M6 |
| Whether a real lead uses `set_plan` and `merge_trial` at all | **Unverified**; the tool descriptions are the lever, and M6 decides whether the prompt note has to say more |
| Resolution seats loop on an unresolvable conflict | One resolution per trial; a second `resolve` is refused with "resolved once already; decide, or stop and redo the part" |

## 14. Where the implementation departed (2026-10-16)

| Designed | Built | Why |
|---|---|---|
| `/team` for the team view | `/work` | `/team <task>` already runs a team |
| A `three_way` fallback where `git merge-tree --write-tree` is absent | `merge_trial` refuses with "git 2.40 or newer is needed for merge trials" | `--merge-base` (2.40) is what makes the base the launch snapshot rather than the commits' ancestor; a fallback that merges from the wrong base is worse than none |
| `workspaces` table and `v_team` view | `merges` table and `v_merges` view only; the console reads `workspaces.json` | The manifest already exists and a client has no reader for it yet; adding a mirror nobody reads would be the dishonest view the rule warns about |
| `turns.origin` gains `'merge'` and `'mail'` | As designed; a turn carrying helper results **and** merge or mail reports is `'background'` | One turn, one origin; the helper's report is the costlier thing it carries |
| An apply request during a turn runs when the turn ends | As designed, through the idle loop; `apply_merge` answers `requested` at once | The repository lock is `try_lock`, so an apply cannot wait for a turn: it is run only where no turn holds the tree |
| A stale trial re-run once, then applied | As designed, inside the apply: a tree that moved is tried again on the tree as it is now, checks run again, and only a clean, passing result is written; the stale trial and the new one are both recorded | Measured in the fixture: a lead's own turn moves the tree (it writes its completion marker), so without the retry no apply would ever land |
| Piped console (no terminal, not a host) asked to apply under `ask` | Declined, reported to the lead as declined | Nobody can answer; the lead is told to use `auto` or a terminal |
| `orochi threads team <id>` | Not built; `v_merges` is readable with `--json` through the store | Nothing on the client side reads it yet |
| Checks of a trial under a project sandbox | Run where the console runs, not in the session's container | `evaluate` is called outside the scheduler's placement; a sandboxed project's trial checks are **unverified** |
| `alongside` says "start helpers only with `start_agent`" | As designed (the sentence the supervision design had planned) | — |

Pinned by `tests/merge.rs` (library), `tests/mailbox.rs` (the tools are requests; only a
session marked manager lists them) and `tests/cli_e2e.rs` (a lead that plans, delegates,
trials, applies under `auto` with the stale retry, the rows and telemetry's silence; a helper's
message waking an idle lead; a restarted host loading the recorded session).
