# Real Validation: Agents Signed In with This Machine's Sign-In (2026-10-01, 3)

Agents inside a sandbox use the sign-in this machine already has, and a local model server here
is reached as `localhost` inside, at the user's request ([Per-Project Sandboxes](sandbox-design.md)
§15). Same machine and VM as [the gateway's record](real-validation-20261001.md).

## What the CLIs' own documentation says

| Agent | Where a login here lives | Documented headless way |
|---|---|---|
| Codex ([docs](https://learn.chatgpt.com/docs/auth)) | `~/.codex/auth.json` (file store) | copy `auth.json` to the other machine; device-code login |
| Claude Code ([docs](https://code.claude.com/docs/en/iam)) | macOS Keychain; `~/.claude/.credentials.json` on Linux | `claude setup-token` → `CLAUDE_CODE_OAUTH_TOKEN` (one year, subscription) |
| Gemini CLI ([docs](https://geminicli.com/docs/get-started/authentication)) | cached locally | cached credentials, `~/.gemini/.env`, `GEMINI_API_KEY` / `GOOGLE_*` |
| Antigravity ([docs](https://antigravity.google/docs/cli/install/)) | OS keyring only | `modelProvider: "gemini"` with `GEMINI_API_KEY` |
| Lima ([docs](https://lima-vm.io/docs/config/network/user/)) | — | the host's loopback is `host.lima.internal`, 192.168.5.2 |

## Results

| Question | Result |
|---|---|
| Can a service bound to this machine's `127.0.0.1` be reached from the VM? | **Yes**: `host.lima.internal:<port>` answered 200 |
| …and as `localhost` inside a sandbox? | **Yes**, through an Incus proxy device (`bind=instance`, to 192.168.5.2). Through Orochi: a stand-in server on this machine's `127.0.0.1:11434` answered `curl localhost:11434` inside with 200 after a run readied the sandbox |
| Can a file be shared by mounting it? | **Read and write, yes; replace, no**: writing in place worked and kept this machine's owner, but `rename` over it — how CLIs save a credential — failed. So files are kept in step instead of mounted |
| Does Codex inside use this machine's sign-in? | **Yes.** A dry run started `codex-acp` inside (initialize and a session, no prompt, no quota), which listed the account's models; `codex login status` inside: `Logged in using ChatGPT`. No login was done inside |
| Does a refresh inside come back? | **Yes**, with a probe file declared by a test agent: in before run 1; changed inside; back here on run 2 |
| Do secrets stay off command lines? | **Yes**: a stored value reached the sandbox only in `/dev/shm/orochi-env-<agent>`, mode `0600` |

## Not verified

- Claude Code with a `claude setup-token` token: it needs the user's browser approval.
- Gemini CLI and Antigravity inside (neither is installed on this machine).
- Two sides refreshing one Codex token at the same moment; this machine's copy wins if both moved.
