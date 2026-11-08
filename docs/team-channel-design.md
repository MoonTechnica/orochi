# The Conversation as a Team Channel: Design

Written: 2026-11-09. **Status (2026-11-09): C1–C5 implemented, automated tests only; C6 was run in part
([real-validation-20261109-2.md](real-validation-20261109-2.md)): real Claude and Codex leads
started a member, a real Codex helper reported in its thread, and the person's `@member` note
continued it with the lead summarising both reports; how real agents keep thread and channel
apart, whether a helper mentions `@you` by itself, and name reuse by a lead are unverified.**
§12 lists where the implementation departed from this design and what is not built. Everything in
§2 ("Where Orochi stands") was read from the code at `956f53a` and is cited.

One goal: **a conversation reads like a project team's Slack channel.** The user, the lead and
every agent the lead works with are members of it. The lead takes the user's requests and hands
parts of the work to the others; what is said about one piece of work is said in a thread under
the post that started it; what everyone needs to know is posted to the channel; anyone can
mention anyone, and when an agent needs the user it mentions them and the user is notified.

The user's words (2026-11-09): the main agent takes the user's instructions and hands tasks to
agents with roles; their conversation with it is all in the body today and would be clearer in
threads, like Slack's; what they most want is **the agent team talking in a Slack channel the
way a real project team does**. Decided with the user the same day: mentions, including the
user mentioning a member directly; members post to the channel only what everyone needs; an
agent reads the rest of the channel **when it needs to**, not by default; the lead's manner is
left to it; **an agent asking the user something mentions the user, and that mention notifies
them**. An earlier idea of one tab per agent was dropped for threads, because a tab has to be
closed and a thread need not be.

