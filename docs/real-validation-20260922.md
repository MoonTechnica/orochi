# Live Validation of MCP Support (2026-09-22)

What the MCP work of 2026-09-21/22 left verified only against fixtures, measured against the
installed, authenticated agents and against real hosted MCP servers. The agents were
claude-agent-acp 0.77.0 over Claude Code 2.1.278, and codex-acp 1.10.0. Every run used a
scratch configuration and data directory, so none of it entered the real telemetry or learning
history. The scratch directory is local only.

| # | Item | Result |
|---|---|---|
| 1 | Which MCP transports the real agents advertise | **Measured.** claude-agent-acp: `http` and `sse`. codex-acp: `http` only, so an `sse` server is correctly left out of a Codex session |
| 2 | A configured stdio server reaches a real agent, which starts and calls it | **Confirmed** on Claude Haiku: the reply was the probe's marker, and the probe logged the call |
| 3 | `orochi serve` passes an editor's `session/new` servers to the agent it picks | **Confirmed** on Claude Haiku, routed by the gateway itself. Before this work the gateway refused such a session outright |
| 4 | OAuth sign-in, keychain, and a real agent connecting with the token | **Confirmed** end to end against a local authorization server: sign-in without a browser, token in the macOS keychain, every request of the real agent carrying it, refresh and logout |
| 5 | Real hosted MCP servers | **Probed read-only.** Linear, Notion, Sentry, Atlassian, Stripe and Vercel all answer Orochi's probe with `401`, and all publish metadata where Orochi looks, with dynamic registration and S256. **Not signed in:** that needs an account at each and a person at a browser |
| 6 | The discovery code against the MCP specification | **Four gaps found and fixed**, one of them found by a real provider's shape (below) |

Cost: three runs on Claude Haiku, about 250,000 tokens by the adapter's own figures, almost all
of it cache reads (each session read 80–85k cached tokens and wrote about 300).

## 1. What the agents advertise

`orochi status --discover` against the scratch configuration, which starts each agent and opens
a session without prompting, read `agentCapabilities.mcpCapabilities` through `acp.rs`:

```
claude | mcp = {'http': True, 'sse': True}  | models = default, opus[1m], claude-fable-5-1[1m], sonnet, haiku
codex  | mcp = {'http': True, 'sse': False} | models = gpt-5.6-sol, gpt-6-astra, gpt-5.6-terra, gpt-5.6-luna, gpt-5.5
```

## 2. A configured stdio server, through a real Claude session

A 60-line stdio MCP server with one tool, `orochi_probe`, that returns a marker and logs each
call, was declared under `[[mcp.servers]]`. One run pinned to `--agent claude --model haiku`:

```
prompt: Call the MCP tool orochi_probe exactly once, then reply with only the text it returned.
reply:  PROBE-DE1AF44C
probe:  {"event": "started", "cwd": ".../live/repo"}
        {"event": "called", "tool": "orochi_probe"}
```

The server ran with the agent's working directory, the repository, as the mailbox server does.

## 3. Through the ACP gateway

A small client played an editor against `orochi serve`: `session/new` carried the probe as the
editor's own stdio server, and one prompt asked for the tool. The gateway routed the prompt
itself (`Route: claude / haiku / agent-default`), and the permission request for the MCP tool was
relayed to the "editor", which allowed it once.

```
session/new: ok
stop:        end_turn
reply:       GATEWAY-C34D41AF
probe:       started, called orochi_probe
```

No real editor was used. What was verified is the gateway with a real agent behind it.

## 4. OAuth, the keychain, and a real agent using the token

`tests/fixtures/mock_oauth.py` was grown into an MCP server a real agent can list and call over
streamable HTTP, and it is its own authorization server: RFC 9728 and RFC 8414 metadata, RFC 7591
registration, PKCE checked on the token request, and `resource` required. With `mcp.keychain =
true`:

1. `orochi mcp` → `remote  not signed in (/mcp login)`
2. `orochi mcp login remote --no-browser`, driven the way a person over SSH would drive it: the
   printed URL was opened by a "browser" that did not follow the redirect, so nothing reached
   the loopback socket, and the address it ended up at was pasted back → `Signed in to remote.`
3. The token was in the login keychain (`svce="orochi-mcp"`, `acct="f0ea…/remote"`), and there
   was **no** `credentials.json` in the data directory
