# Adaptive Routing and the ACP Gateway

Implemented and verified: 2026-09-15–16.
For measurement details, see [Measurement and Session Collaboration Validation (2026-09-16)](real-validation-20260916.md) and [Live Validation of Remaining Tasks (2026-09-16, part 2)](real-validation-20260916-2.md); for communication between independent sessions, see [Session Collaboration](session-collaboration.md).

## Status

| Item | Implementation and verification status |
|---|---|
| Calibration of selection accuracy | Implemented `calibrate`, `benchmark` and `benchmark-tune`. Measured 8 pairs of identical tasks with Claude/Codex. On 2026-09-16, additionally measured 6 tasks differing in language, difficulty and size, and confirmed that per-context EWMA does not share learning across different tasks. Added `pooling`, which uses the same candidate's results from other contexts (disabled by default). Because the data set is small, a general improvement has not been established |
| Continual learning | Implemented per-context EWMA and an epsilon-greedy bandit that stays within constraints. EWMA is the default; exploration must be configured explicitly |
| Subscription quota | Codex's official app-server, Claude's official statusline and on-demand `/usage` verified against the real CLI. The Antigravity `/usage` probe is implemented and awaiting verification against the real CLI after authentication. Gemini is out of scope |
| Antigravity live E2E | CLI and official ACP Server installed. Confirmed up to the authentication request; verification after Google login is pending |
| Router / Frontier Judge | Advisers unified onto ACP agents from `[[agents]]` (2026-09-16). Confirmed the handover from a real Antigravity (unauthenticated) to a real Codex. Semantic pass/fail judgment of deliverables is not implemented |
| Council / session collaboration | Coordinator → parallel implementation → integration over independent ACP sessions, addressed multi-round communication, and application to the original working tree with conflict resolution. Implemented fallback agent/model selection for every role, saving of intermediate state, and `collaborate-resume`. Router / Judge / Council support fallback advisers, and a 2-round Council over ACP was confirmed against the real services |
| What a session costs | The heuristic size prior (`estimated_context + 1500 × scope`) models the work and not the seat, so it under-estimated a real trivial task by about 12× — nearly all of a small session is the system prompt and the cache it reads. `storage::session_floor` measures that floor per (agent, model) from recorded executions and `scorer` adds it to the cost, undiscounted. On 2026-09-20 the same trivial task cost 202,348 tokens on a frontier session and 21,882 on a small one — a 9.2× gap the policy tiers price at 2.46×. **Automated tests only**; the effect on real routing has not been measured, and a seat with fewer than four recorded runs is priced exactly as before |
| Orochi against each agent alone | `examples/compare_live.py` runs the same tasks three ways with real CLIs. A 3-task pilot on 2026-09-20 ([results](real-validation-20260920.md)) found equal accuracy, 5.6× fewer tokens than Claude Code alone and 2.5× more than Codex alone, the gap being the classifier's extra session. That pilot ran before `classifier::worth_asking` existed, which keeps the heuristic where asking would cost more than a quarter of what work like this has cost: on the same measured parts the gate leaves 1.59× over three tasks and 1.18× over fifty, against 1.15× with nothing asked at all. Re-running the suite is what would establish that, and the full 10-task suite is still to be run |
| Ordered parts (work graph) | Parts run in waves in `collaborate`, proposed by a plan file, the first coordinator turn or the console's design step; route claims and a choice order for the seats of one turn replaced the fixed 1.2 s stagger (2026-09-19). **Automated tests only**; behavior with real agents is unverified ([design record](parallel-execution-design.md)) |
| Exposing Orochi over ACP | `serve` supports ACP v1 stdio. Permission, cancel, etc. verified with fixtures. Execution and evaluation from an ACP client to a real Codex succeeded. Confirmed execution and evaluation from the agent panel of Zed 1.19.2 through to a real Codex (UI operation automated with System Events; no screen capture) |

## 1. Calibration and comparison

```sh
orochi calibrate --limit 10000
orochi benchmark --input paired-cases.json --seed 42 --failure-penalty 100000
orochi benchmark-tune --input paired-cases.json --training-cases 4

# For checking that the comparison commands work. Not performance data from a real provider.
python3 examples/benchmark_fixture.py > /tmp/orochi-synthetic.json
orochi benchmark --input /tmp/orochi-synthetic.json
```

`calibrate` compares the success probability and estimated tokens saved before each run with the measured results. It outputs the squared error of the success probability (Brier score), a comparison with the initial prior, predictions and success rates over 10 bins, and the total absolute token error / total measured tokens (WAPE). Lower error is better. No samples or no usage yields `null`. Runs in an existing DB that have no prediction are not included in calibration.

