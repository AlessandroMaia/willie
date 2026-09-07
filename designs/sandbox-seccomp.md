# Sandbox part 2, phase 2 — the syscall filter and the log of what it denied

**F3b, phase 2 of five.** The inner stage phase 1 built now installs a
seccomp-bpf filter and hands the supervisor a notification descriptor, so
every syscall the filter refuses is not merely denied but **observed**:
named in the session's event log rather than reaching the user as the
agent's own confused message.

Read `designs/sandbox.md` — "The denial log" — and `designs/sandbox-report.md`
for the inner stage this builds on. Decision 0016 measured seccomp user
notification present; 0016 left "disable nested user namespaces through the
helper or the filter" open, and this phase closes it. This phase's decision
is 0018.

## Problem

### A blocked syscall is invisible

Part 1's boundary is mounts and namespaces; nothing filters syscalls. An
agent that tries to trace another process, load a kernel module, or open a
raw socket either succeeds (if the namespace happens to allow it — nested
user namespaces still work, 0016) or fails with an error only the agent
sees. There is no record that the session tried, so "why did it fail" has
no answer outside the agent's transcript, and "what did the sandbox stop"
has no answer at all.

### The filter must produce an event, and only the notification kind can

Of the boundary's mechanisms, only seccomp user notification produces an
event on this kernel. Mounts deny by absence (a missing file, a read-only
filesystem — the child sees the errno, nobody else). Landlock is silent by
design. So the syscall filter is the one place the sandbox can name what it
refused, and it must be installed with `SECCOMP_FILTER_FLAG_NEW_LISTENER`,
which bubblewrap's `--seccomp` cannot do.

### Nested namespaces are still open

0016 measured `unshare -U` succeeding from inside the sandbox and left the
close to "part 2". A session that can nest a user namespace can regain
capabilities inside it; the filter is where that closes, together with
`--disable-userns` on the vector, so it is shut from both sides.

## Goals

- A syscall filter installed inside the namespace, denying the dangerous
  classes, with one narrow exception (the netlink route protocol).
- Every denial reaches the supervisor and becomes a `sandbox_denied` event
  naming the syscall; repeated probes of the same denied call are coalesced
  so a hostile loop cannot flood the log.
- Nested user namespaces closed on both sides: `unshare`/`setns` denied by
  the filter, `--disable-userns` on the vector.
- Seccomp joins the required subset: a kernel without user notification
  refuses the session.
- The filter program is data, tested against synthetic syscalls on any
  host, with no kernel.

## Non-goals

- **`TIOCSTI` as the primary defence.** The kernel already refuses terminal
  input injection (0016: `LEGACY_TIOCSTI` unset). The filter denies
  `ioctl(TIOCSTI)` as depth, and the terminal *output* filter is phase 4,
  a different mechanism entirely.
- **Filtering by syscall argument beyond `ioctl`, `socket` and `prctl`.**
  The filter reads the low word of the relevant argument for those three
  and is otherwise a syscall-number allow/deny. Deep argument inspection
  is a seccomp anti-pattern (TOCTOU on pointers) and buys nothing here.
- **A per-project filter, or a configurable deny list.** The list is fixed
  and part of the base boundary.
- **Closing file denials into the log.** Landlock and mounts stay silent to
  everyone but the child; only the syscall class is nameable (0016).

## Design

### The filter as data — `willie-linux/src/sandbox/seccomp.rs`

x86_64 syscall numbers as local constants (the crate stays portable; a
Linux-only test asserts each against `libc::SYS_*`). The program:

1. Load `seccomp_data.arch`. Not `AUDIT_ARCH_X86_64` → `KILL_PROCESS`.
   This also catches the x32 ABI (the high bit of the syscall number),
   which could otherwise reach a denied call by another number.
2. Load `seccomp_data.nr`. Each number in the deny list → `USER_NOTIF`.
3. `ioctl`: load the low word of `args[1]`. `TIOCSTI` → `USER_NOTIF`,
   else `ALLOW`.
