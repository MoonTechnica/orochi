# Parallel Execution Design: Dependency-Ordered Work Inside One Process

Written: 2026-09-19.

**Status (as of 2026-09-19)**

| Phase | Status |
|---|---|
| P1 Work graph, gate and waves in `collaborate` (parts from a plan file) | **Implemented; automated tests only** (mock agents) |
| P2 Parts from the coordinator's first reply, chain contraction, stray-write measurement | **Implemented; automated tests only**. Whether real coordinators propose useful divisions is **unverified** |
| P3 Console: parts from the design phase, progress through the screen, claim-at-choice | **Implemented; automated tests only**, including terminal tests that answer the question and interrupt a running team. Behavior with real agents is **unverified** |

What shipped is described in [Collaboration and Handoff Between Independent Sessions](session-collaboration.md) ("The order of the work") and [Adaptive Routing and the ACP Gateway](adaptive-routing.md) (§4.2 seat spreading, §4.3 console escalation). This document is kept as a record of how the design came about; §0–§10 remain as designed, and the points changed during implementation are collected in §11.

## 0. Goal and starting point

One goal: **what can run in parallel runs in parallel, what must run in order runs in order, and the task decides which — not a shape fixed in code.** The user still hands over nothing but a prompt.

Facts found by surveying the code as of 2026-09-19:

| # | Fact | Basis |
|---|---|---|
| 1 | **Concurrency inside one process already exists.** Agents are child processes; Orochi multiplexes their I/O as futures on one task (`join_all`, `FuturesUnordered`, `buffer_unordered`). Only the ACP connection lifecycle is a spawned task | `scheduler/mod.rs:128`, `router/advisory.rs:298`, `chat/mod.rs:1460`, `collaboration/turn.rs:765`, `acp.rs:417` |
| 2 | **The shape of the work is fixed in code, twice.** The console runs `design → implement → review` strictly in order in the user's tree. `collaborate` runs `[coordinator] → implementers → [merge] → reviewers → [discussion] → integrator → [apply]` | `chat/mod.rs` `PHASES` / `run_team`, `collaboration/mod.rs` `Plan::steps` |
| 3 | **The only thing read from the task is a head count.** `plan::derive` picks 1–3 implementers from complexity, capped by the number of top-level areas, and deals the areas out round-robin. Every implementer receives the whole task text plus "Paths you own" | `collaboration/plan.rs` |
| 4 | **There is exactly one wave.** Every implementer starts from `baseline` and `merge_implementers` merges all of them against `baseline`. Work that must build on another part's result has nowhere to go but the integrator | `turn.rs` `prepare`, `apply.rs` `merge_implementers` |
| 5 | **The console decides whether to split before anything has read the repository.** `team_for` asks the user on the strength of the area heuristic, before the design phase runs | `chat/mod.rs` `team_for` |
| 6 | Collaboration reports progress with `eprintln!`, not through `acp::Progress`, including when the console starts it | `turn.rs:526`, `apply.rs:55` |
| 7 | Seats in a console turn are kept off one account by starting each 1.2 s after the last. A collaboration turn group starts all members at once, each reading `busy_agents` before any has claimed anything (effect on real accounts **unverified**) | `chat/mod.rs:1464`, `turn.rs:428`, `turn.rs:765` |

So the execution machinery is not what is missing. What is missing is (a) a description of the work that carries order, (b) more than one wave, and (c) a decision about splitting made by something that has read the code.

## 1. Requirements

