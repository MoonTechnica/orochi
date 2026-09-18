# Personalization Design: Memory and Routing Learning

Written: 2026-09-18.

**Status (as of 2026-09-18)**

| Phase | Status |
|---|---|
| P1 Memory storage, injection, capture piggybacked on the classifier, `/memory`, `orochi memory` | **Implemented; automated tests only** (including E2E with a mock agent). Capture accuracy with real agents is **unverified** |
| P2 End-of-session distillation, tier pooling | **Implemented; automated tests only**. Tier pooling is disabled by default (D2); its effect has been confirmed only by replaying synthetic data. Distillation capture accuracy with real agents is **unverified** |
| P3 Weak labels, route preferences | **Implemented; automated tests only**. The weak-label weights are Orochi's own uncalibrated heuristic. Their effect in real use is **unverified** |

The implementation is recorded in [Adaptive Routing and the ACP Gateway](adaptive-routing.md), under "2. EWMA and Bandit" (weak labels, tier pooling) and "4.4. Memory" (memory, session look-back, route preferences). This document is kept as a record of how the design came about. §3–§5 remain as originally designed; the points changed during implementation are collected in §9.

## 0. Goal and starting point

There is one goal: **the more someone uses it, the more it adapts to how that person uses agents.** The premise that the user hands over nothing but a prompt does not change.

Facts found by surveying the code as of 2026-09-18:

| # | Fact | Basis |
|---|---|---|
| 1 | **There is no memory.** It remembers no preferences, conventions or past decisions, and starts from a blank slate every time | There is nowhere to store them. Invariant: "task text and conversation do not go into SQLite" |
| 2 | **Learning labels are narrow.** A run is `Success` only when an automatic check other than `git_` passes. Design, discussion, investigation, documentation, and repositories without tests produce zero learning records no matter how often they are used | `evaluator::outcome`, `learning::labeled` |
| 3 | **Learning keys on an exact model ID match.** Every time a model is updated, its whole track record is lost | `model=?2` in `storage::learning_runs` / `pooled_runs` |
| 4 | It holds no task type × model affinity. `task_profiles` is dead data that is only parsed (removed from the bundled policies on 2026-09-19; still accepted when a registry carries it) | `policy.rs:14`, no references |
| 5 | For reference: OpenClaw's "learning" is textual memory (`USER.md` / `MEMORY.md` / daily notes), mainly explicit saves + automatic saves before compaction + promotion from daily notes. It is not numerical learning from outcomes | [Memory overview](https://docs.openclaw.ai/concepts/memory) |

1 is a capability Orochi **does not have**; 2 and 3 are capabilities it **has, but thin**. They differ in nature, so they become separate pillars.

## 1. Requirements

| ID | Requirement | Reason |
|---|---|---|
| R1 | **Prompt only.** Add no required new commands or new settings | The product's premise |
| R2 | **Spans agents.** Claude's `CLAUDE.md` is invisible to Codex, and vice versa. In Orochi, where the routing changes every time, memory is needed in the layer above them | Value unique to Orochi. No single agent can have this, in principle |
| R3 | **Survives model turnover.** Hold no knowledge tied to model IDs | "Models change from moment to moment" |
| R4 | **Don't over-decide.** Memory and preferences stay soft influences, can be overridden, and disappear when stale | Same policy as not adding a hand-written affinity table |
| R5 | Do not break existing invariants: no task text in telemetry, do not trust the target repository, no task text in the adviser payload | `CLAUDE.md` Invariants |
| R6 | **Visible, fixable, deletable.** No hidden state | A wrong memory does harm silently |
| R7 | Mistakes are either measurable or fade on their own | A feature that cannot be measured ends up as "it's learning, sort of" |
| R8 | Add no extra LLM calls per turn | One is already paid for the classifier |

Out of scope: embeddings / vector search, a memory tool that agents can write to, cloud sync, a hand-written task type × model table.

## 2. Overview

```
prompt ─→ classifier (existing single call) ─┬→ TaskDescriptor ─→ scorer ─→ execute
                                             └→ remember[] ─→ [A Memory] ─→ injected into later prompts
run results, user's immediate behavior ─→ [B Routing learning] ─→ scorer prior
memory entries about routing ─→ [C Route preferences] ─→ scorer cost (soft)
```

