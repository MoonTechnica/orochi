# Orochi

**Adaptive Agent & Model Scheduler** — a Rust CLI that chooses an ACP Coding Agent based on the task, usage limits, Provider Policy and local track record.

```sh
orochi "Switch the authentication logic to refresh tokens"
orochi   # started on a terminal, it opens an interactive session (orochi -c resumes the most recent conversation)
```

## Current implementation scope

The [Original Draft Specification](docs/design-draft.md) is being implemented incrementally. A normal run delegates to one Agent; `collaborate` performs implementation, review and integration in multiple independent sessions.

- CLI detection for Claude / Codex / Gemini / Antigravity, plus configuration of any ACP stdio adapter
- Detection of the Claude/Codex CLIs themselves, and automatic installation of missing ACP adapters into a dedicated cache
- Stable protocol v1 connection, streaming and cancellation via the official Rust ACP SDK 2.0.0
- Retrieval of models, reasoning and modes from `configOptions`. After a model change, the returned configuration is re-read and validated
- Compatibility handling for the legacy `models` / `session/set_model` and `modes`. For an Agent with no configuration, the Agent's own defaults are used
- Local Task Profiler, deterministic routing, a lower bound on success probability, Provider hard constraints
- Optional adviser (an ACP agent). It recommends only candidate IDs, and the Scheduler re-validates them
- Agent-wide / per-model cooldown, reset time, 5 min → 15 min → 30 min → 60 min backoff
- When remaining usage can be obtained, estimation of a quota shadow price, history statistics and cache affinity
- Fallback via TaskEnvelope. Files being changed are kept, and the full conversation is not forwarded
- Evaluation by tests, typecheck, lint, build and git diff; local SQLite telemetry
- Session listing, explicit `--resume`, Policy validation and atomic updates
- Exclusive execution per repository, waiting for child processes to exit, process-group termination on Unix

Added: EWMA and a constrained Bandit, prediction calibration and candidate comparison, direct retrieval of Codex quota, ingestion of Claude statusline quota, a Frontier routing Judge, a two-round routing Council, and exposing Orochi over ACP with `orochi serve`. For details, configuration and unverified parts, see [Adaptive Routing and the ACP Gateway](docs/adaptive-routing.md).

Added on 2026-09-18/19 (automated tests only; not yet verified with real agents): a task classifier that asks an agent instead of matching keywords (on by default), team escalation from the console for work that splits across workspaces, `collaborate` without a hand-written plan, memory across sessions and agents, route preferences, weak labels from what you do after an answer, and tier pooling (off by default). See [Adaptive Routing and the ACP Gateway](docs/adaptive-routing.md) and [Personalization Design](docs/personalization-design.md).

[Measurement and Session Collaboration Validation (2026-09-16)](docs/real-validation-20260916.md): confirmed measurement of 8 identical-task pairs on Claude/Codex, holdout validation of the learning coefficients, on-demand quota retrieval for Claude, implementation, review and integration by 3 independent sessions of the same Claude Haiku, and task execution from the exposed ACP gateway to the real Codex. This small data set did not show an advantage for EWMA/Bandit, so the default coefficients are kept.

[Session Collaboration](docs/session-collaboration.md) is used through `orochi collaborate`. Even with the same Agent and model, a different session is a different worker. In every role — coordinator, implementation, review and integration — a usage limit makes the role switch to another available Agent/model. Plans, decisions, intermediate answers and progress are saved to `report.json`, and if no candidate is usable the run can be resumed later with `collaborate-resume`. It supports up to 4 parallel implementers with merging, addressed messages to any participant, and discussion rounds. With `--apply`, a result that passed verification is merged into the original working tree, and conflicts are resolved by a dedicated resolution session. Without `--plan`, the team is derived from the task; `--dry-run` prints it as a plan file that can be edited and passed back with `--plan`.

With the [Agent Mailbox](docs/agent-mailbox.md), `orochi` agents running at the same time in multiple terminals can exchange messages about work on the same repository (including worktrees). `scheduler.shared_workspace = true` also allows concurrent runs in the same directory. Confirmed between real Codex instances: one communicated a function's specification and the other matched its implementation to it.

Agents are started through a supervisor process. Even if Orochi is force-killed, the Agent and its descendant processes are terminated. If the supervisor itself also stops, they are reclaimed at the next start.

The Router / Judge / Council advisers also use `[[agents]]` agents over ACP (the HTTP endpoint version has been removed). Orochi does not handle credentials; it follows each CLI's own authentication (a subscription login or an API key). If an ACP-capable CLI supports local models, they can be used both for execution and as advisers.

The [Live Validation of Remaining Tasks (2026-09-16, part 2)](docs/real-validation-20260916-2.md) confirmed the following.

- Switching from the real Codex's usage limit to the real Claude
- Parallel implementation, discussion and conflict resolution against the working tree by real Claude / Codex
- Recovery and resumption after a forced kill
- Switching between advisers in all 3 ACP adviser modes
- Execution from Zed's agent panel
- Measurement of 6 tasks differing in language and difficulty

