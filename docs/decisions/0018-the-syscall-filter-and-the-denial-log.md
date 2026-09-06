# 0018 — the session runs behind a syscall filter whose every refusal the supervisor answers and records, and nested user namespaces are closed from both sides

- **Date:** 2026-09-05
- **Status:** accepted

## Context

Part 1 confined a session with namespaces and mounts, and 0017 put a
stage of the supervisor inside the namespace to apply and report what
the helper cannot. Two things were still open. A blocked operation was
invisible: mounts deny by absence and only the child sees the error,
path-based restriction is silent by design, so nothing a session tried
and was refused ever reached the event log — and of the boundary's
mechanisms, only seccomp user notification produces an event on this
kernel, which 0016 measured present. And a nested user namespace still
worked from inside the sandbox; 0016 left "through the helper's own
option or through the syscall filter" to part 2.

Two constraints shaped the answer. A filter with a listener cannot come
from the helper's own `--seccomp`, which installs a program and nothing
more; it has to be installed by a process inside the namespace, which
the stage now is. And `seccomp(SET_MODE_FILTER)` is refused (`EACCES`)
to an unprivileged process unless `no_new_privs` is set; the helper sets
that bit for an unprivileged user namespace, and the stage already
refuses the session unless it reads `NoNewPrivs 1` (0017), so the
filter rides on a bit that is checked, not assumed.

## Options

Closing nested user namespaces:

| Option | For | Against |
| --- | --- | --- |
| the filter alone | one mechanism, and the attempt is named in the log | `unshare`/`setns` are refused, but `clone(CLONE_NEWUSER)` is another path: `clone3` carries its flags in a structure behind a pointer no filter can read, so closing it means denying `clone3` outright, which breaks threads |
| the helper's `--disable-userns` alone | closes every path, `clone` included | silent: a refused `unshare` is an error to the child and nothing in the log |
| both | the filter names the attempt, the helper closes the path the filter cannot | two mechanisms to keep in step, and each must be known to be load-bearing |

Who may hold the notification listener:

| Option | For | Against |
| --- | --- | --- |
| the stage keeps it close-on-exec until `exec` | nothing to do | a window in which the process about to become the harness holds a descriptor that can approve its own syscalls |
| the stage drops it the moment the report is sent | no such window | the send has to have happened first — it has: once `sendmsg` returns, the message in flight holds its own reference to the listener, and the stage closing its copy does not close it |

## Decision

The stage installs a classic-BPF filter, built as portable data in
`willie-linux` and tested against synthetic syscalls on any host, right
after the limits and before its report. Any architecture but the native
64-bit one, and the x32 ABI, kill the process. The calls a session has
no business making are marked for user notification: another process's
execution and memory (`ptrace`, `process_vm_readv`, `process_vm_writev`,
`pidfd_getfd`); programs and rings inside the kernel and its
instrumentation (`bpf`, `io_uring_setup`, `io_uring_enter`,
`io_uring_register`, `perf_event_open`, `userfaultfd`); the mount table
through the old interface and the new one (`mount`, `umount2`,
`pivot_root`, `mount_setattr`, `open_tree`, `move_mount`, `fsopen`,
`fsconfig`, `fsmount`, `fspick`); leaving this namespace or making
another (`unshare`, `setns`); kernel modules and a replacement kernel
(`init_module`, `finit_module`, `delete_module`, `kexec_load`,
`kexec_file_load`); the kernel keyring (`add_key`, `request_key`,
`keyctl`); `ioctl(TIOCSTI)`; and, judged by their arguments, packet
sockets, raw sockets of any other family, and every netlink protocol but
one. The one exception is the netlink route protocol, which listing
interfaces and addresses opens as a raw netlink socket: a session whose
network is open learns the same addresses by connecting out, so denying
it breaks ordinary tools and buys nothing. Everything else is allowed;
in particular `clone` and `clone3` are left untouched.

