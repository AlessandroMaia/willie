# Sandbox part 2, phase 1 — the report of what applied, and the limits

**F3b, phase 1 of five.** Part 1 confines a session with namespaces and
mounts and records `sandbox_applied { mechanisms }` with a hardcoded
two-item list. This phase makes that list a measurement instead of a
constant: the supervisor re-executes itself inside the namespace, applies
the resource limits there, and reports back which mechanisms actually took
effect. The later phases plug seccomp and Landlock into that machine
rather than editing the constant by hand.

Read `designs/sandbox.md` first — "The boundary", "Applying it" and "The
required subset". Decision 0016 measured the kernel. This phase's decision
is 0017.

## Problem

### The applied list is a constant, not a measurement

`crates/willie-sess/src/sandbox.rs` defines `MECHANISMS: [&str; 2] =
["namespaces", "mounts"]` and the supervisor emits exactly that, always.
Nothing checks that the namespaces were unshared or that the mounts took;
the event asserts what the code intended, not what the kernel did. The
design's own "required subset" — a session refuses to start when a required
mechanism is unavailable — has nothing to stand on, because no mechanism is
observed.

### The limits have nowhere to be applied

`designs/sandbox.md` lists resource limits (`NPROC`, `NOFILE`, `CORE=0`) in
the base, marked *second enforcement plan*. They cannot be set by the
supervisor before it spawns the helper: `RLIMIT_NPROC` is counted per user
namespace since kernel 5.14, so a limit set outside the session's own user
namespace bounds the `willie` user across the whole distribution, not the
session. They have to be set by a process already inside the namespace.

### The later mechanisms need a process inside the namespace

The syscall filter with a notification listener (phase 2) and Landlock
(phase 3) both restrict the calling process and everything it spawns, so
both must be applied from inside, immediately before the harness's `exec`.
Bubblewrap's own `--seccomp` installs a filter with no listener, and it has
no Landlock option at all. Without a process of Willie's own inside the
namespace, phases 2 and 3 would each have to build that process and
restructure the readiness path on top of a stacked branch — the churn the
phase order exists to avoid.

## Goals

- `sandbox_applied` names the mechanisms that were **measured** to apply,
  and a separate list of the required-optional ones that did not.
- The resource limits are set inside the session's user namespace, so they
  bound the session and not the `willie` user.
- A reusable in-namespace stage — the re-executed supervisor — that phases
  2 and 3 extend by adding fields to one request and one report, without
  touching the readiness path again.
- A session that reports nothing, or reports without a required mechanism,
  **refuses to start**. Fail closed, and the "give up and start anyway"
  branch of part 1 is removed.
- Almost all of it testable on any host: the request, the report, the
  vector, the required-subset check, the fold into the session.

## Non-goals

- **The syscall filter and Landlock themselves.** This phase builds the
  stage they run on and applies only the limits. Phases 2 and 3.
- **The denied log.** No mechanism this phase applies produces a denial
  event; the limits deny by the kernel returning an error to the agent.
  Phase 2 brings `sandbox_denied`.
- **Showing the report in the interface.** It lands in the event log and
  the session record; the Sessions screen reads it in phase 5.
- **Extending `sandbox explain` with the report.** Named follow-up.
- **A size ceiling on the limits' values, or per-project limits.** The
  three values are fixed; making them policy is a later slice if a project
  ever needs it.

## Design

### The shape of the launch

Part 1 runs `bwrap … -- <harness> <args>`. This phase runs
`bwrap … -- <supervisor> --inner <fd>`: the helper's command becomes
Willie's own supervisor binary, which sets the session up from inside the
namespace and only then `exec`s the harness. The two processes talk over a
socket pair created before the spawn and inherited across the fork.

```
supervisor (outside)                 bwrap monitor → reaper        inner (== the future harness)
socketpair() → (outer, inner_fd)
spawn bwrap with inner_fd inherited ───────────────────────────►  reads its fd from --inner
write(Request { argv, rlimits })  ─────────────────────────────►  read(Request)
                                                                  verify: uid_map is not the outer identity,
                                                                          NoNewPrivs == 1   (mounts/ns are fact)
                                                                  setrlimit × 3   (inside this userns)
poll(outer, 2 s)  ◄───────────────────────────────────────────   write(Report::Applied { mechanisms, .. })
check REQUIRED ⊆ mechanisms                                       set the fd close-on-exec
sandbox_applied, resolve harness pid, started, reply ok          execv(harness argv)
```

The harness's `argv` moves out of the vector and into the request: the
vector's command is now the supervisor, and the harness is what the inner
`exec`s. The one new path visible inside the namespace is the supervisor
binary itself, bound read-only at its own path — the same reasoning that
binds the harness binary in part 1.