Naming: Orochi already calls a whole conversation a *thread* (`threads`, the sidebar's rows).
In this document and in code the conversation is the **channel**, a reply chain under a post is
a **thread**, and the agents are **members**. The window says "thread" and "N replies" as Slack
does; it never shows a conversation and a reply thread under the same word.

## 0. Requirements

| ID | Requirement |
|---|---|
| R1 | **The channel is a conversation among named members**: `you`, the lead, and each agent the lead starts or seats, each drawn with its face, name and route |
| R2 | **Work handed out is a thread.** The lead's assignment is a post in the channel; everything said about that work — questions, answers, progress, the report, the trial — is a reply under it |
| R3 | **Members post to the channel only what everyone needs**: done, blocked, a decision that affects others, a request to another member. Otherwise they reply in their thread. A reply may also be sent to the channel, as Slack's *also send to #channel* |
| R4 | **Anyone can mention anyone** with `@name`, the user included. A mention reaches the member mentioned, wherever it was posted. The user mentioning a member reaches that member directly; the lead sees it in the channel |
| R5 | **An agent reads what concerns it and nothing else unless it asks.** Delivered: mentions of it, `@channel`, and replies in threads it is part of. The rest of the channel is one tool call away |
| R6 | **An agent that needs the user mentions `@you`, and the user is notified** — OS notification, dock badge, sidebar mark, the host pane's state — until they have seen it |
| R7 | **Feedback to a member that exists keeps its thread**: a mention or reply to a running member is delivered to it; to one that has finished, it continues that member in the same thread |
| R8 | **Work logs are not posts.** Tool calls, thinking and diffs are folded under the post or reply they led to, as the lead's are today |
| R9 | **Nothing has to be closed.** Threads scroll away with the channel. A thread with something waiting on the user is marked on its parent and listed until settled |
| R10 | The desktop and `orochi threads` read the same views; telemetry, learning, gates, merge rules and approvals are unchanged |

## 1. Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | What a member is | **A name within one conversation** (`seats.member`): `start_agent`'s name for a helper, the role for a seat beside the lead (suffixed if taken). Every seat that ran under the name is the member's | A seat lives for one turn, so the same helper three turns running is three seats today (`implementer joined … left` three times in the user's screenshot). The name is what everyone already calls it |
| D2 | Who is the lead in the channel | **The lead's session, as `manager-design.md` D1.** Its replies to the user are top-level posts; its manner — when it delegates, whom it mentions — is left to it (the user, 2026-11-09) | The lead already plans and delegates; the channel changes what it can say and to whom, not who decides |
| D3 | What a post without a mention is for | **The user's goes to the lead, as a turn does today. A member's goes to the lead.** `@channel` reaches every member live in the conversation | In a team channel an unaddressed post is for whoever runs it; making it reach everyone would make every member pay for every post (R5) |
| D4 | What is delivered to an agent | **Pushed: mentions of it, `@channel`, replies in its own thread and in threads it has posted in.** The rest only through `read_channel` | The user, 2026-11-09: an agent reads the channel as it needs to. Every delivered line is read by a model and paid for; an agent knows better than Orochi when the rest of the room matters |
| D5 | Where the user's question lives | **A post or reply mentioning `@you`.** Permission and apply prompts (`prompts`) are drawn as such a post from the agent that asked, with their buttons | One way to be asked, and one way to be notified, whoever asks and whatever about |
| D6 | When a mention notifies | **On any post or reply mentioning `@you`, until the user has seen it** (`ui_state.last_seen_item` for the channel, its equivalent for a thread). Phrased as a question or not: the agent chose to mention the user | Detecting a question in text is a guess; a mention is the agent's own act |
| D7 | Reaching a member that has ended | **A mention of it, a reply in its thread, or `start_agent` with its name continues it**, in the same thread: a new seat under the member, resuming its ACP session where the agent can `session/load`, else told its own thread, bounded. Decided like any start, under the same gates; a continuation the user asked for is not held back by `worth_seating` | Today a taken name is suffixed (`Background::free_name`) and starts a stranger. `attempts.acp_session_id` keeps what a resume needs; a team the user asked for outright is never gated (`roles::worth_seating`) |
| D8 | Phases of the lead's own turn (design → implement → review) | **Stay the lead's**, in the channel as today, for now; revisited with the user after the real run (T6) | A phase's reply is handed to the next and the last answers the user; splitting them out is a separate decision |
| D9 | Peers of other conversations in the same repository | **Outside the channel**: the mailbox room stays repository-wide, and a message from another conversation's peer is drawn as a direct message from outside | The room is scoped to the repository's git common dir (`mailbox.rs`) so worktrees share it; a channel is one conversation |

## 2. Where Orochi stands

- **Seats are per turn** (`seats`, `src/activity.rs:2706`; `title`, `background`, `origin`,
  `result` in `ADDED`, `:2418`). The lead is ordinal 0, seats beside it 1… (`seat_rows`,
  `src/chat/mod.rs:2024`); a phase is the lead's seat with `phase` set. A helper's seat sits
  under the turn that asked for it and outlives it.
- **A helper is one-shot.** `start_agent(name, title, task, write)` (`src/mailbox.rs:1289`) is
  an `agent_requests` row the console decides; the helper reports once (`seats.result`) and
  ends. The same name again starts `name-2`.
- **The room is one-to-one or to all.** `send_message(to, body)` with `to` a running peer or
  `all` (`Mailbox::send`, `src/mailbox.rs:522`), `read_messages` returns unread messages
  addressed to the reader or to all. `messages(sender_name, recipient_name, via, body,
  thread_id)` (`src/activity.rs:2826`); `via='user'` is a note the user left (`View::say`). A
  message to a peer that has left is refused.
- **The lead's reply to the user is not a message**: it is the lead seat's `agent_message`
  items, and the user's request is a turn's `user_message`.
- **A completion turn can carry several helpers' reports** in one user message
  (`completion_message`, `src/chat/mod.rs:1448`); `turns.origin` is `background`, `merge` or
  `mail`.
- **The desktop** hides items with `lane > 0`, merges every room message into the body in time
  order and shows a completion turn as "from the helpers" (`drawTimeline`,
  `desktop/dist/app.js:1191`); it notifies, only while unfocused, on a new prompt and on a
  conversation that finished or failed (`notify`, `app.js:2482`).
- **What a resume needs is recorded**: `attempts.acp_session_id` (`src/activity.rs:2728`).

## 3. The channel, drawn

```
# greet-fix                      you · lead (claude) · implementer (opencode) · reviewer (codex)

you          Add tests for greet() with a number
lead         On it. @implementer add the tests; @reviewer look them over when they land
               ╰ 💬 implementer, lead · 4 replies · ● implementer working — python3 -m unittest
implementer  @lead Tests only, greet.py left alone?            (posted to the channel too)
lead         @implementer Tests only
implementer  Done: two tests in test_greet.py, 6/6 pass        (posted to the channel too)
               ╰ 💬 3 replies · worktree implementer, trial clean, not applied
reviewer     @implementer the float case's assert is weak, can you tighten it?
               ╰ 💬 2 replies
lead         @you Both are done and the trial is clean. Apply?   [Apply] [See the diff]   🔔
```

- A **post** is the user's message, the lead's reply, or a member's message to the channel.
- A **thread** opens under the post that started it; its parent carries a bar: who spoke in
  it, *N replies*, *last reply …*, and the member's state (working with what it is doing now,
  asking, reported, applied, patch kept, stopped, failed). Where something waits on the user
  the bar is marked and says what.
