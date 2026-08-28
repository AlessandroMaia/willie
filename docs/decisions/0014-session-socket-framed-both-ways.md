# 0014 — One framed socket per session, the daemon as a control client

- **Date:** 2026-08-28
- **Status:** accepted

## Context

Decision 0012 measured the supervisor prototype: the client framed its
messages, the supervisor sent raw PTY bytes back. That left a client
unable to tell "the session ended" from "another attach replaced me" from
"the daemon stopped it", and gave the daemon no way to observe a session
without a second mechanism. The sessions slice needs both.

## Options

| Option | For | Against |
| --- | --- | --- |
| One framed socket, both directions, daemon is a control client | one mechanism; the daemon rides it too; close reasons carried | the terminal side must deframe |
| Two sockets per session (raw PTY + control) | the byte path stays raw as measured | two listeners, two lifecycles, two cleanups; the terminal still cannot see close reasons |
| Daemon owns a socket the supervisors push to | matches the future local-client plan | a second transport now; supervisors must tolerate an absent daemon and queue |

## Decision

Both directions of the session socket are framed (`type:u8 len:u16`).
`willie attach` connects with `role: terminal` and gets output plus a
final `closed` reason; the daemon connects with `role: control` and gets
the event stream and status. One socket, one lifecycle; the daemon is
just another client and never a dependency of the supervisor.

## Consequences

- The terminal deframes a trivial protocol; in exchange it always knows
  why it closed and exits with a code that closes or keeps its tab.
- The daemon re-adopts a running supervisor by reconnecting as control
  and reading status; no separate control channel to manage.
- Raw-byte throughput is unchanged in practice: output is one `OUTPUT`
  frame per PTY read.

## Not decided

An embedded terminal and read-only attach (decision 0006 left them open);
both are future `terminal` clients of this same socket.
