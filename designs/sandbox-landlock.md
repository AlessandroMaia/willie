# Sandbox part 2, phase 3 — Landlock, and the doctor's probe

**F3b, phase 3 of five.** The inner stage applies Landlock immediately
before it `exec`s the harness: read and execute everywhere, write only
where the plan mounted read-write. This is depth on top of the mounts — a
path the plan never granted is readable but not writable, whatever route
reaches it — and it moves the doctor's Landlock check from reading a
filesystem that is not mounted to asking the kernel directly.

Read `designs/sandbox.md` — "The required subset" — and
`designs/sandbox-report.md` for the inner stage. Decision 0016 measured
Landlock ABI 3 through the syscall (securityfs is not mounted, which is why
`willie doctor` prints a false `[skip]`), and named the doctor's move to
the syscall as part 2's. This phase's decision is 0019.

## Problem

### The primary boundary is mounts; nothing restricts the process itself

Part 1 confines by what is mounted. A path that is bound read-write is
writable; a path not bound is absent. But a mount the plan did not intend —
reached through a symbolic link the two-stage guard did not catch, or a
future bind added by mistake — is writable if it is bound writable at all.
Landlock adds a second, path-based restriction applied to the process, so
the writable set is one list in one place and everything else is read-only
even when mounted.

### The doctor reports a false negative

`check_lsm_text` reads `/sys/kernel/security/lsm`. Securityfs is not
mounted in the distribution (0016), so the read comes back empty and the
doctor prints `[skip] landlock LSM` — while the kernel has Landlock built
in and active. The check must ask the kernel through
`landlock_create_ruleset(NULL, 0, VERSION)`, which returns the ABI.

## Goals

- Landlock restricts the harness and everything it spawns: read and execute
  under `/`, write only where the plan mounts read-write.
- The writable set is derived from the same `Plan` that builds the mounts,
  never a second list that could drift.
- Applied when the ABI offers it (2 or 3), reported as unavailable when it
  does not, never assumed away — the required subset honoured on the
  optional side.
- `willie doctor` probes Landlock by syscall and reports the ABI.
- The rule derivation and the ABI decision are pure, tested on any host;
  the enforcement itself has one distro test that proves the depth claim.

## Non-goals

- **W^X (no execute in writable directories).** It breaks `node_modules/.bin`,
  `npx`, and every installer that extracts to `/tmp` and runs. Execute is
  allowed everywhere; the mounts decide what exists to execute.
- **Making Landlock required.** 0016 decided it optional: the mounts do the
  primary work, and a kernel Willie does not build must not turn every
  session into a refusal. ABI absence is reported, not fatal.
- **Landlock network rules.** ABI 3 on this kernel has none (0016); egress
  is the corporate-network slice's concern.
- **A denial event for a Landlock refusal.** Landlock is silent by design;
  a file refusal reaches the agent as `EACCES`. Only the syscall class is
  nameable (phase 2). The applied/unavailable state is what the report and
  the UI show.

## Design

### The rules as data — `willie-linux/src/sandbox/landlock.rs`

```rust
/// What Landlock adds to the mounts: read and execute under `/`, write
/// only at these paths. Derived from the plan, so the writable set has one
/// source. The read set is implicit — a single rule on `/`.
pub struct Rules { pub write: Vec<String> }

pub fn rules(plan: &Plan) -> Rules
```

The writable set is the destination of every `Op::Tmpfs` (the private home,
`/tmp`) and every read-write `Op::Bind` (the workspace, the agent-state
directory, the per-project caches, read-write extra paths), plus `/dev`,
which the base sets up writable. Every other path — the system tree, the
read-only tools, the git config, read-only extras, anything not mounted at
all — is covered only by the read rule on `/`.

```rust
/// What the kernel's ABI offers, and whether to apply at all.
pub enum Apply { Ruleset { handled: u64 }, Unavailable }

pub fn applicability(abi: i32) -> Apply
```

ABI 0 or a negative errno → `Unavailable`. ABI 1 → `Unavailable` too: it
lacks `LANDLOCK_ACCESS_FS_REFER`, so every cross-directory `rename` is
denied and git stops working — reporting it unavailable is more honest than
applying a ruleset that breaks the workload. ABI ≥ 2 → `Ruleset` with the
filesystem access rights that ABI handles; `LANDLOCK_ACCESS_FS_TRUNCATE`
only from ABI 3. This refines 0016's "applied when the ABI probe answers".

`Request` gains `landlock: Rules`. The `libc` provides the three syscall
numbers only, so this module defines the two structs
(`landlock_ruleset_attr`, and `landlock_path_beneath_attr` as `repr(C,
packed)` exactly as the kernel declares it) and the `LANDLOCK_ACCESS_FS_*`
bit constants — all stable ABI.

