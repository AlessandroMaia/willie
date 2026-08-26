# 0005 — The Linux daemon owns the truth; stdio is the first transport

- **Date:** 2026-08-25
- **Status:** accepted

## Context

State could live on the Windows side (simple: one long-lived process) or
in a daemon inside the distribution (where the sessions, the agent state
and the fast filesystem are). The two sides also need a channel, and the
options differ in ports, authentication and lifecycle.

## Options

| Option                                         | For                                              | Against                                         |
| ---------------------------------------------- | ------------------------------------------------ | ----------------------------------------------- |
| Daemon in the distro, stdio via `wsl.exe`      | no ports/tokens/firewall; lifecycle tied to the app; state in ext4 | one native client (the app)         |
| Daemon in the distro, TCP loopback + token     | many clients; survives app restarts              | port and token management; needs a VM anchor    |
| No daemon; Windows state, `wsl.exe` per call   | one process                                      | state on Windows; slow per-call spawn; a daemon appears anyway for sessions |

## Decision

`willied` runs inside the distribution as an unprivileged user and owns
projects, sessions, plugins and the SQLite index. The engine starts it with
`wsl.exe -d willie --user willie --exec /opt/willie/bin/willied --stdio`
and speaks JSON-RPC over the process pipes. Local clients use a Unix
socket inside the distribution. The protocol is transport-agnostic so TCP
can be added later without changes to messages.

## Consequences

- Windows keeps only `engine.toml`; there is no database on Windows.
- When the app closes the daemon exits; sessions keep running under their
  supervisors (0006) and are re-adopted when the daemon returns.
- Privileged operations are one-shot `wsl.exe --user root` calls from the
  engine; the daemon never elevates.
- Reconnection uses snapshot + event stream, so no replay buffer is kept.

## Not decided

TCP loopback for additional clients (a Windows CLI, an editor extension).
Deferred until a second client exists.
