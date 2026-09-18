# Live Validation of Remaining Tasks (2026-09-16, part 2)

Results for the seven items left over from part 1, [Measurement and Session Collaboration Validation (2026-09-16)](real-validation-20260916.md). Evidence is stored in `.orochi/live-e2e/remaining-20260916/` (local only, untracked).

| # | Item | Result |
|---|---|---|
| 1 | Further calibration of selection accuracy | Measured 6 tasks. Confirmed the limits of per-context EWMA and added `pooling` (disabled by default) |
| 2 | Live validation of Antigravity | **Not completed**. Even at the end of validation, Google sign-in had not been done (`agy models` asks to sign in) |
| 3 | Failover caused by real-service limits | Confirmed failover from a real Codex usage limit to real Claude. Fixed a classification bug |
| 4 | Router / Judge / Council on real services | Added advisers that use CLIs over ACP, and confirmed failover on real services for all three modes. Later, advisers were unified on ACP and the HTTP endpoint version was removed (section 8) |
| 5 | ACP E2E from Zed | Succeeded from the Zed 1.19.2 agent panel → Orochi → real Codex → evaluation. Settings were restored afterward |
| 6 | Extending collaboration | Implemented parallel implementation, addressed multi-round messaging, applying to the working tree, and conflict resolution. Confirmed with real Claude / Codex |
| 7 | Recovery from forced termination | Implemented cleanup via a supervisor process and records. Confirmed SIGKILL → cleanup → resume with real Claude |

## 1. Further calibration of selection accuracy

Added `examples/diverse-suite.json`. It provides six tasks in Python, JavaScript, Rust and documentation, differing in difficulty and scale. For each task, we confirmed in advance that its check script fails in the starting state and succeeds with the reference solution.

Because only about 6% of Claude's weekly allowance remained, the candidates were, at the user's decision, **Claude Haiku** and **Codex gpt-5.6-luna / medium**. For the Complex refactoring task, both models' success-probability prior was below `required_success` (0.7), so Orochi itself excluded them from the candidates (expected behavior). For this task only, we measured with **Claude Sonnet / high** and **Codex gpt-5.6-sol / high**, which are candidates under the Policy.

| Task | Profiler | Haiku | luna / medium |
|---|---|---|---|
| Arithmetic expression evaluator (Python) | implementation / normal | **Failed** (accepted `"1 2"` as 12), 189,819 tokens | Succeeded, 28,152 |
| Parsing time strings (JavaScript) | implementation / normal | Succeeded, 86,527 | Succeeded, 25,565 |
| Bracket matching (Rust, cargo test) | test / normal * | Succeeded, 211,159 | Succeeded, 28,348 |
| Fixing monetary calculation with Decimal (Python, 2 files) | bug_fix / normal | Succeeded, 323,755 | Succeeded, 29,466 |
| Adding to a README | documentation / simple | Succeeded, 80,429 | Succeeded, 24,991 |

| Task | Profiler | Sonnet / high | sol / high |
|---|---|---|---|
| Refactoring that preserves output (JavaScript) | refactor / complex | Succeeded, 219,615 tokens, 16 s | Succeeded, 27,907 tokens, 69 s |

\* Because the first line asked to "also add unit tests", the implementation task was classified as test. We fixed this so that English requests starting with "Implement …" are treated as implementation, and added a test.

The tokens are the values reported by each ACP adapter, not charges. Providers differ in how their Usage reports count tokens for cache and the system prompt. Relative to the initial estimate (about 9,500 tokens), the ratio was about 2.7–3× for Codex and about 8.5–34× for Claude.

### Findings

- All six tasks differ in context (task type, language, framework, difficulty). With per-context EWMA, learning is not shared across tasks, and Static, EWMA and Bandit made the same choices.
- The order of candidates with the same initial prior is determined by candidate ID. This time Codex happened to come first, so the best result was obtained even without learning.
- The token overrun ratio tends to depend more on the agent / model than on the task.

### Improvement added and its verification

Added `[learning] pooling` (0–1, default 0). From the same candidate's results in other contexts, it computes the ratio of tokens to the initial estimate and the difference in success, and reflects them in the initial values for the current context. The same processing was added to `benchmark` replay and to the `benchmark-tune` search range (0 / 0.5 / 1.0).