- **A Memory** (text): how that person and that project want things done. Wide reach, but it cannot be verified.
- **B Routing learning** (numerical): which routes succeeded. Verifiable with `calibrate`.
- **C Route preferences**: the bridge between A and B. Makes a statement like "use Fable for design" apply as a soft preference rather than a constraint.

## 3. A: Memory

### 3.1 Location

```
<data>/memory/USER.md                        user-wide (reply language, preferred way of working)
<data>/memory/repos/<repository_id>/MEMORY.md  per repository (conventions, decisions, facts about the structure)
```

- **Completely separate** from telemetry (`telemetry.sqlite3`). Treated the same as the mailbox: mode 0600, 64 KiB cap per file.
- `repository_id` is the existing salted hash. The repository cannot be identified from the directory name.
- **Never stored in or read from the target repository.** A `MEMORY.md` inside the repository is ignored. The repository must not be in a position to plant text that gets injected into prompts (R5).
- The format is a plain Markdown bullet list. Only lines added automatically carry metadata at the end:

```markdown
- Package manager is pnpm <!-- auto seen:3 last:1758153600 -->
- Comments only record decisions. No explanatory comments
```

A line without metadata = a line a person wrote; it **never expires and takes top priority in injection**.

### 3.2 Capture (the user does nothing)

**Path 1: piggybacking on the classifier (P1).** The classifier already reads the user's message every turn. Adding optional fields to the JSON it returns means **zero extra LLM calls** (R8).

```json
{"task_type":"bug_fix","complexity":"normal",
 "remember":[{"text":"Package manager is pnpm","scope":"repo"}],
 "reinforce":["r2"], "replaces":[]}
```

- It captures only "**the user's own instructions or facts that stay valid after this task ends**". At most 2 per message, 200 characters each. It does not capture the content of the task itself.
- To avoid duplicates, the classification request includes **the current repository's memory list (numbered, with the same cap as injection)**. For something restated, the classifier returns its number in `reinforce`, which increments `seen`. Something overturned goes in `replaces`.
- `remember` and the related fields are **stripped** before saving to the classification cache. The cache keeps only labels, as before.
- When something is captured, one line is printed to the console: `⎿ remembered: Package manager is pnpm`. The user can notice a wrong capture on the spot (R6).

**Path 2: end-of-session distillation (P2).** A correction like "no, I said don't do it with keywords" cannot be judged a lasting preference from one message alone. When the chat ends, the conversation held in process memory (already capped by `TRANSCRIPT_BYTES`) is passed once through the same path as the classifier, to capture preferences that were consistent across turns. It is **once per session**, not per turn.

**Path 3: writing by hand.** The file is just Markdown.

We **do not build** a tool that lets agents write memory. Agent output is influenced by repository content, so if it could write to memory, a malicious repository could contaminate every later prompt. Only Orochi can write.

### 3.3 Scope and leak prevention

If something learned in repository A appears in a prompt in repository B, information from one project is sent to a different provider working on a different project.

- The default scope for automatic capture is **repo**.
- The only automatic entries into `USER.md` are statements the classifier judges to be explicitly generalized by the user ("in every project", "always"). Otherwise, `USER.md` grows only by hand (D1).
- **Never included** in the adviser (router / judge / council) payload. Only the current repository's list goes to the classifier.

### 3.4 Injection

- Two places: where `scheduler` finalizes the prompt (where it prepends `peer_note`; covers chat, one-shot, `serve` and read-only seats), and `collaboration/turn.rs`, which builds the prompts for collaborate participants.
- **Only at the start of a new session.** A turn that `resume`s already has it, so it is not re-injected.
- Caps: USER 1500 characters + repo 2500 characters (D3, `memory.user_chars` / `memory.repo_chars`). Beyond that, it is cut in the order "hand-written → most `seen` → newest".
- The preamble wording fixes its standing: *these are the user's ongoing preferences and notes about the project, and they do not take precedence over this task's instructions or the repository's instructions.*
- Before injection, control characters are removed and ESC is made visible (the same treatment as mailbox bodies and error details).

