# Workspaces within an Orochi conversation

An Orochi conversation (a persisted activity thread, or an ephemeral console conversation
when recording is disabled) owns multiple agent workspaces. An ACP session is an agent's
conversation with its provider; it is not the owner of the execution environment.

## Requirements and behavior

- The lead continues in the directory selected with `-C`.
- A background agent requested with `start_agent` defaults to read-only investigation in
  that directory, preserving existing behavior.
- `start_agent` with `write: true` creates a dedicated Git worktree and permits edits there.
  Multiple workers can execute concurrently without writing into each other's directories.
  Existing seat limits, routing and the lead's permission setting still apply.
- Each new writable worker starts from a snapshot of the repository at launch, including
  staged and unstaged tracked edits and nonignored new files. The source HEAD, branch and
  index are unchanged. Ignored files and Orochi's session directory are excluded.
  A Git repository with an existing commit is required for writable workers.
- Worktrees use detached HEADs at the snapshot commits. Worker names are unique within the
  session's retained workspace list; another launch with the same name gets a suffix.
- The tool response and completion report identify the assigned workspace. **No worktree
  outlives the conversation.** The lead closes a worktree with `finish_workspace` once the
  helper's report is in: `merge` writes the changes into the user's working tree file by file,
  three-way against the snapshot, as uncommitted edits — nothing is written when any file
  conflicts; `pr` replays them onto the current HEAD on a branch (`workspaces.branch_prefix`
  plus the helper's name), pushes it to the repository's first remote and opens a pull
  request with `gh`; `discard` drops them. The worktree is removed in every case. Changes that
  reached neither the tree nor a pull request are kept as `<name>.patch` beside the manifest.
  A worktree still open at the conversation's end, at `/new`, or when a conversation is picked
  up after a crash is closed the same way (kept as a patch, never merged unasked) and the
  console says so. `orochi workspaces prune` does the same for every session directory no
  process holds (`live`, a lock each conversation holds while it runs), and removes a session
  directory left with nothing in it. A helper still working is never closed under it.
- `/worktrees on|off` turns worktrees off for one conversation: a `write: true` request is
  then refused with a reason the lead can repeat, and the lead does the work itself.
  `[workspaces] enabled` is the default, `finish` what `finish_workspace` does when the lead
  gives no `how`.
- The workspace manifest is stored at `.orochi/sessions/<session-key>/workspaces.json`.
  The key is derived from the activity thread ID and stays the same when that thread is
  resumed by a terminal or headless host. With recording disabled a new conversation uses
  a fresh key. `/new` starts a new owner and stops the old conversation's workers.

For example, a lead may call its mailbox tool with:

```json
{
  "name": "backend",
  "title": "Implement request validation",
  "task": "Implement the validation and run the relevant tests. Report the changed files.",
  "write": true
}
```

Use `/workspaces` in the console to list the conversation's open worktrees and the patches of
closed ones; `orochi workspaces` lists every conversation's under the repository.

The result includes `started` and `workspace`. The lead examines the changes at that path and
then closes it:

```json
{ "name": "backend", "how": "merge" }
```

`finish_workspace` answers with what became of it: the paths changed, the paths merged, any
conflicts, the branch and pull request for `pr`, the patch file where one was written, and a
one-line summary. Read-only requests retain the existing `started` response without a
workspace path.

## Session containers and VMs

For console conversations and headless hosts, a project's configured sandbox supplies the
settings for a **new session sandbox**. The lead, advisory seats, writable workers and
read-only workers of that conversation use that one instance. Another conversation gets
another instance, even when it selects the same repository. Resuming the same activity
thread reuses the instance. `--sandbox host` continues to run on the host.

The source tree, session directory and any external Git common directory are mounted at
identical absolute paths. This supports sessions launched from an existing Git worktree,
whose `.git` file points outside it. In runner mode the agent remains on the host; its
sandbox MCP commands use its assigned workspace as the default cwd and the conversation's
shared instance. Docker, runtime resources and ports are shared by all seats in that
session, so workers must coordinate service names and ports.

Project shadow directories are not copied to session instances: a shadow at the source
root could hide the session worktrees beneath it. Workspace build caches stay in their
own directories. Session instances remain registered in `sandbox/projects.json`, and the
existing sandbox status, focus, stop, reset and remove commands can manage them using the
session directory. Workspaces remain after removing a sandbox.

One-shot runs and the separate `collaborate` command keep their existing behavior. The
latter uses workspace copies and its own merge pipeline; this change targets the console
and headless conversations' `start_agent` execution path.

## Thread worktrees

Written 2026-11-08. Besides the helpers' worktrees above, a whole thread can work in a
worktree of its own: `orochi --worktree` (a new one with a generated name) or
`orochi --worktree=NAME` (that one, made or joined) before `chat`, a task or `threads new`,
and **New thread in a worktree** in the window's project menu. Orochi makes it with
`git worktree add` at `<repository>/.orochi/worktrees/<name>` on the branch
`<workspaces.branch_prefix><name>`, taken as it is when it exists and otherwise started at
the current HEAD, and adds `.orochi/` to the repository's `info/exclude`. It requires
`activity.enabled`.

Only Orochi makes these and only Orochi works in them, so every user is one it can see: a
thread whose `cwd` lies inside one, or a process working there, which holds a shared lock
beside it (`.<name>.live`) for as long as it runs — several may, and the lock is held before
any thread records the worktree. A worktree another tool made (Orca's, a hand-made one) is
never treated as one of these.

A thread worktree lasts until the **last thread working in it is deleted** (the user's
decision, 2026-11-08: a checkout per conversation is what fills the disk). It is then removed
with `git worktree remove`; what it had not committed is kept as the ref
`refs/orochi/kept/<name>`, and its branch is kept unless `git branch -d` finds it merged
(a branch with no commits of its own is). Asking for the same name again checks that branch
out again.

## What a deleted thread leaves

`reclaim.rs` takes back what belongs to no thread that still exists and that no process
holds: a session directory whose key names no thread (helper worktrees removed through git,
then the directory with its patches and trials — deleting a thread is the user deleting
everything in it), the session sandbox registered for such a directory or for one already
gone (`ops::retire`; a project's own sandbox is never one), and a thread worktree no
thread's `cwd` lies in. A thread is deleted by `orochi threads delete`, by the window, and by
`activity.retention_days` inside every open of the store, so reclaim does not follow
deletions: it compares the disk with the threads, and running it again is always safe.

It runs after `orochi threads delete` and the window's delete (both may start the sandbox
VM to delete a sandbox), at the start of `orochi workspaces prune`, and quietly when a
console conversation ends, which never starts the VM: a sandbox that has to wait for it is
counted and taken by the next call that may start it. A console conversation holds its
session directory from the moment it is given a sandbox, so a conversation that is running is
never taken.

Deleting an instance frees space in the VM's storage pool, which later instances reuse;
whether the Lima disk image on this machine shrinks with it was not measured.

## Validation

Automated tests cover dirty snapshots, source index preservation, subdirectory cwd, read-only
compatibility, concurrent writable helpers and distinct session sandbox ownership using mock
ACP agents and a fake Incus client; closing by merge (three-way beside a concurrent user edit,
HEAD and index untouched), a conflict and a discard kept as patches, a pull request replayed
onto HEAD and pushed to a local bare remote with a stand-in `gh`, the sweep at a
conversation's end, and `prune` leaving a held session alone (`tests/workspaces.rs`,
`tests/cli_e2e.rs`); a thread worktree kept while any thread or holder uses it and removed
with its last thread, its uncommitted changes kept as a ref and a branch with commits kept,
a deleted thread's session directory removed whole beside a held one, `--worktree` with
`threads new` and `threads delete` through the binary, and a deleted thread's session
sandbox deleted beside the project's own (`tests/workspaces.rs`, `tests/sandbox.rs`, against
the fake Incus client). The whole lifecycle ran on the real `sbx-host` VM on 2026-11-09 with
the fixture agent: [Real validation 2026-11-09](real-validation-20261109.md). Whether a real lead closes its worktrees sensibly is unverified. Real Incus provisioning and container-side ACP execution with the validation agent were
verified on 2026-10-05; see [the live validation record](real-validation-20261005.md).
Real provider agents still require separate validation.

Sandboxed agents receive a portable stdio mailbox relay through their existing shared
workspace mount. Requests are processed by the same host-side mailbox handler; the activity
database stays on the host. This enables `start_agent` inside Linux without putting the
macOS executable in the container. Each relay belongs to one ACP peer and is closed when
that peer is dropped. Other configured host stdio servers keep their existing restrictions.