Observing only the chosen candidate cannot tell whether "a different candidate would have done better". `benchmark` is given measurement results from running each task on each candidate **independently from the same starting state**. Failures are saved too, and task IDs must be unique. The order of the tasks is used as the time series, and each policy learns only from the results of the candidates it chose itself. The results of all candidates are used to compute the best measured result that serves as the baseline for comparison.

The input is JSON with `schema_version: 1`, `provenance` (source), `synthetic` (whether the data is synthetic) and `cases`. A case has an `id`, a `descriptor` in `TaskDescriptor` form, and `arms`. An arm holds a `candidate` in `ExecutionCandidate` form carrying the initial prior, the evaluated `success`, the measured `tokens`, and `duration_ms`. See the [generation example](../examples/benchmark_fixture.py).

For Static / EWMA / Bandit it outputs the number of successes, the number of selections and abstentions, total tokens, tokens per success, and the cost including the failure penalty together with its gap from the best result. The failure penalty is a comparison parameter expressed in resource terms, not a price. Abstaining incurs the same penalty, and the baseline is the minimum cost over all candidates and abstention. A result that reduces only tokens without keeping the number of successes is not regarded as an improvement.

### Against each agent used alone

`benchmark` replays measurements; it cannot say whether Orochi beats the agents it routes to. That needs the same tasks run three ways, and [`examples/compare_live.py`](../examples/compare_live.py) does exactly that with real CLIs:

```sh
python3 examples/compare_live.py --suite examples/compare-suite.json \
  --output .orochi/live-e2e/compare-<date> --adapter-cache ~/.local/share/orochi/adapters \
  --agent-path-prepend ~/.local/bin --agent-path-prepend ~/.bun/bin --execute   # consumes quota

python3 examples/compare_live.py --suite examples/compare-suite.json \
  --output /tmp/validation --validate                                          # contacts nothing
```

- **Arms.** Each arm is a configuration of the same binary: an *alone* arm pins one agent and the model and reasoning its owner set as that CLI's default, with one attempt and no classifier; the *orochi* arm gets both agents, its classifier and up to three attempts. Rotating the order per case keeps one arm from always running first.
- **Scoring.** Each case ships visible tests in the repository — what the agents and Orochi's evaluator can see — and a **hidden check** that is run afterwards and is the only thing that decides pass or fail. A trial whose agent could not be discovered, or that recorded no execution, is `blocked` and never a pass.
- **Tokens.** An arm is charged every token its runs recorded: execution, retries, the classifier and any routing advice, exactly as each agent reported it (`orochi runs`). Providers count cache and system prompts differently, so tokens compare arms on the same provider more meaningfully than across providers.
- `--validate` checks that every case fails as it is handed to the agents and passes with the reference solution kept beside it, which is what makes a pass or a failure mean anything. `tests/cli_e2e.rs` runs it, and drives the whole harness against the fixture.

Measurements live in dated documents; the suite itself says which CLI defaults its alone arms reproduce, and when.

Normal routing and replay share the EWMA, exploration and resource scoring functions. Input that has `candidate.prediction.cost_features` reuses its cache/context/quota terms; input without it recovers the proportionality coefficient from the given initial cost. Candidate eligibility by quota/capability/Policy is checked on the input side. Replay does not connect to real agents and does not write learning results to the operational DB.

**A successful comparison on synthetic data does not prove improved selection accuracy or optimality on real providers.** Representative tasks must first be prepared by difficulty, language and size, and measured from independent starting states with a fixed adapter/model/configuration and the same evaluation command.

## 2. EWMA and Bandit

```toml
[learning]
strategy = "ewma"       # static / ewma / bandit
alpha = 0.1             # larger values follow recent results more closely
prior_weight = 16.0
exploration = 0.1       # bandit only
max_cost_ratio = 1.25
pooling = 0.0           # 0–1. How much the same candidate's results from other contexts are reflected. Disabled by default
```

The most recent 500 runs whose agent, model, reasoning, mode, task type, language, framework and complexity all match are processed oldest first. The count is limited after matching. The weight of the initial prior and of older observations decays by a factor of `1-alpha` at each step. For tokens, it learns the ratio of the measured value to that task's initial token estimate and applies it to the size of the current task. To limit the influence of a single run, this ratio is clamped to 0.05–20. Runs without usage are not used for token learning.

When `pooling` is greater than 0, the ratio of tokens to the initial estimate and the success difference (measured − initial prior) are computed from the most recent 500 runs of the same agent, model, reasoning and mode in **other contexts**. These are shrunk by `prior_weight`, multiplied by `pooling`, and applied to the current context's initial value. That context's own results then update further from the corrected initial value. This is Orochi's own correction, based on the measurement that token overrun depends more on the candidate than on the task (system prompt, tool loop, cache, etc.); it is not a value published by any provider. Static does not use it.

