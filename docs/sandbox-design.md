# Per-Project Sandboxes: Design

Written: 2026-09-30.

**Status (2026-09-30): S1–S3 implemented, with focus (S2), snapshots and idle stop (S5 in part)
and the desktop (S6). Verified on a real Lima VM the same day** for the host, the image, Docker
and Supabase inside a project's sandbox, focus, and a full ACP run with the fixture agent
([Real Validation 2026-09-30](real-validation-20260930-2.md)); `<name>.sbx`, real agents inside,
the VM tier and the desktop against a live VM are **unverified**. §15 says where the build
departed from this design, several times because of what that run found.

One sentence: **each project gets its own Linux sandbox with a Docker daemon of its own inside
it, the sandbox can be switched off per project so the same project runs on the Mac as it does
today, and anything a sandbox listens on is reachable from the Mac by the project's name and,
for the project in focus, at the very `127.0.0.1:port` its tools print.**

## 0. Sources

| Source | Gives | Where |
|---|---|---|
| This machine (read 2026-09-30) | Apple M5, 10 cores, 32 GiB, macOS 26.5.2; Lima 2.1.0; OrbStack 2.2.3 (its Docker 29.4.0 is the Mac's `docker`); no `incus`, no `colima`; **19 GiB free of 926** on the data volume; an existing Lima instance `incus-host` (vz, 4 CPU / 8 GiB, 57 GiB on disk, stopped) and an OrbStack machine of the same name (19.8 GB, stopped) | §1, §7, §9 |
| `~/Development/vibe-engine` | The user's own measured notes: `docs/_research/2026-06-11-incus-proxy-lima-portforward.md` (proxy device reaches a service bound to the instance's loopback; Lima forwards any guest listener on 127.0.0.1/0.0.0.0 to the Mac's 127.0.0.1, first matching rule wins, hot-plug works), `2026-06-26-lima-vznat-networking.md` (vzNAT's IP comes from macOS `bootpd`, stable across stop/start, not across recreate, occasionally fails at boot — Lima issue #3100; self-heal only from a `mode: boot` provision), `2026-06-16-mutagen-sync.md`, and `lima/incus-host.yaml` (Incus on Ubuntu 24.04 in a vz VM with vzNAT, HTTPS API on 8443, a `workspaces` Incus project, `dir` pool) | §5, §6, §8 |
| [Incus FAQ](https://linuxcontainers.org/incus/docs/main/faq/), [proxy device](https://linuxcontainers.org/incus/docs/main/reference/devices_proxy/), [disk device](https://linuxcontainers.org/incus/docs/main/reference/devices_disk/), [storage drivers](https://linuxcontainers.org/incus/docs/main/reference/storage_drivers/), [forum: Docker with Incus](https://discuss.linuxcontainers.org/t/using-docker-with-incus/19801) | Docker inside a container needs `security.nesting=true`; the maintainers' full recipe adds `security.syscalls.intercept.mknod=true` and `security.syscalls.intercept.setxattr=true`, never `security.privileged`; on ZFS, `zfs.delegate=true` on the volume; kernel modules must be loaded by the host; `dir` is "a last resort" (no instant clone, no snapshots); `shift=true` depends on the filesystem supporting idmapped mounts | §4, §5 |
| [Lima `templates/default.yaml`](https://github.com/lima-vm/lima/blob/master/templates/default.yaml), [port forwarding](https://lima-vm.io/docs/config/port/), [vmnet](https://lima-vm.io/docs/config/network/vmnet/), [disks](https://lima-vm.io/docs/config/disk/) | `nestedVirtualization` needs `vz` on Apple M3+ and macOS 15+; `vzNAT` needs no root and gives an IP the Mac can reach; `additionalDisks` outlive the instance; `guestSocket`/`hostSocket` forwarding; `virtiofs` is the vz default mount | §4, §6 |
| [Colima README / FAQ](https://github.com/abiosoft/colima) | `colima start --runtime incus` exists (v0.7+); "Incus containers and virtual machines are not reachable from the host by default", `--network-address` needed | §3 |
| [OrbStack docs](https://docs.orbstack.dev/architecture), [machines/network](https://docs.orbstack.dev/machines/network) | Machines "share one kernel … not a substitute for a full VM"; one Docker engine beside the machines; each machine has an IP and `name.orb.local` | §3 |
| [Supabase local development](https://supabase.com/docs/guides/local-development/cli/getting-started) | The stack is Docker containers; Studio 54323, API 54321, DB 54322, Mailpit 54324, printed as `127.0.0.1`; the analytics server mounts `/var/run/docker.sock` | §1, §6 |
| Linux 6.12 release ([Phoronix](https://www.phoronix.com/news/Linux-6.12-FUSE)) | FUSE/virtiofs idmapped mounts arrived in 6.12; Ubuntu 24.04's GA kernel is 6.8 | §5 |
| Orochi at `df0c96f` | `discovery::prepare` builds every agent's launch command; `acp::Client` speaks ACP over the child's stdio; `--internal-mailbox` is a stdio MCP server the agent spawns; `--internal-supervise` leads the agent's process group; `evaluator::evaluate` runs checks in the working directory | §8 |

## 1. Requirements

The request, in the user's words (2026-09-30): a separate sandbox per project, "Incus or
whatever"; **absolutely** a Docker daemon must start inside it so container processes such as
Supabase run there; whether to use the container must be switchable; services started inside
must be properly reachable.

| ID | Requirement | Kind |
|---|---|---|
| R1 | **One sandbox per project**, isolated from the Mac and from every other project: files, processes, network namespace, resource limits | must |
| R2 | **A Docker daemon runs inside the sandbox.** `supabase start` works there unchanged: about a dozen containers, the analytics container's bind mount of `/var/run/docker.sock`, the default ports | must |
| R3 | **Sandbox on or off per project**, switchable later, with the same commands either way. Off means what happens today: the agent and its tools run on the Mac | must |
| R4 | **Services started inside are reachable from the Mac**: browser, `curl`, IDE, Orochi's evaluator. By a stable name per project, with no port collisions between projects; and for the project being worked on, at the `127.0.0.1:port` its tools print | must |
| R5 | **The same working tree in both modes.** Switching a project between sandbox and host moves no files and loses no branch | must |
| R6 | **Coding agents run inside** (Claude Code, Codex, their ACP adapters) with their own authentication, and Orochi drives them as it does today | must |
| R7 | **Cheap.** Creating a sandbox takes seconds from a prepared image; an idle sandbox costs nothing; images and pulled containers are shared copy-on-write, because the disk has 19 GiB free today | must |
| R8 | **Disposable.** A sandbox a bad run wrecked is reset or rolled back to a snapshot in seconds; the project's files are untouched by that | should |
| R9 | **No root in daily use.** Root at most once, at install (a route and a resolver file); no sudo prompts per project or per boot | should |
| R10 | **Remote-ready.** The same design works when the sandbox host is a Linux machine elsewhere, because vibe-engine already runs the same shape there | could |
| R11 | Docker inside is itself optional per project (a project without containers gets a tighter, cheaper sandbox) | could |

**How R3 is read.** "コンテナを使うか使わないか" is taken as *whether the project runs inside the
sandbox at all* — the user calls the sandbox "the container", as the next sentence ("services
started inside the container") shows. The other reading, *whether Docker is available inside*,
costs one profile flag and is R11. Both are built.

Non-goals: hostile multi-tenant isolation (this is one person's machine and their own agents);
exposing sandboxes to the LAN; Windows hosts (a Linux host runs Incus natively and skips §4's
first layer); GPU passthrough.

### 1.1 Threat model

What the sandbox protects: the Mac's home directory and keychain, the other projects, and the
Mac's own services, against an agent that runs commands with too little review, a dependency
script that does more than it says, or a `docker run` that mounts `/`. It is **not** a defence
against a deliberate kernel exploit hopping between projects; for a project that needs that, the
VM tier (§4.3) exists. Two boundaries, then: a **VM** between the Mac and everything Linux, and a
**container** between projects. Credentials the agent needs are, by necessity, inside (§8.3).

## 2. Decisions (2026-09-30)

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | The stack | **Lima (vz) → one Linux VM → Incus → one system container per project, with `security.nesting`; an Incus VM per project as the optional stronger tier** | §3: density and instant copy-on-write clones on a 32 GiB / 19-GiB-free Mac, both tiers under one API that is also remote (R10), and the user already has the measured notes and a template for exactly this shape |
| D2 | Where the working tree lives | **On the Mac, as today**, mounted into the VM (virtiofs) and into the sandbox (disk device); build products are shadowed by sandbox-local volumes (§5.2). Mutagen sync is the fallback for a project where virtiofs is too slow | R5: switching modes then moves nothing. The user's own mutagen notes make the fallback cheap |
| D3 | UID mapping | **`raw.idmap` maps the Mac user's UID (501) both ways**; `shift=true` is not relied on | Idmapped mounts on virtiofs need Linux 6.12; Ubuntu 24.04 boots 6.8 |
| D4 | Storage | **ZFS pool on a dedicated Lima disk**; Docker's `/var/lib/docker` is a per-project volume with `zfs.delegate=true`, cloned from the golden volume | Instant clone and snapshot (`dir` has neither); ZFS is the path the Incus maintainers document for Docker inside; a golden `/var/lib/docker` with Supabase already pulled is shared copy-on-write. **Unverified here** — §12 S1 measures it and names the fallback |
| D5 | Reachability | **Two layers**: by name (`<project>.sbx:<port>`, a route to the Incus bridge plus a resolver file — root once) and **focus** (the one focused project's ports at the Mac's `127.0.0.1`, via Incus proxy devices Lima forwards) | R4 both halves; the proxy device is the one path that reaches a service bound to the sandbox's own loopback (vibe-engine note, 2026-06-11) |
| D6 | The channel between Mac and sandbox | **Every channel is the stdio of an `incus exec`**: the agent's ACP, the mailbox relay, the port watcher | One mechanism, hot-pluggable, and identical for a local Lima host and a remote Incus host |
| D7 | Where Orochi runs | **On the Mac, unchanged.** Only the agent process, its tools and the evaluator's checks move inside | Telemetry, activity and memory stay where they are; nothing new is transmitted |
| D8 | The sandbox as an Orochi backend | **A `Sandbox` trait with `host`, `incus-container`, `incus-vm`**; per-project mode in `<data>/sandbox/projects.json`, `--sandbox` per run | R3 with one command surface |
| D9 | Docker in host mode | **OrbStack stays** the Mac's Docker for projects with the sandbox off | Already installed and in use; nothing to change |
| D10 | Disk | **Refuse to create a sandbox under 10 GiB free** and say what to reclaim | 19 GiB free today; the old `incus-host` Lima disk (57 GiB) and OrbStack machine (19.8 GB) are the obvious candidates, and the user decides |

## 3. Options compared

| Option | Docker inside | Isolation between projects | Density on this Mac | Create time | Reach from Mac | Verdict |
|---|---|---|---|---|---|---|
| A. One Lima VM per project | Native | VM | Poor: memory pinned per VM, 5+ VMs strain 32 GiB | Minutes (cloud-init each time; no clone on vz) | Lima forwards to `127.0.0.1`; two projects on 54321 collide | Right for one or two projects; not per project |
| **B. One Lima VM + Incus containers (+ VMs)** | Nesting (documented) | Container; VM tier where wanted (M5 + macOS 26 allow nested virtualization) | Good: shared kernel, clone from a golden image, one memory budget | Seconds (ZFS clone) | Bridge IP + name; proxy devices for focus | **Chosen** |
| C. OrbStack machines per project | A second engine inside a machine is neither documented nor forbidden; OrbStack's model is one engine beside the machines | Machines share one kernel, by OrbStack's own statement | Good | Seconds | Excellent: own IP, `name.orb.local`, ports on localhost | Closest in ergonomics; proprietary, Mac-only, no remote form, and Docker-inside is off its documented path |
| D. Docker-in-Docker (privileged) per project | Yes | Weakest: a privileged container is root on the VM | Good | Seconds | Port maps | Rejected: no boundary between projects |
| E. Apple `container` (macOS 26) | Undocumented; each container is its own VM | VM | Unknown | Fast | Own IP | Watch; too young |
| F. Colima `--runtime incus` | Same as B | Same as B | Same as B | Same as B | Needs `--network-address` (root, slower start) | B packaged with fewer knobs; keep as the "if Lima becomes a chore" exit |

Why B over C, since C is installed and pleasant: the boundary the design rests on is a VM
around everything Linux and a container per project; C offers only the second and only by its
own account "for most intents and purposes". And R10: the user's other product runs Incus on a
Linux host, and the `incus` client makes a remote daemon a one-line remote.

## 4. Architecture

```
Mac ─ Orochi / Orca / browser / git / OrbStack (host-mode Docker)
 │ virtiofs (~/Development, ~/orca/workspaces) · vzNAT · Lima port forwards
 └─ Lima VM  sbx-host   (Ubuntu 24.04, vz, nestedVirtualization, 6 CPU / 8 GiB, ZFS disk)
     └─ Incus            project "sbx" · profiles sbx-base, sbx-docker, sbx-vm · image sbx-golden
         ├─ container  ficchat      ← docker inside: supabase (12 containers), next dev
         ├─ container  opusia       ← docker inside
         └─ vm         untrusted    ← tier=vm
```

### 4.1 The host VM

One Lima instance, `sbx-host`, forked from `vibe-engine/lima/incus-host.yaml` with these
changes: an `additionalDisks: [sbx-pool]` (raw, formatted by Incus as ZFS — survives the
instance), `nestedVirtualization: true`, the mounts narrowed to the directories projects live
in, a `mode: boot` provision that waits for `lima0`'s IPv4 and reconfigures it when `bootpd` was
late (the fix the vzNAT note arrived at), kernel modules Docker needs (`overlay`,
`br_netfilter`, `ip_tables`, `nf_nat`, `xt_conntrack`, `veth`) in `/etc/modules-load.d/`, and
`root:501:1` in `/etc/subuid` and `/etc/subgid` so `raw.idmap` may name the Mac user's UID. Sizing
is a config value; 6 of 10 cores and 8 of 32 GiB is the starting point (the user chose 8 over 16 on 2026-09-30); the
VM takes memory as it touches it, but Linux keeps it as cache, so `orochi sandbox down` exists.

### 4.2 The container tier (default)

Profile `sbx-docker`, applied over `sbx-base`:

```yaml
config:
  security.nesting: "true"
  security.syscalls.intercept.mknod: "true"
  security.syscalls.intercept.setxattr: "true"
  raw.idmap: "both 501 501"
  limits.cpu: "4"
  limits.memory: "6GiB"
  limits.memory.enforce: "soft"
devices:
  docker:  {type: disk, pool: sbx-pool, source: docker-<project>, path: /var/lib/docker}
```

`security.privileged` is never set; the forum thread and the maintainers say it "will break
things in very odd ways" with nesting. A project with R11 off gets `sbx-base` only: no nesting,
no intercepts, no Docker volume.

### 4.3 The VM tier

`incus launch sbx-golden-vm <project> --vm` on the same host. Docker inside a VM needs
nothing special; the price is a fixed memory reservation and a slower start, and the workspace
arrives by a second virtiofs hop (Incus exports the VM's directory to its guest). Nested
virtualization under Lima on this M5 is **unverified** (§12 S5); until then the VM tier is a
documented option, not an offer.

## 5. Files

### 5.1 The working tree

The Mac's directory is the truth (D2). Chain: Mac → VM by Lima's virtiofs mount (the same
path, so `/Users/titabash/orca/workspaces/orochi/turban` is that inside the VM) → sandbox by an
Incus `disk` device at `/work`. With `raw.idmap: both 501 501`, files the sandbox's user writes
are UID 501 in the VM and therefore the user's on the Mac; no `shift`, no chown.

Known costs, to be measured (§12 S1): virtiofs is slower than local disk for trees with many
small files, and inotify does not cross it unless Lima's experimental `mountInotify` is on. A
project where that hurts switches to **sync mode**: a sandbox-local clone at `/work` kept
two-way with mutagen, exactly as vibe-engine's 2026-06-16 note lays out (labels per workspace,
ignores for build products). Sync mode still satisfies R5 because git is the arbiter either way.

### 5.2 Shadowed directories

Build products must not cross the boundary: `node_modules`, `target`, `.next`, `.venv`,
`.turbo`, `dist` are **shadowed** — a sandbox-local ZFS volume is mounted over each path inside
the sandbox, so `bun install` inside writes locally and the Mac's copy (if any) is untouched.
The list is per project (`shadow = [...]` in the project's sandbox entry), seeded from the
project's language. Switching a project to host mode therefore means installing dependencies on
the Mac again, as moving to another machine would — expected, and said in the switch's output.

### 5.3 Docker's data

`/var/lib/docker` is a per-project ZFS volume with `zfs.delegate=true`, **cloned from the golden
volume** in which Supabase's images were already pulled. So a new project's `supabase start`
pulls nothing, and twelve projects share those gigabytes copy-on-write until a layer diverges.
Each volume carries a quota (`docker_quota_gib`, 30 by default) so one project's image sprawl
cannot fill the pool. Whether Docker inside then chooses `overlay2` over ZFS or its `zfs` driver
is measured in S1; if neither is sound, the fallback is a per-project ext4 image on the pool
disk (reflink-cloned on XFS), which is Docker's best-trodden ground, at the cost of a second
storage mechanism.

### 5.4 The golden image

Built by `orochi sandbox image build`, from Ubuntu 24.04: Docker Engine, the CLIs the agents
need (`claude`, `codex`, `claude-agent-acp`, `codex-acp`, node, bun, deno, uv, the Supabase CLI,
git, ripgrep), a Linux `orochi` binary (§8.2), the port watcher, and a run of `supabase start`
followed by `supabase stop --no-backup` so the Docker volume holds the images; published as the
Incus image alias `sbx-golden` plus the volume `docker-golden`. An image update rebuilds both;
existing sandboxes keep theirs until `orochi sandbox reset <project>` re-clones. No credential
and no project file ever enters the image.

## 6. Reaching services (R4)

Inside the VM, Incus's bridge `incusbr0` gives every sandbox an address and its dnsmasq answers
`<name>.incus`. Two layers put that on the Mac.

### 6.1 By name: `http://ficchat.sbx:54323`

One route on the Mac for the bridge subnet via the VM's vzNAT address, and one resolver file
`/etc/resolver/sbx` pointing at the bridge's dnsmasq. Both need root once; a LaunchDaemon
re-applies the route at boot. `orochi sandbox up` re-reads the VM's address every start, because
the vzNAT note established that it is stable across stop/start and changes on recreate, and
occasionally fails to appear at boot (issue #3100) — the `mode: boot` self-heal covers the
latter. This layer reaches whatever binds `0.0.0.0` inside: every port Docker publishes, hence
all of Supabase; not a dev server bound to its own loopback.

### 6.2 Focus: `http://127.0.0.1:54323` as printed

`orochi sandbox focus <project>` (and the console, when a turn starts in a sandboxed project)
makes that one project's ports appear at the Mac's `127.0.0.1` unchanged. Mechanism, per the
proxy note: an Incus proxy device per port,
`listen=tcp:127.0.0.1:P` on the VM, `connect=tcp:127.0.0.1:P` inside the sandbox — which reaches
a loopback-bound service because forkproxy enters the instance's network namespace — and Lima's
dynamic forwarder, which sees the VM's new listener and opens `127.0.0.1:P` on the Mac. Ports
come from the project's entry (`ports = [3000]`) plus the **port watcher**: a process inside each
running sandbox that reports its listening TCP sockets over an `incus exec` stream (D6); the
controller adds and removes proxy devices as they come and go (hot-plug is supported). One
project is focused at a time, so nothing collides; a port already taken on the Mac — by a
host-mode project's own Supabase, say — is reported with who holds it, never stolen. Unfocus
removes the devices.

### 6.3 Outbound and the Mac's services

Sandboxes reach the internet through the bridge's NAT and the VM's. The Mac's own services are
`host.sbx` (dnsmasq `address=/host.sbx/<vmnet gateway>`), for a project that talks to something
running in host mode. Sandboxes reach each other by `<name>.incus` inside the VM; that is
deliberately not routed to `.sbx` names, and a project needing another's database says so in
its config.

## 7. Lifecycle and resources

```
orochi sandbox up | down | status              the host VM, route, resolver, pool usage
orochi sandbox create <path> [--tier container|vm] [--no-docker]
orochi sandbox mode   <path> host|container|vm  R3: the switch
orochi sandbox enter  <path>                    a shell inside, at /work
orochi sandbox focus  <path> | unfocus
orochi sandbox snapshot | restore | reset <path>
orochi sandbox rm <path> | gc [--deep]
orochi sandbox image build | list
```

Name: the project's directory name, made a DNS label, `-2` on a clash. State per project in
`<data>/sandbox/projects.json`: path, name, mode, tier, docker, shadow, ports, sync-mode,
created and last-used times. Config `[sandbox]`: `host_vm`, `cpus`, `memory`, `pool_disk_gib`,
`docker_quota_gib`, `idle_stop_minutes` (30), `default_tier`, `default_docker` (true).

Idle: a sandbox with no `exec` stream, no focus and no turn for `idle_stop_minutes` is stopped;
its Docker containers stop with it and come back on the next start (Docker's own restart
policies, as on a rebooted machine). `gc --deep` runs `docker system prune` inside stopped
sandboxes. `status` shows per project: state, tier, address, focused ports, Docker volume usage
against quota. D10: `create` refuses under 10 GiB free on the Mac and names the two old VM
images as what could be reclaimed.

## 8. Orochi integration

### 8.1 The backend trait

```rust
trait Sandbox {
    fn wrap(&self, cmd: Command, cwd: &Path) -> Command;   // discovery::prepare's launch command
    fn exec(&self, cmd: Command) -> Child;                  // evaluator checks, port watcher, relay
    fn path(&self, host: &Path) -> PathBuf;                 // /work for the project's tree
}
```

`host` returns things unchanged, which is today's behaviour byte for byte. `incus-container`
and `incus-vm` wrap as `incus --project sbx exec <name> --cwd /work --env … -- sbx-run <cmd>`.
`scheduler::run_with_events` obtains the backend from the project's mode (or `--sandbox`),
`discovery` wraps the launch, `evaluator` runs checks through `exec`, and `acp::Client` is
unchanged: ACP is stdio and `incus exec` is a byte-clean pipe. `--internal-supervise` still leads
the Mac-side `incus exec`; inside, `sbx-run` puts the agent in its own session with
`PR_SET_PDEATHSIG`, so a dropped exec takes the agent down and nothing outlives its lease.

### 8.2 The mailbox and the other stdio servers

`--internal-mailbox` is spawned *by the agent*, so it runs inside. The golden image carries a
Linux `orochi`; inside, `orochi --internal-mailbox` connects to `/run/orochi/relay.sock`, and the
Mac holds that socket's other end as one more `incus exec … orochi --internal-sandbox-relay`
stream (D6), multiplexing the sandbox's MCP sessions onto the Mac-side mailbox exactly as an
in-process session would be. No SQLite crosses the boundary; the mailbox database stays on the
Mac. Repository `.mcp.json` servers (`mcp.rs`) with a stdio command run **inside** — that is
where the repository is — and their commands are resolved in the image; remote ones are
unaffected. Until the relay exists (S4), a sandboxed session runs with no mailbox server and the
scheduler reports it beside the agents that could not start, as it already does for a server an
agent cannot take.

### 8.3 Credentials

Claude Code on Linux keeps its OAuth in `~/.claude/.credentials.json` (the Mac keeps it in the
keychain, which cannot be mounted); Codex keeps `~/.codex/auth.json`. One VM directory per CLI,
`/var/lib/sbx/creds/{claude,codex}`, is mounted read-write into every sandbox at the CLI's
home path, so **one login serves every sandbox** and refreshed tokens stay in sync. Login is
`orochi sandbox enter` + `claude login` once. This is the exposure the threat model names: the
agent's own credentials are inside because the agent needs them, as they are on the Mac today;
nothing else from `~` is. Orochi's own files — telemetry, activity, memory, MCP tokens — never
enter a sandbox (D7). Orca's hook variables are emptied on every launch as `pane.rs` already
does; inside a sandbox there is no pane to claim, but the rule stands.

### 8.4 What stays as it is

Classification, routing, learning, the activity store and the console are untouched: the record
gains one optional field, `sandbox: Option<String>` (`#[serde(default)]`, the tier name), so a
run's cost can later be compared across modes. The Orca report (`View::report`) adds the sandbox
name to the seat line, nothing more.

## 9. Sizing on this machine

| Item | Plan | Basis |
|---|---|---|
| VM | 6 CPU / 8 GiB, shared by every sandbox | one or two Supabase stacks at once; adjustable, recreate required |
| Per sandbox | 4 CPU / 6 GiB soft | Measured: 2.03 GiB with Supabase up, 145 MiB idle; soft limits let a quiet neighbour lend |
| Golden image | 6.32 GiB once, in the pool | Ubuntu + toolchains + Supabase's images; measured 2026-09-30 (the Docker volume was dropped, §15) |
| Per project | the diff | ZFS clone; a `node_modules` shadow of 0.5–2 GiB is typical |
| Pool disk | 60 GiB raw, sparse | does not fit today's 19 GiB free: D10 |

## 10. What the two modes look like

```
$ orochi sandbox create ~/Development/ficchat          # clones sbx-golden, mounts the tree, 3–5 s (target)
$ orochi chat                                          # in ficchat: seats run inside; the route line says [ficchat · container]
> supabase start                                       # inside: prints http://127.0.0.1:54323
$ open http://ficchat.sbx:54323                        # by name, from anywhere on the Mac
$ orochi sandbox focus ~/Development/ficchat           # now http://127.0.0.1:54323 works as printed
$ orochi sandbox mode ~/Development/ficchat host       # R3 off: the next turn runs on the Mac, files already there
```

## 11. Rejected and deferred

- **Per-project loopback aliases** (`127.0.0.2:54321` for the second project): sudo per alias,
  and the URLs tools print still lie. Focus does the same job for one project without it.
- **Routing the Mac straight onto the bridge with macvlan/ipvlan** so sandboxes take vmnet
  addresses: not established to work on Apple's NAT network; the static route is one line.
- **Sysbox** for unprivileged Docker-in-Docker: solves D but not the boundary D lacks.
- **Mounting `<data>` into the sandbox** for the mailbox: SQLite locking over virtiofs is the
  failure `activity.rs` learned to avoid in-process; the relay costs one stream.
- **A second Docker engine at `docker.orb.internal`** in host mode: already what host mode is.

## 12. Phases and what each must measure

| Phase | Builds | Verified when |
|---|---|---|
| S1 host + image | `sbx-host` template, ZFS pool, golden image and volume, `create/enter/rm/status` | On this M5: a clone starts in ≤ 5 s; `supabase start` inside succeeds with its dozen containers; the Docker storage driver Docker chose on the delegated ZFS volume is recorded; a `bun install` in a shadowed `node_modules` against a virtiofs tree is timed against the Mac; RAM of the VM idle and with two Supabase stacks |
| S2 reach | route, resolver, `focus`, the port watcher | `curl ficchat.sbx:54321` and, focused, `127.0.0.1:54323` in a browser; a Vite server bound to loopback reachable only under focus; two projects each with Supabase up, no collision |
| S3 Orochi | `Sandbox` trait, `mode`, `--sandbox`, launch wrapping, evaluator `exec`, credentials volume | `orochi chat` in a container-mode project completes a turn with a real agent and a passing check; `mode host` and back, no file moves; record carries the tier |
| S4 relay | `--internal-sandbox-relay`, mailbox inside | Two seats inside one sandbox exchange a message; telemetry byte-level tests still pass |
| S5 tiers | VM tier, idle stop, `snapshot/restore/reset`, `gc` | An Incus VM boots under Lima on this M5; a sandbox stopped idle resumes with its containers; a reset keeps the tree |
| S6 surfaces | Orca seat line, desktop route pill, `--no-docker` | Orca shows the sandbox name; a `--no-docker` sandbox has no nesting and starts faster (measured) |

Every claim above stays "unverified" until the row's measurement exists in a dated
`docs/real-validation-*.md`.

## 13. Open questions

None blocks S1. Defaults are chosen; each can be changed by an answer.

1. **Project = repository or Orca workspace?** Default: the directory given, so an Orca worktree
   at `~/orca/workspaces/orochi/turban` is its own sandbox if created there and shares the
   repository's if created at `~/Development/orochi` — the mounted path decides. If worktrees
   should share one sandbox by default, `create` keys on the git common dir instead.
2. **Reclaim the old `incus-host` VMs** (57 GiB Lima disk, 19.8 GB OrbStack machine)? They are
   vibe-engine's; nothing here touches them, and S1 cannot start on 19 GiB without something
   moving.
3. **Sync mode by default for JavaScript monorepos?** Decided by S1's `bun install` timing.
4. **Remote Incus host later?** The design already speaks `incus` rather than `limactl`; only
   §6's route and Lima forwards are local. Say so when it is wanted and focus becomes `ssh -L`.

## 14. Cloud sandboxes later: Vercel Sandbox, Daytona (added 2026-09-30)

The user intends to move sandboxes to a cloud provider later, under one constraint they
state: **the subscription-billed agents (Claude Code on a plan, Codex on a plan) stay on the
Mac**, so the cloud sandbox never runs the agent. The agent works on a local tree, pushes to git,
the cloud sandbox takes the result, and something reachable from outside — an MCP server or a
script — runs commands there. This section checks the design against that shape. Nothing in it
is built; provider facts are the vendors' statements as of the dates given.

### 14.1 What the providers offer (read 2026-09-30)

| | Vercel Sandbox ([docs](https://vercel.com/docs/sandbox), updated 2026-09-22) | Daytona ([docs](https://www.daytona.io/docs/en/sandboxes/)) |
|---|---|---|
| Unit | Firecracker microVM, own kernel, `sudo`; "system-privileged processes, such as container runtimes like Docker" are a listed feature; Vercel [published](https://vercel.com/changelog/run-docker-containers-inside-vercel-sandbox) running the Docker engine inside | "Linux containers by default"; VM sandboxes (Linux, Windows, macOS) exist, and nested virtualization is offered for the Linux VM kind only |
| R2 (dockerd inside) | **Yes, by the vendor's own account** | Not in the default container kind; the VM kind would be the route. **Unverified** |
| Run commands from outside | `sandbox run …` CLI, `runCommand` in the JS and Python SDKs, `detached` for daemons | Process and code execution API, sessions |
| Reach services | `domain(port)` gives a public preview URL | Preview links `https://{port}-{id}.proxy.daytona.work` |
| Image / clone | Managed images (`vercel/sandbox/universal`, which ships coding agents), custom OCI images in Vercel Container Registry, snapshots, persistent sandboxes that save on stop | Snapshots, including built from a Dockerfile |
| Size | Per vendor pricing page | Default cap 4 vCPU / 8 GB / 10 GB disk per organization: tight for a Supabase stack whose images alone are several GB |

So the hard requirement R2 is satisfied on Vercel Sandbox today and on Daytona only in its VM
kind, if at all. For a cloud project that only needs a database, a hosted Supabase branch in
place of the local stack removes R2 from the cloud side entirely, and is the cheaper answer.

### 14.2 What changes in the design: the agent and the hands come apart

The local design moves the **whole agent** into the sandbox (§8.1 `wrap`). The cloud shape keeps
the agent on the Mac and moves only **where commands run and where services listen**. The trait
in §8.1 already separates the two, so the cloud case is not a third place to put the agent but
a second thing a session can have:

- **Seat placement** (`wrap`): `host` or a local Incus sandbox. Unchanged.
- **Runner** (`exec`, `path`, plus `sync` and `ports`): where the tree is built, tested and
  served. `local` (the seat's own machine, as today), or a cloud runner
  (`vercel-sandbox`, `daytona`) reached over the provider's SDK or CLI.

A session then has one placement and one runner, and the combinations are all meaningful: the
agent on the Mac with a cloud runner is the user's stated target; the agent in a local container
with a cloud runner keeps the Mac clean and the compute elsewhere; the agent in a local container
with the local runner is §4.

### 14.3 The runner as an MCP server the agent is given

Orochi already attaches stdio MCP servers to every session (`mcp.rs`, the mailbox). The runner is
one more, `orochi --internal-runner`, offering the agent:

| Tool | Does |
|---|---|
| `runner_sync` | Bring the sandbox's tree up to the local one. Default: the SDK's file upload of the working tree's diff against the last synced commit, so uncommitted work runs without a commit. `mode=git`: push the current branch to the remote and pull it in the sandbox, for a reproducible baseline and for CI-like runs |
| `runner_exec` | Run a command in the sandbox, streaming output; `detached` for a server |
| `runner_ports` | List what the sandbox listens on and the preview URL for each (`domain(port)` / preview link) |
| `runner_logs` | Output of a detached command |

The evaluator's checks go through the same runner, so a passing check means passing *there*.
The agent's own shell tool still runs where the agent is; a prompt note in the shape of
`mailbox::prompt_note` tells it that building, testing and serving happen through `runner_*`,
and the console's route line shows the runner beside the placement. This is the "MCP or script"
the user describes, in Orochi's existing form; a standalone script is the same server driven
from a shell.

### 14.4 What carries over and what does not

| Local (§4–§7) | Cloud |
|---|---|
| Golden image + ZFS clone | Custom image in the provider's registry + snapshot / persistent sandbox |
| `create / enter / focus / snapshot / reset / rm / gc` | The same verbs over the SDK; `enter` is `sandbox run bash` |
| By-name reachability (§6.1) | The preview URL, which is **public** on Vercel by the vendor's description: not for a service holding real data without the app's own authentication |
| Focus (§6.2, ports at `127.0.0.1`) | No tunnel is documented; **unverified** whether a local port can be forwarded. Until then a cloud project's URLs are the preview URLs |
| Credentials inside (§8.3) | **None.** The agent never runs there, so no agent credential leaves the Mac. The sandbox gets a deploy key or token for the repository only |
| Mailbox relay (§8.2) | Not needed: every seat is local |
| Idle stop | The provider's auto-stop / persistence; the runner is created lazily on the first `runner_exec` of a turn and stopped with the thread |

### 14.5 Consequences for the phases

Nothing moves earlier. S3 adds the placement/runner split to the trait when it is written, so
the cloud runner is a new implementation and not a refactor; a `vercel-sandbox` runner is a
phase of its own after S6, verified by: a turn on the Mac edits a Next.js project, `runner_sync`
uploads the diff, `runner_exec` runs its tests in the sandbox, `runner_ports` returns a preview
URL that serves the change, and — for R2 — `supabase start` inside a Vercel sandbox brings up
its stack, measured for time and cost. Daytona follows only if its VM kind meets R2.

### 14.6 The premise

The design does not depend on the premise that a subscription agent may not run in the cloud:
if that changes, a cloud sandbox becomes one more seat placement behind the same trait, with
credentials handled as §8.3 handles them locally. The split is worth having regardless, because
it is what keeps agent billing on the plan and compute billing on the provider, and it is the
only shape in which no agent credential leaves the machine.


## 15. What was built (2026-09-30), and where it departs

Built: `src/sandbox.rs`, `src/sandbox/ops.rs` with the embedded `lima.yaml`, `host.sh` and
`image.sh`; `[sandbox]` in the configuration; `orochi sandbox up | down | status | create |
mode | enter | focus [--watch] | unfocus | snapshot | restore | reset | rm | gc | network
[--apply] | image [--vm] | host-script`; `--sandbox=<mode>` per run; the scheduler wraps each
executing agent's launch and the evaluator each check; the desktop asks where a new project runs.

| Design | Built | Why |
|---|---|---|
| The tree at `/work` inside (§5.1) | **At its own absolute path** | The ACP `cwd`, attachment paths and file names in replies then need no translation anywhere, and a subdirectory maps to itself |
| A dedicated Lima disk for the ZFS pool (§4.1) | A sparse ZFS pool file inside the VM's disk (`incus storage create sbx-pool zfs size=…`) | One fewer device to identify at boot; the pool does not outlive `limactl delete`, which the design did not need |
| Incus reached as `incus` (D6) | `sandbox.client = "lima"` runs `incus` in the VM over `limactl shell`; `"incus"` uses a local client and an optional remote | Nothing to install or trust on the Mac; the `incus` client form is the remote-host path (R10) |
| Mailbox relay (§8.2, S4) | **Not built.** A sandboxed session gets neither the mailbox server nor its prompt note, and configured stdio MCP servers are reported as skipped for that agent | They are this machine's executables. Remote (http/sse) servers still reach the agent |
| Routing advisers and the classifier | Stay on this machine | They are shown no repository and no tools; they are only asked when installed here |
| `RunRecord.sandbox` (§8.4) | Not added | Sixteen record literals for a value nothing reads yet; add it with the first analysis that compares modes |
| Idle stop (§7) | `orochi sandbox gc`, by hand or from a scheduler of the user's | No resident process exists to stop things on a timer |
| A port watcher inside (§6.2) | `focus` reads `ss -ltnH` through `incus exec`; `--watch` repeats it every 2 s | The same result without a process in the image |
| `--sandbox host` | `--sandbox=host` | `host` alone is read as the hidden `host` command, since this CLI gives subcommands precedence |
| A per-project Docker volume cloned from a golden one (§5.3, D4) | **None.** Docker's data stays on the instance's root disk, bounded by `sandbox.quota_gib` | Docker 29 keeps images under `/var/lib/containerd`, so the volume stayed empty (measured); the root disk is already a ZFS clone of the image, so the pre-pulled images are shared copy-on-write and a snapshot of the instance covers them |
| `setpriv --pdeathsig` ends the agent inside (§8.1) | `sbx-run` is a supervisor: the command in its own process group, killed when it ends; checks get `SBX_HOLD=1` and the end of the stdin Orochi holds open kills them | `incus exec` leaves the process running when its client is killed (measured); `pdeathsig` did not help |
| ZFS left at its defaults | ARC capped at 1 GiB | It took 2.85 GiB of an 8 GiB VM (measured) |
| Agent credentials (§8.3) | `/var/lib/sbx/creds/{claude,codex}` in the VM, mounted at `~/.claude` and `~/.codex` with `CLAUDE_CONFIG_DIR` / `CODEX_HOME` set | Claude Code keeps `.claude.json` inside `CLAUDE_CONFIG_DIR` when it is set, so one directory holds all of it |

The desktop (S6, built the same day): the new-project question (`placement` / `place`), a
Sandboxes screen over `view::sandboxes`, and every operation as `sandbox::jobs` — the CLI
command run in the background with its output and exit code kept under
`<data>/sandbox/jobs/`, the newest 50 kept. A window has no terminal for `sudo`, so
`network --apply` without one asks for an administrator through `osascript` on macOS.

Answered by the real run ([2026-09-30](real-validation-20260930-2.md)): ACP is carried byte-clean
through `limactl shell` and `incus exec`; the image builds and `supabase start` runs inside a
nested container; Docker uses `overlayfs` on ZFS; a killed agent or check now ends inside.
Still open: whether the bridge's dnsmasq answers queries routed from the Mac, real agents
logged in inside, and the VM tier.
