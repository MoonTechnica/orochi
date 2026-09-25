# Real Validation: what ACP advertises (2026-09-25)

What was read from the CLIs and adapters installed on this machine, and what was measured in this
machine's own telemetry. Nothing here consumed quota: the routing checks are `--dry-run` and
`agents --discover`, which create an ACP session and send no prompt, and the token figures come
from runs that had already happened.

Sources read: `@agentclientprotocol/codex-acp` 1.10.0 (installed) and 1.13.1 (latest, fetched to
read), `@agentclientprotocol/claude-agent-acp` 0.77.0 with `@anthropic-ai/claude-agent-sdk`, the
`codex` and `claude` binaries.

## 1. Effort vocabularies, as the agents actually advertise them

`orochi agents --discover --json` against the real Codex:

| Model | `thought_level` values (selector id `reasoning_effort`) |
|---|---|
| `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra` | `low, medium, high, xhigh, max, ultra` |
| `gpt-5.6-luna` | `low, medium, high, xhigh, max` |
| `gpt-5.5` | `low, medium, high, xhigh` |

Two things follow. The vocabulary is **per model and supplied by the provider's server** —
`codex-acp` builds the option from the app-server's `supportedReasoningEfforts`, and
`claude-agent-acp` from the SDK's `supportedEffortLevels` (`low|medium|high|xhigh|max`) plus a
`default` row of its own — so it can change without either adapter changing, and Orochi cannot
hold a fixed list. And Codex now offers two rungs (`max`, `ultra`) **above** where the OpenAI
policy's `extreme` rule stops (`xhigh`). That cap is deliberate and unchanged here; it is worth
revisiting only with a measurement, since asking for the top rung roughly doubles the token prior.

Claude could not be discovered at the time of the check (`agent/account is in cooldown` from a
previous rate limit), so its live vocabulary was read from the SDK's type declaration rather than
from a session.

## 2. The capability gate, on the real path

`orochi "take a screenshot of the page with playwright in the browser" --dry-run` now returns 5
candidates, each carrying the reason *"no agent here advertises browser; asked anyway rather than
excluding every agent"*. Before this change the same request returned **no eligible execution
candidate** — every agent was excluded by one capability flag nobody had filled in. `codex` and
`claude` now declare `web`, read from `web_search` in the Codex binary and `WebSearch`/`WebFetch`
in the Claude binary.

## 3. What a reported token total covers (the adapters disagree)

This was filed as a suspicion — 52 lines of output reported as 214 output tokens — and it is real.

**`codex-acp`** (identical in 1.10.0 and 1.13.1):

```js
handleTokenUsageUpdated(params) {
  this.sessionState.lastTokenUsage  = toTokenCount(params.tokenUsage.last);
  this.sessionState.totalTokenUsage = toTokenCount(params.tokenUsage.total);
  …
}
// and the prompt response:
return { stopReason: "end_turn", usage: this.buildPromptUsage(sessionState.lastTokenUsage), … };
```

The turn's own total is computed and kept — and used only for the adapter's `/status` text. What
reaches ACP is `tokenUsage.last`: **the final model request of the turn**. A turn that made
seventeen requests reports the seventeenth.

**`claude-agent-acp`** does the opposite: `session.accumulatedUsage` adds every assistant
message's `usage` together and the prompt response reports that sum, i.e. the whole turn.
(Its `usage_update.used` is the context the last assistant message occupied, which is a different
quantity again.)

Measured in this machine's telemetry on 2026-09-25, weighted medians (`Usage::weighted()`):

| Seat | Purpose | n | Median weighted tokens |
|---|---|---|---|
| `codex` / `gpt-5.6-luna` | execution | 7 | 6,244 |
| `codex` / `gpt-5.6-luna` | classification | 3 | 24,262 |
| `claude` / `haiku` | execution | 19 | 49,837 |
| `claude` / `sonnet` | execution | 9 | 112,007 |
| `claude` / `opus[1m]` | execution | 4 | 109,372 |

A classification is a single model request, so on Codex it is reported whole — and it comes out
**four times dearer than an execution on the same seat**, which no amount of model difference
explains. The executions are the under-reported ones. Every cross-agent comparison built on these
numbers is skewed in Codex's favour by roughly the number of requests in a turn:
`storage::session_floor`, `typical_tokens`, the EWMA's measured side, and both gates calibrated
against them (`classifier::worth_asking`, `roles::worth_seating`).

**What was changed:** `Usage::requests` records how many `usage_update` notifications the agent
sent while the turn ran — roughly one per model request in both adapters. Nothing is estimated
from it and no total is altered: a reconstruction would have to invent the cache/output split,
which `Usage` is not allowed to do. It is there so that a figure covering one request is no longer
silently compared with one covering seventeen, and so that the size of the distortion can be
measured rather than argued about.

**What was not changed:** the recorded totals, and therefore the thresholds calibrated on them.
Fixing that properly needs `codex-acp` to report `tokenUsage.total` (or to put it in `_meta`,
which Orochi already reads); until then, Codex token figures are a lower bound and the two agents'
token counts are not comparable.

## Still unverified

- **Claude's live effort selector** — the account was in cooldown; read from the SDK types only.
- **Fable against Opus** — needs a complex console turn on Claude; not run, and it costs quota.
- **`classifier.max_cost_share` = 0.25** — still n=3 classifications on `codex` and n=3 on
  `claude` in this store, and the execution side of that ratio is exactly what §3 under-reports.
  The number cannot be calibrated until Codex's totals are trustworthy.
- **Non-canonical effort vocabularies** — handled and tested against fixtures, but no CLI
  installed here advertises one, so the translation has not met a real agent.