In the 2026-09-16 measurement of 6 tasks, per-context EWMA (`pooling = 0`) shared no learning between tasks and made the same selections as Static. These 6 tasks were replayed in all 720 orders, averaging over both orderings of equal-cost candidates. With `pooling = 0.5`, the mean number of successes improved from 5.5 to 5.8 and mean tokens from about 640,000 to about 410,000. On the other hand, in the actual chronological order the default ordering picked the best candidate. As a result, choosing the coefficient on the first half and validating on the second half produced no improvement. Because the data is small, the default is not changed. For details, see [Live Validation of Remaining Tasks (2026-09-16, part 2)](real-validation-20260916-2.md).

Unverified completions, cancellations, authentication / rate-limit / unreachable / configuration errors, and runs in which model/reasoning/mode switched during execution are excluded from quality learning. Router/Judge/Council usage is also recorded under a separate purpose and is not mixed into learning of implementation quality. Old runs whose `complexity` is unknown are excluded from per-context learning.

Among the candidates estimated for each context, the bandit explores those that pass the success-probability threshold and are within `max_cost_ratio` times the minimum expected cost. The default exploration rate is 10%. Because exploration can also land on the best candidate, the best candidate's probability is `1-epsilon+epsilon/N` and each other candidate's is `epsilon/N`. The selection probability is saved together with the pre-execution prediction. When an LLM recommendation is adopted, the selection probability cannot be computed, so it is `null`.

This is epsilon-greedy partitioned by context, not LinUCB or Thompson Sampling with shared features. The success-probability lower bound is a constraint on the estimate, not a guarantee of actual success or a confidence interval. Candidates that fall below the lower bound are not explored either. The remaining coefficients, such as confidence and the cache discount, are also to be calibrated against future measurements.

### Weak labels

Implemented: 2026-09-18 (P3). **The weights are Orochi's own uncalibrated heuristic. Their effect in real use is unverified.**

Learning labels arise only from automatic checks passing or failing, so for design, discussion and research, and in repositories without tests, learning made no progress however often Orochi was used. In the console, what the user does right after an answer is recorded as a weak label. No new action is asked of the user.

| Action right after | Label | Weight |
|---|---|---|
| Sent the next message after seeing the answer | Success | 0.2 |
| `/reroute` (whether after the answer or after stopping with Esc) | Failure | 0.3 |
| Stopped with Esc, then sent the next message with the same agent | None | — |
| `/new`, exiting, a message queued before seeing the answer | None | — |

"Failure" does not mean that the task failed; it is weak evidence that "this agent × model may not have suited this kind of work". Working it through from Sonnet's initial value of 0.88, it drops to 0.863 after one and to 0.798 after five (0.614 after five verified failures).

**Esc alone is not turned into a label** (changed 2026-09-19). The reason for stopping may be that the agent was off target, or merely that the user noticed they had forgotten to say something, and the two cannot be told apart at the moment of stopping. A subsequent `/reroute` is taken as the former; a follow-up sent with the same agent is taken as the latter.

- **Verified results are never overwritten.** Weak labels apply only to unverified completions and interruptions
- The label is appended to the run record, and only the first one counts. The prediction frozen before execution is not touched
- The weight takes effect through the decay as `decay^w`. A weight of 1 gives the same calculation as before. Confidence counts only verified results
- `orochi calibrate` reports, separately from the existing figures (verified only), the count and Brier for each signal under `weak`. Whether learning from weak labels is working is judged by whether the **verified** Brier falls over time

### Tier pooling (off by default)

Implemented: 2026-09-18. **Confirmed only by replay on synthetic data. Disabled by default.**

Learning looks results up by exact model ID, so when a model is updated all of its history is lost and the new ID starts over from the static prior. When `learning.tier_pooling` is greater than 0, the starting point of a model with no history is shifted by **the results of other model IDs with the same provider, same tier, same task type, same complexity and same reasoning**.

- The tier is derived at read time from the policy's substring patterns (`opus` → frontier), so existing records can be used as they are
- The `unknown` tier is not pooled (it is a grab bag of IDs that matched no pattern)
- Only the starting point moves, and its contribution shrinks as the model's own results accumulate. Its evidence does not overlap with the existing `pooling` (same candidate, other contexts), so the two can be used together

Check on synthetic data (`tests/adaptive.rs`, `tier_pooling_carries_evidence_across_a_model_replacement`): when the frontier model is replaced with a new ID partway through and the new ID's prior is placed below the success floor, with pooling disabled all 30 cases after the replacement abstain, and with it enabled there are 0 abstentions and 58 of 60 cases succeed. **The effect on real data has not been confirmed**, so it stays disabled by default. `tier_pooling` (0 / 0.5 / 1) has been added to what `benchmark-tune` searches, so confirm that it helps on your own measurement data before enabling it.

## 3. Quota

