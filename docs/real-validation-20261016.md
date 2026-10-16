# Real validation: the lead as manager (2026-10-16)

Phase M6 of [The Lead as Manager](manager-design.md): the console driven against the real
CLIs on this machine, signed in with the user's own subscriptions — Claude Code 2.1.301 through
`claude-agent-acp` 0.77.0 and `codex` 0.162.1 through `codex-acp` 1.10.0, both put in by
Orochi's own adapter install into a private data directory. The question was whether a real
lead, told nothing about the tools by name, plans, delegates, asks for a merge trial, reads it
and asks for the apply; what each of those turns costs; and whether a helper's message wakes an
idle lead.

The repository was a two-function Python calculator with one `unittest` check configured
(`python3 -m unittest discover -s tests -t .`), `manager.apply = "auto"` (the console ran with
its stdin piped, so an `ask` would have had nobody to ask), `--permission allow`, the classifier
off. One message:

> Add multiply and divide to calc.py, with tests (divide by zero raises ValueError). Do the
> implementation through a background helper that gets its own worktree, and once its result
> is in, have Orochi try the merge and apply it to my working tree if the checks pass. Keep
> your plan up to date as you go.

The task names what to do (a helper, a trial, an apply, a plan), never the tool that does it.
Everything below is the activity store's rows (`tests/../analyze.py` is in the ignored
validation directory beside the runs).

## Run A — Codex lead (gpt-5.6-luna, medium), helpers on Claude haiku

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `tester` | 314 s | 41,540 | 5,298 | 38 |
| 1 | — | helper `calc-ops` (writes) | 25 s | 351,493 | 88,899 | 13 |
| 1 | — | helper `calc-check-fix` (writes) | 23 s | 292,819 | 80,869 | 11 |
| 2 | background | lead, resumed | 53 s | 47,212 | 7,106 | 12 |

What the lead did, in order, unprompted by any tool name: `set_plan` → `start_agent(calc-ops,
write)` → `list_work` (polling, twelve calls over the run) → `merge_trial(calc-ops)`. The trial
came back clean, 2 files, **checks failed** — the validation repository's own fault (`tests/`
was not a package, so `unittest discover` could not import it) — and the lead did not apply:
it ran the tests itself in the user's tree, read the import error, updated its plan, started a
second helper (`calc-check-fix`, "make the tests discoverable"), ran `merge_trial` on that,
read *clean · 3 files · checks passed*, and called `apply_merge`. All of this happened inside
**one 314-second turn**: a Codex lead does not end its turn to wait, it polls `list_work`.

Two things went wrong, both Orochi's:

1. **The apply raced the exit.** `apply_merge` was approved (auto) during turn 1, which ended
   with stdin already closed; the console's EOF rule counted only live helpers as a reason to
   stay, so it left — the helpers' reports woke the lead for turn 2 first, and when turn 2
   ended the apply started on a blocking thread as the session swept its worktrees. The files
   reached the tree (same second as the exit) and the worktree was closed as a *patch* by the
   sweep, because the apply had not yet removed it from the manifest; the lead was never told.
   Fixed: an approved or running apply, a running trial, and reports not yet handed back all
   keep the console alive after EOF and all hold the exit; an approved apply runs **before** the
   lead is woken, so one wake carries the helper's report and the apply's outcome; and a report
   waits for a running apply, which needs the repository lock the wake would take. Pinned by
   `an_apply_asked_for_inside_a_polling_turn_runs_before_the_lead_is_woken`, whose fixture lead
   polls the way this one did.
