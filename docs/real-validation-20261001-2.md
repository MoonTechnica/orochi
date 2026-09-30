# Real Validation: the Sandbox VM Stopping and Starting Itself (2026-10-01, 2)

The VM powers itself off when unused and Orochi starts it when something needs it, asked for by
the user the same day ([Per-Project Sandboxes](sandbox-design.md) §15). Same machine and VM as
[the gateway's record](real-validation-20261001.md). To finish in minutes, the run used
`vm_idle_minutes = 2` through a temporary configuration; the default (30) was put back after.

## Results

| Question | Result |
|---|---|
| Does the idle check read what Incus reports? | `/1.0/operations?recursion=1&all-projects=true` lists a command running in a sandbox as `("Executing command", "websocket", "Running")`, which the check counts as use |
| Does a command running inside keep the VM up past the idle time? | **Yes.** A 210 s `sleep` in a sandbox (as an agent or a check runs): the VM was still running 60, 120 and 180 s in, past the 2 min idle time |
| Does the VM power itself off once unused? | **Yes**, at 02:20:58, about 70 s after the command ended at 02:19:48. The idle time counts from the last minutely check that saw use, so the VM can stop up to a minute before the full idle time since the last use has passed |
| Does looking at sandboxes start it? | **No.** `orochi sandbox status` printed `stopped (starts when needed)` and the VM stayed stopped |
| Does using it start it? | **Yes.** `orochi sandbox ports` in the project started the VM (`Starting the sandbox VM sbx-host…`) and answered in **15.2 s** in all; the sandbox inside was running again with it |
| Is a connection from the Mac through the gateway seen as use? | **Yes.** With a connection held open to `127.0.0.1:1355` from the Mac, the check's own detection returned true for the gateway, and false for exec (nothing was running) |

## Not verified

- A full 30-minute idle period at the default.
- A browser tab left open on a sandbox's page: whether its idle keep-alive connection is
  held long enough to count as use is the browser's choice.
