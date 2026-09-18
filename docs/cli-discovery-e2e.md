# CLI Auto-Discovery: Live E2E (2026-09-15)

## Problem and changes

Previously, whether Claude was installed was decided solely by the presence of `claude-agent-acp`, so Claude was excluded from the candidates even when the `claude` CLI itself was present.

- Separated the states of the CLI itself, the ACP adapter, and connection readiness.
- If the Claude/Codex CLI itself is present, a missing ACP adapter is automatically installed into Orochi's `adapters/` cache.
- The four known presets are also filled in for configurations that omit them. An explicit setting with the same ID, and `enabled = false`, take precedence.
- `agents` does not download anything; it displays states such as `adapter_required`. Preparation happens at run / discovery time, and the reason for a failure is displayed.
- Uses pinned versions, a dedicated npm cache, disabled install scripts, mutual exclusion, atomic installation, and a separate preparation timeout.

Auto-discovery covers Codex, Claude Code, Gemini CLI and Antigravity ACP. For anything else, register a custom ACP command. The implementation does not guess an unknown protocol and send it to an arbitrary CLI.

## Verification against the real CLIs

On macOS, the coding CLIs available from PATH were Codex and Claude Code. Gemini/Antigravity were not installed.

| Check | Result |
|---|---|
| Inventory with only the Claude CLI itself | `installed: true`, `adapter_required` |
| Automatic installation of the missing adapter | Successfully installed `@agentclientprotocol/claude-agent-acp@0.77.0` into the dedicated cache |
| Using the existing CLI | Set `CLAUDE_CODE_EXECUTABLE` to the detected `claude` |
| Claude ACP discovery | `default`, `opus[1m]`, `claude-fable-5-1[1m]`, `sonnet`, `haiku` |
| Codex ACP discovery | `gpt-5.6-sol`, `gpt-6-astra`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5` |
| Dry run with the original web app request | 10 Claude/Codex candidates in total were routing targets |
| Delegating execution to Claude | With `--agent claude`, Haiku was selected; created one file and passed content verification |
| Duration of the Claude run | 10.46 s including discovery, 1 attempt, exit code 0 |

With the original request, `gpt-5.6-luna / medium` came first, and `haiku` was also among the candidates at the same estimated cost. This run verified candidate registration and execution connectivity; it is not a benchmark showing the relative quality or cost of providers. An explicit `--agent claude` was used to confirm delegation to Claude.

## Local artifacts for reproduction

The following were saved under `.orochi/live-e2e/native-discovery/` (not tracked by Git).

- `evidence/discovery.json`: the real agents' lists of models, reasoning levels and modes
- `evidence/route.json`: all candidates and scores for the original web app request
- `evidence/claude-run.log`, `claude-result.json`, `runs.json`: results of the Claude run
- `config.toml`, `task.txt`, `repo/result.txt`: run conditions and the generated file

No further changes were made to the app used for verification.

## Regression tests

Network-free tests verify detection with only the CLI itself, distinguishing missing npm from disabled installation, preserving explicit settings, mutual exclusion of concurrent installs, cache reuse, retry after a failed install, and the path from CLI execution through candidate registration to file creation.

All 36 tests, Clippy (with warnings treated as errors) and the Rustfmt check passed. The release build and the no-account demo also succeeded.

## Sources for the connection method

- [CODEX_PATH in Codex ACP](https://github.com/agentclientprotocol/codex-acp/blob/main/README.md)
- [CLAUDE_CODE_EXECUTABLE in Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/acp-agent.ts)