4. `socket`: load the low word of `args[0]` (domain) and `args[1]` (type).
   `AF_PACKET` → deny. `AF_NETLINK` → deny unless `args[2]` (protocol) is
   `NETLINK_ROUTE`. Any domain with `SOCK_RAW` or `SOCK_PACKET` in the
   type → deny (the kernel rewrites `AF_INET` + `SOCK_PACKET` into a
   packet socket, so the family check alone misses it). Otherwise
   `ALLOW`.
5. `prctl`: load the low word of `args[0]` (option). `PR_SET_SECCOMP` →
   `USER_NOTIF` — a filter can be installed this way too, without ever
   calling `seccomp(2)`, so it gets the same refusal for the same
   reason. Every other option → `ALLOW`.
6. Everything else → `ALLOW`.

The deny list: `ptrace`, `process_vm_readv`, `process_vm_writev`,
`pidfd_getfd`, `bpf`, `io_uring_setup`, `io_uring_enter`,
`io_uring_register`, `perf_event_open`, `userfaultfd`, `seccomp` (a
nested filter with a listener would receive the notifications for the
calls this one marks, and could continue them), `mount`, `umount2`,
`pivot_root`, `mount_setattr`, `open_tree`, `move_mount`, `fsopen`,
`fsconfig`, `fsmount`, `fspick`, `unshare`, `setns`, `init_module`,
`finit_module`, `delete_module`, `kexec_load`, `kexec_file_load`,
`add_key`, `request_key`, `keyctl`.

`prctl(PR_SET_SECCOMP)` is refused for the same reason as `seccomp`
itself — it installs a filter without ever calling `seccomp(2)` — so the
two are the deny list's only "a filter of its own" refusals; `prctl` is
judged by argument, like `ioctl` and `socket`, since its other options
must pass.

`clone` and `clone3` are left untouched: the namespace-creating flags are
refused by the kernel because the vector gains `--disable-userns` and an
unprivileged process creates no other namespace. Denying `clone` outright
would break threads; denying it by flag argument is the TOCTOU anti-pattern
above. This is the two-sided close 0016 asked for.

The netlink-route exception is the design's one exception: `iproute2`
(`ip`, which the image ships) enumerates interfaces over
`AF_NETLINK`/`NETLINK_ROUTE`, and a session whose network is open learns
the same addresses by connecting out, so denying it breaks ordinary tools
and buys nothing.

A `name(nr) -> &'static str` table gives the log the syscall's name.

### Installing it — `willie-sess/src/sandbox/seccomp.rs` (Linux)

Inside the inner stage, after the limits and before the report:

```rust
let fd = seccomp(SECCOMP_SET_MODE_FILTER,
                 SECCOMP_FILTER_FLAG_NEW_LISTENER, &prog)?;  // returns the listener fd
```

`EINVAL`/`ENOSYS` here is a kernel without user notification →
`Refused { sandbox_backend_missing }`, because seccomp is required. Any
other errno → `Refused { sandbox_apply_failed }`. The listener descriptor
travels to the supervisor **inside** the `Applied` message as an
`SCM_RIGHTS` ancillary payload; the inner then closes its own copy and sets
nothing inheritable, so the harness never holds the listener — a process
that held its own listener could approve its own syscalls with
`SECCOMP_USER_NOTIF_FLAG_CONTINUE`. All of `seccomp_notif`,
`seccomp_notif_resp`, the two ioctls, `SECCOMP_RET_USER_NOTIF` and the
listener flag are in the pinned `libc`.

### The notification loop — `willie-sess/src/sandbox/seccomp.rs`

The supervisor receives the descriptor with the report and spawns a thread:

```
loop {
    ioctl(fd, SECCOMP_IOCTL_NOTIF_RECV, &mut notif);   // blocks
    let name = seccomp::name(notif.data.nr);
    ioctl(fd, SECCOMP_IOCTL_NOTIF_SEND, &resp{ id: notif.id, error: -EPERM });
    tally.record(name);
}
```

Every intercepted syscall is answered `EPERM` — one class, not two: a
filter where some refusals are logged and others silent is a distinction to
maintain with nothing to show for it. A `NOTIF_SEND` that returns `ENOENT`
(the process died before the answer) is ignored. If the loop ends on any
other error, the descriptor drops with it and every intercepted syscall
then fails `ENOSYS` — the closed direction — and the session records
`sandbox_degraded { mechanism: "seccomp", message }` and continues, as the
design promises.