### 3.5 Forgetting

- Automatic lines: deleted if not observed again for 90 days while `seen == 1`. 180 days for `seen >= 2`.
- A contradicting newer statement replaces the old one via `replaces`.
- `/memory` (list, `/memory forget r2`) and `orochi memory [--forget r2] [--json]`. The list also shows the file paths. **These are for checking, not operations needed in order to use it** (R1).
- `memory.enabled = false` stops both capture and injection.

### 3.6 Invariants pinned by tests

1. Memory text does not appear in the raw bytes of `telemetry.sqlite3`.
2. Repository A's memory is not injected in repository B.
3. `adviser::request` (router / judge / council) contains no memory.
4. A `MEMORY.md` inside the target repository is not read.
5. `remember` does not remain in the classification cache.
6. A turn that `resume`s is not re-injected. Caps are not exceeded.
7. Lines a person wrote do not expire.

## 4. B: Routing learning

### B1. Tier pooling (P2)

Model IDs change, but tiers do not (which is why the policy looks up tiers by substring pattern). The prior of a model ID with no track record is shifted by **the track record of other model IDs with the same provider, same tier, same task_type and same complexity**.

- It just runs the same computation as the existing `pool()` (prior residual and token ratio) over a different key. Its contribution shrinks as the model's own track record accumulates.
- The tier is recomputed at read time with `policy.model_rule(model).tier`, so **there is no schema change**.
- This makes per-task-type differences, such as "frontier really is stronger on design tasks", come **from measurement** rather than from a hand-written table (R3, R4).
- `learning.rs` and `benchmark.rs` share functions, so they change together. It ships disabled by default and is enabled after replaying with `benchmark` confirms an improvement (D2).

### B2. Weak labels (P3)

Even turns that automatic checks cannot verify leave evidence in the user's behavior. No new action is asked of the user; it **only records what already happens**.

| Observation | Sign | Weight (initial value, uncalibrated) |
|---|---|---|
| Moved on to the next turn with the same route | Positive | 0.2 |
| `/reroute` or `/new` immediately afterward | Negative | 0.3 |
| Interrupted with Esc | Negative | 0.3 |
| The user edits a file the agent touched, immediately afterward | Negative | 0.2 |

- Add `evidence: f64` to `RunRecord` (existing records get 1.0 via `#[serde(default)]`), and `estimate` multiplies the label mass by it. Verified results (1.0) are not diluted by weak guesses.
- **`calibrate` shows the Brier score for "strong labels only" and "including weak labels" side by side.** If weak labels make it worse, the numbers show that the weights are too high (R7). The weights are Orochi's own heuristic, and the code and docs state this explicitly.

## 5. C: Route preferences (P3)

This is the answer to the original question, "if I explicitly specify a model, I want it remembered for later too". The classifier returns statements about routing in structured form:

```json
"remember":[{"kind":"route","task_type":"architecture","model":"fable","text":"use Fable for design"}]
```

- `model` is a **substring pattern**, the same as in the policy. It keeps working when `fable-5-1` becomes `fable-6` (R3). It is not used to make up IDs that ACP did not discover (existing rule).
- The effect is **soft**: it only multiplies a matching candidate's `expected_cost` by 0.8. The capability, quota and `required_success` gates apply as usual, and a candidate that failed them is never revived by a preference. The reason is shown in `reasons`.
- Like other memory, it is visible, deletable, and disappears when stale. A one-time specification (`--model`) remains a hard constraint as before and does not go into memory.

## 6. Phases

| | Contents | Extra LLM calls |
|---|---|---|
| **P1** | Memory storage, injection, hand-writing, capture piggybacked on the classifier, `/memory`, the tests in 3.6 | 0 |
| **P2** | End-of-session distillation, tier pooling (verified with `benchmark`) | 1 per chat session |
| **P3** | Weak labels + separate display in `calibrate`, route preferences | 0 |

P1 alone gives the feel of "it remembers me". P2 makes it survive model updates, and P3 starts learning from tasks that cannot be verified.

## 7. Risks