4. `orochi mcp` → `remote  signed in, 59 minutes left`
5. One run on Claude Haiku asked for the remote server's tool. The reply was `OAUTH-3F355AF1`,
   and what the server saw was:

   ```
   initialize | token: token-1        (Orochi's own probe, from step 4)
   server/discover | token: token-1   (the agent from here on)
   initialize | token: token-1
   notifications/initialized | token: token-1
   tools/list | token: token-1
   tools/call | token: token-1
   ```

6. The keychain item was aged past its expiry in place. The next `orochi mcp` refreshed it:
   `token-1` → `token-2`, written back to the same item (the keychain's update path)
7. `orochi mcp logout remote` removed the item (`security` exit 44: not found), and nothing named
   `orochi-mcp` was left in the keychain

After the discovery fixes in §6, steps 1–4 and 7 were run again against an issuer under a path
whose metadata is only where OpenID Connect appends to it. Same result.

The `security -i` line limit was measured before the keychain code was written: a 4089-byte line
went through and a 4099-byte one was split and its remainder run as another command. Hence one
item per server, and argv only past 4000 bytes, as Claude Code does.

## 5. Real hosted MCP servers, read-only

Six public remote MCP servers were configured and listed with `orochi mcp`, whose probe is an
unauthenticated `initialize`:

```
linear     not signed in (/mcp login)      https://mcp.linear.app/mcp
notion     not signed in (/mcp login)      https://mcp.notion.com/mcp
sentry     not signed in (/mcp login)      https://mcp.sentry.dev/mcp
atlassian  not signed in (/mcp login)      https://mcp.atlassian.com/v1/sse
stripe     not signed in (/mcp login)      https://mcp.stripe.com/
vercel     not signed in (/mcp login)      https://mcp.vercel.com/
```

Their metadata was then fetched with plain GETs at the URLs Orochi's discovery computes. All six
publish authorization server metadata where Orochi looks, each with a `registration_endpoint`
and `S256`. Five name their protected resource metadata in the `401` (Atlassian does not, and is
found at the root). Stripe's issuer has a path (`https://access.stripe.com/mcp`), and its
metadata is at the RFC 8414 insertion, the first place Orochi tries.

Nothing was registered at any provider and nobody signed in. Dynamic registration creates an
OAuth client at the provider, and signing in needs an account and a person at a browser. That
step is the remaining one: `orochi mcp login <name>` against a provider you use.

A probe with Python's `urllib` got something other than `401` from Notion, Sentry and Atlassian,
which serve an anonymous `Python-urllib` differently. Orochi's client sent no user agent at all
and was answered normally, but it now says `orochi/<version>` rather than depend on that.

## 6. The discovery code against the specification

Reading the probe results against the MCP authorization specification (2025-11-25) found four
things the fixture could not:

| Requirement | Before | Now |
|---|---|---|
| Authorization server metadata, issuer with a path: RFC 8414 insertion, OIDC insertion, OIDC appending (MUST) | Insertion only, then the root, then default endpoints | The specified order; no default endpoints |
| Refuse a server without `code_challenge_methods_supported` (MUST) | Not checked | Refused before anything is registered |
| The `401`'s `scope` outranks `scopes_supported` (SHOULD) | `scopes_supported` only | The challenge's scope first |
| `WWW-Authenticate` parameters | A string split on `resource_metadata=` | Quoted and bare parameters parsed |

Each has a test in `tests/mcp_oauth.rs` against a fixture switch built to fail without it.

## What was found about Claude Code, and applied

Read from Claude Code 2.1.278's documentation and, where the documentation is silent, its
installed binary:

- `.mcp.json` servers are asked about before use in an interactive session, with "Continue
  without" focused and Esc meaning it too. Several new servers come as one checklist. Orochi's
  console does the same (`tests/test_chat_terminal.py`, `RepositoryMcpServers`)
- `-p` and SDK sessions load them without asking. So does a run of Orochi that nobody can be
  asked in, and it says so once per repository
- In a remote server's `url` and `headers` from a project file, `ANTHROPIC_API_KEY`,
  `ANTHROPIC_AUTH_TOKEN`, `AWS_BEARER_TOKEN_BEDROCK`, `HTTPS_PROXY` and `NPM_TOKEN` read as empty.
  Before this, Orochi expanded them, so a repository could have had an agent send them to a
  server it named. It now withholds those and the credentials of the other agents it drives
- Keychain writes go through `security -i` on stdin while the line fits and through argv beyond
  that. Orochi does the same, one item per server

One deliberate difference: Claude Code approves an entry by name, and Orochi approves the entry
as written, so a later rewrite of an approved entry is asked about again.