| ID | Requirement | Reason |
|---|---|---|
| R1 | **Prompt only.** No new required command or setting; a plan file stays optional | The product's premise |
| R2 | **Order comes from the task.** Independent parts run concurrently, dependent parts in sequence | The goal |
| R3 | **Serial unless shown independent.** Parallelism needs evidence (disjoint, declared write sets); any doubt resolves to serial | A wrong "serial" costs time. A wrong "parallel" costs a merge conflict and a session spent resolving it |
| R4 | **Decomposition is advice; Orochi gates it.** A model may propose the graph, never enforce it | Invariant: constraints bound every "smart" layer |
| R5 | Existing invariants hold: the user's tree changes only through a verified apply; only real check passes are `Success`; nothing sensitive reaches SQLite; the target repository is never trusted | `CLAUDE.md` Invariants |
| R6 | **No extra LLM call to decide the split.** It rides on a turn that already runs | Same rule as memory capture riding on the classifier |
| R7 | **Small work pays nothing.** A task that does not split takes today's path unchanged | Most turns are one agent |
| R8 | **Resumable and backward compatible.** Schema-2 reports without the new fields resume; step numbering for plans without parts is unchanged | Existing test-pinned behavior |
| R9 | **Bounded.** Width, part count and brief size are capped in code | One prompt must not start twelve agents |
| R10 | **Visible.** The graph is shown before it runs and `--dry-run` prints it as an editable plan file | A hidden split is a hidden bill |
| R11 | **Measurable.** The report records enough to tell whether splitting helped and whether declared write sets were honored | R3 rests on declared write sets being true; that has to be checkable |

Out of scope: several writers in one working tree within one process (attribution of a failed check to a run becomes impossible, and the unchanged-`tree_fingerprint` shortcut breaks); ready-queue execution of the graph (§4.2); running separate console messages concurrently; moving runs onto `tokio::spawn`; per-part cancellation; one-shot `orochi "<task>"`, which stays a single agent.

## 2. Overview

```
task ─→ design turn (already runs) ─→ parts[] ─→ gate (Orochi, deterministic) ─→ waves
                                                  │ invalid / absent
                                                  └→ today's shape

waves:   [a ‖ b] ─→ merge ─→ [c] ─→ [d ‖ e] ─→ merge ─→ review ─→ integrate+verify ─→ apply
         copies     3-way     copy    copies     3-way                real checks        guarded
```

Two ideas carry the design:

- **Work and workers are separate.** A *part* is a piece of work with an order. An implementer *seat* is routing (agent pin, allowed agents, fallback). Seats bound how wide a wave can be; parts say what runs in it.
- **The graph is folded into waves.** A wave is a turn group, which is what `Step::Turn` already is. Nothing new executes anything; the step list just gets longer.

## 3. The work graph

### 3.1 Shape

`Plan` gains one optional field. A plan without it behaves exactly as today (R7, R8).

```json
{
  "participants": [ ... ],
  "parts": [
    { "id": "schema",  "brief": "Add the orders table and its migration", "paths": ["db"],  "after": [] },
    { "id": "api",     "brief": "Expose POST /orders",                     "paths": ["api"], "after": ["schema"] },
    { "id": "console", "brief": "Add the orders screen against the agreed route", "paths": ["web"], "after": [] }
  ]
}
```

`brief` is task-derived text. It lives in `report.json`, which already holds the task and the replies, and never in SQLite (R5).

### 3.2 Where parts come from

In order; the first that yields a gated graph wins:

1. **`--plan` with `parts`** — the user's own file.
2. **The design turn's reply.** In `collaborate` that is the coordinator's stage-0 turn, whose JSON-only reply gains an optional `"parts"`. In the console it is the design phase, asked to end with one `{"parts":[…]}` object only when the work really divides (§6).
3. **Neither** — today's derived shape: the area heuristic, one wave. With fewer than two areas, one implementer.

Parts are accepted **once**, when the design turn completes, written into `report.plan.parts`, and never rewritten. A coordinator's later turns cannot reshape the graph. This is the same stance as a frozen prediction, and it is what keeps resume deterministic: `Plan::steps` is recomputed from the saved plan, and the step list before and after acceptance shares the prefix `[Turn(coordinator)]`, so `next_stage` stays valid.

### 3.3 The gate

