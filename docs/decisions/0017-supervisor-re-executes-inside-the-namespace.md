# 0017 — the supervisor re-executes inside the namespace to apply and report what the helper cannot, and a session that reports nothing refuses

- **Date:** 2026-09-05
- **Status:** accepted

## Context

Part 1 confined a session with namespaces and mounts and recorded a
hardcoded `sandbox_applied` list. Three things the enforcement's second
part needs cannot be done from outside the namespace: the resource
limits (`RLIMIT_NPROC` is counted per user namespace since kernel 5.14,
so a limit set outside bounds the whole distribution's `willie` user,
not the session), a report of which mechanisms actually applied (only a
process inside can measure it), and — in the phases that follow — a
seccomp filter with a notification listener and Landlock, both of which
restrict the calling process and everything it spawns.

## Decision

The supervisor re-executes itself as bwrap's command,
`willie-sess --inner <fd>`, on a socket pair inherited through the
helper. The inner stage proves the namespace is real (its own uid map is
remapped and `no_new_privs` is set), applies the limits from inside,
reports over the socket which mechanisms applied and which
required-optional ones the kernel does not offer, marks the socket
close-on-exec so the harness cannot inherit it, and execs the harness.

The supervisor blocks on that report before announcing the session
ready. A report naming every required mechanism starts the session and
records the measured list; a report short a required mechanism, a
refusal, a helper that died before reporting, or nothing within a
two-second deadline, each refuses with a code. The part-1 branch that
gave up waiting and started the session anyway is removed: readiness
means a running, confined harness, or it means a refusal.

An exec failure inside the stage happens after the report was sent, so
the session ends as an ordinary `exited 127` rather than a coded
refusal. That window is narrow — the supervisor checked the binary is
executable moments earlier — and is accepted.

## Consequences

- The applied mechanisms are measured, not asserted; a session that ran
  with less says so, and the required-subset rule has something to stand
  on.
- The limits bound the session, not the user.
- Phases 2 and 3 add one field to the request and one to the report; they
  do not touch the readiness path again.
- The supervisor binary is bound read-only inside the namespace, at its
  own path, the way the harness binary already is.
- A helper that did not pass an inherited descriptor to its command would
  break the report; the integration tests and the acceptance walk confirm
  bubblewrap does.

## Not decided

- Whether the harness pid should travel over the socket as
  `SCM_CREDENTIALS` rather than being resolved from `/proc`; phase 2
  introduces cmsg handling and may fold it in.
- Per-project or configurable limit values; the three are fixed until a
  project needs otherwise.
