# 0007 — Sandbox capabilities are named, positive and monotonic

- **Date:** 2026-08-25
- **Status:** accepted

## Context

The kernel shipped with WSL enables unprivileged user namespaces,
seccomp filtering and the Landlock LSM (filesystem rules; no network
rules at this kernel version). That allows a rootless sandbox built from
namespaces, a syscall blocklist and filesystem rules. What the agent may
reach must be expressed so a user can understand it in the UI and so a
repository the agent can write to cannot widen it.

## Options

| Option                               | For                                     | Against                                  |
| ------------------------------------ | --------------------------------------- | ---------------------------------------- |
| Named capabilities, layered config   | explainable; per-project; safe defaults | vocabulary must be designed once, well   |
| Permissive/restrictive modes         | simple                                  | opaque; all-or-nothing                   |
| Free-form mount lists                | flexible                                | unexplainable; easy to get wrong         |

## Decision

A fixed base (namespaces, no new privileges, read-only system, ephemeral
home, no Windows interop, allowlisted environment, syscall blocklist) is
always on. On top, capabilities with positive names (`project.rw`,
`agent.state`, `tools.ro`, `caches.rw`, `git.identity`,
`home.persistent`, `extra.paths`, `ssh`, `mnt.all`, `windows.interop`)
are enabled per project. Configuration is layered with increasing
authority: Willie defaults ← project profile ← a file inside the
repository that may only **tighten**. Invalid configuration refuses to
start the session. `willie sandbox explain` prints the exact invocation.

## Consequences

- Each capability carries a one-line consequence shown in the UI.
- Network stays always-on because the harness needs it; fine-grained
  egress is a later capability.
- Windows executables are unreachable from inside a session unless
  `windows.interop` is explicitly enabled.
- Tests must prove denials, not only permissions.

## Not decided

Egress filtering by host and per-session resource limits through
cgroups. Both are in the growth list.
