# 0019 — Landlock grants reading and executing everywhere and writing only where the plan mounts read-write, and ABI 1 counts as no Landlock at all

- **Date:** 2026-09-06
- **Status:** accepted

## Context

Part 1 confines a session by what is mounted (0016): a path bound
read-write is writable, a path not bound is absent, and nothing
restricts the process itself. So a mount the plan never intended — one
reached through a link the two-stage guard did not catch, or a bind a
later change adds by mistake — is writable if it is bound writable at
all. The writable set was, in effect, whatever the helper happened to
mount that way. 0016 measured Landlock ABI 3 on this kernel, decided it
optional ("applied when the ABI probe answers and its absence is
reported, never assumed away"), and left the doctor's move to the
syscall for part 2: the doctor read `/sys/kernel/security/lsm`, which
the distribution does not mount, and printed `[skip] landlock LSM` on a
kernel that has Landlock built in and active. 0017 put a stage of the
supervisor inside the namespace, which is where a ruleset has to be
applied: `landlock_restrict_self` needs `no_new_privs`, the bit the
helper sets for its namespace and the stage already refuses without,
and the ruleset has to be in force in the process that will `exec` the
harness so that everything the harness spawns inherits it.

Three constraints shaped the answer. The writable set must not be a
second list that can drift from the mounts. The syscall filter (0018)
is installed by the same stage, and Landlock's three syscalls have to
run before that filter is in force. And a kernel Willie does not build
must not turn every session into a refusal: the mounts do the primary
work, and Landlock is depth on top of them.

## Options

What Landlock restricts:

| Option | For | Against |
| --- | --- | --- |
| read and execute everywhere, write where the plan mounts read-write — the rules derived from the plan | one source: the writable set is the plan's own read-write destinations, so it cannot disagree with the mounts; the read side is a single rule on `/`; a mount the plan never granted read-write is read-only whatever bound it | reading is not narrowed below the mounts — but the mounts already decide what is visible, and Landlock is depth on top of them, not a second boundary |
| a second, independent list of paths | could be tighter than the mounts — deny reading part of the system tree, say | two lists to keep in step; drift is a session refused for a path the mounts grant, or a path the list forgot that the mounts allow; and the tightening it offers is a policy question the capabilities answer, not the rules — rejected |
| no execute in writable directories on top (W^X) | a file the agent wrote cannot run | the executables a package manager installs under the project, every tool run from the workspace, and every installer that extracts into the temporary directory and runs — all break; execute is allowed everywhere and the mounts decide what exists to execute — rejected for this slice |
| Landlock in the required subset | every session gets the depth, or none starts | a kernel Willie does not build turns every session into a refusal; the mounts do the primary work and 0016 decided it optional; the honest answer to a kernel without it is a record that says so — rejected |

Which ABI to apply on:

| Option | For | Against |
| --- | --- | --- |
| any ABI that answers, 1 included | the most kernels get the depth | ABI 1 does not handle `LANDLOCK_ACCESS_FS_REFER`: with a ruleset in force every rename or link across directories is refused, and git — which renames into place — stops working; a sandbox that breaks the workload it contains is worse than none |
| from ABI 2, with 1 reported unavailable | the workload works wherever it runs, and the session's record says what it did not get | a kernel at ABI 1 runs on the mounts alone — which is where every session ran before this decision |

## Decision

The rules are data in `willie-linux`, derived from the plan that builds
the mounts: the writable set is the destination of every private
filesystem and every read-write bind, in the order the plan mounts them,
plus `/tmp` and `/dev`, which the base sets up writable without being
policy, each path once; the read set is implicit — one rule on `/`. The
stage inside the namespace applies them after the limits and before the
syscall filter, so Landlock's own syscalls run before the filter is in
force, and both before the report, so the report is true. It probes the
ABI with the version query of `landlock_create_ruleset`. An ABI of 1, an
answer of zero or an errno is *unavailable*: `landlock` goes in the
report's `unavailable` list, the session runs on the mounts, and the
record says so — ABI 1 lacks `REFER`, without which every rename across
directories is refused and git breaks, so a ruleset there would break
the workload it means to contain. This refines 0016's "applied when the
ABI probe answers" without superseding it. ABI 2 handles every
filesystem right through `REFER` (bits 0 to 13); ABI 3 and above add
`TRUNCATE`. With an ABI the stage creates a ruleset handling those
rights, adds the `/` rule with read-file, read-dir and execute — no
write bit — and one rule per writable path with every handled right,
then restricts itself. A rule whose path is a file rather than a
directory — a read-write extra path can be one — is masked to the
rights a file can carry (execute, write, read, truncate), because the
kernel refuses a file rule that carries a directory right and the
session would be refused for a configuration that was fine. A kernel
that answered an ABI and then refused — a rule path that will not open,
a ruleset or a rule the kernel rejects, a `restrict_self` that fails —
is `sandbox_apply_failed` naming the path or the errno, never a silent
downgrade to unavailable: the kernel said it could and then did not,
and that is a defect to surface. Landlock stays optional: the required
subset is unchanged (`namespaces`, `mounts`, `rlimits`, `seccomp`), and
a report on this kernel names five mechanisms, `landlock` fourth.
`willie doctor` asks the kernel the same question and prints the ABI —
`[ok ] landlock ABI 3` here; ABI 1 is a skip that says renaming is
unsupported and the sandbox reduced; no Landlock (`ENOSYS`,
`EOPNOTSUPP`) is a skip that says so; the check never fails the doctor,
because the mechanism is optional.

## Consequences

- A writable mount the plan did not grant is read-only to the harness.
  Proven by a distribution test: a helper that binds a host directory
  read-write at `/leak` ahead of the plan's own options runs a session
  in which a file under `/leak` is read and a write there is refused
  with `EACCES`, while a rename across directories inside the workspace
  succeeds and the report names `landlock`. One boundary of this
  guarantee is worth stating: a `PATH_BENEATH` rule grants write beneath
  the whole subtree of the directory it names, so a path nested under a
  directory the plan does grant read-write — anything under the home
  tmpfs, for one — is writable as far as Landlock is concerned. The
  read-only mounts that live under the home (`~/.gitconfig`, the managed
  tool roots) stay read-only there because their bind is `--ro-bind`
  (a write is `EROFS`), not because Landlock refuses it. This is the
  design's own division of labour — the mounts do the primary work,
  Landlock is the depth on top — and it means a hypothetical future
  read-write mount mistakenly nested under the home would not be caught
  by Landlock, only by the mount that put it there; the same mistake
  outside the granted directories is caught by Landlock.
- Writes under `/proc/self/*` are refused: `/proc` is a fresh mount and
  not in the writable set. Nothing in the harness is known to need
  them; a session that works through the acceptance walk is what
  confirms it, and a workflow that turns out to need one is a change to
  the rules and a release note, not a project setting.
- A workflow that wrote somewhere the mounts happened to allow but the
  plan never granted read-write stops with `EACCES` — silently as far as
  the log is concerned: Landlock names no denial (0018 left file
  denials unrecorded), so the message reaches the agent's terminal and
  nothing else.
- A read-write extra path that is a file carries file rights only: the
  harness can read, write, truncate and execute it, not make or remove
  entries beneath a name that has none.
- The doctor shows the real ABI; the false `[skip] landlock LSM` is
  gone, and the distribution's sample output and the F0 checklist's
  undated row follow it.
- The rule derivation and the ABI decision are pure and tested on any
  host; the two kernel structures are declared in `willie-linux` with a
  size test (twelve packed bytes and eight) as the layout guard; the
  file-rights masking, the probe and the enforcement itself are tested
  inside the distribution.

## Not decided

- `/dev` as a whole or node by node. The whole device tree is granted
  for now: it is already a minimal fresh tree, and enumerating nodes is
  churn for no boundary gain; revisited if a device node ever has to be
  denied.
- No execute in writable directories. Rejected for this slice for the
  reasons in the table; a later slice that wants it needs an answer for
  the executables a project installs under itself and for installers
  that run from the temporary directory.
- Landlock network rules: this ABI has none (0016); egress stays with
  the corporate-network slice.
- Recording file denials. Landlock is silent by design, and the syscall
  class stays the one the sandbox can name (0018).
- The path rules in `sandbox explain`: a named follow-up of the
  enforcement design, not part of this phase.
- A persistent home is writable by construction — it becomes a
  read-write mount and the derivation picks it up — noted so the
  capability's own slice does not have to decide it again.