### The coalesced log — `willie-sess/src/sandbox/seccomp.rs`, `Tally`

A pure `Tally` with an injected clock, tested on the host and living in
`Shared`:

- the first denial of a given syscall emits `sandbox_denied { class:
  "syscall", name, count: 1 }` at once;
- further denials of that syscall accumulate and emit as one event with the
  running count when 5 s have passed since that syscall last emitted, or at
  `finish`, before `exited`.

`class` is on the event from the start because phase 4's terminal filter
reuses this same `Tally` and event with `class: "terminal"`. `apply_event`
folds by (class, name) into `SandboxState.denied: Vec<Denied { class,
name, count, first_at, last_at }>`, so the UI sees per-syscall totals.
`SandboxState` also gains `degraded: Vec<String>`.

### Errors and edge cases

| Condition | Code | Behaviour |
| --- | --- | --- |
| kernel without seccomp user notification | `sandbox_backend_missing` | inner refuses; seccomp is required |
| the filter is rejected by the kernel | `sandbox_apply_failed` | inner refuses, naming the errno |
| the listener fd cannot be sent to the supervisor | `sandbox_apply_failed` | inner refuses |
| `Applied` names seccomp but carries no descriptor | `sandbox_apply_failed` | supervisor treats it as a refusal, not as applied |
| the notification thread dies | — | intercepted syscalls fail `ENOSYS`; `sandbox_degraded` recorded; session continues |
| `NOTIF_SEND` returns `ENOENT` | — | the process died first; ignored |
| a hostile loop probes one denied syscall repeatedly | — | first denial immediate, the rest coalesced ≤ every 5 s |

## Testing

Host, no kernel — a minimal BPF interpreter (only the handful of
instruction forms the program uses) evaluates the program against synthetic
`seccomp_data`:

- each denied number returns `USER_NOTIF`;
- `ioctl(TIOCGWINSZ)` allows, `ioctl(TIOCSTI)` denies;
- `socket(AF_INET, SOCK_STREAM)` allows;
- `socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE)` allows (iproute2's own
  call); `socket(AF_NETLINK, SOCK_DGRAM, NETLINK_KOBJECT_UEVENT)` denies;
- `socket(AF_PACKET, SOCK_RAW)` denies;
- `prctl(PR_SET_NAME)` allows, `prctl(PR_SET_SECCOMP)` denies;
- `clone3` and `execve` allow;
- the wrong `arch`, and an x32 syscall number, kill;
- every jump lands inside the program (structural);
- each deny-list number maps to `libc::SYS_*` (Linux-only);
- the `Tally` coalesces: first immediate, repeats within the window folded,
  a flush at `finish`.

Distro, through `just test-linux`:

- a fake harness runs `unshare -U true`, writes its exit code — non-zero,
  and `sandbox_denied` names `unshare`;
- a 200-iteration loop over a denied call produces at most three events
  whose counts sum to 200;
- `ip link show` exits 0 and produces no event;
- a raw `socket(PF_PACKET, SOCK_RAW)` fails `EPERM` and logs `socket`;
- the confinement test asserts `/proc/self/status` shows `Seccomp: 2`.

## Rollout / compatibility

`sandbox_denied` and `sandbox_degraded` are new event kinds;
`Session.sandbox` gains `denied` and `degraded`, both defaulted — additive.
The vector gains `--disable-userns`, which the image's bubblewrap (0.12.0)
supports. Seccomp joins the required subset, so a kernel without user
notification refuses; this kernel has it (0016), so no working session
changes. The release note says the syscall filter is now enforced and its
denials are logged, and that nested user namespaces no longer work.

## Open questions

- Should `clone3` be forced to `ENOSYS` in the filter (the trick that makes
  glibc fall back to `clone`)? Favoured: no — it is a second, silent class
  of refusal, unnecessary once `--disable-userns` closes namespace nesting.
- Should the coalescing window be tunable? Favoured: no — five seconds
  keeps a hostile loop to a trickle and a genuine denial visible at once.