### The request and the report — `willie-linux/src/sandbox/inner.rs`

Pure data, no I/O, in the shared Linux crate beside the plan, so both the
supervisor that writes them and the tests exercise them on any host.

```rust
/// What the inner stage must do before it execs the harness. Phases 2 and
/// 3 add fields (the seccomp program, the Landlock rules); a spec written
/// before them deserialises with those absent.
pub struct Request {
    pub argv: Vec<String>,        // the harness command the inner execs
    pub rlimits: Rlimits,
}

/// The three limits, each applied as min(value, current hard limit) so a
/// stricter host is never loosened.
pub struct Rlimits { pub nproc: u64, pub nofile: u64, pub core: u64 }

impl Rlimits {
    /// 4096 processes, 65536 descriptors, no core dump.
    pub const DEFAULT: Rlimits = Rlimits { nproc: 4096, nofile: 65536, core: 0 };
}

/// The inner's one-line answer, before it execs.
pub enum Report {
    /// The mechanisms that took effect, and the required-optional ones
    /// that this kernel does not offer (phase 3's Landlock).
    Applied { mechanisms: Vec<String>, unavailable: Vec<String> },
    /// The inner refused before applying anything it could not undo.
    Refused { code: String, message: String },
}

/// A mechanism this build always requires: absent, the session refuses.
/// Phase 2 adds "seccomp".
pub const REQUIRED: &[&str] = &["namespaces", "mounts", "rlimits"];
```

`Request` and `Report` are one JSON line each over the socket, framed by
the newline, the same shape the event log already uses. A line that does
not parse is a refusal, never a panic.

### The vector — `willie-linux/src/sandbox/{mod.rs,bwrap.rs}`

`Plan` gains `inner_exe: String`: where the re-executed supervisor lives.
`plan()` takes it as a parameter — the supervisor passes its own resolved
`/proc/self/exe`, so a test drives the staged binary rather than the
installed one; a daemon that ever renders a vector would pass
`paths::SUPERVISOR_BIN`. `Plan.argv` stays the harness command, the source
the supervisor builds the `Request` from; what changes is that
`bwrap::argv` no longer appends it. The plan's ops gain one final bind:
`inner_exe` read-only at its own path.

`bwrap::argv(plan, inner_fd)` renders the base, the ops (the inner binary
bind last), then `--chdir <workspace> -- <inner_exe> --inner <inner_fd>` —
the harness argv is absent from the rendered vector; it travels in the
`Request` instead. The `--chdir` still puts the harness in its workspace,
because the inner `exec`s without changing directory.

### The inner stage — `willie-sess/src/sandbox/inner.rs` (Linux)

`willie-sess/src/sandbox.rs` becomes the directory
`willie-sess/src/sandbox/` with `mod.rs`; this phase adds `inner.rs`, which
applies the limits itself. `seccomp.rs` and `landlock.rs` arrive with their
own phases.

The inner reads its request, then:

1. **Proves it is inside.** Reads `/proc/self/uid_map` and
   `/proc/self/status`. If the uid map maps the session's uid to the outer
   identity unchanged, or `NoNewPrivs` is not 1, the namespace was not
   built — `Refused { sandbox_apply_failed, "not inside the sandbox
   namespace" }`. This is what turns "namespaces" and "mounts" from an
   assumption into a measured fact: the inner is the first code that runs
   with the namespace in force.
2. **Applies the limits.** `setrlimit` for `RLIMIT_NPROC`, `RLIMIT_NOFILE`,
   `RLIMIT_CORE`, each clamped to the current hard limit. A failure is
   `Refused { sandbox_apply_failed, "cannot set <limit>: <errno>" }`.
3. **Reports.** `Applied { mechanisms: ["namespaces","mounts","rlimits"],
   unavailable: [] }`. Phase 3 adds `landlock` to one list or the other.
4. **Hands off.** Sets the socket fd close-on-exec so the harness never
   sees it, then `execv(argv[0], argv)`. An `exec` failure here writes
   `Refused { harness_exec_failed }` and exits 127; by then the supervisor
   has already read `Applied` and replied ready, so the session ends as
   `exited 127`. That window is narrow — `prepare` checked the binary is
   executable moments earlier — and is accepted, recorded in 0017.

### The supervisor's readiness — `willie-sess/src/sandbox/mod.rs`, `main.rs`

Part 1's `wait_for_harness` polled `/proc` for the harness to appear. That
whole poll is replaced by one read of the socket, with the same 2 s
ceiling, and four outcomes:

