# Real Validation: Per-Project Sandboxes (2026-09-30, 2)

What [Per-Project Sandboxes](sandbox-design.md) does on a real machine, measured the day it was
built. Everything before this ran against a fake `incus` only.

## Environment

| | |
|---|---|
| Mac | Apple M5, 10 cores, 32 GiB, macOS 26.5.2 |
| Lima | 2.1.0, `vmType: vz`, instance `sbx-host`: 6 CPU, **8 GiB** (the user's choice over 16), 100 GiB disk |
| VM | Ubuntu 24.04, kernel 6.8.0-142, ZFS 2.2.2 |
| Incus | 7.5.1 (Zabbly stable), pool `sbx-pool` = ZFS on a 60 GiB sparse file |
| Image | Ubuntu 24.04 container, Docker 29.8.1, Node 22.23.3, the Claude Code / Codex CLIs, the pinned ACP bridges, the Supabase CLI and its images pulled |
| Projects | throwaway directories `~/orochi-sbx-validation/{demo,second}`, removed afterwards |
| Final image | rebuilt after every fix below, with nothing patched by hand; the last rows were measured on it |

## Results

| Question (design §12, §15) | Result |
|---|---|
| Does `orochi sandbox up` make a working host? | **Yes**, after the fixes below. VM created and booted in 6.6 min including the Ubuntu image download; the Incus/ZFS setup then runs to completion and again idempotently |
| Can the golden image be built, with Supabase's images pulled inside a nested container? | **Yes.** `supabase start` ran to completion inside the build container (Docker inside an unprivileged Incus container with `security.nesting` and the mknod/setxattr intercepts). About 10 min per build; the image is 6.32 GiB in the pool |
| What storage does Docker use on ZFS? | Docker 29's containerd image store with the `overlayfs` snapshotter, on the instance's ZFS root. It works; see *Docker's data* below |
| How long does creating a sandbox take? | **87 s** for the first instance from a new image (Incus unpacks the image into the pool once), **4.2 s** for the next. `image build` now makes and deletes one instance so the unpack happens there: from the final image the first sandbox took **3.9 s** (the build, warm-up included, about 13 min) |
| Snapshot and restore | `orochi sandbox snapshot --name clean` then `restore clean` on a sandbox with Supabase's data: both succeed, the project's files untouched |
| Is the tree at the same path, and are files the user's? | **Yes.** Inside, `pwd` is the Mac path; a file written inside as uid 501 is `titabash:staff` on the Mac; shadowed `node_modules` is owned by the sandbox user |
| Does `supabase start` work inside a project's sandbox, as the agent's user? | **Yes**, 12 containers, from the pre-pulled images (nothing downloaded) |
| Memory | The demo sandbox with Supabase up: **2.03 GiB**; an idle sandbox: **145 MiB**. Largest container: analytics, 554 MiB |
| Does focus put the sandbox's ports on the Mac's `127.0.0.1`? | **Yes.** From the Mac, `127.0.0.1:54323` (Studio) answered 307 and `127.0.0.1:54321` (the gateway) answered, both served inside the sandbox |
| Does ACP survive Orochi → supervisor → `limactl shell` → `sudo incus exec` → `sbx-run` → agent? | **Yes.** A full `orochi "task"` with the fixture agent inside: session, prompt, reply, file written, **check run inside** (`uid 501 host demo`), outcome `success`, 1.8 s in all |
| When Orochi kills an agent, does it die inside? | **Not at first**: `incus exec` leaves the process running when its client is killed (a `sleep` was still there 12 s later), and `setpriv --pdeathsig` did not help. A process that reads stdin (every ACP agent) ended within 2 s. After the fix below, a killed check, a killed agent with a background child, and a command that exits leaving a child are all gone within 1.5 s, and exit codes pass through |

## Found on the real machine, and fixed

| Finding | Fix |
|---|---|
| `incus network create` read the rest of the setup script as a YAML configuration: Incus reads stdin when it is not a terminal, and the script arrived on stdin (`bash -s`) | Scripts are copied in as files and run with stdin closed (host setup and image build) |
| Lima's default rootless containerd ran in the VM, unused | `containerd: {system: false, user: false}` in the template |
| The VM's user could not reach Incus: joining `incus-admin` applies at a new login and Lima keeps its SSH connection | The client runs `sudo incus` in the VM |
| Docker refused the sandbox user: `incus exec --user --group` gives one group and no supplementary ones, so membership of `docker` never applies | A `docker.socket` drop-in gives the socket the user's own group (`daemon.json` is ignored under socket activation) |
| The per-project Docker volume stayed empty: Docker 29 keeps images under `/var/lib/containerd`, on the root disk | The volume is gone. Docker's data stays on the root disk, which is a ZFS clone of the image, so pre-pulled images are shared copy-on-write, snapshots cover them, and `sandbox.quota_gib` (30) bounds the root disk |
| ZFS's cache took 2.85 GiB (limit 3.87 GiB) of the 8 GiB VM; the VM read 5.65 GiB used with one Supabase stack | ARC capped at 1 GiB: 3.74 GiB used, 4.18 GiB available with the same load |
| Killing the Mac side left processes inside (above) | `sbx-run` is a small supervisor: the command runs in its own process group, which is killed when the command ends; for checks Orochi holds stdin open (`SBX_HOLD=1`) and its end kills the group |

## Not verified

- **`<name>.sbx` from the Mac** (`orochi sandbox network --apply`): it changes routing and needs an
  administrator's password, which was not asked for during an unattended run.
- **Real agents inside**: Claude Code and Codex need a login inside (`orochi sandbox enter`, then
  `claude` / `codex`), which is interactive. The ACP path was proven with the fixture agent.
- **The VM tier** (`--mode vm`, nested virtualization under Lima on the M5).
- **The desktop window** against the live VM.