```sh
orochi quota --refresh
orochi quota                  # saved snapshots and stale flags
orochi quota-ingest --agent claude < claude-statusline.json
```

```toml
[quota]
refresh_before_run = true     # default false. Also fetches on dry-run
max_age_secs = 300
timeout_secs = 30
```

### Codex

When the native CLI of the known `codex` agent is detected, `quota --refresh` calls `account/rateLimits/read` on the official `codex app-server`. No prompt is sent. The CLI's own authentication is used.

Multiple windows are saved, and only those still within their validity period are applied. `rateLimitsByLimitId` takes precedence, and buckets other than `codex` are only displayed, because their mapping to models is unknown. A window missing `usedPercent` is not interpreted as 100% remaining. Once quota information expires, it stops being applied to routing. Separate cooldowns observed during execution are kept.

On 2026-09-15, retrieval of the primary / secondary quota and reset times from a logged-in real Codex CLI was confirmed. The evidence is `.orochi/live-e2e/adaptive/quota-live.json`. Quota changes after retrieval, so this file does not hold current values.

### Claude

`quota-ingest` ingests the `rate_limits.five_hour` / `seven_day` / `spend_limit` that Claude Code passes to the statusline. This is an observation of CLI events, not an API for re-fetching quota at an arbitrary time. The input is passed from a statusline script that the user configures. Existing Claude configuration files are not modified. The conversation, paths and other fields in the input are not saved. The remaining context window is not mistaken for remaining subscription quota.

This format is provided only by supporting Claude Code versions and accounts. When it is missing, quota is treated as unknown. Ingesting a real statusline from Claude Code 2.1.272 and saving it to a separate verification DB have been confirmed.

On-demand retrieval uses the `claude_usage` probe, which reads the official CLI's `/usage` over a POSIX PTY. Python 3 is required. With the standard Claude preset it is auto-detected from the native CLI, and real retrieval and saving via `quota --refresh` was confirmed. It starts in a dedicated empty temporary directory and does not save the raw screen or the account name.

A similar `antigravity_usage` probe is implemented for Antigravity. When unauthenticated, it does not start OAuth and reports unknown. Checking it against the real account's quota display is still needed after login.

### Other sources and custom probe commands

```toml
[[quota.probes]]
agent = "gemini"
kind = "command"
command = "/absolute/path/to/your-quota-reader"
args = []
```

The command returns the following JSON on stdout. No shell expansion is performed, and it runs in a temporary directory. A 1 MiB output limit and a timeout are applied, and neither stderr nor the full returned JSON is saved. On failure, the previous snapshot is kept while it is still valid.

```json
{"schema_version":1,"windows":[{"bucket":"daily","remaining":0.35,"reset_at":2000000000,"model":null,"affects_routing":true}]}
```

`model:null` means the whole agent. When specifying a model, use the actual ID obtained over ACP. A Gemini-specific probe is not implemented and is out of scope for this round of additions. No private APIs are called and no authentication tokens are extracted.