`collaboration/graph.rs`, pure and deterministic. Rejection of the list means fallback 3, never an error.

| Check | On failure |
|---|---|
| ≤ 12 parts; `id` matches `[a-z0-9-]{1,32}` and is unique; `brief` ≤ 2 KiB; ≤ 32 `paths`, each passing the existing `relative()` | reject the list |
| every `after` names a listed part; no cycle | reject the list |
| two parts with no order between them whose `paths` overlap (equal, or one a prefix of the other) | **add an edge**, earlier-listed first |
| a part with empty `paths` (write set unknown = the whole tree) | **add edges** so it runs alone in its wave |
| a wave wider than the number of implementer seats (≤ 4) | split the wave in listed order |

Then two normalizations:

- **Chain contraction.** A part whose only successor has it as its only predecessor is fused with that successor (briefs concatenated in order, paths united). A straight chain becomes one part: one agent holding the whole context does a sequence better than three fresh sessions handing notes along, and it costs no copies or merges. Separate sessions are spent only where they buy concurrency.
- **Layering.** `wave(p) = 1 + max(wave(q) for q in after(p))`, `0` with no predecessors.

A graph whose widest wave is 1 after contraction is a single part — today's path.

## 4. Execution

### 4.1 Steps

`Step` gains `Parts(usize)` (a wave). With parts, `Plan::steps` yields:

```
[coordinator]
for each wave w:  Parts(w) [coordinator]   and, when the wave is wider than 1:  Merge
reviewers … [discussion …] integrator [apply]
```

Plans without parts never produce `Parts`, so their numbering does not move.

| | Without parts (today) | With parts |
|---|---|---|
| Workspace of a writer | `<output>/<implementer id>/` from `baseline/` | `<output>/parts/<part id>/` from the wave's base |
| Base of wave 0 | `baseline/` | `baseline/` |
| Base of wave w+1 | — | `merged/<w>/`, or the single part's workspace when wave w had width 1 |
| Merge | all implementers against `baseline/` | the wave's parts against the wave's base |
| Reviewed / integrated tree | `merged/` or the one implementer's | the last wave's result |

A part runs on seat `index_in_wave mod seats`, so a hand-written plan that pins implementer 1 to one agent and implementer 2 to another still means something. Every part turn is a fresh session, as every collaboration turn already is.

### 4.2 Why waves and not a ready queue

Folding the graph into waves makes a slow part hold up parts that only depended on its faster wave-mates. A ready queue would not. Waves are chosen anyway: they are `Step::Turn`, they checkpoint and resume with the machinery that exists, and a merge between waves is a well-defined point with nothing else running. A ready queue needs a merge base per part and a resume story for a partially merged frontier. Revisit only if reports (§7) show waves idling.

### 4.3 Evaluation and failure

- **Part turns are never gated.** A part in the middle of a graph may legitimately leave the project failing (the interface changed; the callers are the next wave). Only the integrator's turn runs the evaluator, as with parallel implementers today. `Success` still means real checks passed on the integrated tree, and apply still requires it (R5).
- **A failed part** keeps its wave-mates' completed work; resume reruns only what did not complete. Later waves wait — that is what the order means.
- **A conflicted merge between waves** must not become the next wave's base. The integrator seat runs one `TurnKind::Resolution` turn in the merged workspace (the machinery apply already uses). Markers still present afterwards end the run `Blocked`, resumable. The last wave's conflicts go to the integrator as today.
- Declared write sets make conflicts the exception: the gate only lets disjoint `paths` share a wave. `paths` remains guidance, not enforcement — but each merge records how many changed files fell outside their part's `paths` (`MergeRecord::strays`, `#[serde(default)]`). That number is the evidence for or against R3's premise (R11).

### 4.4 Prompts and talking

