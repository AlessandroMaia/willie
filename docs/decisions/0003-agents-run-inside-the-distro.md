# 0003 — Agents run inside the Willie distribution

- **Date:** 2026-08-25
- **Status:** accepted

## Context

The user's repositories live under `C:\`. Running the agent CLI natively
on Windows keeps that layout untouched but forfeits the sandbox and keeps
every Windows-specific failure mode. Running it inside the distribution
gives the sandbox and POSIX semantics, at the cost of slower access to
`C:\` through `/mnt/c` and of toolchains having to exist in Linux.

## Options

| Option                          | For                                        | Against                                         |
| ------------------------------- | ------------------------------------------ | ----------------------------------------------- |
| Always inside the distro        | sandbox always available; one code path    | `/mnt/c` is slower; toolchains needed in Linux  |
| Always native Windows           | no workflow change                         | no sandbox; the distro loses its purpose        |
| Selectable per session          | maximum flexibility                        | two code paths forever; twice the platform bugs |

## Decision

Agent CLIs always run inside the distribution. Repositories may stay on
`C:\`, accessed through `/mnt/c` in a *supported-with-warnings* mode
(slower git and search, no file watchers). A faster workspace inside the
distribution is a later improvement, not a prerequisite.

## Consequences

- Willie's own state and its plugins' state always live in ext4, never
  under `/mnt/*`.
- Toolchains (.NET, Node, …) are provisioned into the distribution as
  managed tools.
- Corporate proxy and TLS-inspection certificates must be propagated into
  the distribution by the engine.
- The Windows ↔ Linux boundary is an explicit transport, never a socket
  file on `/mnt/c`.

## Not decided

When (or whether) to offer managed workspaces inside the distribution.
The first slice measures `/mnt/c` performance on a real repository and
that number decides.