- **Work** (tool calls, thinking, diffs) is folded under the post or reply it led to.
- A completion turn is not drawn as a post: its reports are the members' replies in their
  threads, posted to the channel when the member chose to; the lead's reply to them is a post.
  A `merge` or `mail` turn likewise shows as the lead's reply only, its cause in the thread.
- joined / left / route / status are a thread's own quiet notes, never channel posts.
- A header control, *threads (N)*, lists threads that are running or waiting on the user.

Opening a thread replaces the right pane's tabs with it until closed with ×, as Slack's panel
does: the parent, every reply and every seat of the member with a divider where it was
continued, its worktree and trials as the Work pane draws one lane, and a composer — *Reply in
thread…*, *also send to the channel*.

### 3.1 The terminal console

A terminal only scrolls: a line once written is not rewritten. So the channel is the part that
scrolls and the threads are the part that is pinned, which `chat/term.rs` already redraws.

```
you          Add tests for greet() with a number
lead         On it. @implementer add the tests; @reviewer look them over
             └ 💬 implementer's thread                          ← an assignment: one line
implementer  Done: two tests in test_greet.py, 6/6 pass         ← only what was sent to the channel
reviewer     @implementer the float case's assert is weak
lead         @you Both are done and the trial is clean. Apply?  ← reverse video, one bell
──────────────────────────────────────────────────────────────
 💬 implementer ● working python3 -m unittest · 4 │ reviewer ✓ reported · 2   ← pinned thread row
> @im▏
   implementer  opencode · qwen3.8-27b  working                   ← the @ list
```

- **The transcript is the channel**: the user's messages, the lead's replies, an assignment as
  one line naming its thread, and members' posts to the channel. Thread replies are not written
  to it.
- **A pinned thread row** above the input lists the threads that are running or wait on the user,
  each with its state, what it is doing now and its reply count — the parent's bar, in the one
  place a terminal can update. It joins the status row and attachment rows `term.rs` already
  sizes, and follows its rule: rows it stops covering are freed and cleared.
- **Opening a thread** is the background-agent list that ↓ already opens at an idle prompt
  (`show_manager`, `Manage`, as in Claude Code), widened: one row per member, ended ones
  included; Enter shows that thread (its replies, reports and seats, the newest that fit);
  **what is typed while a thread is open is a reply in it**, with the input's prompt naming the
  thread; Esc returns to the channel. No new key is added.
- **`/thread <member>`** writes a thread into the transcript, to be scrolled back to later.
- **`@`** opens the member list where `/` opens the command list, with the same keys (§5).
- **`@you`** is drawn in reverse video and rings the bell once; inside an agent host's pane the
  console reports `blocked` (§6).

### 3.2 `orochi threads` from a shell

- `orochi threads show <id>` prints the channel, each thread folded to its parent line and bar;
  `--member <name>` prints one thread.
- `orochi threads send <id> "@implementer …"` reaches the member directly, since mentions are
  read from the text; `--thread <member>` sends it as a reply in that member's thread.
- `orochi threads mentions` lists the user's unseen mentions across conversations (`v_mentions`),
  for scripts and other notifiers; `--json` as every listing has.

## 4. How agents speak

The mailbox tools become channel tools. Names follow what an agent already knows from the
mailbox note; the old ones remain as aliases so a running session is never broken.

| Tool | What it does |
|---|---|
| `post(text, thread?, also_channel?)` | Posts to the channel, or replies in `thread` (a member's name; a member's own thread by default for a member, the channel for the lead). `@name`, `@you` and `@channel` in the text are mentions |
| `read_messages(wait_seconds?)` | Unchanged in shape; returns what is delivered (D4), each with its thread and mentions |
| `read_channel(since?, limit?)` | **New.** The channel's posts — not replies — the reader has not been delivered, newest last, bounded. What "as needed" calls |
| `read_thread(member, limit?)` | **New.** One thread's replies, for an agent mentioned into a thread it has not been in |
| `send_message(to, body)` | Alias: to the lead → a reply in the sender's thread; to a member → a post mentioning them; to `all` → a post with `@channel` |
| `start_agent`, `stop_agent`, the manager's tools | Unchanged, except `start_agent` with an existing member's name continues it (D7) or, for one still running, is refused with *implementer is running; mention it* |

