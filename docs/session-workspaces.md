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
- The tool response and completion report identify the assigned workspace. Workspaces and
  edits survive completion, interruption, `/new` and process exit. They are not implicitly
  merged, committed or deleted. A restarted worker is not automatically resumed: the lead
  can inspect and integrate the retained result explicitly.
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

Use `/workspaces` in the console to list the conversation’s retained worktrees.

The result includes `started` and `workspace`. The lead should examine the changes at that
path and perform any requested integration itself. Read-only requests retain the existing
`started` response without a workspace path.

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

## Validation

Automated tests cover dirty snapshots, source index preservation, retained workspaces,
subdirectory cwd, read-only compatibility, concurrent writable helpers and distinct session
sandbox ownership using mock ACP agents and a fake Incus client. Real Incus provisioning and container-side ACP execution with the validation agent were
verified on 2026-10-05; see [the live validation record](real-validation-20261005.md).
Real provider agents still require separate validation.

Sandboxed agents receive a portable stdio mailbox relay through their existing shared
workspace mount. Requests are processed by the same host-side mailbox handler; the activity
database stays on the host. This enables `start_agent` inside Linux without putting the
macOS executable in the container. Each relay belongs to one ACP peer and is closed when
that peer is dropped. Other configured host stdio servers keep their existing restrictions.