Sources: [Codex app-server](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md), [Claude official statusline](https://code.claude.com/docs/en/statusline), [Antigravity interactive usage display](https://www.antigravity.google/docs/cli/commands/usage).

## 4. Router, Frontier Judge and Council

An adviser helps choose, before execution, "which candidate (agent × model × reasoning) to run on". It does not execute the task itself. All advisers use agents from `[[agents]]` over ACP. The OpenAI-compatible HTTP endpoint version that existed until 2026-09-16 was removed when everything was unified onto ACP (the old `endpoint` and `api_key_env` settings produce an error with migration guidance).

```toml
# An agent used only as an adviser. It never becomes an execution candidate
[[agents]]
id = "judge-claude"
provider = "anthropic"
command = "claude"
routing_only = true

# Consulted on candidate order for low-confidence tasks
[router]
agent = "codex"                   # ID from [[agents]]
model = "gpt-5.6-luna"            # optional. A model ID obtainable over ACP
reasoning = "low"                 # optional
session_overhead_tokens = 24000   # match to measured values. 20000 if omitted

# Extreme tasks, or tasks with ambiguity 0.7 or higher
[frontier]
agent = "codex"
model = "gpt-5.6-sol"

# Fallback when the adviser above cannot be used (up to 3, no nesting)
[[frontier.fallbacks]]
agent = "judge-claude"
model = "haiku"
session_overhead_tokens = 29000
```

| Field | Default | Description |
|---|---|---|
| `model` | automatic | If omitted, Orochi chooses from the models that agent offers (see below) |
| `timeout_secs` | 120 | Limit for one call, including CLI startup |
| `max_resource_fraction` | 1.0 | Upper limit on the tokens used for advice, as a fraction of the estimated tokens of the minimum-expected-cost candidate |
| `max_output_tokens` | 512 | Assumed size of the answer, used in the budget calculation. It cannot be cut off midway over ACP |
| `session_overhead_tokens` | 20000 | Additional tokens per turn from the system prompt and the like |

- `agent` must be the ID of an enabled `[[agents]]` entry. An agent with `routing_only = true` is excluded from execution discovery, candidates, `--agent` and `collaborate`, and is used only as an adviser.
- Authentication follows each CLI (a subscription login, or an API key the CLI supports). Orochi itself does not read credentials. Orochi's environment variables and `[agents.env]` are passed on to the CLI.
- Every call starts a new session in an empty temporary directory, and permission requests are always denied. Only task attributes, at most 12 candidate summaries and peer votes are sent. Task text, filenames and repository paths are not sent. However, some CLIs can read outside the working directory without asking for permission, so not using tools is an instruction, not an OS-level guarantee.
- The only exception is `[classifier]` (next section). Classification alone cannot work without the task text, so the task text is sent only when explicitly opted in.

## 4.1. Classifier (task classification)

`router::profiler` decides task type and complexity by keyword matching, so phrasings outside its vocabulary fall through to `implementation` / `Normal`. 「CIが赤いので直して」 ("CI is red, so fix it") does not become `bug_fix`, and 「全部TypeScriptに書き換えて」 ("rewrite it all in TypeScript") does not become `Extreme`. This is overridden by an ACP agent's classification.

**Enabled by default. No configuration needed.** Without `agent`, it tries the enabled (non-`routing_only`) agents in configured order, within one agent's worth of time in total. Sending to an agent used for execution means sending the task text where it was going anyway, which is why the default recipients are limited to those.

```toml
[classifier]
enabled = false        # turn it off
agent = "claude"       # pin the recipient; omitted, the agent that has answered most cheaply
model = "haiku"        # if omitted, chosen automatically from the policy at Simple difficulty (= the cheapest candidate)
timeout_secs = 60
max_cost_share = 0.25  # the largest share of what work like this costs that asking may take
```

**Asking costs a session of its own, so it has to be worth it.** Measured on 2026-09-20 with real CLIs: the same question cost 37,630 tokens on Claude and 23,800 on Codex, almost all of it the session's fixed cost rather than the question ([pilot](real-validation-20260920.md)). So what each answer costs is recorded as a `classification` run — never an execution, so it never teaches the router — and two things follow from it. The agent asked is whichever has answered most cheaply here, an agent nobody has priced going first so its own price becomes known. And once both sides are measured, a question that would take more than `max_cost_share` of what work like this has cost is not asked at all: the local profile stands, the evaluator still checks the result, and a failure still moves to another candidate. Until there is something to weigh, Orochi asks.

**Nothing with no choice left to make starts an agent.** A `--dry-run`, a read-only seat, and a route with both `--agent` and `--model` specified proceed with the heuristic's label.

**The only path that sends the task text.** Like any other agent, the recipient is a locally started CLI, authenticated as that CLI itself. Orochi does not save the text anywhere. The cache key is a salted hash of the task text, and only the classification label remains in the DB (`classifier_sees_the_task_but_caches_only_a_hash_of_it` in `tests/core.rs` checks that the text does not appear in the raw bytes of the DB file).

The merge goes **one way only**.

| Field | Behavior |
|---|---|
| `task_type` | Replaced by the classification. Anything other than the 11 types in `classifier::TASK_TYPES` is discarded (an unknown label would split the learning strata) |
| `complexity` | `max(heuristic, classifier)`. **Never lowered** |
| `requires_*` / `long_horizon` / `requires_architecture_change` | Logical OR. A flag the heuristic set is **never cleared** |
| `ambiguity` | The larger of the two |
| `collaborative` / `seats` | Not classified. How many agents to use is set only by the user |

This follows the asymmetry that underestimating does real harm ("routed to a model that cannot do it, and it fails"), while overestimating only costs more.

**Effect on cost.** A failure (cooldown, rate limit, timeout, invalid reply) always falls back to the heuristic result, so classification never stops a run. However, the latency of one CLI startup is added to every run. From the second time onward, the same text is served from the cache (30 days). Also, when `complexity` is raised to `Complex`/`Extreme`, `roles::seats` seats a second, read-only agent, so in chat the cost of a turn can double.
- Only the JSON `{"candidate_id": "..."}` at the end of the answer is accepted (notices the CLI prints first are ignored). It is not adopted unless it is one of the presented candidate IDs and within 1.25× the minimum expected cost.
- When an adviser hits an account-wide limit or an authentication error, or is unreachable, this is recorded in that agent's cooldown. Because the execution side uses the same account, the scheduler rechecks quota before execution. An adviser in cooldown is not started.
- When `model` is omitted, the difficulty of advising is fixed per role, and the cheapest eligible model and reasoning are chosen under the same Policy (success-probability lower bound, constraints, cost) and quota as execution candidates. The Router is treated as simple, each Council seat as normal, and the Frontier Judge as complex. Under the current Policy this gives the Router a small model with reasoning low, the Council a small model with reasoning medium, and the Frontier Judge a mid-tier model with reasoning high. The mapping from role to difficulty is Orochi's own rule of thumb, and coding track records are not used for the choice. Models in cooldown are not chosen. If only `reasoning` is specified, the choice is made among the models that support that reasoning.
- Advice is recorded in `runs` with the actual adviser's agent, the model (actually used), usage, and the failure reason (`rate_limit`, `authentication`, `timeout`, `cooldown`, `invalid_advice`, etc.). Because its purpose is not `execution`, it is not used for learning implementation quality.

The Council registers 2–4 seats in `[[council.members]]` in the same format and is run explicitly with `orochi --council "task"`. To use it all the time, set `[council] enabled = true`.

1. Each seat chooses a candidate independently.
2. The list of valid candidate IDs from round 1 is passed to each seat for reconsideration. The session that answered round 1 is reused as is, and only the votes are sent.
3. The candidate agreed on by a majority of the configured number of seats is adopted. Failed seats do not reduce the number of votes required.

Each seat's fallback adviser is given the same task attributes, candidates and peer votes. A seat that switched to its fallback in round 1 continues with that fallback in round 2. A fallback first used in round 2 is given the candidate list and the peer votes together.

Before starting, the maximum estimated tokens for all seats, all fallbacks and both rounds are summed, and the budget is checked against the smallest `max_resource_fraction` among all advisers. Because round 2 reuses the session, it is estimated at a size that includes the first request and answer. When over budget, without agreement, or when every seat fails, it falls back to the local choice. The Router, Judge and Council are never called at the same time.

In the 2026-09-16 measurement (ACP advisers, values reported by each adapter), one call took about 23,000 tokens and about 5–7 seconds on Codex gpt-5.6-luna, and about 28,000 tokens and about 7–8 seconds on Claude Haiku. For details, see [Live Validation of Remaining Tasks (2026-09-16, part 2)](real-validation-20260916-2.md).

Advisers decide only the routing. The final edits are made by a single ACP agent. To implement, review and integrate in independent sessions, use the separate [`collaborate`](session-collaboration.md) command.

### Local models

For both advisers and execution, local models can be used if a CLI reachable over ACP supports them. Orochi does not distinguish whether the model it connects to is in the cloud or local. Configure the local model on the CLI side and set `model` to the model ID reported over ACP. `provider` is used to select the Policy (priors and constraints). An unknown model ID is evaluated with that Provider Policy's default rule (`*`, tier unknown). Nothing has yet been verified against a real CLI with a local model.

## 4.2. Deriving the team automatically

`collaborate` used to require the team (Coordinator / Implementer / Reviewer / Integrator) to be declared as JSON with `--plan`; when that is omitted, the team is now derived from the task.

```sh
orochi collaborate "I want to overhaul the entire architecture" --dry-run   # only show the lineup
orochi collaborate "..." --output report/                                   # run it as is
orochi collaborate "..." --dry-run > team.json                              # save it and edit by hand
orochi collaborate "..." --plan team.json --output report/
```

`--dry-run` prints in the same format as a plan file, so its output can be passed straight to `--plan`. If `[classifier]` is enabled, the `task_type` / `complexity` used for the derivation are the ones that went through it.

| Role | How the count is decided |
|---|---|
| Coordinator | 1 when there are 2 or more Implementers, or `long_horizon`, or `Extreme` |
| Implementer | `Extreme` 3 / `Complex` 2 / otherwise 1, but **capped by the number of areas the work can be divided into** |
| Reviewer | 1, +1 for a structural change, +1 for `Extreme` (at most 4) |
| Integrator | Always 1 |
| Discussion | When `collaborative` or `Complex` and above. 3 rounds for `Extreme`, otherwise 2 |

If `seats` is given (「5人のエージェントで」, "with five agents"), the team grows or shrinks to that number. Roles are added in the order Coordinator → Implementer → Reviewer and removed in the reverse order.

**The key point is that Implementers are capped by the number of areas.** Putting two Implementers in the same directory does not halve the work; they only collide. So the count is never raised beyond the number of top-level directories of `candidate_files` (the directories at the repository root if the task mentions no files). When there are 2 or more, those areas are handed out as `paths` without overlap.

Each participant's `agent` is empty (automatic selection). Which agent × model to assign is the scheduler's job, and because `scorer` prices an agent that another seat is using at 1.4×, concurrent seats naturally spread across different accounts. A seat claims its route at the moment it chooses (`mailbox::claim`), before anything is awaited, so the next seat of the same process sees it at once rather than after the first has registered with the mailbox. The seats of one console turn also take their places in order (`mailbox::Order`): the one doing the work first, then each seat after the one before it, while their discoveries still run side by side. A place is taken by choosing a route **and joining the mailbox** under it, so no seat starts talking before the one it answers to is there to hear it. This replaced a fixed 1.2-second head start per seat.

## 4.3. Team escalation from the console

Just typing a prompt into `orochi` (or `orochi chat`) goes as far as collaboration when needed. The user does not have to choose between `chat` and `collaborate`.

Classification happens once per turn. That one result is used for the escalation decision, the phase split and routing alike (it is passed as `RunOptions::descriptor`, so the scheduler never reclassifies the decorated text).

Escalation is decided **after the design step**, by something that has read the code, not by counting directories. Work split into phases starts with a design step; its instruction asks it to end with one `{"parts": [...]}` line when the work divides into parts that can be built without each other's code. Orochi orders those parts as `collaborate` does ([The order of the work](session-collaboration.md#the-order-of-the-work-parts-and-waves)), and escalates only when something can run side by side. The team is `collaboration::plan::with_parts`: one implementer seat per part that runs at once (at most 4), reviewers as for a derived team, and no coordinator, since the design step was that turn. The design's reply (without the parts) is added to the collaboration's task, so every part starts from it.

The `{"parts": ...}` object is for Orochi: it is held back from the transcript while it streams and removed from what is handed to the next step. Text that only looked like its start is shown as soon as that is clear, and an object that turns out not to end the reply is shown at the end.

When escalating, it **asks once**, showing the order it would run in, as Claude Code asks to approve a plan.

```
 ◆ This divides into parts that can run side by side
   alpha ‖ beta → gamma
   2 implementer(s), 3 reviewer(s), integrator
   each part works in its own copy; a verified result is merged back

 ❯ 1. Yes, run the parts side by side
   2. Yes, one agent carries it out here
   3. No, keep the design (esc)
```

`2` carries on with the implementation step in your own working tree, as if no parts had been proposed. `3` or Esc builds nothing, as declining a plan in Claude Code does: the design is kept in the conversation and the next message carries on from it. While the team runs, its progress appears as notes in the transcript, and Esc or Ctrl-C stops it; the report can be resumed.

This is the only path on which several agents run at once and, once verification passes, write back to the working tree, so this alone is never run silently. Without a terminal (stdin is a pipe) there is no escalation, because in an environment where each line is read as one message, the confirmation prompt would swallow the next message.

Turns that explicitly use `/solo` or `/team` are not subject to escalation. Whether real design steps propose useful divisions is **unverified**; the flow is covered by automated tests with mock agents, including a terminal test that answers the question. The report remains in `<data>/collaborations/<id>/`, so an interrupted run can be continued with `orochi collaborate-resume --output <path>`.

## 4.4. Memory

Implemented: 2026-09-18 (P1). For the design history and what is not implemented, see [Personalization Design](personalization-design.md). **Automated tests only; capture accuracy with real agents is unverified.**

The agent a task is routed to changes every time, and Claude's `CLAUDE.md` is invisible to Codex. What should hold across agents is remembered on Orochi's side.

```
<data>/memory/USER.md                          whole user
<data>/memory/repos/<repository_id>/MEMORY.md  per repository (the directory name is a salted hash)
```

**Capture needs no extra LLM call.** The classifier reads the request every turn anyway, so its reply carries up to 2 items of "the user's own instructions or facts that stay valid after this task is done". When one is saved, a line is printed.

```
⎿ classified as bug_fix / normal
⎿ remembered: the package manager is pnpm
```

**Injected only when a new session starts.** It goes into chat, one-shot runs, seats and every `collaborate` participant, and not into a turn that uses `resume` (that session heard it when it started). A preamble positions it as "not taking precedence over the task's instructions or the repository's instructions".

**How it forgets.** Lines Orochi wrote carry `<!-- auto seen:N last:T -->`; something heard only once disappears after 90 days, twice or more after 180 days. Saying it again increases the count, and saying the opposite replaces it. **Lines written by hand (unmarked) never disappear, are never replaced, and are injected first.**

```sh
orochi memory                 # list (with IDs u1, r2 ...)
orochi memory --forget r2     # forget one item
```

In the console, `/memory` and `/memory forget r2`. You may also edit the files directly. `memory.enabled = false` stops both capture and injection.

**What is guaranteed** (`mod memory` in `tests/core.rs` and `a_remembered_note_opens_each_fresh_session_once`)

- Memory text does not go into `telemetry.sqlite3`. Nor does it remain in the classification cache
- Notes from repository A do not appear in repository B's prompts
- It is not passed to the router / judge / council. Only the classifier sees it, and only the current repository's listing
- **`MEMORY.md` and `USER.md` inside the target repository are never read.** This keeps a repository from planting text in every prompt
- **No tool lets an agent write memory.** Agent output is influenced by the repository's content

**Session look-back** (P2). For things a single message cannot reveal to be a lasting preference (such as the same correction repeated), at the end of a console session that had 2 or more messages, what the user said in that session (across `/new`; agents' replies not included) is reviewed once, through the same path as the classifier.

```
⎿ looking back over this session · Esc skips
⎿ remembered: don't classify by keyword matching
```

One LLM call per session. It is not made for a session with only one message (the classifier read it when it was sent). What was said is kept only in process memory, and it goes to the same recipient as the classifier.

**Route preferences** (P3). If you say which agent or model you want for which kind of work, like "use Fable for design" (or write it into memory by hand), the classifier applies it to the current task and returns a name fragment, and the expected cost of matching candidates is multiplied by 0.8.

```
⎿ classified as architecture / complex · you prefer fable
```

- **It does not open gates.** A candidate dropped by capability, quota or the success floor does not come back because of a preference. If a candidate is much cheaper or much more likely to succeed, that one wins
- Matching is by name fragment (`fable`), so it keeps working when the model is updated to `fable-6`. It is never used to create an ID that was not discovered
- 0.8 is Orochi's own heuristic, not a measured value
- A one-off choice (`--model`) is a hard constraint as before, and does not go into memory

**Limits.** What gets remembered depends on the classifier's judgment and is not guaranteed. Not picking up text pasted in from outside that says "remember this" relies on an instruction; the rest of the defense is the caps on item count and length and the display at capture time. Nothing is captured on turns where the classifier does not run (`--dry-run`, a one-shot with both agent and model pinned).

## 5. Exposing Orochi over ACP

```sh
orochi --config /absolute/path/config.toml --data-dir /absolute/path/data serve
```

Register the above as a stdio agent on the ACP client side. The SDK is the official Rust ACP 2.0.0, and the exposed protocol is v1. It supports `initialize`, `session/new`, `session/prompt`, `session/cancel` and request cancellation. stdout carries JSON-RPC; progress goes to stderr. The backend's answer is streamed, and under the default ask, permission requests are relayed to the upstream client. Explicit deny/allow settings take precedence, and permission is granted only as allow_once.

The previous user input and answer in the session are kept in memory, up to about 24 KiB, and used as context for the next prompt. Each prompt runs the normal scheduler. Keeping the full conversation, persistent session/load, model/mode switching, forwarding MCP configuration, additional workspaces, and image/audio input are not supported. Unsupported capabilities are not advertised, and unsupported input is an error. Up to 128 sessions.

Concurrent prompts on the same session are rejected, and runs against the same repository are also made mutually exclusive by the normal workspace lock. On cancellation or disconnection, the worker and the backend's child processes are terminated. A gateway run interrupted midway may not have its final telemetry saved. Completed runs are recorded to the normal evaluation and telemetry.

## 6. Live E2E

```sh
# CLI connection and model discovery only
python3 examples/live_e2e.py --agent gemini --output .orochi/live-e2e/gemini

# With a logged-in CLI, create one file in an isolated temporary repo and verify its content
python3 examples/live_e2e.py --agent gemini --execute --output .orochi/live-e2e/gemini
python3 examples/live_e2e.py --agent antigravity --command /absolute/path/agy_acp_server.par --execute --output .orochi/live-e2e/antigravity
```

`--command` and the repeatable `--arg` specify the ACP command of the real environment. Example: `--arg=--acp`. Each CLI must be installed and logged in following its official instructions. The script does not log in automatically and does not modify existing projects. `--execute` consumes the real account's quota.

CLI not installed, cannot connect, not executed, execution failed, and verification succeeded are distinguished and recorded in `result.json`. Discovery success alone is not reported as E2E completion. For Antigravity, after installing CLI 1.2.3 and ACP Server 1.1.1, confirmed up to `Authentication required`. The latest evidence is `.orochi/live-e2e/session-work/antigravity/result.json`. Further verification of Gemini is out of scope at the user's instruction.

For how Gemini is launched, see the [official ACP mode](https://geminicli.com/docs/cli/acp-mode/).

## Verification results

- Verification of handover, resumption, limit scope and preservation of intermediate state is recorded in [Measurement and Session Collaboration Validation (2026-09-16)](real-validation-20260916.md), and verification including the period after advisers were unified onto ACP in [Live Validation of Remaining Tasks (2026-09-16, part 2)](real-validation-20260916-2.md). Adviser tests use ACP fixtures.
- For a list of what was measured, what was verified against the real CLI, and what is unverified, see [Measurement and Session Collaboration Validation (2026-09-16)](real-validation-20260916.md).
