# Real validation: the team channel (2026-11-09)

What was run, against what, and what it showed. Two runs of `orochi chat` with piped input
(no terminal) against the real Claude and Codex CLIs on this machine, in a throwaway repository
(`README.md`, `greet.py`) with a fresh data directory, the lead pinned to `claude` / `haiku`.
The prompt asked the lead to start one read-only helper named `reader` with `start_agent`, to
read `greet.py` and report what `greet()` returns, to say so, and to mention `@you` if it needed
anything. Built from the commit that introduced `docs/team-channel-design.md`.

## What held

- **A real lead used the new surface unprompted.** Claude (haiku) found `start_agent` through
  its tool search and called it with the name `reader`; Orochi answered `{"started":"reader"}`.
- **The assignment is the first line of the member's thread.** Both runs wrote the lead's
  `via='handoff'` post (`@reader Read greet.py …`, `mentions = ["reader"]`) and the console drew
  it as `└ reader's thread …`, one line, without the thread's other traffic.
- **A member is a name.** `v_members` held one row for `reader` (`done`, `codex`, `gpt-6-luna`);
  `orochi threads show <id> --member reader` printed the task, the helper's working notes and its
  report (`greet(name)` returns `"Hello, " + name`, no default argument).
- **The helper ran on another agent than the lead** (Codex, 29 s) and its report came back as
  "1 agent finished / handing back what reader reported".

## What did not, and why

- **Run 1** failed to start the helper: discovery of every agent timed out
  (`reader · claude unavailable: Timeout: agent discovery timed out`, also codex and opencode) —
  a cold data directory preparing the adapters at once, not the channel. Run 2 reused the data
  directory and the adapters were ready.
- **Run 2 ended without the lead's summary**: Claude answered `RateLimit: Internal error` and
  then sat in cooldown, so the completion turn that hands the report to the lead found no
  candidate and the run exited 1. The helper's work had been done and recorded.

## Still unverified

Neither run exercised: the helper using `post` or `@you` (it had nothing to ask), a member being
mentioned or continued in its thread, `read_channel` / `read_thread`, the notification path, or
a lead reusing a member's name. They are covered only by the automated tests with the fixture
agent, as `docs/team-channel-design.md` §12 says. The lead's own text in both runs was cut by the
non-terminal console and is not evidence either way.

## Second session, after the go-ahead to run again (same day)

Three more runs against the same throwaway repository. Claude (haiku) was rate-limited by now
(`claude unavailable: agent/account is in cooldown`), so two runs with a Claude lead stopped
before the lead could start a helper or take its report. The third pinned **Codex as the lead**
(`--agent codex`, `gpt-6-luna`), asked it to start `reader`, and fed a second line after the
first report: `@reader please also say what a formal version would return`.

### What held

- **A Codex lead started the helper unprompted, summarised its report, and the person's
  `@reader` note continued the member in its own thread.** The console printed
  `continuing reader in its thread`, `reader finished · codex · gpt-6-luna · 6s` (against 32 s
  for the first run: the session was picked up, not started cold) and the lead then answered the
  question with the new report. The run exited 0.
- **The note was a note to the member, not a turn for the lead**: a `via='user'` row in
  `reader`'s thread with `mentions = ["reader"]`.

### What the runs found, and what was fixed

- **A continuation between turns left no seat behind.** `helper_seat` needed a running turn, so
  a member continued from the idle prompt ran but had no `seats` row: no second seat, no
  `continues`, nothing for its report. A continuation now sits under the turn of the member's
  last seat (`Activity::seat_turn`); `tests/test_chat_terminal.py` checks the seat count and
  `continues` after one.
- **An assignment that told a member to "mention @you" counted as a mention of the user**, so
  `v_mentions` listed the lead's instruction as unseen mail for the person. Assignments
  (`via='handoff'`) are excluded from `v_mentions`; `tests/mailbox.rs` pins it.
- **The helper's own instructions said "use send_message only…"** and never named `post` or
  `@you`; they now do (`chat/background.rs::prompt`).

### Still unverified

The helper (Codex) did not `post` with `@you` in any run, even when the lead's task asked it to;
the lead asked the user in its own text instead. With the corrected helper instructions it was
run once (the Codex-lead run above), and still did not. So how a real helper decides to mention
`@you`, `read_channel` / `read_thread`, a lead reusing a member's name by itself, and the
notification path are unverified outside the automated tests.
