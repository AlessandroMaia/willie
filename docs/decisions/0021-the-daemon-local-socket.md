# 0021 — The daemon's local socket: one dispatcher, request and reply, no shutdown

- **Date:** 2026-09-06
- **Status:** accepted

## Context
`docs/ARCHITECTURE.md` promised `/run/willie/willied.sock` for local CLI
clients and nothing served it, so `willie sandbox explain` had no
transport. The daemon's request loop read stdin in place and owned stdout
through a single writer; whatever served the socket had to keep the
"one request at a time" invariant and not race the engine's calls.

## Options
| Option | For | Against |
| --- | --- | --- |
| One inbound channel, two origins (chosen) | one dispatcher, no new lock, the existing ordering; stdout writer untouched | a thread per connection and per stdin |
| A second dispatcher on the socket sharing Ops/State | true concurrency | races session.create's busy-check-then-spawn; two writers |
| The server behind a Mutex both transports lock | simple to state | a lock on session.create's multi-second readiness wait |

## Decision
The loop consumes one `Inbound` channel. A stdin reader thread feeds it as
before; an accept thread spawns one thread per socket connection, each
sending a line with a reply channel and blocking for the response. The
dispatcher handles messages in arrival order, so the daemon still answers
one request at a time. Notifications stay on stdio; a socket client calls
`state.snapshot`. `daemon.shutdown` over the socket is refused
(`method_not_served`) — the engine owns the daemon's lifecycle. The socket
is `0600` and never mounted into a session.

## Consequences
`willie sandbox explain` works from inside the distribution while the app
is open; future local commands (`willie reindex`, once the index exists)
plug into the same transport. A connection parks a thread while idle;
acceptable for the local user's own CLI. The engine's behaviour is
unchanged.

## Not decided
- A connection cap. Unbounded for now; revisit if a non-human socket
  client appears.
- TCP loopback (PROTOCOL names it "later"); not needed by any client yet.