Switching from the real Claude's usage limit to the real Codex has not been confirmed. Live E2E for Antigravity is waiting on a Google login. Additional validation of the Gemini CLI was made out of scope at the user's instruction. Verification against the real CLI with local models has not been done yet.

## Build

Requires Rust **1.96.0** and C/C++ build tools. SQLite is built from the bundled source.

```sh
cargo build --release --locked
cargo install --path . --locked
orochi --help
```

Rust is pinned in `rust-toolchain.toml`. If you use mise, select the same Rust version in this directory.

## Preparing Agents

Install the CLIs you will use, and authenticate with **each Agent's own official login procedure**. Orochi neither collects credentials nor logs in on your behalf.

| Agent ID | Detection / connection method | Source |
|---|---|---|
| `codex` | The `codex` CLI itself, or `codex-acp`. A missing adapter is installed automatically | [Codex ACP](https://github.com/agentclientprotocol/codex-acp) |
| `claude` | The `claude` CLI itself, or `claude-agent-acp`. A missing adapter is installed automatically | [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp) |
| `gemini` | `gemini --acp` | [Gemini CLI ACP mode](https://geminicli.com/docs/cli/acp-mode/) |
| `antigravity` | `agy_acp_server.par` | [Antigravity ACP Registry listing](https://zed.dev/acp/agent/antigravity-acp) |

Claude/Codex are detected even when only the CLI itself is installed. If the adapter is missing, it is prepared at the first discovery using Node.js 22 or later and npm.

```sh
orochi config init
orochi agents
orochi agents --discover
```

The presets detect executables on PATH. An existing ACP command takes precedence, and whatever is missing is installed into `adapters/` in the data directory at pinned versions (Codex ACP 1.10.0, Claude Agent ACP 0.77.0). Global installations and the target repository's dependencies are not modified. Installation is from the official npm registry with install scripts disabled, and a completed cache is reused. The CLI itself is passed to the adapter via `CODEX_PATH` / `CLAUDE_CODE_EXECUTABLE`. Explicitly configured env settings are kept.

If a CLI is not on PATH, you can set an absolute path in `command`. The native Claude/Codex executables are identified by the file names `claude` / `codex`. A custom ACP command and arguments are used as-is. Gemini uses its native ACP, and Antigravity uses its ACP executable. For any other Agent, register an ACP command in `[[agents]]`.

The four known presets are filled in even when omitted from the configuration file. An explicit entry with the same ID takes precedence, so `enabled = false` excludes one. To control the automatic fill-in and downloads:

```toml
[discovery]
auto_add = true          # if false, use only what is written in [[agents]]
auto_install = true     # if false, do not auto-install missing adapters
setup_timeout_secs = 120 # separate from the ACP connection timeout
```

`orochi agents` / `status` do not download anything. `ready` means ready to connect, and `adapter_required` means the CLI itself was detected and first-time setup is pending. `setup_unavailable` means Node/npm is missing, and `setup_disabled` means automatic installation is disabled. Authentication and ACP connection failures are shown separately by `agents --discover` and at routing time.

`agents --discover` and `--dry-run` prepare the needed adapters, create an ACP process and a temporary session, and retrieve the configuration. The task prompt is not sent. Even when installed, a problem with authentication, execution permissions or ACP compatibility is shown as a failure reason.

## Usage

```sh
orochi "Implement this issue"
orochi -C /path/to/repository "Fix the bug"

# Check the candidates and the reason for the choice without sending the prompt
orochi --dry-run --json "Refactor this module"

# Policy and quota constraints apply even when specifying an Agent or model
orochi --agent claude "..."
orochi --model '<ID obtained from agents --discover>' --reasoning high "..."

orochi status                  # readiness, CLI path and saved usage for all agents
orochi --status                # short form of status
orochi status --discover       # actually connect to check available models and failure reasons
orochi status --json           # also available as JSON
orochi sessions
orochi peers --messages        # agents running in the same repository, and their messages
orochi --peer-name backend "..."  # name in the mailbox
orochi --resume '<session ID>' "Continue by adding input validation"
orochi runs --limit 20
orochi policy status
orochi memory                  # what Orochi remembers about you and this repository
orochi memory --forget r2      # forget one item
orochi collaborate "..." --dry-run   # the team Orochi would derive, printed as a plan file
```

Normal Agent answers go to stdout; routing reasons and verification results go to stderr. `--json` is available for status commands and `--dry-run`. If the task is exactly a subcommand name, separate it as in `orochi -- "status"`.

`status` lists all agents regardless of whether they have run history. It reflects the current installation, enabled/disabled state and cooldown, so even an Agent that succeeded in the past shows that state if it has since been removed or disabled. Plain `status` does not start any Agent, and authentication state is unchecked. `status --discover` prepares missing adapters and checks the connection, showing `connected` and the model list on success, or the reason on failure. The task prompt is not sent, and an Agent in cooldown waits before reconnecting. Saved quota and recent run results can also be checked afterwards.

Exit codes are `0` (completed / completed without verification), `1` (execution, evaluation or configuration error), `2` (CLI argument error) and `130` (interrupted / permission denied).

### Interactive mode

Starting `orochi` on a terminal without a task opens an interactive session. `orochi chat` does the same. The screen, controls and message flow follow the conventions of the [OpenHands CLI](https://docs.openhands.dev/openhands/usage/cli/terminal) ([source](https://github.com/OpenHands/OpenHands-CLI)) and [Claude Code](https://code.claude.com/docs/en/interactive-mode). Both of those are full-screen TUIs, whereas Orochi writes its output to the terminal sequentially.

```sh
orochi                              # start a conversation
orochi -c                           # continue the most recent conversation in this directory (--continue)
orochi --resume '<session ID>'      # continue the conversation from a recorded session
orochi --agent claude               # Agent for the first message and for rerouting
orochi --yolo                       # approve every permission request once each time (same as --always-approve and --permission allow)
```

```text
> Update the notes
  ⎿ codex · gpt-5.6-luna · medium            the chosen Agent (colored per Provider); shown only when the route changes
✻ Inspecting the fixture                    thinking summary (dim italics)

⏺ I'll list the files first.                the Agent's answer; text appears as it arrives and is formatted once a line is complete

⏺ List fixture files: $ ls ✓                while running: a grey line with a spinner; when done the line turns into a green ✓ / red ✗
  ⎿ a.txt … +1 lines                        first line of the result and the number of remaining lines

⏺ Edit notes: notes.txt ✓
  ⎿ Updated notes.txt with 1 addition and 1 removal
      - old line                            diff (red / green)
      + new line

⏺ Plan                                      plan (☒ done; ☐ in progress is highlighted)
  ⎿ ☒ Inspect files
    ☐ Report back

✻ 12s · codex · gpt-5.6-luna · ↑ 10.2k ↓ 1.1k · cache 40%
```

- **Output always keeps flowing upward.** Even when the input box grows or shrinks across multiple lines, or you type the next message during a run, earlier output is never redrawn or overwritten (pinned by a test that reproduces the screen in a pseudo-terminal).
- **The input box is always pinned to the bottom of the screen.** History flows above it and also stays in the terminal's scrollback. The status row below the input box shows the approval mode, the number of queued messages and the work status (`⠹ Working (12s) · esc to interrupt`).
- **You can keep typing during a run.** Pressing Enter puts the message in the queue (`⏸ queued (1): …`), and queued messages are processed in order as soon as the running work finishes. Esc stops the running work and the queue is still sent next; ↑ takes everything queued back into the input, one per line, to edit or drop — as in Claude Code. Unlike Claude Code, a queued message always waits for the next turn: it is not handed to the agent in the middle of the current one.
- **Shift+Tab switches the approval mode.** If the agent provides session modes over ACP (e.g. `default` / `acceptEdits` / `plan` for Claude Agent ACP, `read-only` / `auto` for Codex ACP), it cycles through those; otherwise it cycles through Orochi's own `ask` / `always` / `never`. `/confirm` also changes it.
- **Files and images can be attached.** Paste a path (drag & drop), or write a path after `@` and complete it with Tab; an `[Image #1]` / `[File #1]` tag is inserted in the input box and a list appears below it. Images are sent as ACP `image` blocks and text as `resource` blocks. If the agent does not support them, or the file exceeds 8 MiB, the file's location is sent instead (`resource_link`).
- **Requests are classified by an agent, not by keywords.** Before routing, Orochi asks an agent that would run the work anyway what kind of task it is and how hard, and shows `⎿ classified as bug_fix / normal`. The answer can only raise the difficulty, never lower it, and if no agent answers in time Orochi keeps its local keyword profile. The classifier is the one adviser that sees the request text; none of it is stored (its cache is keyed by a salted hash). `[classifier] enabled = false` turns it off. Asking is a whole extra session and costs like one (measured on 2026-09-20: 37,630 tokens on Claude against 23,800 on Codex for the same question, almost all of it the session's fixed cost), so what each answer cost is recorded: the cheapest answerer is asked first, and once Orochi has measured both what asking costs and what work like this costs, it stops asking where the question would take more than `classifier.max_cost_share` of the work — the keyword profile stands instead, and the checks still decide whether the result was right. Automated tests only; its accuracy with real agents is unverified.
- **Work the design divides asks once, then runs as a team.** When the design step ends by dividing the work into parts that can run side by side, Orochi shows the order and asks, the way Claude Code asks to approve a plan:

  ```text
   ◆ This divides into parts that can run side by side
     alpha ‖ beta → gamma
     2 implementer(s), 3 reviewer(s), integrator
     each part works in its own copy; a verified result is merged back

   ❯ 1. Yes, run the parts side by side
     2. Yes, one agent carries it out here
     3. No, keep the design (esc)
  ```

  `1` runs the parts as a team, `2` carries on with the implementation step in your working tree, and `3` or Esc builds nothing: the design stays in the conversation, and your next message carries on from it. It asks only on a terminal, and `/solo` and `/team` turns are never escalated. While the team runs, Esc or Ctrl-C stops it; the report stays in `<data>/collaborations/<id>/`, so it can be resumed with `orochi collaborate-resume --output <path>`.
- **It remembers, across sessions and across agents.** When you state something that should hold beyond the current task ("keep diffs small", "this project uses pnpm"), the classifier picks it up in the same call (`⎿ remembered: keep diffs small`), and from then on it opens every new agent session, whichever agent is chosen. A session with two or more messages is looked back over once when you quit, for preferences that only show across messages (`looking back over this session · Esc skips`). Notes are plain Markdown in `<data>/memory/`: one file per repository, plus `USER.md` for what you say holds for every project. Lines you write yourself never expire; a note Orochi heard once expires after 90 days, one heard again after 180. `/memory` lists them, and `/memory forget r2` removes one. A `MEMORY.md` inside the repository is never read, no agent can write memory, and routing advisers never see it. Automated tests only.
- **Say which agent you want for a kind of work.** "Use Fable for design", said once or written into memory, makes matching candidates cheaper for that kind of work: `⎿ classified as architecture / complex · you prefer fable`. It only tips a close call (cost × 0.8, Orochi's own heuristic): a candidate that fails a capability, quota or success-floor gate is never brought back. It matches a name fragment, so it survives model updates.
- **Orochi decides the sequence of steps.** Each message is classified: a small request runs as-is in a single pass, while one involving design changes or of large scale is split into "design → implement (→ review)", and an Agent and model are re-chosen for each step. This results in a strong reasoning model for design and a fast model for implementation. Each step's answer is handed on to the next step, and follow-up instructions continue in the session that did the implementation. The decision is shown as, e.g., `⎿ complex task · design → implement`.
  - If the implementation did not finish normally, a review is added, and only if the review answers `VERDICT: fix` is one more fix step added — just once.
  - `/team <task>` forces the three steps, and `/solo <task>` forces a single run.
- **Ask for a number of agents, and that many are started in the same process.** A request like 「5人くらいのエージェントでディスカッションして」 ("have about five agents discuss this") makes Orochi **start that many Agent sessions concurrently within one process** (up to 6 seats). The seats are assigned **different perspectives**, such as `facilitator` (moderator) + `skeptic` / `architect` / `simplifier` / `operator` / `advocate`. For a discussion-only request, **every seat is read-only**, and the repository is not changed. Orochi does not have an agent run the `orochi` command to launch a separate process (this is also explicitly forbidden in the prompt).
- **A large request gets a second seat beside it.** For a request judged to involve design changes, to be large in scale, or to be long-running, Orochi starts **another Agent session concurrently in the same process** beside the worker. The seat's name and role are determined from the task (`architect` for a request that changes structure, `researcher` for an ambiguous request, `reviewer` otherwise; the worker likewise becomes e.g. `implementer` / `fixer` / `migrator`). The second seat **only reads**: Orochi refuses tools that change files, so there remains a single working tree and no conflicts. The two talk over the mailbox (the instructions Orochi adds are in English, but it tells them to **write both their answers and their messages to each other in the same language as the request**), and that conversation flows straight into the chat display (which also shows which Agent and model occupies each seat, as in `⎿ reviewer test · astra-test`). **Each seat is assigned a different Agent and model.** An (Agent, model) combination used by another seat is removed from the candidates (the same one is used only when there is no other choice), so the same model is not lined up twice. When the worker finishes, the second seat ends too. When **the request itself asks for collaboration**, as in 「エージェント同士で相談しながらやって」 ("do it with the agents consulting each other"), it gets 2 seats regardless of scale. **Seats last for that turn only**, so asking to continue a discussion re-seats the same number of agents (the previous turn's content is carried over). `/solo <task>` fixes it to one agent.
- **When the conversation outgrows its model, it chooses again.** A conversation continues with the same Agent and model, but if a request becomes heavy enough that **that model would not have been chosen in the first place** (it does not reach `scheduler.required_success`), the pin is lifted for that turn only and the route is chosen again (`⎿ this needs more than the model in this conversation · picking again`). Design changes and large requests are split into steps, so they are re-chosen there as well.
- **When a strong model is avoided because of usage limits, it says so.** A model with low remaining quota gets a higher selection cost, so it may give up its place as first choice. In that case one line is left, such as `⎿ fable is close to its limit here; using sonnet instead`.
- **A missing capability is handed to an Agent that has it.** If image generation (`image`), browser operation (`browser`) or web search (`web`) becomes necessary mid-conversation and the Agent in charge does not support it, an Agent that does runs that one request only, and the conversation itself returns to the original Agent (the result is passed on to the original Agent with the next request). Capabilities are declared with `image` / `browser` / `web` in `[[agents]]`. Even when an Agent is specified with `--agent`, this hand-off takes precedence when a capability is missing.
- **Agents running in the same repository share resources.** An Agent (account) in use by another participant (not only Orochi in a separate process, but also another seat in the same turn) has its selection cost multiplied by 1.4, lowering its priority. If there is no other choice, it is used anyway. The instructions to agents also state explicitly that "when you need the same file, account or tool, consult first and decide the order".
- Exchanges with other Orochi instances running in the same repository (in separate processes or separate worktrees) are shown in chat form. **Participants are per Agent session**, so multiple Agents driven by one Orochi (the steps of `/team`, or the roles of `collaborate`) also know about each other and can send messages. Only sessions that actually started work are registered as participants; sessions opened to retrieve the model list are not.
- It shows **which Agent and which model** each participant is using. You can see this in a message's header (`✉ backend (codex · gpt-5.6-luna) → frontend`), in the `/peers` list, and in the result of `list_peers` (`agent` / `model`).

```text
● backend joined · ~/dev/app-worktree · feature/api · codex / gpt-5.6-luna
✉ frontend (this session) → backend  12:03
  ▎ The list endpoint now returns 201 for creates.
✉ backend → frontend (this session)  12:03
  ▎ Understood, I will update the client.
○ backend left
```

  Each peer gets its own color, and your own session is shown in the brand color. Joins and departures are shown as well. Messages that arrive while you are typing are shown together right after you send, so the input line is not broken. When your own Agent uses a mailbox tool, a status such as "Messaging another agent" is shown instead of a tool line. `/peers` lists the Agents currently running. Nothing is shown when `[mailbox] enabled = false`.
- Colors are fixed per role. The logo, prompt, spinner and the in-progress item of a plan use the brand color; Agent answers are blue, success green, failure red, warnings and permission requests yellow, and supplementary information grey. Markdown headings, bold, `code`, code blocks and bullet lists are colored too. No color is used when `NO_COLOR` is set or when output is not a terminal.
- **The default for permissions is automatic approval (auto).** By default, interactive mode automatically answers an Agent's permission request with a one-time approval. To stop and have you confirm, use `--permission ask` or `/confirm ask`; to deny everything, `/confirm never` (if the configuration sets `scheduler.permission` explicitly to `allow` / `deny`, that is used instead. The default for a one-shot run `orochi "task"` remains `ask` as before).
- When confirmation is needed, **the input box is replaced in place by the confirmation screen**, with Claude Code's choices: `1 Yes` / `2 Yes, and don't ask again this session` / `3 No, and say what to do instead (esc)`, by number key (`y` / `a` / `n` also work) or ↑↓ and Enter. Once you answer, it returns to the input box, and only one line, `⏺ <tool name> → allowed once`, remains in the history.
- **Orochi's own mailbox tools (`orochi-mailbox`) are not confirmed.** They only contact other Agents and touch neither files nor commands. The exchanges appear in the chat display. If no other Agent is running, `read_messages` returns immediately without waiting.
- If you deny, the work for that message stops, as Claude Code stops the turn on a No without a comment: the agent is cancelled rather than left to find another way to do the same thing. Give a different approach in the next message.
- Agents that are not installed are not shown. Only Agents that are actually unusable, e.g. due to an authentication failure, are warned about, once per session. When a failure has no known kind, the warning includes the cause the agent reported.

Differences from a normal run:

- Input is sent as-is. The normal run's preamble "verify it as a coding task" is not added.
- The Agent and model are chosen on the first message, and from the second message on, that session is loaded with `session/load` and continued. The pinned versions Codex ACP 1.10.0 and Claude Agent ACP 0.77.0 advertise `loadSession` in their source. For an Agent that does not support `session/load`, the conversation so far (up to about 32 KiB, in memory) is passed as context to a new session with the same Agent and model.
- Checks (evaluation) run only if files changed during that message. In a git repository this is determined from the files git tracks; elsewhere, from the size and modification time of files excluding `.git`, `node_modules` and `target`. A message with no changes is recorded as unverified (`partial_success`) and never counts as a verified result. What you do next labels it weakly instead: carrying on after reading the answer counts slightly for its route (0.2), and `/reroute` counts against it (0.3). Stopping a turn with Esc labels nothing by itself; only a `/reroute` after it does. These weights are Orochi's own heuristic, and `orochi calibrate` reports them apart from verified outcomes.
- If a check fails after the Agent has finished its answer, the work is not handed over to another Agent automatically. The result is shown and it returns to waiting for input. An error in the middle of an answer, such as a usage limit, switches to another candidate as usual.
- While a conversation continues, the Agent and model are pinned, so it does not switch to another Agent automatically even on a usage limit or similar. `/reroute` reroutes the next message while still passing the conversation so far as context.
- The repository lock is held only while a message is running.
- Input history and the conversation are kept in memory only and are not saved to disk. As with a normal run, only run records without the text and session IDs are saved to SQLite.

| Input | Action |
|---|---|
| `/help` | List of commands and shortcuts |
| `/new` (`/clear`) | Discard the conversation and route the next message afresh |
| `/resume` (`/history`) | List recent conversations in this directory. `/resume <number or ID>` resumes one |
| `/reroute` | Reroute the next message while passing the conversation as context |
| `/confirm` (`/permissions`) | Show or change the policy for answering permission requests (ask / always / never) |
| `/team <task>` | Run design → implement → review in order, choosing a different Agent for each step (`/collaborate` is the same) |
| `/peers` | Other Agents running in the same repository, with their working directory, branch, route and status |
| `/status` | Working directory, the continuing Agent, model and session, and the answering policy |
| `/memory` | What Orochi remembers about you and this repository. `/memory forget <id>` removes one item |
| `/exit` (`/quit`) | Quit |
| Typing `/` | Matching commands are listed above the input; ↑↓ select one, Tab or Enter takes it |
| Tab while typing `@` | Complete a file name and attach it |
| Shift+Tab | Switch the approval mode (auto → ask → never) |
| ↑↓ | Input history for this session (while command candidates are listed, ↑↓ moves through them; while messages are queued during a run, ↑ takes them back into the input) |
| `\` at end of line + Enter | Insert a newline and keep typing. A multi-line paste becomes one message |
| Esc during a run | Stop that work (including the request's classification). What you typed stays in the input, and queued messages are sent next |
| Esc twice at the prompt | Clear what you typed; ↑ brings it back. Esc never quits |
| Ctrl-C | During a run, stop it (what you typed stays). Otherwise clear the input; a second press quits |
| Ctrl-D | Delete the character after the cursor; on an empty line, a second press within 0.8 s quits |
| Esc in a question | Close it: No on a permission request, keep the design on the team question |

These keys follow Claude Code ([interactive mode](https://code.claude.com/docs/en/interactive-mode), [permissions](https://code.claude.com/docs/en/permissions)); where OpenHands CLI differs — its Ctrl-C asks to quit instead of stopping the run, and its agent carries on after a rejected action — Orochi follows Claude Code. Keys typed during a run edit the input line as usual, and Enter queues. When stdin is not a terminal, `orochi` without a task shows help as before. `orochi chat` processes each line of standard input as one message. In that case output is produced without decoration, thinking or in-progress lines, permission requests are denied, and input lines are not used as answers to permission requests.

Line editing and screen control are implemented in-house (`src/chat/term.rs`). The bottom rows are pinned with the terminal's scroll region (DECSTBM), and input reads keys directly in raw mode. It does not switch to a full screen (alternate screen), so history stays in the terminal's scrollback. On exit, the scroll region, bracketed paste and termios are restored. Even when several characters arrive at once, as when committing Japanese input, none are dropped.

With a mock ACP fixture and a pseudo-terminal (pty), the following were confirmed: the input box pinned to the bottom, typing and queueing during a run, Shift+Tab switching, image and file attachments, incremental display and formatting, tool line updates, diffs, plans, arrow-key operation of the permission panel, display of inter-agent messages (both while running and while idle), `/peers`, session continuation, `/reroute`, `/new`, interruption with Esc, Tab completion, Japanese input, and narrow terminals. With the real Claude / Codex, one conversation was held on the version before the change, and a problem was confirmed where a check failure was handed over to another Agent. Verification of the version after the change against the real CLI has not been done yet. OpenHands' command palette, the conversation-history and plan side panels, output folding, and Claude Code's input frame and bottom status line are not implemented in the sequential output approach.

### Permissions

The default is `ask`. ACP `session/request_permission` is shown, and an approval selects `allow_once`. Without a TTY, requests are denied.

```sh
orochi --permission allow "..."  # automatically answer ACP one-time permission requests
orochi --permission deny "..."
```

Orochi is not an OS sandbox. The Agent's own tools, authentication, sandbox and approval settings also apply. Operations for which the Agent does not ask permission over ACP cannot be mediated by Orochi. Orochi does not advertise client-side filesystem/terminal capabilities, and uses the Agent's native tools.

### Configuration

The default configuration file is `$XDG_CONFIG_HOME/orochi/config.toml`, or `~/.config/orochi/config.toml` when that is unset.

```sh
orochi config path
orochi config init
orochi config show
orochi --config /path/to/config.toml --data-dir /path/to/data "..."
```

`OROCHI_CONFIG` / `OROCHI_DATA_DIR` can also change these. Configuration inside a repository is never loaded automatically as run configuration. A sample is in [examples/config.toml](examples/config.toml).

```toml
[[agents]]
id = "codex"
provider = "openai"
command = "codex-acp"
args = []
enabled = true
browser = false
web = false

[scheduler]
required_success = 0.7
routing_confidence = 0.8
max_attempts = 3
discovery_timeout_secs = 30
prompt_timeout_secs = 1800
permission = "ask"
```

```toml
[classifier]
enabled = true        # default; false keeps the local keyword profile only
# agent = "claude"    # default: whichever agent has answered most cheaply here, within one agent's worth of time
max_cost_share = 0.25 # skip asking when it would cost more than this share of what work like this costs

[memory]
enabled = true        # default
user_chars = 1500     # budget for USER.md in each new agent session
repo_chars = 2500     # budget for this repository's notes
```

Specifying `agents` replaces the default 4 entries. Multiple custom IDs can be registered. If an Agent can use a browser or web search, declare `browser` / `web` explicitly. These two items are not inferred from ACP's standard capabilities alone.

### Adviser (Router)

With no additional configuration, Orochi runs with local deterministic routing. To consult another agent about the choice of candidate, specify an `[[agents]]` ID and model. The adviser connects over ACP, and authentication follows each CLI.

```toml
[router]
agent = "codex"
model = "gpt-5.6-luna"   # if omitted, a model suited to advising is chosen automatically by Policy
session_overhead_tokens = 24000
```

It is consulted only when confidence is low and there are multiple candidates. It runs in a session in a new temporary directory, with permissions denied. Task text, file names and repository paths are not sent; only classification attributes and the top 12 candidates are sent. Unknown candidate IDs, failed responses and oversized answers are not adopted. A recommendation exceeding 1.25× the expected cost of the best candidate is not adopted either. An agent used only as an adviser is set to `routing_only = true`. For details on the Frontier Judge, the Council and the configuration items, see [Adaptive Routing and the ACP Gateway](docs/adaptive-routing.md#4-router-frontier-judge-and-council).

### Evaluation

The default automatic evaluation is as follows.

- Rust: `cargo test --quiet`
- Go: `go test ./...`
- Node.js: the existing `test` / `typecheck` / `lint` / `build` scripts, run with the package manager matching the lockfile
- Git repository: `git diff --check` for both staged and unstaged changes

These run with `CI=true` and no stdin. Explicitly configured checks replace auto-detection. For Python and others, configure as follows.

```toml
[evaluator]
auto = false
timeout_secs = 300

[[evaluator.checks]]
name = "tests"
command = "python3"
args = ["-m", "pytest", "-q"]
```

Commands are not expanded into a shell string; the executable and arguments are passed separately. `--no-eval` skips evaluation. An Agent's `end_turn` alone does not produce a success label. No verification / diff-only verification is recorded as `partial_success`, a substantive check pass as `success`, and failure or incompletion as `failure`. Passing checks does not guarantee that the requirements were fully met in meaning.

## Policy and cost model

The JSON files in [policies/](policies/) are bundled. Reasoning guidance based on official documentation and Orochi's own performance estimates are handled separately.

**`success_prior`, `relative_tokens` and the cache discount rates are Orochi's own heuristic, not calibrated against benchmarks. They are not success rates or prices published by the Provider.** Patterns are used only to apply priors to model IDs obtained over ACP; they never generate model IDs.

Roughly, the score is `(expected tokens × cache adjustment + context restoration) × quota factor + latency`, divided by the estimated success probability. With the initial prior weighted at 16, it is blended with local evaluation history through a per-context EWMA. Candidates below the lower bound on success probability, violating a hard constraint, or in cooldown are excluded. Optional Bandit exploration is likewise limited to candidates that passed these constraints. `learning.tier_pooling` (off by default) starts a model that has no history of its own from what other models of the same provider and tier measured on the same kind of work, so measurements survive a model being replaced; turn it on after `orochi benchmark-tune` shows it helps on your data. `orochi calibrate` shows prediction error, and `orochi benchmark --input ...` shows a comparison of measured candidates.

```sh
orochi policy update                          # install the bundled version
orochi policy update --from ./policies        # read the 3 JSON files
orochi policy update --from ./registry.json   # read a whole registry
orochi policy update --url https://example.org/registry.json --sha256 '<64-digit digest>'
```

The registry format is `{"schema_version":1,"policies":[...]}`. All entries are validated before being replaced atomically, and the existing version is kept if validation fails. Remote updates require HTTPS and an explicit digest. Automatic scraping of official documentation, a signed distribution service, and the update server itself are not implemented. Choosing the update source is up to the user.

Policy sources:

- [OpenAI model guidance](https://developers.openai.com/api/docs/guides/latest-model)
- [Anthropic thinking / effort](https://platform.claude.com/docs/en/build-with-claude/thinking-steering-and-cost)
- [Google thinking](https://ai.google.dev/gemini-api/docs/thinking), [context caching](https://ai.google.dev/gemini-api/docs/caching)

## Quota, usage and telemetry

Data is saved to `$XDG_DATA_HOME/orochi`, or `~/.local/share/orochi` when that is unset.

- `telemetry.sqlite3`: runs, sessions, runtime, quota_snapshots, the schema version and a local salt (migrated to schema v2 in this release)
- `policies.json`: the installed Policy
- `locks/`: locks for same-repository runs
- `adapters/`: auto-installed ACP adapters and the npm cache

Task text, conversations, source code, diff bodies, Agent stderr and test output are not saved to the DB. The repository identifier is a hash using a local salt. The TaskEnvelope is held only in memory during a run. Conversations saved by the Agent itself and records on the Provider side are a separate matter.

Common authentication / rate-limit errors, structured `resetAt` / `reset_at` / `resetsAt` (Unix seconds) and `retryAfter` / `retry_after` (seconds) are observed. Unless `data.scope = "model"` is explicit, a rate limit applies to the whole Agent. Non-standardized subscription quota is never guessed and shown as healthy.

If an adapter can provide quota, it can use the following extension in a response or session update.

```json
{"_meta":{"orochi.dev/quota":{"remaining":0.08,"reset_at":2000000000,"model":null}}}
```

`remaining` is 0–1, and `model: null` means the whole Agent. Private, undocumented quota APIs internal to a Provider are not called. `orochi quota --refresh` retrieves quota directly from the official Codex CLI. For Claude, on-demand retrieval via the official `/usage` (Python 3 / POSIX) and official statusline input via `quota-ingest` are supported. Retrieval via `agy`'s official `/usage` is also implemented for Antigravity, but parsing of the authenticated display is unverified. See [sources, expiry and constraints](docs/adaptive-routing.md#3-quota).

Usage is taken from the draft `usage` in the response (camelCase) or from `_meta["orochi.dev/usage"]` (per turn). OpenAI-compatible and Anthropic formats are normalized, and reasoning/cache are not double-counted. A missing value is `null`, neither zero nor estimated tokens. Because this depends on the draft usage semantics and the adapter's reporting accuracy, a strict comparison of measured values requires checking on the adapter side.

Cache affinity is estimated from recent sessions with the same repository, configuration, instruction files, Agent, model, reasoning and mode. If ACP does not expose a cache key or adaptive thinking settings, Provider-specific parameters are not sent on Orochi's own initiative; this is left to the Agent.

## Development and verification

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The E2E tests start Python 3 ACP fixtures as real child processes. The adviser is also verified with an ACP fixture. No real account, external LLM or API billing is needed. CI targets Linux/macOS. Verification on real Windows machines and bulk termination of descendant processes on Windows are not supported.

[examples/compare_live.py](examples/compare_live.py) runs the tasks of [examples/compare-suite.json](examples/compare-suite.json) three ways — each CLI alone as its owner configured it, and Orochi with both — scoring every arm by a hidden check and charging it every token its runs recorded, the classifier's included. `--validate` checks the suite without contacting an agent; `--execute` consumes quota.

[examples/demo.py](examples/demo.py) lets you try the whole flow without an account.

```sh
cargo build --locked
python3 examples/demo.py
```

The tests verify against the ACP contract. Compatibility with each real Provider in a logged-in environment should be checked separately, together with the adapter version you use.

[CLI Auto-Discovery: Live E2E](docs/cli-discovery-e2e.md): candidate registration for both Claude/Codex and delegation of execution to Claude have been confirmed.

## Layout

```text
src/acp.rs          ACP connection, sessions, dynamic configuration, streams, permissions
src/discovery.rs    CLI detection, resolving the ACP connection method, preparing missing adapters
src/agents.rs       Per-Provider error / quota / usage normalization
src/router/         Task Profiler and task classification over ACP, candidate generation and scoring, Router / Judge / Council via ACP advisers
src/scheduler/      Execution, fallback, quota / circuit breaker
src/process.rs      Agent supervisor processes, recording descendants, reclaiming after a forced kill
src/mailbox.rs      Inter-process messages between agents (MCP server, retention-limited storage)
src/interrupt.rs    One process-wide count of interrupts, so work between two waits still hears Esc
src/memory.rs       Memory across sessions (user preferences, repository notes; stored separately from telemetry)
src/context.rs      TaskEnvelope, Git information, cache key
src/evaluator.rs    Local evaluation and process management
src/storage.rs      SQLite, schema, repository lock
src/learning.rs     EWMA, Bandit, prediction calibration
src/benchmark.rs    Time-series comparison of measured candidates, holdout coefficient search
src/collaboration/  Steps of independent ACP sessions, team composition from the task, the order of the work (parts in waves, graph.rs), merging parallel implementations, messages, applying to the working tree
src/quota_terminal.py Read-only quota retrieval from native CLIs
src/quota_sources.rs CLI quota retrieval, statusline ingestion
src/gateway.rs      Exposing Orochi over ACP v1 stdio
src/chat/           Interactive mode (screen display, queueing, approval modes, attachments, session continuation)
src/chat/term.rs    Terminal control (input box pinned to the bottom, key input, scroll region)
src/policy.rs       Policy validation and updates
src/config.rs       Configuration, default Agent presets
src/cli.rs          User-facing commands
```

ACP specification references: [Rust SDK](https://github.com/agentclientprotocol/rust-sdk), [Session Config Options](https://agentclientprotocol.com/protocol/v1/session-config-options).

## License

MIT