A part turn's prompt carries the user's task, its own `brief` and `paths`, and the list of all parts with their state, so it knows what it is *not* doing. Parts in one wave are live at the same time and already have the mailbox; `mailbox::prompt_note_for` already asks a peer to hand over an interface as soon as it holds. Two parts that only need to *agree* on an interface can therefore share a wave; two where one needs the other's *code* cannot — that distinction is put to the design turn in so many words.

## 5. Process-level concerns

- **Execution model: unchanged.** Futures on one task. Moving runs to `tokio::spawn` would force `Config`, `Registry` and `Store` (`rusqlite::Connection` is `!Sync`) behind `Arc`/`Send + 'static` to gain nothing the child processes do not already provide.
- **Blocking file work.** `copy_workspace`, `scan` and `three_way` are synchronous and bounded at 100 MiB / 20,000 files. They run before a wave starts and between waves, when no agent of this run is streaming. Under the console they would stall the key loop; wrap them in `spawn_blocking` in P3 if `tests/test_chat_terminal.py` shows it. **Unmeasured.**
- **Claim at the point of choice.** Members of a turn group choose routes concurrently, each reading `busy_agents` before any has registered (fact 7). Because everything runs on one task, "read busy → choose → claim" with no `.await` in between is atomic: an in-process claim table consulted alongside `busy_routes`. This replaces the console's 1.2 s stagger with ordering instead of time, and gives collaboration the spreading it lacks.
- **Interruption** stays process-wide: Esc / Ctrl-C cancels the run, the report is saved `Cancelled`, and resume continues with the incomplete parts. Per-part cancellation is not needed for that.

## 6. Console

1. `Session::steps` decides, as today, whether a design phase runs. Work that gets no design phase is `Normal` work and is not split.
2. The design instruction gains one sentence: end with `{"parts":[…]}` only if the work divides into pieces that can be built without each other's code. The console holds that trailing object back from the transcript and shows the gated graph instead, in one line: `schema → api ‖ console`.
3. **The question moves to after the design.** `team_for` stops asking on the area heuristic. If the gated graph has a wave wider than 1, the console asks once, showing the graph; otherwise the implement phase runs in the user's tree exactly as today. Off a terminal it never asks and never splits. It remains the one escalation that asks first, for the reason it always was: it starts several agents at once and writes back.
4. On yes, `collaboration::run` starts with the parts in the plan, **no coordinator** (the design phase was that turn; its reply seeds `management`), implementer seats equal to the widest wave, one reviewer — two for structural work — and the integrator.
5. Collaboration reports through an optional `EventSink` instead of `eprintln!` (fact 6), each part shown as an aside under its id, as seats are today.

## 7. Measurement

`report.json` gains, all `#[serde(default)]`: per merge `strays`; per run `elapsed_ms` beside the existing per-session `duration_ms`. From these: wall-clock against the sum of part durations (did the split save time), conflicts and resolution turns per wave (what it cost), strays per part (whether declared write sets can be trusted). None of it enters SQLite or routing learning; collaboration sessions stay out of per-route learning as they are now.

## 8. Tests that pin it

| Test | Pins |
|---|---|
| gate: cycle, unknown `after`, bad id, oversize → list rejected, plan falls back | R4 |
| gate: overlapping or empty `paths` without an order → serialized; same input → same waves | R3, R8 |
| gate: straight chain → one part; no `Parts` step produced | R7 |
| plan without `parts` → step list identical to today's | R8 |
| wave 1's workspace contains wave 0's merged result (mock agents) | R2 |
| interrupted mid-wave → resume runs only the incomplete part | R8 |
| conflicted mid-graph merge → resolution turn; markers left → `Blocked`, next wave not started | R5 |
| unverified integrated result → not applied | R5 |
| coordinator's later reply carrying `parts` → ignored | §3.2 |

All with mock agents; none of this is verified against real CLIs until a dated validation document says so.

## 9. Phases