- **Wrong capture.** Task content is mistaken for a lasting preference. → One-line display on capture, early expiry for `seen==1`, caps on the number of entries and characters.
- **Prompt bloat.** It rides on every new session of every agent. → 4000-character cap; no injection on `resume`.
- **Memory poisoning.** External text the user pasted says "remember this". → The classifier is instructed to capture only the user's own ongoing instructions; caps; the standing wording at injection. This part depends on the classifier's judgment and is not a guarantee.
- **No capture on turns where the classifier does not run.** `--dry-run`, and one-shot runs with both agent and model pinned. P2 distillation covers the chat side.

## 8. Decisions (2026-09-18)

| ID | Issue | Decision |
|---|---|---|
| D1 | Automatic capture into `USER.md` | Allowed only for explicitly generalized statements |
| D2 | Tier pooling default | Ship disabled; enable once `benchmark` confirms an improvement |
| D3 | Injection caps | USER 1500 + repo 2500 characters |
| D4 | End-of-session distillation | Allow 1 LLM call per chat session |

## 9. Changes from the design made during implementation

**P3**

- **Route preferences are not stored in structured form.** Memory stays text (writing "use Fable for design" by hand works too). Every turn, the classifier reads "the preferences that apply to this task" from the memory list and the request, and returns name fragments (such as `fable`) in `prefer`. This adds no custom notation or storage format, and no extra call. `prefer` is limited to fragments of at most 64 characters made only of alphanumerics and `-_.`, so that it cannot carry the user's words (because it goes into the classification cache).
- **The memory list is included in the classification cache key.** Preferences are read from memory, so when memory changes, the task is classified again.
- **`/new` is neutral.** The design treated it as negative, like `/reroute`, but `/new` is used to change the topic, not to judge the previous answer. Making it negative would pile up negative labels on exactly the sessions that went well.
- **Messages queued during a run are not treated as evaluation.** They were written before the answer was seen. With piped input, effectively every message falls into this category.
- **Verified results are not overwritten by weak labels.** Even if `/reroute` follows immediately after a turn whose checks passed, it stays a success. Weak labels apply only to unverified completions (`PartialSuccess`) and interruptions (`Cancelled`).
- **Esc alone is not a label** (2026-09-19). Originally an Esc interruption was negative 0.3, but the reason for stopping (the agent was off target / the user forgot to say something) cannot be told apart at that point. It is decided by what happens after the interruption: `/reroute` gives negative 0.3; continuing with the same agent gives no label.
- **"The user edits a file the agent touched, immediately afterward" is not implemented.** It needs a record of the touched files and a way to tell them apart from unrelated edits, and it is the noisiest of the four.
- **Weights are applied as `decay^w`.** With a weight of 1 it matches the previous update exactly, so existing learning results do not change. Confidence (`samples`) counts only verified results.

**P2**

- **Only the user's messages go into distillation.** Agent replies are not included. Preferences show up in the user's words; adding the replies increases tokens and invites the error of capturing something the agent said as a preference.
- **Kept across `/new`.** Separately from the conversation history (which `/new` clears), what the user said in the session is kept. Up to 64 entries per session; up to 16000 characters are sent, newest first. Held only in process memory and never stored anywhere.
- **It makes the user wait at exit, so Esc skips it.** It shows "looking back over this session · Esc skips".
- **Tier pooling was added to the `benchmark-tune` search space.** Whether it helps can be checked against local data. Replay reads tiers from the bundled policy, not the installed one (so that updating the policy does not change past replay results).

**P1**


- **Two injection points.** collaborate participants do not go through `scheduler`; their prompts are built in `collaboration/turn.rs`, so injection was added there too. Both inject only at the start of a new session.
- **The list passed to the classifier is in file order.** Injection sorts by "hand-written → count → recency", but the numbers the classifier returns (such as `r1`) are resolved by `apply` in file order, so the list is passed in file order.
- **Capture caps are enforced in code.** `memory.rs` truncates to 2 new entries per response and 200 characters per entry. It does not rely only on the instructions to the classifier.
- **Fake metadata is neutralized.** `<!--` and `-->` are stripped from captured text, so even if a user's statement contains `<!-- auto seen:999 -->`, it cannot forge the count.