2. **Woken before the apply, the lead asked again** (`apply_merge` → "already under way"), then
   closed the superseded worktree with `finish_workspace(calc-ops, discard)`, and tried
   `finish_workspace(calc-check-fix, merge)` as a fallback — which **Codex's own Guardian hook
   denied** (the "Guardian Review" rows in the transcript are this machine's Codex hook, not
   Orochi's); the lead read the denial as Orochi's refusal and told the user it would not
   bypass the apply flow. With the ordering fixed, the second ask has no occasion.

Also seen: the lead wrote its plan, its helpers' titles and its answer in **Spanish** to an
English request; the LANGUAGE note asks for the user's language and `codex` did not follow it.
Not investigated here.

## Run B — Claude lead (haiku, medium), helper on Codex gpt-5.6-luna

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `tester` | 30 s | 472,215 | 107,120 | 17 |
| 1 | — | helper `calc-muldiv` (writes) | 45 s | 23,768 | 3,170 | 11 |
| 2 | background | lead, resumed | 11 s | 256,226 | 33,536 | 10 |
| 3 | merge | lead, resumed | 7 s | 132,770 | 16,660 | 6 |
| 4 | merge | lead, resumed | 9 s | 203,780 | 24,997 | 8 |

The clean run, end to end, on the fixed binary: `set_plan` → `start_agent(write)` → the lead
**ended its turn** ("the trial report arrives as a separate message") → the helper finished and
**sent the lead a message** (its file list) four seconds before the lead was woken: the inbox
held it and turn 2 carried both the report and the message → the lead reviewed the diff,
`merge_trial` → turn 3 carried *clean · 2 files · checks passed* → `apply_merge` (auto) →
the apply wrote 2 files and closed the worktree → turn 4 carried *Applied* → the lead ran
`git status` and the tests in the user's tree (5 passing) and reported, correctly, that the
worktree had been closed by the apply and nothing was committed.

Two Claude-side stumbles, both answered: `set_plan` was first called with `plan` as one
numbered paragraph and refused — `set_plan` now reads a paragraph line by line — and
`start_agent` failed three times before the lead loaded it with Claude Code's `ToolSearch`
(the deferred-tool mechanism the prompt note already warns about).

## Run C — Codex lead again, on the fixed binary (helper on Codex gpt-6-luna)

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `tester` | 104 s | 35,507 | 4,963 | 20 |
| 1 | — | helper `calc-impl` (writes) | 41 s | 33,922 | 5,677 | 9 |
| 2 | background | lead, resumed | 18 s | 38,014 | 5,707 | 4 |

The same lead, the same shape — `set_plan`, `start_agent(write)`, `list_work` polling inside
one turn, `merge_trial`, `apply_merge` once the trial read clean with the check passing — and
this time the apply ran when the turn ended, **before** the lead was woken: the one wake
carried the helper's report and *Applied `calc-impl` (trial calc-impl-1): 2 files written*,
the lead verified the tree (5 tests, `git diff --check`) and updated its plan to done. The
worktree was closed by the apply; the sweep at exit found nothing open and wrote no patch.
Two turns, 122 s, 10,670 weighted on the lead in all.

## Run D — a conflict: Claude lead (haiku), helper and resolver on Codex gpt-5.6-luna

The same task, with the user kept working: a sidecar appended a `multiply` of its own to the
user's `calc.py` five seconds after the helper's worktree appeared, at the end of the file,
where the helper would put its own `multiply` and `divide`.

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `tester` | 23 s | 298,558 | 84,493 | 11 |
| 1 | — | helper `calc-ops` (writes) | 38 s | 23,874 | 3,236 | 11 |
| 2 | background | lead, resumed | 9 s | 188,507 | 23,978 | 9 |
| 3 | merge | lead, resumed | 12 s | 196,572 | 29,201 | 8 |
| 3 | — | `resolve-calc-ops` (writes, in the trial's worktree) | 46 s | 24,206 | 3,640 | 11 |
| 4 | background | lead, resumed | 13 s | 208,129 | 32,629 | 8 |
| 5 | merge | lead, resumed | 11 s | 218,260 | 31,036 | 8 |

`merge_trial(calc-ops)` → *1 conflict (calc.py); merge_trial("calc-ops", resolve=true)
resolves them* → the lead read it, said why ("your uncommitted `multiply` clashed with the
helper's") and asked for the resolving trial → trial 2 reproduced the conflict and Orochi
seated `resolve-calc-ops` in trial 2's worktree with the `untangle` instruction → it kept the
user's `multiply` (docstring and all), added the helper's `divide` and its three tests, and
cleared the markers in 46 s → the recheck ran the project's check: *clean · 2 files · checks
passed · 1 file both sides changed (calc.py)* → the lead reviewed the trial's output and tests
before asking for the apply → 2 files written, 5 tests passing in the user's tree, the helper's
worktree closed by the apply. The lead's report to the user was accurate to the row, down to
noting that the conflicted first trial's worktree was still on disk until the conversation's
end — which is where the sweep then removed it. Five lead turns, 111 s of lead time, 201,337
weighted on the lead in all; the conflict cost two wakes and one resolution seat over run B.

## Run E — `ask` through a client, and a restarted host: Claude lead (sonnet-5), helper on Codex

The headless path: `orochi threads new` / `threads send`, `orochi host --thread`,
`manager.apply = "ask"`, and a client script standing in for the desktop's card — it polls
`prompts` for a row of kind `apply` and answers it `yes`, as the window does. Then the host was
left to its idle timeout (60 s), a second message was queued ("without starting anything or
running any tool: what did your plan say, which trial was applied, what was the helper
called?"), and a second host started on the same thread.

| Turn | Host | Origin | Seats | Wall | Reported total | Weighted | Resumed |
|---|---|---|---|---|---|---|---|
| 1 | 1 | user | lead `tester` | 28 s | 339,755 | 84,739 | no |
| 1 | 1 | — | helper `mathops` (writes) | 39 s | 25,034 | 4,740 | — |
| 2 | 1 | background | lead | 17 s | 289,726 | 36,344 | yes |
| 3 | 1 | merge | lead | 11 s | 149,782 | 18,522 | yes |
| 4 | 1 | merge | lead | 12 s | 152,352 | 17,683 | yes |
| 5 | 2 | user | lead `planner` | 6 s | 77,234 | 8,969 | **yes** |

The lead's `apply_merge` was answered *asking the user*; the `prompts` row carried the trial's
files, overlap and the line the dialog shows ("2 files · checks passed (unittest)"); the client
answered it (`answered_by = desktop`); the apply ran, turn 4 told the lead *Applied*, and the
lead confirmed the tree. The second host loaded the recorded sonnet-5 session (`resumed`
on turn 5, 6 s, 8,969 weighted) and the lead answered from memory, correctly: the plan ("added,
merged and applied, no further step"), the helper `mathops`, the trial `mathops-1`. A
conversation's manager survives its host.

The terminal form of the same question — the dialog drawn in the console's pinned area — was
not driven live; it is the same `Ask` queue the seats' permission questions use, pinned by
`tests/test_chat_terminal.py`.

## Runs F and G — the project in its sandbox (container mode, Codex inside)

The sandbox host of [real-validation-20261005.md](real-validation-20261005.md) brought back by
`orochi setup --sandbox` into the private data directory (Lima 2.2.1 installed there, the
existing `sbx-host` VM started, the golden image reused), a copy of the repository under the
home directory the VM mounts, `orochi sandbox create` (container, Docker inside), and the same
task with one more ask: the hostname of the machine the lead runs on. Claude Code keeps its
sign-in in the macOS keychain, which `sandbox auth` reports as "nothing to carry" without
`orochi sandbox login claude`, so these two runs were Codex only, lead and helpers.

**Run F** got as far as the trial and then lost its lead: the trial read *clean · 2 files ·
checks failed* because the validation config named the check's interpreter by its path on the
Mac, which does not exist inside — the check had run **inside** the conversation's container
(this is the departure §14 of the design named, closed the same day: a trial's checks now go
through the session's placement as the scheduler's do). Then the lead's ACP process died
mid-turn: *Incoming transport closed*. The VM's journal says why: Ubuntu's `apt-daily-upgrade`
ran at 09:24:20, upgraded systemd, re-executed it and restarted `incus.service`, closing every
agent's `incus exec`. Not Orochi's code, but Orochi's VM: `host.sh` now disables the apt timers
and unattended-upgrades in it (`orochi sandbox up` re-applies the script).

**Run G**, on the repaired VM with the check as `python3`:

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `tester` (inside) | 60 s | 23,701 | 17,232 | 6 |
| 1 | — | helper `calc-ops` (writes, inside) | 91 s | 23,709 | 3,043 | 13 |
| 2 | background | lead | 37 s | 26,487 | 4,688 | 5 |
| 3 | merge | lead | 24 s | 27,766 | 3,516 | 3 |
| 4 | merge | lead | 30 s | 29,775 | 4,053 | 4 |

The lead reported its hostname as `fdb83a46f1c4…`, the name of the conversation's own session
container (`orochi sandbox status` lists it beside the project's); the helper ran inside it
through the mailbox relay; the trial's `unittest` ran inside and passed; the apply wrote the two
files onto the mounted tree, where the Mac sees them and the tests pass; the helper's worktree
was closed by the apply and nothing was left for the sweep. The lead's first `start_agent` used
an underscore in the name and was refused; it read the reason and retried. Codex also noted its
own sandbox "hit the environment's sandbox networking setup" on a read-only command and
retried; that is Codex's sandbox inside Orochi's, not Orochi's.

## Run H — runner mode: a Claude lead on this machine, the runtime in the sandbox

`orochi sandbox mode runner` on the same project: the agents run here, signed in as they are
here (so Claude, keychain and all), and the project's runtime is the sandbox, reached through
the `orochi-sandbox` MCP tools. The question was whether real agents use those tools for what
runs code instead of their own shell. One more ask in the task: the hostname of the machine
where the tests actually ran.

| Turn | Origin | Seats | Wall | Reported total | Weighted | Requests |
|---|---|---|---|---|---|---|
| 1 | user | lead `implementer` (sonnet-5, here) | 30 s | 335,536 | 65,902 | 15 |
| 1 | — | helper `calc-ops` (sonnet-5, here, writes) | 53 s | 436,674 | 74,258 | 19 |
| 2 | background | lead | 12 s | 206,075 | 25,812 | 10 |
| 3 | merge | lead | 10 s | 160,639 | 19,317 | 8 |
| 4 | merge | lead | 11 s | 165,050 | 19,774 | 8 |

Every shell-shaped call of the run, from the store: the lead's own shell twice, both read-only
(`find` over the tree, `git diff` in the helper's worktree); the helper's own shell once, to
`cat` the file; and the helper's tests through **`sandbox_exec`**, whose output begins with the
container's hostname `p-03bec0…` — the conversation's own session sandbox, made on the first
turn. The trial's `unittest` ran inside too; the apply wrote the two files here, where the
tests pass; the lead answered the hostname question with the sandbox's name and said so.

## What it costs

A wake is a resumed session. On haiku, the three Orochi-written turns of run B cost 16,660 to
33,536 weighted (133k–256k reported, nearly all cache reads) and 7–11 s each; on Codex, the one
wake of run A read 7,106 weighted (`codex-acp` reports the last request only, so Codex figures
stay a lower bound, as [real-validation-20260925.md](real-validation-20260925.md) says). The
trial itself and the apply are git and the project's check: under a second here. The helper's
work — one function pair with tests — cost 3,170 weighted on Codex and 80,869–88,899 on haiku
through `claude-agent-acp`, which accumulates the turn.

## What held and what did not

| Claim | Result |
|---|---|
| A real lead plans, delegates, trials, reads the trial and applies without being told the tools | **Held**, on both CLIs; Codex polls inside one turn, Claude ends its turn and is woken |
| A failed check stops the lead from applying | **Held**: the Codex lead investigated and fixed the cause through a second helper rather than applying |
| A helper's message to an idle lead wakes it | **Held** (run B): delivered in the wake with the report |
| The apply runs, closes the worktree, and the lead hears the outcome | **Held** on runs B and C; on run A it raced the exit — fixed, pinned, and run C is the fix on the same lead and shape |
| `manager.apply = "ask"` from a client | **Held** (run E): a `prompts` row the client answered, the apply following it |
| `manager.apply = "ask"` as the console's own dialog | Automated tests only (runs A–D were piped) |
| A conflicted trial with `resolve = true` | **Held** (run D): the lead asked for it on reading the conflict, the resolution seat kept the user's side and cleared the markers, the recheck ran the project's check, and the lead reviewed before applying |
| A restarted host resuming with plan and trials | **Held** (run E): the second host loaded the session and the lead recalled plan, helper and trial without a tool |
| The same flow with the project in its sandbox | **Held** (run G, container mode, Codex): lead and helper inside, the trial's checks inside, the apply on the mounted tree. Run F found the VM's unattended upgrades restart Incus under running agents; fixed in `host.sh` |
| Claude inside a container | **Not run**: its sign-in is in the keychain; `orochi sandbox login claude` is interactive |
| Runner mode: the agent here, code run through the `orochi-sandbox` tools | **Held** (run H, Claude): the helper ran its tests with `sandbox_exec`, the agents' own shells were used only to read; the trial's checks ran inside |

Nothing was committed to any repository by the agents; the validation directory
(`.orochi/live-validation-20261016/`) keeps the private config, data, the five repositories
as the runs left them (the sandbox runs' repository is `~/orochi-sbx-validation/repo`), every
transcript, run D's sidecar, run E's client and `analyze.py`. The sandbox VM and the three
session containers were left as they were; `orochi sandbox gc` or `sandbox down` takes them.
