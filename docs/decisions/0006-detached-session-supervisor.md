# 0006 — One detached supervisor per session

- **Date:** 2026-08-25
- **Status:** accepted

## Context

Sessions are interactive terminals the user opens in Windows Terminal and
may keep for hours. If the daemon owned their PTYs, closing the app (and
therefore the daemon) would hang up every session. A future embedded
terminal must also attach to the same PTY without changing how sessions
are started.

## Options

| Option                                 | For                                              | Against                                    |
| -------------------------------------- | ------------------------------------------------ | ------------------------------------------ |
| Supervisor process per session         | sessions outlive the daemon and the app; any client can attach | one more binary; re-adoption logic |
| Daemon owns PTYs                       | simplest                                         | sessions die with the daemon               |
| Terminal owns the process directly     | no supervisor                                    | no supervision, no event log, no re-attach |

## Decision

`willie-sess <id>` is spawned detached (own session and process group).
It opens the PTY, builds the sandbox, runs the harness, serves attach
clients on `/run/willie/sessions/<id>.sock`, keeps a ring buffer for late
attachers and appends to `sessions/<id>/events.jsonl` without needing the
daemon. Windows Terminal runs `willie attach <id>` through `wsl.exe`.

## Consequences

- Killing the daemon does not kill sessions; on restart it rediscovers
  supervisors through their sockets and rebuilds the index from the event
  logs.
- Files are the truth for session history; SQLite is a rebuildable index.
- The embedded terminal, when it comes, is one more attach client.
- Multiple simultaneous attaches are allowed (all read-write).

## Not decided

Read-only attaches and per-client input arbitration. Not needed for a
single user.
