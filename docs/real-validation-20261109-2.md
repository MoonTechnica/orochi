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
