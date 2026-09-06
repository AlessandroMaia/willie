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
| 5 | Stop the session; `wsl -d willie --user willie -- cat /var/lib/willie/sessions/<id>/events.jsonl` | the `sandbox_applied` line names `["namespaces","mounts","rlimits"]`, before `started` |

## Results

| # | Date | Result | Notes |
| - | ---- | ------ | ----- |
