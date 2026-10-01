# Real Validation: Runner Mode (2026-10-01, 4)

The agent on this machine, the project's runtime in its sandbox, chosen by the user as the
arrangement with the fewest conflicts ([Per-Project Sandboxes](sandbox-design.md) §15). Same
machine and VM as [the gateway's record](real-validation-20261001.md).

## Results

The `orochi-sandbox` MCP server (`orochi --internal-sandbox-runner`) driven over stdio, as an
agent drives it, in a project created with `orochi sandbox create --mode runner`. No agent
prompt was sent.

| Tool | Result |
|---|---|
| `sandbox_exec` | `docker version` answered 29.8.1 inside; the host name was the sandbox's (`runner`) and the user `dev`; exit code and output came back |
| `sandbox_start` | a web server (`python3 -m http.server 8000`) started in the background and kept running after the call |
| `sandbox_logs` | its output and `(running)` |
| `sandbox_ports` | `[{"port": 8000, "url": "http://8000-runner.localhost:1355"}]` |
| from the Mac | `http://8000-runner.localhost:1355/` served the project's page through the gateway |
| `sandbox_stop` | the server ended (`(not running)`) |

## Not verified

- A real agent choosing these tools over its own shell, guided by the note. The note says
  which tools to use; nothing stops an agent's own shell from running a command on this
  machine.
