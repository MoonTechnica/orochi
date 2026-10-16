# Real validation: OpenCode and a local model (2026-10-16)

OpenCode 2.0.30 (`~/.opencode/bin/opencode`, its own ACP server through `opencode acp`) added
as a preset, and a model served from this machine run through it. The questions were whether
the preset finds the CLI with no configuration, what the CLI advertises over ACP, how the new
`policies/opencode.json` ranks a local model against OpenCode's free tier and the paid
catalog, and whether a task actually completes on the local model.

## Discovery

With no config file, `orochi agents` lists `opencode` as `ready` beside `codex` and
`claude`; `orochi setup --check` lists it as ready with nothing to sign in to, because its
`opencode.json` names a provider of its own (`providers.lmstudio`, LM Studio on
`127.0.0.1:1235`), which `setup::opencode_has_provider` reads.

`agents --discover` connected and listed 11 models with their selectors: one local
(`lmstudio/qwen/qwen3.8-27b`, no effort selector) and ten of OpenCode's free tier
(`opencode/glm-5.3-flash-free`, `opencode/deepseek-v4.1-flash-free`, `opencode/big-pickle`,
…), each with the effort words its upstream uses (`low … max, default`; one with `none`
and `minimal`) and the two session modes `build` and `plan`, which are OpenCode's agents.
The model ID carries the upstream provider as a prefix, and that prefix is what the policy
matches on.

## Routing

`--dry-run` on "add a unit test for greet in test_greet.py" (profiled `test / python /
normal`), default config, no agent pinned:

| Rank | Agent | Model | Expected cost | Estimated success |
|---|---|---|---|---|
| 1 | opencode | `lmstudio/qwen/qwen3.8-27b` | 2,557 | 0.70 |
| 2 | opencode | `opencode/glm-5.3-flash-free` (low) | 3,952 | 0.76 |
| 3 | opencode | `opencode/deepseek-v4.1-flash-free` (low) | 3,952 | 0.76 |
| … | claude / codex | `haiku`, `gpt-5.6-luna` | ≈13,000 | 0.74 |

The local model wins small work on price alone (`price_premium` 0.1, free tier 0.2: Orochi's
own heuristics for "billed nothing but the machine's time" and "billed nothing, shared and
throttled", not published numbers) and is held back from hard work by its prior: at
`Complexity::Complex` the local prior (0.40) and the free prior (0.55) fall under
`scheduler.required_success` (0.70) and only the paid catalog remains
(`a_local_model_through_opencode_takes_small_work_and_is_not_offered_hard_work`). The
EWMA moves both with evidence.

## Execution

`orochi --agent opencode --model lmstudio/qwen/qwen3.8-27b --permission allow` on the same
task in a one-file scratch repository (the task text reached the agent with its backticks
eaten by the shell, so it read "make `make` pass"):

| | |
|---|---|
| Wall time | 2 min 51 s |
| Reported usage | 106,852 total; 103,734 input; 1,617 output; 1,501 reasoning; `requests` 1 |
| Outcome | `partial_success` (no substantive verification: the repository had no checks) |
| Result | `test_greet.py` with three tests and a `Makefile`; `greet.py` untouched; **the agent committed the result itself** |

What the reported total covers is the adapter's choice (§7.4 of
[adaptive-routing.md](adaptive-routing.md)): OpenCode produced one `usage_update` for the
whole turn, so whether 106,852 is the turn or its last request is not known from the
payload, and no cache breakdown was reported. Until that is read from OpenCode's source,
its figures are not comparable with Claude's or Codex's.

The agent committing on its own is OpenCode's default `build` agent at work, not anything
Orochi asked for; a conversation that wants the tree left uncommitted will want to say so.

## Failure scope

A limit or an outage on one of OpenCode's upstreams is read as the model's, not the agent's
(`ProviderAdapter::classify_error`, `model_scoped` for `Provider::OPENCODE`): a spent free
model must not cool down the local one beside it. This is the narrowest scope the runtime
has, and narrower than the truth — the free models share one account — so after one free
model's limit the next free one is tried and will likely meet the same limit, bounded by
`scheduler.max_attempts`. **Not exercised live**: the free tier did not limit during this
run.

