# Real Validation: the Sandboxes' `*.localhost` Gateway (2026-10-01)

Reaching sandboxes' services from the Mac with nothing configured on the Mac and no root,
replacing the route and resolver file of [Per-Project Sandboxes](sandbox-design.md) §6.1 at the
user's request. Same machine and VM as [2026-09-30](real-validation-20260930-2.md): Apple M5,
macOS 26.5.2, Lima 2.1.0 (`sbx-host`, 8 GiB), Incus 7.5.1, the golden image of that day.

## What was set up

`orochi sandbox up` alone, on the running VM. It copied `src/sandbox/gateway.py` in and started
two units: `orochi-sandbox-dns` (systemd-resolved sends `~sbx` to the bridge's DNS) and
`orochi-sandbox-gateway` (on the VM's `127.0.0.1:1355`, `DynamicUser`). Lima forwarded the port
to the Mac's `127.0.0.1:1355` by itself. Nothing on the Mac was changed: no route, no
`/etc/resolver` file, no LaunchDaemon, no password asked.

## Results

Two throwaway projects at once: `demo` (container, Supabase started inside as the sandbox's
user) and `web` (container without Docker, `python3 -m http.server 8000` on `0.0.0.0`). No
project was focused. Requests from the Mac with `curl`:

| Address | Result |
|---|---|
| `http://1-nothing.localhost:1355/` | 502 from the gateway (it is reachable; nothing listens there) |
| `http://54323-demo.localhost:1355/` (Supabase Studio) | 307 to `/project/default` |
| `http://54321-demo.localhost:1355/rest/v1/` (Supabase API) | 200, the PostgREST description |
| `http://8000-web.localhost:1355/` (the other project) | 200, the page served inside `web` |
| `http://54321-demo.localhost:1355/realtime/v1/websocket?…` with `Upgrade: websocket` | `101 Switching Protocols` |

`orochi sandbox ports` in `demo` listed 54321–54324 and 54327, each with its address.

## Not verified

- A browser. Only `curl` was used; browsers resolving `*.localhost` to loopback is their
  documented behavior (RFC 6761), not measured here.
- A service bound to `127.0.0.1` inside a sandbox is not reachable through the gateway by
  design; `focus` is the path for it, and for anything that is not HTTP.
