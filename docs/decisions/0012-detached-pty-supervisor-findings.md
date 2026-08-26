# 0012 — the detached PTY supervisor is the session path (measured)

- **Date:** 2026-08-26
- **Status:** accepted

## Context

Decision 0006 put every agent session in a detached supervisor that
owns the PTY and serves attach clients over a Unix socket, with Windows
Terminal as a plain client through `wsl.exe`. Spike S2 built a prototype
of that shape — `willie-sess run --detach` and `willie attach`, about
1 400 lines on branch `spike/s2-s5`, kept for reference and never
merged — and measured it inside the `willie` distribution (WSL 2.6.1,
kernel 6.6.87, Debian 13) against Windows Terminal, with Claude Code
2.1.246 installed in the user's home as the real harness.

Prototype shape: double fork + `setsid` + a readiness pipe back to the
launcher; `/dev/ptmx` with two ioctls instead of `openpty`; client →
supervisor frames (`type:u8 len:u16 payload` — input, resize, detach),
supervisor → client raw PTY bytes; a 64 KiB replay ring; one client at a
time; a plain-text session log of lifetime events; `libc` as the only
dependency.

## Findings

| Question | Result |
| --- | --- |
| Does the session survive its launcher? | **Yes, three ways:** the `wsl.exe --exec` that started it returned; that relay was killed; a fake daemon and its whole process group were killed with `SIGKILL`. The supervisor is reparented to init with its own session id; `SIGTERM` to `willied` does not reach it. |
| Keystroke echo latency through socket + PTY | **median 165 µs, p95 219 µs** (raw loop, 200 iterations); 211–230 µs median / 258–320 µs p95 with an interactive `bash` doing the echoing |
| Attach cost | connect 77–151 µs; first byte 18–153 µs after connect |
| Idle cost of a parked session | **256 KiB RSS, 0.0 % CPU, 2 threads** for the supervisor |
| Resize | the client's `TIOCGWINSZ` (51×60 in a WT tab) and every `SIGWINCH` become a resize frame; `TIOCSWINSZ` on the master delivers `SIGWINCH` to the child, which sees the new size (44×133 forced deterministically) |
| Bytes | 24-bit colour sequences and UTF-8 (`ola ✓ acentuação`) pass unchanged in both directions; `Ctrl-C` reaches the child, never the client |
| Full-screen programs | `top` and Claude Code redraw on reattach; Claude Code reflowed from 140 to 100 columns on a reattach with no keystroke — **the exit criterion** |
| Second attach | replaces the first, which is closed |
| Exit propagation | a harness ending with `exit 7` leaves `exit=7` in the log and no socket |
| Socket path | a live path is refused, a dead one is cleared before binding |
| Windows Terminal | `wt.exe -w 0 nt … wsl.exe -d willie --user willie --exec /opt/willie/bin/willie attach <sock>` opens a tab with a real TTY without touching `settings.json` |

Installing the harness surfaced two image defects F1 must fix: the empty
`~/.claude.json` placeholder is rejected as corrupted ("Unexpected EOF")
and aborts the first install — it must contain `{}`; and the image has no
`en_US` locale, so processes must run with `LANG=C.UTF-8` (present) or
UTF-8 output breaks. The `~/.claude.json` symlink into the agent-state
directory survived both the install and the first runs.

### Manual walk in Windows Terminal

Walked by the user on 2026-08-26 against the two sessions the prototype
left running; the client ran both through `wt.exe nt … wsl.exe --exec
willie attach` and from a shell already inside a Windows Terminal tab of
the distribution. Every check passed as written.

| Check | Result |
| --- | --- |
| `stty size` follows a drag-resize with the tab focused | pass |
| 24-bit gradient without banding | pass |
| Arrows, Home/End, Ctrl-arrows, Alt-b/f, F1–F4, Shift+Tab, Backspace, Delete | pass |
| `Ctrl-C` during `sleep 30` returns the prompt, tab stays | pass |
| Bracketed paste of two lines | pass |
| `top` reflows on repeated resizes | pass |
| `Ctrl-]` detaches, reattach restores the screen | pass |
| Closing the tab (`SIGHUP` to the client) leaves the session alive | pass |
| Claude Code: theme picker, login, line editing, Shift+Tab modes, reflow while streaming, `Ctrl-C` handling, reattach | pass |
| `pkill -TERM willied` with a tab attached changes nothing in the tab | pass |

## Gaps the prototype exposed

1. **No control channel.** Supervisor → client carries only PTY bytes, so
   a closed client cannot tell "session ended" from "replaced by another
   attach" or "stopped by the daemon".
2. **The readiness handshake confirms the socket, not the harness.**
   `-- /no/such/program` is reported as started and then logs `exit=127`.
3. **`SIGTERM` is not a shutdown.** No handler: a stale socket and no
   final `exit=` event (self-healing on the next bind, but the last event
   is lost).
4. **The replay ring is bytes at the old width.** A full-screen program
   flashes garbage until it redraws.
5. **The quality gate never linted this code.** `cargo clippy` on the
   Windows host skips every `cfg(target_os = "linux")` line; two warnings
   passed the gate. Fixed in the same change set: `just lint` runs clippy
   for the Linux crates against the musl target.

## Decision

Decision 0006 stands, confirmed by measurement: one detached supervisor
per session, PTY on one side, Unix socket on the other, a thin
byte-moving client that Windows Terminal runs through `wsl.exe`. Neither
session count nor terminal fidelity is a risk for F1.

F1's `willie-sess` keeps from the prototype: double fork + `setsid` +
readiness pipe (the intermediate keeps `setsid` so the supervisor never
leads its session); framing in the client → supervisor direction only;
the client writing to file descriptor 1 directly (never a line-buffered
handle); `SIGWINCH` installed without `SA_RESTART` so the interrupted
read is the notification; `/dev/ptmx` plus ioctls; fail-closed socket
path handling; the session log as the source of lifetime events
(`events.jsonl` is this, in JSON).

F1's design must add: a per-session control channel (a second socket or
a control connection) carrying close reasons, the harness exit code and
the daemon's stop request; a close-on-exec errno pipe from the harness
child so the handshake confirms the harness; `SIGTERM`/`SIGHUP` as a
clean shutdown that writes the final event and unlinks the socket; replay
suppressed while the harness is in the alternate screen
(`ESC [ ? 1049 h/l`); a bounded queue and writer thread per client so a
wedged terminal cannot stall the PTY pump (0006 allows several clients;
that is only safe with per-client queues); `chdir("/")` in the supervisor
with the project directory passed to the harness explicitly.

## Consequences

- A parked session costs 256 KiB and no CPU; a keystroke costs 0.17 ms —
  a third of one engine RPC (0010). Budgets in F1 are set by the harness,
  not by the transport.
- The risks left for the session slice are the ones S2 did not touch:
  the sandbox around the harness (S3) and re-adoption of running
  supervisors by a restarted daemon.
- The image gains `LANG=C.UTF-8` as the default locale and a `{}`
  placeholder for `~/.claude.json` (F1 change to `distro/provision.sh`).
