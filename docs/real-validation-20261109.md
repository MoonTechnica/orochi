# Real validation: thread worktrees and reclaim (2026-11-09)

Thread worktrees (`--worktree`) and `reclaim` were run on the existing `sbx-host` Lima VM
(Lima 2.2.1, vz, the existing `sbx-golden` image), with real Incus containers. The ACP agent
was the Python fixture (`tests/fixtures/mock_acp.py`, committed into the disposable
repository so it is present in every checkout), not a real provider agent: no credentials or
paid calls. The repository, config and data directory were disposable, under
`~/orochi-reclaim-validation/` (inside the VM's only mount, the home directory), and were
removed afterwards; the VM was returned to its stopped state.

## Before

The VM held four Incus instances and six shadow volumes left by the 2026-10-16 validation,
whose repository and data directory had already been deleted, so no `projects.json`
recorded them and `reclaim` could not have found them. They were deleted by hand
(`incus delete`, `incus storage volume delete`). Each conversation instance used about 2 MB
of the ZFS pool: an instance is a copy-on-write clone of the 6.38 GB image, so instances
without Docker data are not what fills the disk; checkouts are.

## Results

| Step | Observed |
|---|---|
| `sandbox create --no-docker` | instance `repo` created |
| `--worktree=feat chat`, then `chat` in that worktree, then `chat` in the main checkout | worktree `.orochi/worktrees/feat` on `orochi/feat`; three conversation instances, the two in the worktree with session directories inside it; the fixture's log written in both checkouts |
| `threads delete` of the first thread in `feat` (an uncommitted edit left in the worktree) | its instance and session directory deleted; the worktree kept for the second thread; 6.8 s |
| `threads delete` of the second | its instance and session directory deleted, then the worktree; the edit kept as `refs/orochi/kept/feat` (read back with `git show`); `orochi/feat`, with no commits of its own, deleted; 4.6 s |
| VM stopped, then `threads delete` of the main-checkout thread | the VM started, the instance was deleted; 16.7 s |
| `--worktree=again threads new`, `threads delete` | the worktree removed with its only thread |
| End | only the project's own `repo` remained until `sandbox rm`; then no containers or custom volumes in the pool |

## Problem found and fixed

`worktrees::create` joined an empty relative path onto the worktree, so the thread's `cwd`
was recorded with a trailing `/`. Path comparison is by component, so nothing misbehaved, but
the stored path differed from every other thread's; it is now the worktree itself, and
`tests/workspaces.rs` asserts it.

## Not covered

Real provider agents; a session sandbox with Docker data; the window's delete and its
background reclaim; the quiet reclaim at a console's end with the VM stopped (it leaves the
sandbox for a later call, which is covered only by the code path, not observed here);
whether the Lima disk image on this machine shrinks after instances are deleted (it is a
sparse 100 GB file using 17 GB, and was not measured before and after).

## The sandbox host on first use

Later the same day, after the setup dialog stopped ticking the sandbox host by default and
`setup::sandbox_on_demand` was added: with a fresh data directory holding no Lima and the
existing `sbx-host` VM stopped (its image already built), `orochi sandbox create --no-docker`
downloaded Lima 2.2.1 from its GitHub release into `<data>/toolchain/` (checksum verified),
started the VM and created the sandbox, in 20 s. A second `sandbox create` set nothing up and
created at once. Both sandboxes were removed and the VM stopped again. Creating the VM itself
on first use was exercised only against the fake `limactl` (`tests/sandbox.rs`), not on a
machine without one.
