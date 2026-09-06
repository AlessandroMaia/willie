# Sandbox mechanisms acceptance — part 2

The boundary walk (`sandbox-acceptance.md`) proves part 1. This walk
proves the mechanisms part 2 adds, phase by phase. Each phase appends its
rows and fills the Results table; do not renumber earlier rows.

Before the walk, the part-1 preamble applies: `just distro-push`, confirm
`/etc/willie/image-version` names the commit under test, close and reopen
the app. Every command is typed inside the Claude Code session, prefixed
with `!`.

## Phase 1 — the limits and the measured report

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Projects → **Open session** | a session opens as before |
| 2 | `!ulimit -u` | `4096` — the process limit is set |
| 3 | `!ulimit -n` | `65536` — the descriptor limit is set |
| 4 | `!ulimit -c` | `0` — no core dumps |
| 5 | Stop the session; `wsl -d willie --user willie -- cat /var/lib/willie/sessions/<id>/events.jsonl` | the `sandbox_applied` line names `["namespaces","mounts","rlimits","landlock","seccomp"]`, before `started` (phase 2 added `seccomp`, phase 3 `landlock`) |

## Phase 2 — the syscall filter and the denial log

| # | Step | Expected |
| - | ---- | -------- |
| 6 | Projects → **Open session**; `!grep Seccomp /proc/self/status` | `Seccomp: 2` — filter mode, inherited by every process in the session |
| 7 | `!unshare -U true` | fails with `Operation not permitted`, non-zero exit — the filter refused `unshare` |
| 8 | `!ip link` | the interfaces are listed, exit 0 — the netlink route exception holds |
| 9 | `!perl -e 'socket(S, 17, 3, 0) or die $!'` | `Operation not permitted` — a packet socket (`AF_PACKET`, `SOCK_RAW`) is refused |
| 10 | `!for i in $(seq 200); do unshare -U true 2>/dev/null; done` | ends within a few seconds with nothing printed — each probe is refused at once, not stalled |
| 11 | Stop the session; `wsl -d willie --user willie -- cat /var/lib/willie/sessions/<id>/events.jsonl` | after `started`: a `sandbox_denied` line with `"class":"syscall","name":"unshare","count":1` for step 7; the 201 `unshare` refusals of steps 7 and 10 add up across a handful of `unshare` lines — three or so, the coalescing — not 201 lines; one line names `"socket"` for step 9; no line names step 8's netlink call; no `sandbox_degraded` line; every `sandbox_denied` line sits before `exited` |

## Phase 3 — Landlock over the mounts

Landlock's visible effect beyond the mounts — a writable mount the plan
never granted being read-only to the session — can only be forced by the
automated depth test (`landlock_denies_a_write_the_mounts_would_have_allowed`,
run by `just test-linux`); the walk proves the doctor's answer, the
applied mechanism and the rename that ABI 1 would have refused.

| # | Step | Expected |
| - | ---- | -------- |
| 12 | `wsl -d willie --user willie -- /opt/willie/bin/willie doctor` | the line `[ok ]  landlock                     ABI 3` — the kernel is asked through the syscall, no longer a file securityfs never provided |
| 13 | Projects → **Open session**; `!mkdir sub && touch a && mv a sub/b && echo mv_ok` | `mv_ok` — a rename across directories inside the workspace works: the `REFER` right ABI 1 lacks, and the reason ABI 1 counts as unavailable; `!rm -r sub` afterwards |
| 14 | Stop the session; `wsl -d willie --user willie -- cat /var/lib/willie/sessions/<id>/events.jsonl` | the `sandbox_applied` line names `["namespaces","mounts","rlimits","landlock","seccomp"]` with an empty `unavailable` list, before `started` — Landlock applied between the limits and the filter |

## Results

| # | Date | Result | Notes |
| - | ---- | ------ | ----- |
