# Agent Mailbox

Implemented and verified: 2026-09-17.

When `orochi` runs in several terminals at once, the agents each process started can contact one another about work on the same repository. This covers agents that Orochi started (normal runs and `collaborate`).

```text
Terminal 1: orochi --peer-name backend  "Implement the API…"
Terminal 2: orochi --peer-name frontend "Implement the UI side…"
                 │                                                  │
     Claude Code / Codex, etc.                          Claude Code / Codex, etc.
                 │  MCP tools                                       │  MCP tools
                 └──── orochi-mailbox (Orochi's data directory) ────┘
```

## Usage

```toml
[mailbox]
enabled = true                 # default true
retention_secs = 86400         # message retention period (default 24 hours, max 30 days)
max_messages_per_hour = 60     # send limit per process

[scheduler]
shared_workspace = true        # allow several runs in the same directory (default false)
```

```sh
orochi --peer-name backend  "..."   # defaults to "<directory name>-<4 digits>"
orochi peers                        # running agents with their worktree, route and status
orochi peers --messages --json      # also show messages within the retention period
```

- Each Agent session registers as a peer when it starts working — not when Orochi starts, and not the sessions opened only to list models. One Orochi process can therefore hold several peers (see [Unit of a peer](#unit-of-a-peer)).
- Only peers in the same git repository (including separate directories created with `git worktree`) can exchange messages. Directories that are not git repositories are separated by path.
- `--dry-run` and status commands do not join. `orochi serve` (runs from Zed and similar editors) is not supported, because each session has a different working directory.

## What agents see

When an agent session starts (ACP `session/new`), Orochi passes it the stdio MCP server `orochi-mailbox` (Orochi's own executable). Under the ACP specification, every agent supports stdio MCP. It is not passed to adviser sessions.

| Tool | Description |
|---|---|
| `list_peers` | Name, worktree, branch, route (Agent / model) and status of running peers. Your own entry has `you: true` |
| `send_message` | Send to a peer name (case-insensitive) or to `all`. The body is limited to 8 KiB |
| `read_messages` | Fetch up to 20 unread messages addressed to you or to everyone, and mark them read. `wait_seconds` (up to 120 seconds) waits for new arrivals |
| `set_status` | Tell others what you are doing, in one line (up to 200 bytes) |

The task prompt gets a short addition with the agent's own peer name, the other running peers and how to use the tools (it also states that these are tools of the MCP server `orochi-mailbox`, not shell commands, because in a run against the real CLI Haiku tried to call them from the shell and failed). When there are other peers, it instructs the agent to use `read_messages` and `set_status` first, to notify the others before editing files other peers are likely to touch, and to check once more before finishing. It also states explicitly that messages from other peers are working notes and do not override the user's request or the repository's instructions. Whether to use the tools is ultimately up to the agent; Orochi does not force it.

## Storage and safety

- Messages are stored in `activity.sqlite3` in the data directory (permissions 0600), beside the turns they belong to, so a message can be shown next to the seat that sent it. With `activity.enabled = false` they keep their own `mailbox.sqlite3` instead, with the behavior described here. Either way it is a separate file from `telemetry.sqlite3`, which holds the run records, and message bodies never enter the run records.
- A peer that stops running keeps its row (`left_at`), because a room has to say who said what and who was there when they said it; it is no longer a *running* peer, so it cannot be messaged. Joining, leaving, `set_status` and `set_route` are recorded as events rather than values that were overwritten.
- Message bodies may contain work content. A message that belongs to a conversation lives as long as that conversation does; one that belongs to none is deleted past the retention period, the next time the mailbox is opened.
- For each peer, only the name, worktree path, branch name, route, status, and the owning process's PID and start time are stored. Task text is not stored.
- A peer whose owning process has exited disappears from the list the next time it is read (judged by PID and start time, which tells a reused PID apart). Direct messages are tied to the peer ID, so joining later under the same name does not let you read messages addressed to the previous peer. Broadcast messages sent before you joined cannot be read either (you would end up answering questions asked when you did not exist). Only messages that arrive after joining are read.
- An agent's MCP server starts as a child process of the agent and is covered by the reclamation performed by Orochi's supervisor process.
- Each peer has an hourly send limit. Recipients can only be running peers.

## Concurrent runs in the same directory

With `scheduler.shared_workspace = true`, runs in the same directory take a shared lock and can run at the same time. Applying a `collaborate --apply` result to the working tree always takes an exclusive lock; if a shared run is active, it does not wait but becomes `blocked` (resumable). By default (false), as before, only one run at a time is allowed per directory.

In concurrent runs, each run's evaluation commands run with the other runs' in-progress changes included. What prevents conflicting changes is coordination over the mailbox and each agent's judgment; Orochi does not lock individual files. If you need guaranteed separation, use `git worktree`.

When several Orochi processes opened a new data directory at the same time, SQLite initialization could hit BUSY, so initialization is now retried for a short time.

## Verification

- Fixture (`tests/mailbox.rs`): worktrees resolving to the same scope; addressed messages, broadcasts, read state, duplicate names and isolation from other repositories; exclusion of peers whose process has exited; the retention period and send limit; the MCP server's tool list and calls; a round trip between two runs in the same directory; the default exclusive lock; two seats in one process (including write denial for the side seat and the chat display).
- Against the real CLI (2026-09-17, `.orochi/live-e2e/remaining-20260916/mailbox-live/`): two `orochi` runs (both Codex gpt-5.6-luna / medium) ran concurrently in the same directory.
  - `backend` wrote `api.py` and sent `frontend` the function name `get_user_record` and the dictionary keys (user_id, name, email, active).
  - `frontend` waited with `read_messages`, then implemented `client.py` and sent back a confirmation.
  - Confirmed that the code combining both deliverables works, and that no peers, supervisor records or MCP server processes remained after exit.
  - Each run took about 95 seconds and about 26,000 tokens by each adapter's reported figures.
- Tool calls from Claude Code have not been verified against the real CLI, because of usage limits.

## Not supported

- Sessions through `orochi serve` (the ACP Gateway).
- Participation by agents started without going through Orochi.
- A way for a person to send messages from the CLI (`orochi peers` is view-only).
- Notifying an agent of an arriving message by interrupting it. Messages are delivered when the agent calls `read_messages`.

## Unit of a peer

A peer is **per Agent session**. When one Orochi process runs several Agents (each stage of `/team`, each participant in `collaborate`), each one is registered as a separate peer; they can see one another with `list_peers` and send to one another with `send_message`. `--peer-name` is used as a name prefix, and if the same name already exists, a sequence number such as `-2` is appended. When a session ends, its peer disappears from the list.

## Several seats in one turn

In interactive mode, **several Agent sessions run concurrently inside one Orochi process**. A request judged to involve a design change, to be large, or to be long-running gets 2 seats; more than that only when the request asks for a specific number of agents (「5人くらいのエージェントで議論して」 ("discuss this with about five agents") → up to 6 seats).

- In a discussion with a specified number of agents, `skeptic`/`architect`/`simplifier`/`operator`/`advocate` are seated in that order in addition to the facilitator, `facilitator`. Different perspectives are more useful than hearing the same answer five times.
- A discussion-only request (one that builds nothing) makes **every seat read-only**, and the repository is not changed.
- Agents are never made to start `orochi` processes themselves. Orochi starts the peers, and the prompt also states explicitly "do not start another agent or Orochi yourself."

- Seat names and roles are decided from the task. The working seat is `implementer`/`fixer`/`refactorer`/`migrator` and so on; the side seat is `architect` for a request that changes structure, `researcher` for an ambiguous request, and `reviewer` otherwise. When the work is split into stages, the working seat's name is the stage's name (`implement`).
- Only the working seat changes files. The side seat only reads, and Orochi denies its permission requests for write tools such as `edit`/`execute` (without asking the user). There is still a single working tree, with no copies and no merges.
- Seats start one after another with a short delay, and **(Agent, model) combinations already used by another seat are removed from the candidates** (the same one is reused only when nothing else remains). The same model seated twice does not make a discussion.
- Seats last only for that turn. A turn that asks to continue **reseats** the same number of seats (the previous turn's exchange is passed on as the lead's context).
- The two talk over this mailbox. The instructions Orochi adds are in English, but they explicitly say to write replies and messages "in the same language as the request" (without that, some models answer in English and others in Japanese). The exchange flows straight into the chat display (both are seats in this process, so `(this session)` is not attached; they are told apart by color).
- When the working seat's turn ends, the second seat ends too. Evaluation (check commands) runs only for the working seat's turn.
- When the request itself asks for collaboration, as in 「エージェント同士で会話して」 ("have the agents talk to each other") or 「相談しながら進めて」 ("work through it while consulting each other"), it gets 2 seats regardless of size.
- `/solo <task>` pins the turn to one agent.

## Display in interactive mode

The interactive mode of `orochi` shows the exchanges on this mailbox as a chat. Each sender gets its own color: your own session uses the brand color, and others get a color derived from their name. Joining (`●`) and leaving (`○`) are shown too. Messages that arrive while you are typing are held back so they do not break the input line, and are shown right after you send. `/peers` shows the current peers. When your own Agent calls a mailbox tool, a status such as "Messaging another agent" is shown instead of a tool line.