| Read | Behaviour |
| --- | --- |
| `Applied` with `REQUIRED ⊆ mechanisms` | append `sandbox_applied { mechanisms, unavailable }`, resolve the harness pid once (the inner exists, so the process shape exists — no poll), append `started`, reply ok |
| `Applied` missing a required mechanism | `failed { sandbox_backend_missing, "<m> unavailable" }`, kill the group, reply |
| `Refused { code, message }` | `failed { code, message }`, kill the group, reply |
| EOF (helper died before the inner ran) | drain the PTY, `apply_failure` with the helper's own words — part 1's existing path, unchanged |
| 2 s passes with nothing read | `SIGKILL` the group, `failed { sandbox_apply_failed, "the sandbox reported nothing within 2 s" }` |

The timeout branch replaces part 1's "give up and start the session
anyway": a session whose sandbox never confirmed is now a refusal, not a
started session with the promise abandoned. The `HarnessWait::GaveUp`
variant and its "starting anyway" line are removed.

### The session record — `willie-core/src/session.rs`

`SessionEventKind::SandboxApplied` gains `unavailable: Vec<String>` with
`#[serde(default)]`, so a part-1 log line still parses. `Session` gains:

```rust
/// What the sandbox reported for this session. Empty until the
/// sandbox_applied event is folded; a session from a pre-part-2 log
/// leaves it default.
#[serde(default)]
pub sandbox: SandboxState,

pub struct SandboxState {
    pub applied: Vec<String>,
    pub unavailable: Vec<String>,
    // phase 2 adds `denied` and `degraded`.
}
```

`apply_event` folds `SandboxApplied` into `session.sandbox` instead of
ignoring it. Phase 5 reads this field; phases 2 and 4 add to it.

### Errors and edge cases

| Condition | Code | Behaviour |
| --- | --- | --- |
| the inner finds it is not in a namespace | `sandbox_apply_failed` | inner refuses; the session does not start |
| a `setrlimit` fails | `sandbox_apply_failed` | inner refuses; the message names the limit and errno |
| the inner reports without a required mechanism | `sandbox_backend_missing` | supervisor refuses; the message names the mechanism |
| nothing is read within the ceiling | `sandbox_apply_failed` | supervisor kills the group and refuses |
| the helper dies before the inner runs | as part 1 | drain the PTY, carry the helper's words (unchanged) |
| the inner's `exec` of the harness fails | `harness_exec_failed` | ready was already sent; the session ends `exited 127`; accepted (0017) |
| a request or report line does not parse | `sandbox_apply_failed` | treated as a refusal, never a panic |

## Testing

Host, no kernel:

- the `--inner <fd>` argument parses;
- `Request`/`Report` round-trip through their JSON line, and a malformed
  line is an error not a panic;
- `Rlimits::DEFAULT` clamps each value to a smaller hard limit;
- `bwrap::argv` binds `inner_exe` read-only and ends
  `-- <inner_exe> --inner <fd>`, with the harness argv absent from the
  option list;
- `REQUIRED ⊆ applied` accepts the full set and rejects a missing member;
- `apply_event` folds `SandboxApplied` into `Session.sandbox` and a
  part-1 log line (no `unavailable`) still parses.

Distro, through `just test-linux`:

- a fake harness writes `/proc/self/limits` into the workspace; the test
  reads back 4096 / 65536 / 0;
- `sandbox_applied` names `["namespaces","mounts","rlimits"]` before
  `started`;
- a fake helper that `exec`s the inner's command **without** bwrap makes
  the inner refuse with "not inside the sandbox namespace";
- a fake helper that sleeps forever produces the timeout refusal and the
  process group is killed;
- part 1's "helper refuses after exec" test still holds.

## Rollout / compatibility

`SandboxApplied` gains `unavailable` (defaulted) and `Session` gains
`sandbox` (defaulted): both additive, and a spec or log from part 1
resolves to the empty state. The distribution already ships bubblewrap and
the supervisor binary, so the image does not change; the supervisor is now
bound into the namespace it already had. `sandbox explain`'s output is
unchanged this phase.

Behaviour a user could notice: a session that previously started with its
sandbox silently reduced now refuses if a required mechanism is missing.
On this kernel (0016) all three required mechanisms are present, so no
working session changes; the release note says the limits are now applied
and the applied set is measured.

Rollout list position: this is design item 6 (the limits and the
required-subset report) landing together with the re-exec that item 7
(Landlock) needs.

## Open questions

- Should the harness pid travel over the socket as `SCM_CREDENTIALS`
  rather than being resolved from `/proc`? Favoured: not yet — it trades
  the tested `/proc` walk for cmsg parsing that phase 2 introduces anyway,
  so fold it in then if at all.
- Should the ceiling be configurable in production, not only in tests?
  Favoured: no — two seconds is far inside the daemon's ten-second budget
  and a slow sandbox is a defect to see, not to wait out.