We replayed the six tasks in all 720 orders, averaging over both directions of candidate ID ordering (`analysis/order-robust-6.json`, `analysis/order_robust_replay.py`).

| Strategy | Ordering: original | Ordering: reversed | Average |
|---|---|---|---|
| Static / EWMA / Bandit (pooling 0) | 6.0 successes, 164,429 tokens | 5.0 successes, 1,111,304 | 5.5 successes, about 640,000 |
| EWMA / Bandit (pooling 0.5 or 1.0) | 5.8 successes, 315,462 | 5.8 successes, 507,170 | **5.8 successes, about 410,000** |

- pooling made the result less dependent on the chance of the ordering, and on average improved both the number of successes and tokens. On the other hand, when the default ordering was already the best, it got worse by the cost of trying Claude once.
- When coefficients were chosen from the first 2–4 tasks in actual chronological order, every choice stayed at the defaults (alpha 0.1, prior_weight 16, exploration 0.1, pooling 0), and there was no improvement on the later tasks (`analysis/tune-6-train{2,3,4}.json`).
- Bandit gave the same result as EWMA. After learning, no other candidate came within 1.25× of the minimum cost, so no exploration occurred.

**This is small-scale data (6 tasks, 2 candidates), and no general improvement has been established. The defaults are not changed.** In environments with many kinds of tasks, where token tendencies differ greatly between agents, it is worth trying `pooling = 0.5` explicitly.

Evidence: `calibration/{dataset,evidence}.json`, `calibration-complex/`, `analysis/`.

## 2. Antigravity

`agy models` returned "Please sign in" both at the start and at the end of validation. After signing in, we plan to check the following.

```sh
.orochi/tools/antigravity/agy          # interactive Google sign-in
orochi quota --refresh                  # antigravity_usage probe
orochi status --discover                # model list
orochi --agent antigravity "..."        # run and evaluate
```

The official ACP Server (1.1.1), not signed in, returned `Authentication required` during adviser validation. This was recorded as `authentication`, and the run failed over to the next candidate (section 4).

## 3. Failover caused by real-service limits

Because Codex's credits had run out, we confirmed the **real Codex usage limit → real Claude** direction. Real Claude's weekly allowance had about 5% remaining and had not reached its limit. Failover in the Claude → Codex direction on real services remains unconfirmed.

Actual response from codex-acp 1.10.0:

```json
{"code":-32603,"message":"Internal error","data":{"message":"You've hit your usage limit. ... try again at 9:11 PM.","codexErrorInfo":"usageLimitExceeded"}}
```

On the first run this was classified as `Other: Internal error`; the run failed over to Claude, but no account-wide cooldown was recorded. We added `data.message` and `data.codexErrorInfo` to what gets classified. "try again at 9:11 PM" is now interpreted as the next 9:11 PM in local time, and a regression test using the actual payload was added.

After the fix (`failover-run2/`):

1. `codex / gpt-5.6-luna`: `RateLimit` (1.6 s). Saved a cooldown on `codex/*` until 21:11.
2. Within the same run, failed over to `claude / haiku`, which passed the checks (145,906 tokens).
3. On a second run with the same data, Codex was excluded before connecting with `agent/account is in cooldown`, and the run completed on Claude. Haiku's token results for the same context had been learned, and this time Sonnet / medium was selected.

In collaboration (section 6) too, the coordinator's real Codex failed on the same limit and real Claude took over. The parallel implementer that requested Codex started on Claude because of the cooldown.

## 4. Router / Judge / Council (ACP advisers)