## Not done

- A lean classification session (`LeanConfig::known`) has no entry for OpenCode:
  `OPENCODE_CONFIG_CONTENT` can set an agent's `prompt` file, but whether that replaces or
  extends the default system prompt is undocumented and unmeasured, so the classifier asks
  OpenCode at a full session's cost.
- No quota probe: OpenCode publishes no usage CLI; its free models say only "for a limited
  time".
- The policy's paid rows cover the Claude, GPT and Gemini-free families by the Zen price
  list; everything else (Kimi, GLM, Qwen, DeepSeek, MiniMax paid) is the `*` row, since the
  catalog says nothing about what they are for and a `-free` suffix would otherwise be
  outscored by a family pattern.

## Roster proposal

`orochi roster suggest --dry-run` with the four presets discovered (Antigravity not installed,
so no models), no runs measured and nothing remembered. Codex answered first, in **4.2 s and
15,552 tokens** (recorded as a `roster` run):

> Kept a low-cost and capable option for each agent, retained the local model, and removed
> duplicate or older variants; Antigravity lists no models.

| Agent | Proposed |
|---|---|
| claude | sonnet, opus, haiku |
| codex | gpt-6.1-sol, gpt-6-astra, gpt-6-luna |
| opencode | lmstudio/qwen/qwen3.8-27b, opencode/big-pickle |

No preferences were proposed, which is right with nothing measured. Every ID named was one the
agent advertised. Not applied here; the real configuration was not touched.

## Roles: a roster with a note per model, run for real

The roster was set by hand on this machine — OpenCode: the LM Studio model alone; Claude:
`fable`, `opus`, `sonnet`, `haiku`; Codex: `gpt-6-astra`, `gpt-6.1-sol`, `gpt-6-luna`;
Antigravity off — with a note per model in Japanese (`opus`: 設計とレビュー、複雑な実装;
`haiku`: 調査、ドキュメント、小さな修正; `gpt-6-luna`: 小さな修正、整形、定型作業; the local
model: ローカルで済む小さな編集と、外に出したくないコード; and so on). A dry run of a small
task then listed exactly the eight rostered models and nothing else. Three tasks were run in
the one-file scratch repository, classifier on:

| Task | Classified by | Read as | Route | Wall |
|---|---|---|---|---|
| Add a one-line docstring | codex / gpt-6-luna, 6 s | small_edit, `suited: gpt-6-luna` | opencode / local | 2 min 04 s |
| Propose a test layout for 10+ modules, change nothing | claude / haiku, 12 s | architecture, `suited: opus` | opencode / local | 3 min 27 s |
| Add tests for `""` and `None` | opencode / local: **timeout at 60 s** | heuristic `test` | opencode / local | 4 min 38 s |

All three finished with correct work (the third's `unittest` ran green), but two things were
wrong, and both were fixed the same evening:

- **The note did not decide the route.** Haiku read the design question as suited to `opus`,
  the note having said so, and the scorer multiplied Opus's cost by 0.35 — to 8,812 against
  the local model's 2,557. No tilt reaches past a model that costs a tenth. A `suited` whose
  model the user noted is now `spoken_for`: sorted before price and alone in the selection
  pool. Rerun from the cache: `Route: claude / opus / medium — you said opus is for this kind
  of work`, answered in 36 s (176,766 tokens, most of it reading the tree; the run ended in
  `cancelled` because `--permission deny` refused its `git diff`, with the five-point answer
  already written).
- **The local model was asked to classify, and timed out, and would have been asked first
  again.** An answerer nobody has timed is asked first; the local model took over the 60 s
  budget to classify, the budget is one agent's worth, and a timed-out answer was not counted
  as timed. It is now counted at its elapsed, so it goes last: `roster suggest` afterwards
  went to Codex in 5 s. The test task itself, rerun, still met the one remaining timeout
  record from before the fix and ran locally; its six tests pass.

What a classification costs here, measured: Codex `gpt-6-luna` 5–6 s and ~15,500 tokens,
Claude `haiku` 12 s and ~4,200 tokens, the local 27B model more than 60 s.
