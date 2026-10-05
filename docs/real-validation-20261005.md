# Real validation: session workspaces (2026-10-05)

The session workspace implementation was run on the existing `sbx-host` Lima VM, with real
Incus containers created from the existing `sbx-golden` image. The ACP agents were the Python
validation fixture, not real provider agents. No provider credentials or paid model calls
were needed. The Incus client, mounts, Linux processes and Docker daemons were real.

The disposable Git repository and its linked source worktree live under
`.orochi/live-validation-20261005/`. Orochi used a private config and data directory there;
the repository's normal configuration and activity database were not changed. Every worker
recorded its actual hostname, `OROCHI_SANDBOX`, cwd, uid, Git common directory and launch time.

## Results

| Check | Observed result |
|---|---|
| Two writable workers in one conversation | Both `start_agent(write: true)` requests succeeded through the container-side MCP relay |
| One container per conversation | Both workers reported the same container name and kernel hostname; the lead and all seats used that conversation instance |
| Separate workspaces | `alice` and `bob` ran in different Git worktrees and retained separate result files; neither result was written into the source worktree |
| Actual overlap | Start times differed by 1.026 s in the first successful session and 0.972 s in the second; both workers remained active for at least 5 s |
| Separate conversations | The two conversations reported different container names and hostnames |
| Dirty starting state | All four workers read the source's uncommitted `dirty launch state` content |
| Git metadata outside the source worktree | Inside Linux, `git rev-parse --git-common-dir` resolved to the original repository's `.git`, mounted outside the linked source worktree |
| Source index preservation | The linked source worktree's index was byte-identical before and after the two runs |
| Mapped ownership | Workers ran as uid 501, matching the host user |
| Resume | `--thread` reopened the second conversation, `/workspaces` listed its retained results, and every newly launched ACP process used its existing container; no new sandbox was registered |
| Docker available | `docker info` completed inside both conversation containers. The golden image's Docker daemon ID was cloned, so that ID alone is not evidence of state isolation |
| Cleanup | All five containers registered in the private validation state, including earlier trial runs, were removed. Worktrees, manifests and evidence files were retained; the now-empty VM was returned to its original stopped state |

Evidence is retained in the ignored validation directory: `run.py`, `finalize.py`,
`proofs-1.json`, `proofs-2.json`, `summary.json`, `chat-*.stderr` and `resume.stderr`.

## Problems found and fixed

The existing sandbox path deliberately withheld the mailbox MCP server because its executable
was the host's macOS Orochi binary. This meant a container-side lead could not call
`start_agent`, despite the worktree and instance ownership tests passing with a fake Incus.

A portable Python stdio endpoint now transports requests and replies through a private
per-peer directory in the existing workspace mount. The host uses the same mailbox RPC
handler as the normal stdio server; SQLite and permission/routing decisions remain on the
host. No network listener, extra port or Linux Orochi binary is required. The relay closes
when its ACP peer is dropped. Peer registration also uses each ACP session's actual cwd,
rather than the host process's cwd, so worker paths in the mailbox are accurate.

An automated regression test now drives a sandboxed lead through this relay, starts two
writable helpers, checks their workspace paths and verifies all launches use one instance.
The mailbox, sandbox and workspace suites passed (65 tests), as did Clippy with warnings
rejected.

The fixture's tiny token usage caused a later run's helper requests to be refused by the
normal cost gate. For the controlled validation the minimum work threshold was set to 1;
production routing behavior was not changed.

## Remaining live coverage

Real provider agents, the VM tier and runner mode's full session lifecycle were not exercised
in this live run. Docker commands were checked, but cross-container Docker state isolation
was not directly measured with a marker object; a follow-up probe ran after cleanup and the
instance had already been removed.
## Environment initialization

`orochi init` was exercised against the existing `sbx-host` Lima VM on this Mac.
Two consecutive runs completed, reusing installed Lima, the prepared Incus project and
the `sbx-golden` container image, without rebuilding the image. The VM was returned to its
original stopped state. Logs are retained under the ignored
`.orochi/live-validation-20261005/init-{1,2}.{stdout,stderr}` paths.

The first trial failed in the host setup script without a useful diagnostic. A subsequent
trial passed twice; the original failure was not reproduced or conclusively diagnosed.
The host script now waits for Incus readiness after restarting the daemon and reports the
line of an unexpected script failure.

The missing-Lima installation branch was verified using a simulated Homebrew executable:
installation, reuse, nonzero exit, missing post-install executable and missing Homebrew.
This Mac already had Lima and Incus installed, so fresh package installation was not
verified on a clean machine. The desktop's setup job, progress display and disabling
conflicting operations were covered by its UI tests.

## Linux bootstrap and Windows launcher

The Linux host scripts were run in a disposable privileged Ubuntu guest nested in the real
Incus VM. Incus was initially absent. The scripts installed the packages and prepared its
project, profiles, directory storage and HTTP gateway. A normal `dev` user could immediately
use the installed `orochi-incus` wrapper, without a new login or sudo exemption. That user
launched a real nested Ubuntu container; the wrapper also passed multiline `raw.idmap`
configuration successfully, and `incus exec` returned the container's `smoke` hostname.
No host poweroff timer was installed. The disposable guest and nested container were
removed and the outer Lima VM was stopped again. Evidence is retained in the ignored
`linux-setup.py` and `linux-setup.log` files.

Trials exposed unavailable loop devices and absent systemd net-device units in the nested
environment. The final host script selects directory storage when loop devices cannot be
used, and orders DNS setup after Incus rather than requiring a net-device unit. This is a
Linux bootstrap/runtime smoke test, not validation of every distribution, the Linux source
installer's full build, or Linux desktop rendering.

The Windows launcher was exercised using official portable PowerShell 7.5.4 on macOS with
a simulated `wsl.exe`. Tests cover absent/existing distributions, setup and desktop flags,
Windows path translation, literal task text, pass-through CLI flags and failure propagation.
A Windows CI job runs the same tests. Actual WSL2 installation, Windows networking and WSLg
desktop rendering remain unverified because no Windows machine was available.