The filter is installed with a user-notification listener, and the
listener travels to the supervisor inside the `Applied` report as an
`SCM_RIGHTS` control message. The stage then drops its own copy at once,
not merely close-on-exec: a process holding its own listener could
approve its own intercepted calls with the *continue* flag, so the
descriptor never survives the exec. On the supervisor's side a report
that names seccomp without a listener, or whose control message is
truncated or carries more than one descriptor, is refused as
`sandbox_apply_failed` — a descriptor that cannot be trusted is closed,
never kept. A kernel that refuses the install with `EINVAL` or `ENOSYS`
has no user notification, and the stage refuses
`sandbox_backend_missing`; any other errno is `sandbox_apply_failed`.
Seccomp joins the required subset: the stage reports `namespaces`,
`mounts`, `rlimits` and `seccomp`, and a report short any of them
refuses the session.

The supervisor answers every intercepted call `EPERM` and records it,
from a thread started before the session is announced ready, so a
harness whose very first call is intercepted only waits for the answer.
One class, not two: a filter where some refusals are logged and others
are silent is a distinction to maintain with nothing to show for it. The
thread waits in `poll`, not in the receive `ioctl`: a receive with
nothing pending sleeps on the filter and is not woken when the last
process under the filter exits, whereas `poll` reports that as
`POLLHUP` — the loop's one orderly end, and what tells a session ending
from a server dying under a live one. The latter is recorded as
`sandbox_degraded { mechanism: "seccomp", message }` and the session
continues: the listener dropped with the thread, so the kernel answers
every later intercepted call `ENOSYS` — closed, not open. A thread that
cannot start is the same condition from the first call.

Denials are coalesced per (class, name) by a pure tally with an injected
clock. The first refusal of a syscall is recorded at once as
`sandbox_denied { class: "syscall", name, count: 1 }`; repeats fold and
are recorded as one count once five seconds have passed since that row
last reported; whatever is still folded is flushed before `exited` is
recorded, because every reader of the log stops folding there. A hostile
loop over a denied call costs one round trip per probe and a trickle of
events, not a stall and not a flood. `class` is on the event from the
start so the terminal filter can record its drops through the same
tally.

Nested user namespaces are closed from both sides, and both sides are
load-bearing. The filter refuses `unshare` and `setns`, which is what
names the attempt in the log; but `clone` and `clone3` stay allowed —
denying them breaks threads, and `clone3` keeps its flags where no
filter can read them — so a `clone(CLONE_NEWUSER)` would slip past the
filter. The vector therefore also carries `--disable-userns`, which sets
the sandbox's `max_user_namespaces` to zero, so even an allowed `clone`
cannot make one. Neither closes the path alone.

## Consequences

- The sandbox can now say what it refused: a denied syscall is a named
  line in the session's log with a count, and the session record carries
  the totals per syscall (`Session.sandbox.denied`) and the mechanisms
  that fell back (`Session.sandbox.degraded`) for the Sessions screen to
  show.
- A session cannot trace or read another process, alter the mount table,
  load a module, open a packet or raw socket, or nest a user namespace.
  A workflow that did any of these stops, with `EPERM` and a line in the
  log that says so.
- Seccomp is required: a kernel without user notification refuses every
  session. This kernel has it (0016), so no working session changes.
- The filter is a fixed list in the base, not configurable; a call that
  turns out to be needed is a change to the list and a release note, not
  a project setting.
- The report channel carries a descriptor, so the supervisor reads it
  with room for exactly one descriptor's control message; anything else
  in that message is a refusal, not a surprise.
- A test that runs a denied call inside the sandbox expects `EPERM` and
  a `sandbox_denied` line; a test of the coalescing counts events by
  their summed `count`, not by their number.

## Not decided

- Forcing `clone3` to `ENOSYS` in the filter, the trick that makes a C
  library fall back to `clone`. Rejected for now: it would be a second,
  silent class of refusal, and `--disable-userns` already closes the
  nesting it would guard against.
- A tunable coalescing window. Five seconds keeps a hostile loop to a
  trickle and a genuine denial visible at once; none until a session
  hurts.
- The harness pid over the socket as `SCM_CREDENTIALS` (0017). The
  control message now carries exactly one descriptor and nothing else,
  and the pid is still resolved through `/proc`; widening the message
  would widen the trust rule above.
- Recording file denials. Mounts and path-based restriction stay silent
  to everyone but the child (0016); the syscall class is the one the
  sandbox can name.
