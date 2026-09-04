# 0016 — the required subset holds on this kernel; the launch clears the environment, and the stop ladder reaches the harness through the helper (S3, measured)

- **Date:** 2026-09-04
- **Status:** accepted

## Context

`designs/sandbox.md` confines a session with namespaces and mounts,
a syscall filter, resource limits and, when the kernel offers it,
path-based restriction, all applied through the image's namespace
helper. Spike S3 (docs/ARCHITECTURE.md §5.4) measured what the kernel
the distribution runs on actually provides, and how a session's
lifecycle — start, stop, exit status — travels through that helper.
Environment: WSL 2.6.1.0, kernel 6.6.87.2, Debian 13, bubblewrap
0.12.0, user `willie` (1000), Claude Code 2.1.251 as installed by the
Dashboard.

## Findings

### Compile-time, from the kernel's own configuration

| Option | Value | Consequence |
| --- | --- | --- |
| USER_NS, PID_NS, IPC_NS, UTS_NS, NET_NS | y | every namespace the base asks for exists |
| SECCOMP, SECCOMP_FILTER | y | a filter, and user notification (kernel ≥ 5.0) |
| SECURITY_LANDLOCK | y, first in the built-in LSM list | Landlock is compiled in and active by default |
| LEGACY_TIOCSTI | not set, and `dev.tty.legacy_tiocsti` is 0 | the kernel itself refuses terminal input injection; a seccomp rule is depth, not the defence |
| SECURITY_YAMA | y, `ptrace_scope` 1 | ptrace scope applies |
| BINFMT_MISC | y; the interop entry is registered with the `F` flag | the kernel holds the interpreter open, so an absent `/init` does **not** stop a Windows executable — see the runtime table |
| PROC_CHILDREN | y | `/proc/<pid>/task/<pid>/children` exists, so the harness pid can be found through the helper |

### Runtime, measured inside the distribution

| Question | Result |
| --- | --- |
| Landlock ABI (`landlock_create_ruleset(NULL, 0, VERSION)`) | measured: 3 — filesystem rules, no network rules |
| `/sys/kernel/security/lsm` | measured: absent — securityfs is not mounted, which is why `willie doctor` prints `[skip] landlock LSM`; the check must ask the kernel through the syscall instead |
| unprivileged user namespace | measured: ok, also with pid namespace and a fresh procfs |
| seccomp `SECCOMP_GET_NOTIF_SIZES` | measured: 0, sizes 80/24/64 — user notification available |
| the base vector starts; inside: uid 1000, `NoNewPrivs 1`, no capabilities, pid 1 is the helper's reaper | measured: yes |
| `/mnt/c`, `/init`, `/run/WSL`, `/run/willie`, `/var/lib/willie`, the real home | measured: all absent |
| the interop environment (`WSL_INTEROP`, `WSL_DISTRO_NAME`, `WSLENV`, the terminal's own variables) | measured: **inherited into the sandbox** — mounts do not clear an environment; the helper's `--clearenv` does |
| a Windows executable bound inside | measured: refused, exit 1 — but not because the interpreter is missing: the kernel runs `/init` from the descriptor its binfmt entry holds, and that only fails because `/run/WSL` is not in the namespace. With `/run/WSL` bound in, the same executable runs and prints |
| `sudo` | measured: inert — permission denied, exit 127 |
| the project read-write, the managed tools read-only, the git configuration read-only, the private home writable | measured: all four as designed — a write into `~/.local/bin` and one into `~/.gitconfig` were each refused as a read-only file system |
| `agent.state` as a directory bind plus the two links; the CLI's version, login and one prompt | measured: works, the prompt was answered; `~/.claude.json` is still a link after the session, and the file it points at has different contents |
| exit status through the helper | measured: `exit 7` → 7; harness killed by TERM → 143 (128 + n); harness exiting 3 from its own signal handler → 3 |
| SIGINT or SIGTERM to the harness pid only | measured: the harness decides, and its own status is what the helper reports — a handler that exited 3 gave 3, a harness ignoring SIGTERM ran on and gave 0. The monitor is not the target and is not killed by it |
| SIGINT to the whole process group | measured: the monitor dies with 130 — it catches nothing and blocks only SIGCHLD; SIGTERM gives 143 and SIGKILL 137, and nothing of the sandbox outlives the monitor |
| terminal input injection from inside | measured: EIO, inside the sandbox and outside it alike |
| a nested user namespace from inside | measured: allowed until the syscall filter arrives |

## Decision

Namespaces and mounts, the syscall filter and the limits are the
**required subset**; a session refuses to start without them.
Landlock is applied when the ABI probe answers and its absence is
reported, never assumed away. `willie doctor` probes Landlock with the
syscall, not with securityfs.

The **environment is cleared and rebuilt**, not merely unmounted. What
keeps a Windows executable out of a session is that `/run/WSL` — the
interop socket the kernel's interpreter connects to — is not in the
namespace, so `/run` is never bound; and `WSL_INTEROP` names that
socket, so the launch clears the environment and sets back only the
allowlist a session needs. Mounts alone leave both halves reachable
the moment a policy widens `extra.paths` towards `/run`.

The supervisor's stop ladder sends `SIGINT` and `SIGTERM` to the
**harness process**, found through the helper as the only child of the
helper's only child, and `SIGKILL` to the whole process group. The
helper's monitor is never sent the polite signals, because it dies of
them, and with `--die-with-parent` the sandbox goes with it: the harness
would end without ever seeing the signal that was meant for it.
When the harness pid cannot be resolved, the rung goes to the group:
that ends the session, which is the closed direction.

The helper reports a harness killed by signal `n` as exit `128 + n`;
the supervisor maps codes 129–192 back to the signal so the event log
says what happened. A harness that itself exits with such a code is
recorded as a signal death; that convention is the shell's own and the
ambiguity is accepted.

The pid a session records at `started` is the helper's monitor, the
supervisor's direct child; the harness pid is resolved when needed.

## Consequences

- Sessions become confined: no Windows drive outside the project, no
  Windows executable, no write into the managed tools, no real home.
  `extra.paths` is where a workflow that needs more comes back, and
  `/run` is the one path it must never be allowed to name.
- The launch owns the environment. A session that inherits the
  supervisor's environment is not confined however right its mounts
  are, so the base carries `--clearenv` and the allowlist is built at
  the call site.
- The doctor's Landlock line changes from a securityfs read to a
  syscall probe (part 2).
- The terminal filter stays as designed; the kernel already refuses
  input injection, so the filter defends the emulator side only.
- A test that measures the helper's signal behaviour must start it the
  way the supervisor does, with default dispositions and its own
  process group. Started as a shell's background job the helper has
  SIGINT and SIGQUIT already set to ignore, and that artefact makes the
  monitor look like it survives the polite signals when it does not. A
  terminal-injection test carries a trap of its own: the ioctl needs a
  writable buffer, and with a literal it is never issued at all.

## Not decided

- What the environment allowlist must contain. The spike measured only
  that clearing the environment empties it, never that the harness
  starts from a rebuilt one; the spike's own failure on a merely
  non-login `PATH` shows how sharp that edge is. The list the launch
  sets back is the one the harness already builds; whether it is
  complete, and what the network slice must add to it, is settled by
  the first session that runs confined.
- Whether to also disable nested user namespaces through the helper's
  own option or only through the syscall filter; part 2 chooses.
- A size ceiling for the private home; none until a session hurts.
