# 0015 — A session's git identity is resolved and required

- **Date:** 2026-08-28
- **Status:** accepted

## Context

An agent commits inside the ext4 workspace; those commits need an author.
The projects slice copied the source checkout's HEAD author into the
clone's local config as a stopgap. That is wrong once a real identity
exists: a repository's local identity outranks the global one, so a
copied author — possibly a different person — would win over the user's
own, and the distro home still had no identity for any git run outside a
workspace.

## Options

| Option | For | Against |
| --- | --- | --- |
| Resolve to the distro user's global `~/.gitconfig`, required | one identity for every workspace and shell; matches the user's Windows identity | the daemon writes a config file |
| Keep the per-clone stopgap | no new code | wrong author under the common case; no identity outside a workspace |
| Ask in the UI | explicit | more UI; a value to keep synced with Windows |

## Decision

At `session.create` the daemon ensures `/home/willie/.gitconfig` has a
`user.name` and `user.email`, in order: keep an existing one; else write
the Windows global identity the engine read; else the source checkout's;
else refuse with `git_identity_missing`. This supersedes the projects
slice's `copy_source_identity` stopgap — copying the source's `HEAD`
author into the clone's *local* configuration during `add` — because a
local identity outranks the global one this decision writes, so the
stopgap must go. As of this record that removal has **not** landed:
`copy_source_identity` (`crates/willied/src/projects.rs`) still runs on
every `add`. It is a follow-up, not a design choice left open — see
Consequences and Not decided.

## Consequences

- Every session's commits resolve to one identity, written once to the
  distro user's global config and reused by every workspace and shell —
  once the stopgap below is gone.
- A user with no git identity anywhere is told to set one at
  `session.create`, rather than a session committing as a stranger.
- Until `copy_source_identity` is removed, **every** project's workspace
  — new ones included, not only ones from before this record — still
  gets the source checkout's `HEAD` author written as its *local* git
  identity by `add`. A local identity always wins over the global one
  this decision resolves, so a plain `git commit` made directly in that
  workspace (by a person, or by the agent through a shell outside a
  session) still uses the copied author, not this resolution, until the
  call is deleted or the local override is unset by hand
  (`git config --unset user.name` / `user.email`, run in the workspace);
  the acceptance checklist names the exact commands.

## Not decided

Mounting the user's real `~/.gitconfig` (the `git.identity` sandbox
capability, ARCHITECTURE §3.3) belongs to the sandbox slice. Removing
`copy_source_identity` from `projects.rs`'s `add` job is not left open
by choice — it is a known gap this record surfaces so a later task in
this branch, or the whole-of-Plan-A gate, closes it deliberately instead
of by accident.
