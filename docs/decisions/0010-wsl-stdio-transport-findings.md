# 0010 — stdio through `wsl.exe --exec` is the engine↔daemon transport (measured)

- **Date:** 2026-08-26
- **Status:** accepted

## Context

Decision 0005 chose the daemon's stdio, forwarded by `wsl.exe --exec`, as
the first transport. Before building on it we measured the properties the
design depends on, against a scratch distribution registered from the
pinned base image — WSL 2, Debian 13 (trixie). The numbers come from
`crates/willie-engine/tests/wsl_stdio.rs`, which stays in the tree and
skips with a printed reason unless `WILLIE_TEST_DISTRO` names a
registered distribution, plus manual probes for the window and
process-death questions.

## Findings

| Question | Result |
| --- | --- |
| Round-trip latency of one ndjson line through `/bin/cat` | median **0.47–0.55 ms** over 100 lines (three runs: 546.4 µs, 514.5 µs, 474.8 µs) |
| Cost of the first line, which also pays for starting the child | **846 ms** with the VM cold, **226 ms** with it warm; an isolated probe puts the first round trip at 869 ms cold and 214–221 ms warm, against 0.60–0.70 ms for the second |
| Bytes written by the Linux child | raw UTF-8, unchanged (`olá ✓` round-trips) |
| `wsl.exe`'s own messages | UTF-16LE; decoded by `text::decode_wsl_output`; no replacement characters |
| Console window when spawned from a GUI process | none with `CREATE_NO_WINDOW`, over 16 spawns; the same command without the flag opens one, so the probe does detect windows |
| Linux child when its Windows parent is killed | **dies** with it, reproduced twice — teardown is free, but the engine must not depend on the child outliving it, and the daemon still exits on stdin EOF for a graceful stop |
| Host prerequisite | creating the WSL 2 VM requires `NT VIRTUAL MACHINE\Virtual Machines` to hold "Log on as a service"; otherwise registration fails with HCS `0x80070569` (`ERROR_LOGON_TYPE_NOT_GRANTED`) |
| `.tar.xz` accepted by `wsl --import` | n/a — the base is a gzip layer, imported directly |

## Decision

Keep stdio as the transport. `willied` exits when stdin reaches EOF; the
engine never relies on process-tree death. Messages are ndjson UTF-8;
anything non-JSON on the daemon's stdout is logged and ignored.

## Consequences

- Steady-state calls are sub-millisecond, so the per-call budget is set
  by the first line and not by the median: the RPC client allows seconds
  for the first request after a spawn (869 ms observed with a cold VM)
  and a much shorter timeout afterwards. A 50 ms floor would be wrong for
  the first call on any machine.
- `xtask distro build` imports the base as a gzip layer, with no
  conversion step.
- The engine's `doctor` maps HCS `0x80070569` to this remediation — the
  virtual-machine account needs the "Log on as a service" right. The raw
  message says only "logon failure", which tells the user nothing about
  what to ask an administrator for.
- Stopping the engine is enough to stop the daemon; a session supervisor
  that has to outlive it must detach itself. That belongs to 0006 and is
  S2's to confirm.

## Not decided

TCP loopback and Hyper-V sockets remain unexplored until a second client
exists.