With the HTTP endpoint version, Orochi itself connects using an API key. Because of the policy of not using API keys, it is unverified on real services. Instead, we added an `acp` setting ([configuration](adaptive-routing.md#4-router-frontier-judge-and-council)). Authentication for ACP advisers follows each CLI. In this environment there were no Provider API key environment variables; Claude showed its 5-hour and weekly allowances, and Codex showed a limit message prompting a plan change. Both were running on subscription sign-ins. The executing agent was a fixture; only the advisers were real services.

| Strategy | Adviser order | Result |
|---|---|---|
| Frontier Judge | Antigravity → Codex luna → Claude Haiku | Antigravity failed with `authentication`, and Codex returned a valid candidate ID (23,316 tokens, 4.5 s). The recommendation passed the constraints |
| Council (2 seats × 2 rounds) | Seat 1: Claude Haiku; seat 2: Antigravity → Codex luna | In round 1, seat 2 failed over to Codex. Round 2 started from the failover target. The majority recommendation passed the constraints. Advisers used 103,612 tokens in total |
| Lightweight Router (forced with `routing_confidence = 1.0`) | Antigravity → Claude Haiku | Haiku made the recommendation (28,326 tokens, 6.6 s) |

- Consumption per call was about 23,000 tokens for Codex and about 28,000 tokens for Claude Haiku. This comes from the system prompt and the like, and was the basis for adding `session_overhead_tokens` to the estimated budget.
- To pass the Council budget check (about 155,000 tokens), a synthesized long migration specification (about 180,000 characters) was used as the task. The task text was not sent to the advisers; it was used only to increase the local token estimate.
- Fixture tests confirm that adviser prompts contain neither the task text nor repository paths, and that the working directory is a temporary directory outside the repository that is deleted afterward.

Evidence: `advisers/` (configuration, stderr, data directory).

## 5. ACP E2E from Zed

With the user's approval, we added `agent_servers` to `~/.config/zed/settings.json` and a temporary binding for `agent::NewExternalAgentThread` to `keymap.json`. We opened a window for a dedicated project and sent keystrokes via System Events.

1. Zed launched `orochi serve` (Zed log: `connection; name="zed"`).
2. Sent a prompt from the agent panel.
3. Orochi selected Codex gpt-5.6-luna / medium and created `zed_result.txt` (`OROCHI_ZED_OK\n`). It passed evaluation (24,170 tokens, 23.8 s). Zed's log recorded the routing result that Orochi wrote to standard error.
4. Closed only the test window and confirmed that the `serve` process exited. Restored the two settings files from backup and confirmed that their SHA-256 hashes matched.

Screen capture is not permitted in this environment, so the on-screen display was not checked visually or by image. What was checked: Zed's log, Orochi's run records, and the generated file.

Evidence: `zed/` (backups, gateway configuration, data).

## 6. Extending collaboration (real Claude / Codex)

`collab-live/`: a coordinator (Codex preferred), two implementers (alice: Claude Haiku; bob: Codex preferred), a reviewer, an integrator (Claude Sonnet), 2 discussion rounds, `--apply`. Right after starting, `textstats/__init__.py` and `README.md` in the original working tree were edited by hand.

- Coordinator: real Codex hit its usage limit → failed over to Claude Haiku.
- alice and bob started at the same time (26.6 s, 36.3 s). bob ran on Claude because of the Codex cooldown. Zero merge conflicts.
- 11 messages (0 rejected). Sent coordinator → each implementer, bob → alice, reviewer → integrator, and so on. alice replied in discussion round 1 and then re-merged.
- The integrator (Sonnet) passed the checks.
- Applying to the working tree produced a conflict in `__init__.py` (the user's `__version__` addition versus the collaboration's export addition). A resolution session (Sonnet) kept both, and passed the checks and the marker check. Three files were applied.
- Independent check: the checks pass in the original working tree. The user's README edit and `__version__` remain. There are no markers. The session IDs of all 11 runs are distinct. No leftover processes or records.
- 1,672,438 tokens in total (adapter-reported values); agent run time was about 223 s.

## 7. Recovery from forced termination (real Claude)

`kill-resume/`: while the implementer (Claude Haiku) was responding, `orochi collaborate` was killed with SIGKILL.

- Four processes had been recorded: the supervisor process, `claude-agent-acp` (node), the `claude` CLI, and an MCP server started by the CLI (provided by a separate app). A check 5 seconds later found all four terminated and their records deleted.
- The report retained the `running` run and a 93-character partial response.
- `collaborate-resume` marked that run `interrupted` and redid the same step in a new session. Three steps completed, and the integrated result passed the checks.

Fixture tests confirm cleanup when the supervisor process itself is killed (on the next startup), termination of descendants that left the group, and that records whose owner is still running are left untouched.

## 8. Addendum: unifying advisers on ACP

At the user's decision, the HTTP endpoint version of Router / Judge / Council was removed and everything was unified on ACP. The `acp` setting in section 4 is the pre-unification format; the current format is as follows.

- Advisers are specified by an `[[agents]]` ID and model (`agent`, `model`, optional `reasoning`). An agent used only as an adviser has `routing_only = true` and is not a candidate for execution.
- Old settings containing `endpoint`, `api_key_env` or `acp` produce an error that explains how to migrate. Defaults are `timeout_secs = 120`, `max_resource_fraction = 1.0` and `max_output_tokens = 512` (for budget calculation).
- When an adviser hits an account-wide limit or an authentication error, or cannot be connected to, this is recorded in that agent's cooldown, and the agent is not launched during the cooldown. Records keep the agent and model of the adviser actually used.
- Council round 2 sends only the votes to the session that answered in round 1.
- `model` is optional. If omitted, the agent's model is chosen based on Policy at a per-role difficulty (Router: simple, Council: normal, Frontier Judge: complex). With real Codex, the Frontier Judge chose `gpt-5.6-sol` / high (24,927 tokens, 5.2 s) and the Router chose `gpt-5.6-luna` / low (23,332 tokens, 5.5 s). When an intermediate implementation temporarily used the CLI's default model, it was `gpt-6-astra` (20,830 tokens). Most of the total tokens are the system prompt, and the differences between models are small. Tokens are not charges, and do not reflect per-model unit prices or differences in how fast usage allowances are consumed.

Real-service check after unification (`advisers-acp/`; the executing agent is a fixture; all advisers are `routing_only`):

| Strategy | Result |
|---|---|
| Frontier Judge: Antigravity → Codex luna → Claude Haiku | Antigravity failed with `authentication`, and a cooldown was recorded on `antigravity/*`. Codex luna made the recommendation (23,374 tokens, 5.5 s) |
| Council: seat 1 Codex luna; seat 2 Antigravity → Codex sol (same data directory) | Seat 2's Antigravity was in cooldown, so it failed over without being launched (0 s). The majority recommendation passed the constraints |

Council session reuse (values reported by each adapter):

| Adviser | Round 1 (new) | Round 2 (same session) |
|---|---|---|
| Codex luna | 23,387 tokens (cache 11,008), 4.9 s | 29,323 tokens (cache 22,272, non-cache input 7,003), 2.4 s |
| Codex sol | 24,897 tokens (cache 0), 4.4 s | 30,806 tokens (cache 24,704, non-cache input 6,081), 2.5 s |

Reuse roughly halved the elapsed time, and most of the input became cache. On the other hand, because it includes the history, total tokens (including cache reads) increased. The budget check includes this increase in its estimate.

Local models are designed to be usable for both execution and advisers if the ACP-capable CLI supports them, but this has not been verified against a real CLI.

## 9. Addendum: agent-to-agent mailbox (2026-09-17)

Added messaging between agents launched by multiple Orochi processes. For the specification and verification against the real CLIs, see [Agent Mailbox](agent-mailbox.md). Two real Codex agents running concurrently in the same directory told each other function names and dictionary keys, and aligned their implementations.

## Tests

- `cargo test --locked`: 103 Rust integration tests (after ACP unification, automatic adviser model selection and the mailbox addition) and 4 Python quota parser tests run from within them passed.
- `cargo clippy --locked --all-targets -- -D warnings` and `cargo fmt --check` passed.
- Tests added: cleanup after forced termination (with / without the supervisor), merging parallel implementations and the marker check at integration, addressed multi-round messages, applying to the working tree / conflict resolution / protecting edits made during resolution / not applying unverified results, the ACP adviser Judge and Council (moved to `tests/router_acp.rs`; includes session reuse, cooldown recording, routing_only, and the migration error for old settings), classification of Codex's usage-limit payload, pooling estimates and replay, and validation of the bundled plans.

## Official references

- [Zed external agent configuration](https://zed.dev/docs/ai/external-agents)
- [Codex ACP](https://github.com/agentclientprotocol/codex-acp)
- [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp)
- [Antigravity CLI installation](https://www.antigravity.google/docs/cli/install)