`start_agent` writes the lead's assignment as the thread's parent post: `@name` followed by the
title and the task. The member's final reply (its report) is a reply in that thread, also sent
to the channel unless the lead's assignment asked it not to; it reaches the lead as today.

The mailbox prompt note (`mailbox::prompt_note_for`) gains the etiquette, in a few lines: talk
about your work in your thread; post to the channel only what others need; mention who you
mean; when you need the user, mention `@you` and say plainly what you need from them; read the
channel only when you need to.

## 5. How the user speaks

The composer is the channel's. A post with no mention, or mentioning the lead, is a turn for the
lead as today. A post mentioning a member is a `messages` row from the user with that mention,
shown in the channel:

- a running member reads it on its next `read_messages`;
- an ended member is continued (D7): the host's mail tick (the console's, without a host)
  finds a user mention of a member with no live peer and opens a continuation request,
  `requester = 'user'`, the post as its task;
- the lead is not woken by it, but its next session note carries one line per such post
  ("you told implementer: …"), so it can plan around it; the member's report comes back to
  the lead as any report does.

**Typing `@` offers the members** (the user's request, 2026-11-09), in the channel's composer
and a thread's alike: a list above the caret of the lead, every member of this conversation
and `@channel`, each with its face, name, route and state (working, asking, done — *done*
saying that mentioning it continues it), filtered by what follows the `@` as a prefix of the
name, the agent or the role. Running members come first, then by last activity, `@channel`
last. Up/Down move, Tab or Enter takes one and inserts `@name `, Esc closes the list and keeps
what was typed — the keys of the console's `/` list, which the terminal console reuses for
`@` with the same members from `v_members`. A taken mention is drawn as a chip in the draft;
the text sent is still `@name`, so a mention typed by hand without the list works the same.

Mentioning several members reaches each. A reply in a thread reaches that thread's member the
same way; *also send to the channel* additionally posts it. A user's post to a member writes no
`feedback` signal: it is not a verdict on the lead's route.

## 6. Notifying the user

A post or reply mentioning `@you` is a **mention**. While the user has not seen it:

- the desktop sends an OS notification — *implementer mentioned you*, the first line of the
  post — and clicking it opens the conversation at that post (its thread, for a reply); it
  notifies **whether or not the window is in front** when the conversation is not the one in
  view, since a mention is addressed to the user;
- the dock badge counts open prompts plus unseen mentions;
- the sidebar marks the conversation with `@`, above *unread*;
- inside an agent host's pane the console reports `blocked`, as it does for an open
  permission question (`pane.rs`);
- the console rings the terminal bell once per mention.

Seen means the post scrolled into view in the window or the user answered in that channel or
thread; `orochi threads show` marks what it printed as seen. Permission and apply prompts keep
their own rows and buttons (`prompts`) and are drawn as a mention from the asking agent (D5),
notified once, not twice.

## 7. Store

Additive, through `ADDED` (`USER_VERSION` does not move):

| Column | Meaning |
|---|---|
| `seats.member TEXT` | The member's name; NULL for the lead and its phases |
| `seats.continues TEXT` | The seat this one continues, NULL for a member's first |
| `seats.continuation TEXT` | JSON as `threads.continuation`, written when the seat ends |
| `messages.parent INTEGER` | The post a reply is under; NULL for a channel post |
| `messages.member TEXT` | The thread it belongs to (the member's name), NULL for a channel post that started none |
| `messages.mentions TEXT` | JSON array of names mentioned, `you` and `channel` included |
| `messages.broadcast INTEGER` | A reply also sent to the channel |
| `ui_state.seen_mention INTEGER` | The newest mention of the user seen in this conversation |

`recipient` stays for what `read_messages` already relies on and for old rows; delivery reads
`recipient`, `mentions` and the reader's threads together. Mentions are parsed by Orochi from
`@name` tokens matching a member, `you` or `channel` — never trusted from the agent as a list.

The lead's reply to the user stays an `agent_message` item; the channel is a view over items and
messages, not a copy of one into the other:

- **`v_channel`** — the channel's posts in order: user messages and the lead's replies (lead
  seat, lane 0) with their folded work, messages with `parent IS NULL`, each with its thread's
  bar where it started one, and `mentions_you`.
- **`v_members`** — one row per member: name, route, first and last activity, replies, state,
  `doing`, reported, worktree and trial state (from `v_team`), and `needs`: `asking`,
  `mention` (mentioned the user, unseen), `worktree`, `trial`, or NULL.
- **`v_member_timeline`** — one thread: its parent, its replies, its seats' items with their
  work, reports, trials and applies, continuation dividers.
- **`v_mentions`** — unseen mentions of the user across conversations, for the badge and the
  notifications.

`Activity::channel`, `members`, `member_timeline` and `mentions` read them, and §3.2's commands
print them.

## 8. What is unchanged

- Gates, routing, learning and telemetry; a continuation is a seat like any other.
- `manager-design.md`'s merge rules: trials, applies and the user's approval are the same rows,
  drawn in the member's thread and in the Work pane.
- The content rule: everything here is in `activity.sqlite3`, retained and deleted with its
  conversation; nothing reaches `telemetry.sqlite3`.
- The repository-wide room for peers of other conversations (D9).

## 9. Tests

- `tests/mailbox.rs`: `post` with mentions delivers to each mentioned member and `@channel` to
  all; an unmentioned member's post reaches the lead only; a thread reply reaches the thread's
  member and its other posters; `read_channel` returns only undelivered posts; mentions are
  parsed from text and an unknown `@name` mentions nobody; `send_message` aliases map as §4.
- `tests/activity.rs`: one member across turns is one `v_members` row; `v_channel` excludes
  replies and members' work; `needs` moves through `mention` → NULL on seen, `worktree` → NULL on
  close, `trial` → NULL on apply; `v_mentions` empties when seen.
- `tests/cli_e2e.rs`: `threads show` folds threads and `--member` prints one; `start_agent` on an
  ended name continues it (one member, two seats, `continues` set), on a running name is refused;
  a user post mentioning an ended member starts a continuation without consulting
  `worth_seating`; a fake agent with and without `session/load` pins resume versus transcript.
- `tests/test_chat_terminal.py`: a mention of `@you` rings the bell once and reports `blocked` to
  the host pane; typing `@im` lists `implementer`, Tab inserts `@implementer `, Esc keeps `@im`;
  the pinned thread row grows and shrinks without leaving a half-written row; ↓ lists members,
  Enter opens a thread, a line typed there is a reply in it, Esc returns to the channel.
- `tests/cli_e2e.rs`: `threads send` with a mention and with `--thread`, `threads mentions`.
- `desktop/dist/preview-data.js`: a channel with a lead, two members (one continued), a mention
  of the user and an open trial, so the channel, the bars, the panel and the notification state
  can be looked at without an agent.

## 10. Phases

| Phase | Content |
|---|---|
| C1 | Store: the columns, mention parsing, `v_channel`, `v_members`, `v_member_timeline`, `v_mentions`; `orochi threads show`, `--member`, `send --thread`, `mentions` |
| C2 | Agents: `post`, `read_channel`, `read_thread`, delivery by mention and thread, aliases, the prompt note's etiquette; `start_agent` writes the parent post |
| C3 | Desktop: the channel, thread bars, the thread panel, *threads (N)*, the composer's `@` list (§5); the console's channel transcript, pinned thread row, ↓ thread view with replies, `/thread`, `@` list (§3.1) |
| C4 | Mentions of the user: notifications, badge, sidebar mark, pane state, bell; prompts drawn as mentions |
| C5 | Continuing a member: by `start_agent`, by a mention, by a thread reply; the lead's note |
| C6 | A real run on both CLIs, recorded in a dated `real-validation-*.md`: a lead delegates to two members, they talk in threads and post to the channel, the user mentions one while it runs and the other after it ended, an agent mentions `@you` and the notification arrives |
| C7 | D8 with the user, on what C6 looked like |

## 11. Risks and what is unverified

- **Whether agents keep the etiquette.** Nothing yet shows real leads and helpers choosing
  between thread and channel, mentioning each other, or mentioning `@you` only when they need
  the user. C6 measures it; the prompt note is the only lever, and it costs tokens on every
  fresh session.
- **Whether a lead reuses a name** when it means the same helper. If not, the user's mentions
  and replies still keep threads, and the lead's second request opens a second thread.
- **Cost of conversation.** Members that talk more cost more: every delivered message is read by
  a model. D4 keeps what is pushed small; `mailbox.max_messages_per_hour` still bounds a member.
  C6 records what a team's talk cost against the same work done by one agent.
- **Resume across a gap.** No helper has been resumed yet; a provider that expires sessions falls
  back to the bounded transcript.
- **Notifications in front.** Notifying while the window is focused is new (§6); if it proves
  noisy it is narrowed to mentions in a conversation not in view.

## 12. Where the implementation departed (2026-11-09)

Implemented and covered by automated tests (`tests/mailbox.rs`, `tests/activity.rs`,
`tests/cli_e2e.rs`, `tests/test_chat_terminal.py`, `desktop/tests/ui.test.mjs`); none of it has
been run against a real agent CLI.

- **Views.** There is no `v_channel` and no `v_member_timeline`: the channel is `v_timeline`
  (which gained `member`) beside `v_room` (which gained `parent`, `member`, `mentions` and
  `broadcast`), and `v_roster` gained `member` and `continues`. New are `v_members`,
  `v_mentions` and `v_sidebar.mentions`. One member's thread for `orochi threads show --member`
  is `Activity::member_thread`, which joins the thread's messages and its seats' items.
- **Tools.** `post`, `read_channel` and `read_thread` come **after** the original four
  (`list_peers`, `send_message`, `read_messages`, `set_status`), whose order tests and the fixture
  agent read. `send_message` stays and is a post like any other: between the lead and a member it
  is a reply in that member's thread. A member's `post` is a reply in its own thread unless
  `thread: "channel"`; `also_channel` shows a reply in the channel too; the lead's is a channel
  post unless it names a thread. A post pushes nobody but whoever it mentions (`@channel`
  everyone), else the thread's other party, else the lead; the lead's unmentioned channel post
  pushes no one (recipient `-`).
- **Members.** A helper's seat and a seat beside the lead carry `seats.member`; a continuation
  writes the next seat with `continues` and the session left in `seats.continuation`
  (`Background::start` writes it when the run ends). The assignment post is a `messages` row
  with `via='handoff'` (an existing, unused value of the column's check), written by `grant`
  when a helper starts, and it is what a thread's later rows hang from (`parent`).
- **Continuing.** `start_agent` with a member's name continues it when it has finished
  (`grant`), is refused while it runs, and a member that finished is also continued by the
  person's note: a note mentioning it, or `threads send --thread`, is written with recipient `-`
  and `member` set, and the console's mail tick (`Session::continue_mentioned`) turns each such
  note into a continuation request marked `payload.user`, which skips `worth_seating`. A member
  whose worktree is still open continues in it. The lead is told of the note with the member's
  report (`Manager::later_note`), not woken by it. Only a console (terminal or host) continues a
  member: with nobody running the conversation the note waits.
- **Console.** Built: thread replies are folded out of the transcript (an assignment is one
  `└ name's thread` line, what was sent to the channel and what mentions `@you` are written),
  `@you` is reverse video with a bell, `@` lists the members with the `/` list's keys, a message
  mentioning a member (and not the lead) is a note to it, `/thread [member]`, and the member
  threads in a row of their own pinned above the input (`💬 spec ● 3 │ explorer ✓ 1 @`: running
  `●`, finished `✓`, its replies, `@` where something waits on the person; `Term::threads`, no
  row while the conversation has no members). ↓'s list holds every member, the running ones with
  `x` to stop and the ones that finished below them; Enter opens a thread and a line typed there
  is a reply in it (a finished member is continued by it). Nothing in §3.1 is left unbuilt;
  `@channel`, `@you` and the lead's own name are never taken as a note to a member.
- **Pane.** An agent's mention of `@you` puts the host pane in `waiting` with the text, until the
  person next types (`Session::mentioned`); it is not tied to the mention being marked seen.
- **Desktop.** As §3, §5 and §6, built by `desktop/dist/app.js`, except that a thread opens in a
  column of its own inside the chat area, beside the body and left of the right section (whose
  tabs it never replaces), with a draggable width; the lead is offered in the `@`
  list as `lead`. The window does not pass the name the lead has in the room, so `@lead` in a
  post is for the lead as a turn.
- **Prompts as mentions (D5), decided against.** Permission and apply prompts keep their own
  rows, buttons and notification; they are not also drawn as `@you` posts. A prompt already
  notifies once (window, dock badge, pane `waiting`), marks its member's bar (`needs = asking`)
  and sits in the body where the answer is given; a second post for the same question would be
  the double notification D5 set out to avoid.
