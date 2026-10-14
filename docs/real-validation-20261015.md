# Real validation: a lean classification session (2026-10-15)

The question was whether an intent-analysis step in the shape of a decision model (typed
questions, a few hundred milliseconds, no API key) can be paid for from a subscription over
ACP, and what it would cost. Measured on this machine with the real CLIs and the real
adapters, signed in with the user's own subscriptions: Claude Code 2.1.299 through
`claude-agent-acp` 0.77.0, and `codex` 0.160.1. One classification question each time, the
same task text, the reply read back and checked.

## What asking costs, by how the session opens

| Path | Context tokens | Turn | Reply |
|---|---|---|---|
| `claude-agent-acp`, adapter default (preset system prompt, user's default model) | 43,854 | 28.0 s | correct |
| `claude-agent-acp`, `_meta.systemPrompt` + options `settingSources = []`, `strictMcpConfig`, `disallowedTools = ["*"]`, `maxTurns = 1`, `haiku` | **815** | **6.9 s** | correct |
| `claude -p`, no flags, in this user's home | 380,569 | 30.9 s | correct |
| `claude -p --strict-mcp-config --mcp-config '{"mcpServers":{}}' --setting-sources "" --tools "" --system-prompt …` | 730 | 10.4 s | correct |
| `claude --bare -p` | — | — | `Not logged in`: bare mode never reads the subscription login |
| `codex exec -m gpt-6-luna`, default | 20,587 input | 29.5 s (first start) | correct |
| `codex exec -m gpt-6-luna -c model_instructions_file=…` | 14,645 input | 5.8 s | correct |

The 380,569-token `claude -p` run is the MCP tool definitions configured in this user's
`~/.claude`; a plain `-p` session loads everything an interactive one would. The remaining
14,645 on Codex are its tool definitions, which no configuration key drops.

Session start over ACP was 31 s (lean) and 37.5 s (default) wall clock on top of the turn,
measured from the adapter's launch to `session/new` returning. The turn is 7 s; the start is
not, and it is paid by every classification today. A classification session that is kept
open and asked again would remove it; that is not implemented.

## What changed because of this

- `discovery::lean` says the instruction in each agent's own words, from `[agents.lean]`
  (`LeanConfig`: `meta` templated into `_meta` on `session/new`, `env` onto the launch). ACP
  has no field for a system prompt, so this is per agent by nature; Orochi fills it for the
  commands it ships. The `_meta` keys for `claude-agent-acp` are the adapter's
  (`params._meta.systemPrompt`, `params._meta.claudeCode.options`, read from its
  `dist/acp-agent.js`); `CODEX_CONFIG` is documented by `codex-acp` as merged into the
  session configuration and was measured here through `codex exec`, not through the adapter,
  which was not installed on this machine; `GEMINI_SYSTEM_MD` is from Gemini CLI's
  documentation and was not run here.
- `classifier::agents` orders answerers by measured wall time (`storage::advice_latency`,
  session start included, median, refusals excluded) instead of by tokens, and within an
  agent a model measured faster than the scorer's cheapest is taken instead
  (`storage::advice_latency_models`). A spent account is in cooldown and not started, so the
  next fastest answers.

## Where a subscription stands (read 2026-10-15)

- Anthropic's legal page limits subscription OAuth to Claude Code and Anthropic's own
  applications and tells developers of products to use API keys; the billing split that
  would have moved ACP, `claude -p` and the Agent SDK to a separate credit was paused on
  2026-06-16 and they still draw from subscription limits, with a change promised to come with
  notice. A tool the subscriber runs on their own machine is the case left standing; routing
  other people through one's plan is not.
- OpenAI documents `codex app-server` driven by a third-party app under a ChatGPT plan, and
  `gpt-6-luna` is in the plan's model list. The Decisions API (`POST /v1/decisions`, the same
  model, $0.10 per million input tokens) takes an API key; the plan-usage documents cover the
  Responses API only.
- Jev (TypeSafe) is API-only, closed-weight and waitlisted. Local decision models (kev,
  Laya, GLiNER2.5-Decide) run on this Mac but are 2–10 GB on disk and not ACP agents; they
  were not run here, and their accuracy figures are their authors' own.

Unverified: `CODEX_CONFIG` through `codex-acp` itself, `GEMINI_SYSTEM_MD` through
`gemini --acp`, and whether the replaced system prompt changes the classifier's answers on
real tasks beyond the one measured.
