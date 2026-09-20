# Orochi Against Each Agent Used Alone (2026-09-20, pilot)

A first measurement of the question Orochi exists to answer: does routing beat using one CLI on its own? Three tasks, three arms, real Claude Code and Codex. Evidence is in `.orochi/live-e2e/compare-20260920-*` (local only, untracked). The harness is [`examples/compare_live.py`](../examples/compare_live.py) with [`examples/compare-suite.json`](../examples/compare-suite.json); every case fails as handed to the agents and passes with the reference solution kept beside it (`--validate`, checked by `tests/cli_e2e.rs`).

**This is a pilot of 3 of the suite's 10 tasks, one sample each.** Codex's weekly allowance was at 7% (resetting 2026-09-22 17:29), so the full suite is still to be run. Nothing here establishes a general result.

## Arms

| Arm | What it is |
|---|---|
| `claude-alone` | Claude Code as its owner had configured it: `claude-fable-5-1[1m]`, effort xhigh, one attempt, no classifier |
| `codex-alone` | Codex as its owner had configured it: `gpt-6-astra`, reasoning high, one attempt, no classifier |
| `orochi` | Both agents, classifier on, up to 3 attempts, verified by the repository's visible tests |

Every arm is the same binary and is scored only by a hidden check run afterwards; the repository's visible tests are what the agents and Orochi's evaluator can see. An arm is charged every token its runs recorded.

## Result

| Task | claude-alone | codex-alone | orochi |
|---|---|---|---|
| Fixing decimal money handling (Python, 2 files) | ✓ 436,255 / 142 s | ✓ 23,848 / 70 s | ✓ 58,322 / 80 s |
| Adding a README section (docs) | ✓ 202,348 / 21 s | ✓ 21,882 / 26 s | ✓ 54,931 / 29 s |
| TTL + LRU cache (Python, many edge cases) | ✓ 319,099 / 279 s | ✓ 23,665 / 77 s | ✓ 59,071 / 98 s |
| **Total** | **3/3, 957,702** | **3/3, 69,395** | **3/3, 172,324** |

The `orochi` numbers are its own re-run with the classifier's usage recorded (`.orochi/live-e2e/compare-20260920-pilot-orochi`); the first pass of the same three tasks (`compare-20260920-pilot`) was measured with a binary that did not yet record what the classifier spent, and showed 80,450 tokens for the same three passes.

- **Accuracy did not separate the arms**: every arm passed every task. These three tasks are not hard enough to tell the arms apart, which is itself a finding about the suite.
- Orochi routed all three tasks to `codex / gpt-5.6-luna` — the cheapest model that cleared the policy floor — and passed with it.
- Tokens are what each ACP adapter reported, not charges, and most of them are cache reads (Claude: 371,125 of 436,255 on the money task; Codex: 23,040 of 23,848). **Totals compare arms within one provider far better than across providers.**

## What the classifier costs

Orochi asks an agent what the task is before routing it. That question is one more ACP session, and a session costs about as much as the work on a small task:

| Who classified | Classification | Execution | Total |
|---|---|---|---|
| `claude / haiku` | 37,630 | 24,356 | 61,986 |
| `codex / gpt-5.6-luna` | 23,800 | 24,148 | 47,948 |

Almost all of it is the session's fixed cost — system prompt and cache reads — not the question. So on these tasks Orochi pays roughly twice what the same work costs on `codex-alone`, and the classifier's agent is currently chosen by the order agents appear in the configuration, not by what asking each one costs.

## Standing conclusions

1. **Against `claude-alone` Orochi was 5.6× cheaper in reported tokens** (172,324 vs 957,702) at the same accuracy, by choosing a cheaper provider and model for work that did not need the expensive one.
2. **Against `codex-alone` Orochi was 2.5× more expensive** (172,324 vs 69,395) at the same accuracy. The gap is the classifier's extra session, not the executions (24k vs 24k per task).
3. **Neither result is yet the goal**: to be better than the best single agent, Orochi must stop paying for a question whose answer cannot change what it does. Options measured here: ask the agent that answers cheapest (24k instead of 38k), or ask only when the label could change the chosen candidate.

## Changed because of this

`classifier::agents` now asks whichever agent has answered most cheaply, from what its `classification` runs recorded (`storage::advice_cost`), instead of following the order agents appear in the configuration; an agent nobody has priced yet is asked once first. On the machine measured here that is 23,800 tokens instead of 37,630 per classification. What the classifier spends is recorded at all only since 2026-09-20 (`what_the_classifier_spent_is_recorded_beside_the_run_it_decided`); before that a run's cost left out what deciding it took.

Since then Orochi also **stops asking where the question is not worth its price**: `classifier::worth_asking` weighs what asking costs here against what work like this has cost here (`storage::typical_tokens`), and keeps the local profile when asking would take more than `classifier.max_cost_share` (0.25) of the work. With nothing measured yet it asks — asking is how both costs become known — so the rule takes hold after about three runs and never applies to a cached answer, which is free. The evaluator still checks the result, and a failure still moves the work to another candidate.

On the numbers above that turns Orochi's ~24k-per-task classification into a one-off: the arm that spent 172,324 tokens on three tasks would spend about 24k less on every task after the third. **Unverified with real agents**: the rule is covered by tests only (`a_question_that_costs_more_than_the_work_it_decides_is_not_asked`, `it_stops_paying_to_ask_what_a_request_is_once_that_costs_more_than_the_work`), and the pilot was too short for it to take hold.

## Still to do

The full 10-task suite, after Codex's weekly allowance resets (2026-09-22 17:29), with the `orochi` arm keeping one data directory across the suite as a user's own Orochi does:

```sh
python3 examples/compare_live.py --suite examples/compare-suite.json \
  --output .orochi/live-e2e/compare-<date> --binary target/release/orochi \
  --adapter-cache ~/.local/share/orochi/adapters \
  --agent-path-prepend ~/.local/bin --agent-path-prepend ~/.bun/bin \
  --agent-path-prepend /Applications/ChatGPT.app/Contents/Resources \
  --agent-path-prepend ~/.cargo/bin --timeout 900 --execute
```

That run should show whether the classifier's price stops being paid per task, whether the harder tasks separate accuracy at all, and whether Orochi's verify-and-retry recovers a failure that a single agent does not.