- **P1** — `parts` in `Plan`, `graph.rs` (gate, contraction, layering), `Step::Parts`, per-wave workspaces and merges, mid-graph resolution, `--dry-run` output. Input only from `--plan`, so the engine is tested with no model in the loop.
- **P2** — parts from the coordinator's stage-0 reply, accepted once; `strays` and `elapsed_ms`.
- **P3** — console: design-phase parts, the question after the design, `EventSink` progress, claim-at-choice replacing the stagger.

## 10. Open decisions

| # | Question | Recommendation |
|---|---|---|
| O1 | Keep the console's question before splitting? | Keep, moved after the design and showing the graph. Dropping it is a separate decision about cost, to be taken with §7's numbers in hand |
| O2 | Caps: 12 parts, width 4 | Width 4 is today's implementer cap. 12 is a guess; tighten it if real design turns over-decompose |
| O3 | Run checks after every wave and pass the result forward as information? | Not in P1–P3. It is the obvious next step if reports show integrators spending their turn discovering what broke three waves earlier |

## 11. Changes during implementation

| Design | Implemented | Why |
|---|---|---|
| A gate rejection always falls back to today's shape (§3.3) | Only for model proposals. A plan file whose parts cannot be ordered is a `validate` error, like any other invalid plan | The user wrote the file; silently running something else would hide their mistake |
| — | A part may not share a participant's ID; `parts` joined the reserved participant IDs | Parts are sessions in the report and addressees in prompts; an ID must name one thing |
| The design reply seeds `management` (§6.4) | It is appended to the collaboration's task text ("The design step concluded: …") | Every part, reviewer and the integrator read the task; `management` is only the coordinator's, and a console team has no coordinator |
| Claim at the point of choice replaces the stagger (§5) | Claims (`mailbox::claim`) **and** an order in which the seats of one console turn take their places (`mailbox::Order`, `RunOptions::place`); a place is taken once the seat has chosen and joined the mailbox | Claims alone let whichever seat finished discovery first choose first, so a read-only seat could take the route the lead should have. Releasing the next seat at the choice alone was not enough either: a seat could join and prompt while the lead was still being set up, find nobody in the mailbox and stop waiting (the mock five-seat test failed 6 of 8 runs until the place was released at joining). The order keeps the lead first without delaying anyone's discovery. Collaboration parts are equals and use claims only |
| — | Collaboration sessions claim their route too | They never announced one to the mailbox, so the parts of one wave could not see each other at all; a test with two otherwise equal accounts pins the spreading |
| — | The conflict-marker check runs whether or not a turn is gated | A mid-graph resolution turn must clear the markers without having to pass the project's checks |
| Hold the trailing object back (§6.2) | Held back while it streams; text that only looked like its start is released as soon as that is clear, and an object that turns out not to end the reply is shown at the end | Nothing but the division itself may go missing from what the agent said |
| — | Interrupting a running console team reports "team stopped" with the resume command | The console had no key handling during a team run; Ctrl-C and Esc now stop it through the interrupt the collaboration already listens for |
| — | Interrupts are counted process-wide (`interrupt.rs`); runs stop at their next wait on an interrupt that came while nothing was waiting | A `ctrl_c()` future only hears what arrives while it waits, so a part between two attempts could carry on after the user stopped the team |
| The question stays a yes/other-key prompt (§6.3) | A Claude Code plan-approval dialog: `1` team, `2` one agent here, `3`/Esc keep the design and build nothing (2026-09-20) | The console's keys follow Claude Code. Esc had meant "carry on as one agent", which started writing to the user's tree — the opposite of what Esc means everywhere else |
| `spawn_blocking` for workspace copies if the terminal test shows a stall (§5) | Not done | The terminal tests show no stall at fixture sizes. Copies of large repositories inside a console team are **unmeasured** |
| — | Without the 1.2 s stagger, a read-only seat beside a fast mock lead now actually runs; two tests that counted prompts count only the steps' own | The stagger had kept the seat from ever speaking in those tests, which is not what happens with real agents |

