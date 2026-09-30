# Real Validation: Background Agents Inside Orca (2026-09-30)

Phase S7 of [Supervising Background Agents](supervision-design.md): the console driven against
real agents, inside a real Orca pane, with what Orca recorded about that pane read back through
Orca's own CLI.

## Setup

| | |
|---|---|
| Orochi | `a3ad0d9` plus the fixes below, release build as `~/.local/bin/orochi` |
| Host | Orca 1.4.215; the pane was opened with `orca terminal create`, driven with `orca terminal send` / `read`, and Orca's view of it read with `orca worktree ps --json` |
| Agents | `codex` through `codex-acp` (`~/.bun/bin/codex-acp`); `claude` through `claude-agent-acp` 0.77.0 with Claude Code 2.1.285. Gemini and Antigravity not installed |
| Repository | A scratch `git clone --local` of this repository, so nothing the agents did could reach the working tree |
| Task | 「このリポジトリについて、(1) ルーティング（router/ と scheduler）の流れ と (2) 学習（learning.rs とテレメトリ）の仕組み を、それぞれ別の補助エージェントに並行して調べさせてください。あなたは補助を起動したらすぐ返事をして、報告が戻ってきたら2つを統合して要点を5行でまとめてください。ファイルは変更しないでください。」 (Japanese as typed: investigate routing and learning with a separate helper each, answer at once, then merge both reports into five lines, change no files) |

## What happened

| Step | Observed |
|---|---|
| Orca pane report (S0) | Before any message, `worktree ps` listed the pane as `agentType: "orochi"`, `state: "done"`; while a seat's question was open, `state: "blocked"`; after the last turn, `done` again. Orca accepts an agent type it has no hook route for |
| Classification | `investigation / complex`; a read-only `reviewer` seat was seated beside the lead (Orochi's own companion seat) |
| Routing | Lead `codex · gpt-5.6-sol · high`; reviewer `codex · gpt-5.6-luna · medium`; Claude was in cooldown (see finding 1) |
| Delegation | **The real lead called `start_agent` twice**, unprompted beyond the task: `routing` (ルーティングとスケジューラの調査) and `learning` (学習とテレメトリの調査). Orochi routed them to `codex · gpt-5.5 · medium` and `codex · gpt-5.6-terra · medium` — three different Codex models in one conversation, spread by `busy_routes` |
| Lead's first turn | Answered at once (「補助エージェント2名を並行起動しました…」) and ended: 1 m 59 s |
| Helpers | `learning` finished at 2 m 36 s, `routing` at 4 m 21 s, both after the lead's turn had ended |
| Results | Both reports came back to the lead as completion turns; the lead's final answer was the five requested lines, each citing a file and line in the clone |
| Files | None changed |

Per seat, from `activity.sqlite3` (`Usage::total`; Codex figures are a lower bound, `codex-acp`
reports its last request only — see `real-validation-20260925.md`):

| Seat | Model | Wall | Tokens (cached) | Requests |
|---|---|---|---|---|
| investigator, turn 1 | gpt-5.6-sol high | 119 s | 26,722 (26,368) | 4 |
| routing (helper) | gpt-5.5 medium | 261 s | 116,215 (110,976) | 14 |
| learning (helper) | gpt-5.6-terra medium | 156 s | 64,526 (62,208) | 13 |
| investigator, completion of `learning` | gpt-5.6-sol high | 181 s | 30,355 (29,568) | 6 |
| investigator, completion of `routing` | gpt-5.6-sol high | 92 s | 34,873 (33,536) | 3 |
| reviewer (companion) | gpt-5.6-luna medium | — | not recorded: stopped when the lead's turn ended | — |

## Findings, and what was done

| # | Finding | Status |
|---|---|---|
| 1 | The first attempt failed to start both adapters — `mise`'s untrusted `mise.toml` in this Orca workspace stops its `node` shim — and that local launch failure put `claude` into an **account-wide cooldown** (`update_failure` with `*`), which kept Claude out of the whole run | **Open, a decision for the routing policy**: a process that never initialised says nothing about the account. Environment note: in this workspace, run from a directory without an untrusted `mise.toml` |
| 2 | With no agent able to start, the console **exited** instead of reporting and waiting | Fixed: at a terminal or for a host the session continues; piped runs still end with the error. Pre-existing |
| 3 | Codex asks for permission by tool-call **id alone**, so every question from a Codex seat read `a tool` | Fixed: the console remembers each seat's announced calls by id and names the question from them; the lead's questions fall back to the calls already on screen |
| 4 | A request with **no `kind`** from a read-only seat was taken for a read and auto-allowed (found while testing 3) | Fixed: a read-only seat's request that does not say what it is, is asked about |
| 5 | Two launch blocks for one request: a real lead asks one call at a time with its own review between calls, seconds apart, longer than the 1.5 s burst | Fixed: the block is printed when the lead starts to answer (20 s fallback) |
| 6 | `learning` came back alone while `routing` still worked; the lead spent a whole turn — 30,355 tokens, 3 minutes — saying it was still waiting, and waited on `read_messages` | Fixed: a report is held while other helpers still run, up to `HOLD` (5 minutes), so siblings come back in one turn. Departs from D3 as designed |
| 7 | The companion seat's run left no outcome or usage once the lead ended it | Open: pre-existing; a stopped seat is not recorded |

## Not verified

- **Orca's sidebar drawing the helpers as child rows.** `worktree ps` shows no `subagents`
  field, so whether Orca draws what the OSC report sends has to be looked at in the window.
- The same flow with **Claude as the lead** (cooldown, finding 1), and with a mixed field.
- The desktop window's notifications and dock badge.
- Whether a lead delegates **without being asked to**: this task asked for helpers by name.
