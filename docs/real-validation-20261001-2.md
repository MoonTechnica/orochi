# Real Validation: the Sandbox VM Stopping and Starting Itself (2026-10-01, 2)

The VM powers itself off when unused and Orochi starts it when something needs it, asked for by
the user the same day ([Per-Project Sandboxes](sandbox-design.md) §15). Same machine and VM as
[the gateway's record](real-validation-20261001.md). To finish in minutes, the runs used
`vm_idle_minutes = 2` through a temporary configuration; the default (30) was put back after.

*Use* is judged by the agent's processes, at the user's direction: any process in any sandbox
carrying `OROCHI_SANDBOX` (Orochi sets it on everything it starts there; every descendant
inherits it), an Incus operation in progress, or a connection from the Mac through the gateway
or a focused port. An earlier version judged by Incus exec operations only; the user rejected
it because an agent's background work would not have counted.

## Results

| Question | Result |
|---|---|
| Does a command running inside keep the VM up past the idle time? | **Yes.** A 210 s command in a sandbox: still running 60, 120 and 180 s in |
| **Does work an agent left running after its session ended keep the VM up?** | **Yes.** An agent session that started `nohup sleep 200 &` and ended: `status` said `in use: agent work in bg (sleep)` at every look until the work ended at 03:14:20, then `unused for 1 min`, and the VM powered itself off at about 03:16. Confirmed again on the rebuilt image (`in use: agent work in final (sleep)` 65 s after the session ended) |
| Is that work left alone when the agent's session ends? | **Yes**, after a fix found here: `sbx-run` killed its process group when the agent exited, taking plain `&` background work with it (detached `setsid` work survived). It now kills its group only for a check Orochi stopped; a killed check is still gone within 1.5 s |
| Does a long Incus operation keep it up? | **Yes**, after a fix found here: an image build (23 min, including publishing and pre-unpacking) ran to completion with the idle time at 2 min; `status` showed `in use: Incus: …` during it. Before the fix only processes counted, and a build would have been cut |
| Is a connection from the Mac through the gateway seen as use? | **Yes**, with a connection held open to `127.0.0.1:1355` |
| Does the VM power itself off once unused? | **Yes.** The idle time counts from the last minutely check that saw use, so it can stop up to a minute before the full idle time since the last use has passed |
| Does looking at sandboxes start it? | **No.** `orochi sandbox status` printed `stopped (starts when needed)` and the VM stayed stopped |
| Does using it start it? | **Yes.** `orochi sandbox ports` started the VM and answered in **15.2 s** in all; the sandbox inside was running again with it |

## Not verified

- A full 30-minute idle period at the default.
- Docker containers an agent starts (`docker compose up -d`) are the Docker daemon's children,
  not the agent's, so they do not carry the mark and are not use by themselves; a Supabase
  stack left idling is deliberately not use.
- A browser tab left open: whether its idle keep-alive connection is held long enough to count
  is the browser's choice.