### Applying it — `willie-sess/src/sandbox/landlock.rs` (Linux)

In the inner stage the order is: limits, **Landlock**, seccomp, report,
`exec`. Landlock before seccomp so that installing the ruleset is not
itself intercepted, and both before the report so the report is true.

1. `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)` for
   the ABI. `applicability`:
   - `Unavailable` → the inner adds `landlock` to the report's `unavailable`
     list and moves on. The session runs; the mounts still confine it.
   - `Ruleset { handled }` → create the ruleset with `handled`, open each
     path (`/` and each writable path) with `O_PATH | O_CLOEXEC`, add a
     `PATH_BENEATH` rule (`/` with read-file, read-dir, execute; each
     writable path with all handled rights), then `landlock_restrict_self`.
     `landlock` goes in the report's applied `mechanisms`.
2. A writable path that fails to open, or a `restrict_self` that fails on a
   kernel that offered the ABI, is `Refused { sandbox_apply_failed }`
   naming the path or errno: available-but-failed is a defect to surface,
   not silently downgrade to unavailable.

### The doctor's probe — `willie-linux/src/doctor.rs`

`check_lsm_text` and the `/sys/kernel/security/lsm` read are removed. In
their place:

```rust
/// Landlock is optional (0016): its absence reduces the sandbox but never
/// fails the doctor. `probe` is the ABI, or the errno if the syscall is
/// not there.
pub fn check_landlock(probe: Result<i32, i32>) -> DoctorCheck
```

- ABI ≥ 2 → `[ok ] landlock  ABI <n>`;
- ABI 1 → `[skip]`, "Landlock ABI 1 has no rename support; sandboxing will
  be reduced";
- `ENOSYS`/`EOPNOTSUPP` → `[skip] kernel without Landlock`;
- any other errno → `[skip]` naming it (the check is optional, so an
  undecidable answer is a skip, not a fail).

`run_all` calls it with `libc::syscall(SYS_landlock_create_ruleset, 0, 0,
LANDLOCK_CREATE_RULESET_VERSION)` under `cfg(linux)`, a stub elsewhere. The
doctor's sample output in `distro/README.md` and the F0 checklist is
updated where it is an undated sample, left where it is a dated result.

### Errors and edge cases

| Condition | Code | Behaviour |
| --- | --- | --- |
| ABI 0, 1, or the syscall absent | — | `landlock` reported unavailable; session runs on the mounts |
| a writable path fails to open | `sandbox_apply_failed` | inner refuses, naming the path |
| `restrict_self` fails on an ABI that answered | `sandbox_apply_failed` | inner refuses, naming the errno |
| the agent writes outside the writable set | — | `EACCES` reaches the agent; Landlock is silent to the log (by design) |
| the agent writes under `/proc/self` | — | denied (not in the writable set); nothing in the harness is known to need it; the acceptance walk confirms |

## Testing

Host, no kernel:

- `rules()` against plans with each capability on and off: the writable set
  is exactly the read-write mount destinations plus `/tmp` and `/dev`, and
  the workspace is always in it;
- `applicability` for ABI 0, 1, 2, 3 and a negative errno;
- `check_landlock` for each outcome (ok, ABI-1 skip, no-Landlock skip,
  other-errno skip).

Distro, through `just test-linux` — the test that proves the depth claim:

- a fake helper runs `exec /usr/bin/bwrap --bind <host dir> /leak "$@"`, a
  writable mount the plan never asked for; inside, the fake harness reads a
  file under `/leak` successfully and fails to write there with `EACCES`,
  and `sandbox_applied` names `landlock` — Landlock refusing a write the
  mounts allowed;
- `git mv` inside the workspace succeeds (`REFER` in action);
- `willie doctor` inside the distribution shows ABI 3.

## Rollout / compatibility

`Request` gains `landlock` and the report's lists gain the `landlock`
entry: additive within part 2, and this phase stacks on phases 1–2 which
introduced both. The doctor's Landlock line changes from a securityfs read
to a syscall probe — its check name and shape are the same, so a client
reading the report is unaffected. No image change; no protocol change
beyond the report already carrying `unavailable`. The release note says the
agent can no longer write outside the project and its caches even through a
stray mount, and that the doctor now reports the real Landlock ABI.

This is design rollout item 7 (path-based restriction after the re-exec)
and the 0016 consequence "the doctor's Landlock line moves to a syscall
probe (part 2)".

## Open questions

- Should `/dev` be in the writable set, or only the specific device nodes
  bubblewrap creates? Favoured: the whole `--dev` mount, which is already a
  minimal fresh tree; enumerating nodes is churn for no boundary gain.
- Should a future `home.persistent` capability's home be writable under
  Landlock? Yes by construction — it becomes a read-write mount and
  `rules()` picks it up — noted so the capability's own slice does not
  forget it.
