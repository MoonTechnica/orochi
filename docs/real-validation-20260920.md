# Live Validation of the Conversation Store and the Desktop Window (2026-09-20)

What [Desktop App Design](desktop-app-design.md) left unverified, measured against the real
CLIs. Everything below was run through the installed, authenticated `claude` and `codex`, not
the fixture agent. Evidence is the store itself; the scratch directory is local only.

| # | Item | Result |
|---|---|---|
| 1 | A real rate limit is classified, not quoted | **Confirmed.** Codex's account was spent; the store recorded `rate_limit` and the runtime went to an account-wide cooldown. The provider's message reached the conversation and **not** telemetry |
| 2 | A real turn is recorded losslessly | **Confirmed** against Claude Haiku. Every ACP kind P0 added appears, including ones the fixture never sends |
| 3 | The ACP fields `acp.rs` stopped dropping | **Confirmed.** Real Claude sends `available_commands_update`, `usage_update`, and tool calls with `kind` and `rawInput` |
| 4 | A patch's path | **Bug found and fixed.** A real agent names the file absolutely; the pane wanted the repository's own name |
| 5 | P4: does an agent act on a note left mid-turn? | **Confirmed with Sonnet, not with Haiku.** The mechanism works; whether it is read depends on the model |

Cost: four runs, about 771,000 tokens by the adapters' own figures, all on the cheapest models
that could answer.

## 1. A real rate limit reports its kind, not the text it was read from

Codex's account was exhausted (`You've hit your usage limit… try again at Sep 22nd`), which
made this the first real test of the invariant that a classified failure reports its kind.

```
attempts:  codex | gpt-5.6-luna | failure | rate_limit | "RateLimit: Internal error"
v_runs:    codex | gpt-5.6-luna | failure | rate_limit
v_runtime: codex | *            | cooldown, still cooling
```

The provider's own message — which names the account's plan and carries two URLs — appears in
`activity.sqlite3` as what the agent said, and **nowhere** in `telemetry.sqlite3` or its WAL
(checked on the raw bytes). The cooldown is on `codex/*`, the account-wide scope, which is
what a rate limit without `data.scope = "model"` is supposed to do.

This also exercised the new telemetry views (`v_runs`, `v_runtime`) against a real failure.

## 2 and 3. A real turn, and the ACP fields that were being dropped

One task on Claude Haiku (`Create a file called hello.txt whose only content is the word:
orochi`), which the agent completed in about 9 seconds. What the store holds:

| Item | Where it came from |
|---|---|
| `user_message` | the task as typed |
| `route` | claude / haiku / agent-default |
| `commands` | **`available_commands_update`** — Claude sends its own slash commands |
| `context` | **`usage_update`** — the context gauge |
| `tool_call` | `title: "Write hello.txt"`, **`tool_kind: "edit"`**, `detail` from `rawInput.file_path` |
| `agent_message` | "Done. Created `hello.txt` with content "orochi"." |
| `checks` | `partial_success` — nothing to verify, correctly not called a success |

The three in bold are the ACP kinds and fields `acp.rs` dropped before P0 and which the
fixture agent does not send. This is the first evidence that P0 reads what a real agent
actually emits. `attempts.run_id` linked to the telemetry run, with 70,479 tokens recorded.

## 4. A patch is filed under the path the repository uses

The one defect this validation found. Real Claude names the file it changed with an **absolute**
path in its ACP diff, so a patch was stored as
`/private/tmp/.../scratchpad/live/repo/hello.txt` — which a review pane would show in full and
`v_turn_files` would group by. The design says the path is relative to the seat's workspace.

Fixed in `activity::patch`: a path inside the thread's own directory is made relative to it,
and one outside keeps the only name it has. The thread's `cwd` is also resolved, because
`/tmp` and `/private/tmp` are the same directory on macOS and an agent may name either.
Pinned by `a_patch_is_filed_under_the_path_the_repository_uses`.

## 5. P4: a note left in the room while a turn is running

The question the design refused to answer without a real run: a person leaves a note
mid-turn, and it is delivered the way every mailbox message is — when the agent next calls
`read_messages`. Does an agent ever call it?

The harness starts a turn whose task tells the agent to call `read_messages` and write down
what it hears, waits for its session to appear in the room, leaves a note with
`orochi peers --say "the magic word is beryl"`, and reads back what the agent wrote.

| Model | Read the note? | What it wrote |
|---|---|---|
| Claude Haiku | **No** | `none`. It never called the tool, and spent the turn describing the repository instead |
| Claude Sonnet | **Yes** | `the magic word is beryl` |

The store shows why this is a result about the agent and not about the delivery: the note was
in the room three seconds after the peer joined, addressed to `all`, with the peer's
`last_read` still at 0 — visible by every rule `read` applies. Sonnet's peer ended with
`last_read = 2`, so it read and marked it.

So the mechanism is sound and **a capable model does act on a mid-turn note**. What cannot be
claimed is that any agent will: Haiku, told plainly to call the tool, did not. The Team pane's
wording — "delivered when the agent next checks" — is the honest one, and it should stay.

## What is still not established

- **Antigravity and Gemini** remain unvalidated here, as in earlier records.
- **The desktop window inside Tauri.** Its page was checked by rendering the same `app.js` and
  `app.css` in a browser (`desktop/dist/preview.html`), which found and fixed four layout
  defects; the packaged window uses the same WebKit but its own chrome, and has not been
  looked at.
- **A collaboration on real agents under the store.** `collaborate` records rows in the
  fixture tests only; Codex's account was spent before a two-agent run could be measured.
