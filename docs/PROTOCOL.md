# Control protocol

Transport-agnostic ndjson: one JSON-RPC 2.0 object per line, UTF-8, `\n`
terminated. The same messages travel over the engine's stdio pipe to the
daemon, over the daemon's Unix socket to local clients and, later, over
TCP loopback. Types live in `crates/willie-proto`.

## Envelopes

`id` is a `u64`, `method` is `<ns>.<name>`, `params` is an object and
`result` is any JSON value.

Request:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "daemon.health", "params": {} }
```

Response, either a result or an error:

```json
{ "jsonrpc": "2.0", "id": 1, "result": { "pid": 42 } }
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": "snake_case",
    "message": "one sentence",
    "remediation": "what to do"
  }
}
```

Notification (daemon → client), recognised by having no `id`:

```json
{ "jsonrpc": "2.0", "method": "state.event", "params": {} }
```

## Compatibility rules
- Unknown fields are ignored; new fields have defaults. Additive
  changes keep `PROTOCOL_VERSION`.
- A client sends `daemon.hello` first. Engine and daemon must be the
  same Willie version. F0 reports a mismatch as `version_mismatch` with
  the remediation to reinstall the distribution; updating the daemon
  binaries in place is a later slice.
- The daemon exits when its stdin reaches EOF.
- Lines on the daemon's stdout that are no envelope are ignored. The
  engine keeps the last few of them: when `wsl.exe` itself refuses to
  start the daemon, its message arrives there and is the only account
  of the failure.

## `daemon.*`
| Method | Params | Result |
| --- | --- | --- |
| `daemon.hello` | `Hello { client, willie_version, protocol_version }` | `HelloReply { willie_version, protocol_version, distro_image_version? }` |
| `daemon.health` | `{}` | `Health { pid, uptime_secs, willie_version }` |
| `daemon.doctor` | `{}` | `DoctorReport { checks: [ { name, status: ok|fail|skip, detail, remediation?, required } ] }` |
| `daemon.shutdown` | `{}` | `null` — the daemon replies, then exits 0 |

## `project.*`

Long operations (`add`, `remove`, `sync_to_windows`, `update_from_windows`,
`relocate`) validate on the calling thread, then run their git work as a
background job: the reply carries a `JobRef`/`AddResult` and the outcome
arrives later as a `state.event`. `rename` is synchronous.

| Method | Params | Result |
| --- | --- | --- |
| `project.list` | `{}` | `ProjectList { projects: [Project] }` |
| `project.add` | `AddParams { windows_path, name? }` | `AddResult { project_id, job_id }` |
| `project.remove` | `RemoveParams { id, delete_workspace?, force? }` | `JobRef { job_id }` |
| `project.sync_to_windows` | `{ id }` | `JobRef { job_id }` |
| `project.update_from_windows` | `{ id }` | `JobRef { job_id }` |
| `project.relocate` | `RelocateParams { id, windows_path }` | `JobRef { job_id }` |
| `project.rename` | `RenameParams { id, name }` | `Project` |

A `Project` is `{ id, name, slug, source, workspace, branch, state,
source_present, created_at }`; `state` is `preparing`, `ready` or `failed
{ code, message, remediation }`. `source_present` is recomputed from the
filesystem on every `state.snapshot`, never trusted from disk.

## `job.*`
| Method | Params | Result |
| --- | --- | --- |
| `job.list` | `{}` | `{ jobs: [Job] }` |
| `job.get` | `{ id }` | `Job` |
| `job.cancel` | `{ id }` | `null` — trips the cancel flag; a no-op once finished |

A `Job` is `{ id, kind, project_id, state, started_at, finished_at?,
log_tail }`; `state` is `running`, `done` or `failed { code, message,
remediation }`.

## `state.*`
| Method | Params | Result |
| --- | --- | --- |
| `state.snapshot` | `{}` | `Snapshot { seq, projects: [Project], jobs: [Job] }` |

`state.event` is a notification (daemon → client), never a request. Its
params are `Event { seq, kind }` where `kind` is `project_changed
{ project }`, `project_removed { id }` or `job_changed { job }`. `seq` is a
monotonic counter shared by the snapshot and every event: a client that
holds a snapshot at `seq = N` applies every event with `seq > N` in order.
A single writer owns stdout, so events never interleave and their `seq`
values always arrive strictly increasing.

There is no replay buffer. If an event's `seq` is not exactly one past
the client's own — a gap, however it happened — or the daemon restarts
(a fresh process starts its `seq` back at zero), the client discards
what it has and calls `state.snapshot` again instead of trying to
reconcile the hole.

## Error codes (daemon)
| Code | Meaning |
| --- | --- |
| `method_not_found` | unknown method |
| `invalid_params` | params did not deserialise |
| `invalid_request` | the line was not a JSON-RPC request (malformed JSON or missing fields); the daemon answers with id 0 and keeps serving |
| `protocol_version_mismatch` | client speaks another `PROTOCOL_VERSION` |
| `internal_error` | handler failure; message says what, remediation says what to do |

## Error codes (project and job)

These come back as the `error` of a `project.*` or `job.*` reply when the
fast validation refuses before any job starts.

| Code | Meaning |
| --- | --- |
| `path_not_windows` | the source is not a path on a Windows drive |
| `not_a_git_repository` | the source is not a git checkout |
| `project_exists` | this checkout is already registered |
| `workspace_exists` | the workspace directory is already present |
| `project_not_found` | no project with that id |
| `project_busy` | a job is already running for that project |
| `source_detached_head` | the source is on a detached HEAD; no branch to track |
| `job_not_found` | no job with that id |

A job that starts and then fails carries its own code in the resulting
`job_changed` event's `state: failed { code }`, and — for `add` — in the
project's `state: failed { code }`. Those codes include `git_failed`,
`cancelled`, `interrupted`, `job_panicked`, `source_missing`,
`windows_tree_dirty`, `windows_branch_mismatch`, `workspace_diverged`,
`workspace_dirty` and `source_unrelated`. A daemon that stops mid-`add`
turns the stuck project `failed { code: "interrupted" }` on its next
start.

## Project problem codes

These are the codes a project-related refusal or job failure carries,
whether they come back synchronously as the `error` of a `project.*`
reply or later as a `code` inside a `job_changed`/project `failed`
event. `path_not_found` is the one exception: the engine checks the
Windows path exists before it ever calls the daemon, so that refusal
never reaches `project.*` at all — it is listed here because it belongs
to the same add/relocate flow as the codes around it.

| Code | When | When not | Remediation |
| --- | --- | --- | --- |
| `path_not_windows` | `project.add`/`project.relocate` was given a path that is not on a Windows drive: relative, a UNC share, or anything else `windows_to_drvfs` cannot map | the path is a drive path that does not exist — that is the engine's own `path_not_found`, refused before the daemon is asked | register a path on a Windows drive |
| `path_not_found` | the engine's pre-flight, before sending `project.add`/`project.relocate`, finds the given Windows path missing or not a directory | the path exists but has no `.git` — that is `not_a_git_repository`, only checked once the path is confirmed to exist | pick a folder that exists on this machine |
| `not_a_git_repository` | the source (`add`) or the new source (`relocate`) has no `.git` | the folder is a repository — with no commits yet, that is `source_no_commits`; on a detached HEAD, that is `source_detached_head` | run `git init` and a first commit in the folder, then add again — Willie never creates history for the user |
| `source_no_commits` | `project.add`'s fast validation finds the source is a repository whose `HEAD` resolves to no commit — a folder `git init`'d but never committed to | the folder is not a repository at all — that is `not_a_git_repository`, checked first; a repository with a commit but a detached `HEAD` — that is `source_detached_head`, checked right after | make the first commit in this checkout, then add it again |
| `source_detached_head` | `project.add`'s fast validation reads the source's current branch and finds `HEAD` itself, no branch checked out | the source is on a branch that later turns out to differ from the workspace's — that is `windows_branch_mismatch`, only seen at sync time | check out a branch in the Windows checkout, then add again |
| `project_exists` | `project.add`'s source matches an already-registered project's source, compared case-insensitively with a trailing separator ignored | the *workspace directory* for the derived slug already exists but no project references it — that is `workspace_exists` | this checkout is already registered; use its existing row instead of adding it again |
| `workspace_exists` | `project.add` derives a slug for the workspace and a directory of that name already exists under `/home/willie/projects/` | the same checkout is already a registered project — that is `project_exists`, checked first | delete the kept workspace directory the message names (`rm -rf` inside the distribution), moving it aside first if it still holds work you want, then add the checkout again |
| `project_not_found` | any `project.*` method (`remove`, `sync_to_windows`, `update_from_windows`, `relocate`, `rename`) names an id no longer in the daemon's state | the id is valid but a job is already running for it — that is `project_busy` | check the project id and try again; a stale UI should re-snapshot first |
| `project_busy` | a `project.*` operation that starts a job is called while that project already has one job running — one job per project at a time | the daemon's 3-job pool is full but this project is idle — that job is queued, not refused; `project_busy` is per project | wait for the current job to finish, or cancel it with `job.cancel` |
| `source_missing` | `sync_to_windows` or `update_from_windows` runs and the project's Windows source is gone — the directory no longer exists, or it exists but its `.git` does not (the same `source_present` check the project row uses) | the source exists but is dirty or on the wrong branch — that is `windows_tree_dirty`/`windows_branch_mismatch`, only checked once the source is confirmed present | relocate the project to a checkout that still exists |
| `windows_tree_dirty` | `sync_to_windows` runs `git status --porcelain` on the Windows checkout before pushing and finds it non-empty | the tree is clean but on the wrong branch — that is `windows_branch_mismatch`, checked right after | commit or discard the changes in the Windows checkout, then try again — nothing was touched |
| `windows_branch_mismatch` | `sync_to_windows`'s Windows checkout is clean but checked out on a branch other than the project's recorded one | the checkout is on the right branch but missing entirely — that is `source_missing`, checked first | check out the project's branch in the Windows checkout, then try again |
| `windows_diverged` | `sync_to_windows`'s Windows checkout is clean and on the right branch, but after a `fetch` its `windows/<branch>` holds commits the workspace's `HEAD` does not — pushing would be rejected non-fast-forward | the workspace is simply ahead, or the two sides hold the same commits — either way the push proceeds and the job ends `done` | click Update from Windows first, then Send to Windows again |
| `workspace_diverged` | `update_from_windows` fetches `windows` and the workspace's `HEAD` is not an ancestor of `windows/<branch>` — the workspace holds commits Windows does not | the workspace is simply behind — that fast-forwards silently and the job ends `done` | the workspace and the Windows checkout have both moved; reconcile them (rebase or merge in a terminal), then sync |
| `workspace_dirty` | `remove` is called with `delete_workspace: true` and `force: false`, and the workspace has uncommitted changes | the same case with `force: true` — the workspace is deleted regardless | commit or discard the workspace's changes, or remove with force (a one-click "Remove anyway" on the row) |
| `source_unrelated` | `relocate`'s new source is a git repository whose history does not contain the workspace's current `HEAD` commit | the new source is not a git repository at all — that is `not_a_git_repository`, checked first | point relocate at a checkout that shares history with the workspace |
| `git_failed` | any `git` invocation inside a job exits non-zero, or `git` itself cannot be spawned — the message carries git's own error output: the last few lines for an ordinary git command, and the whole clone log when `add`'s clone itself fails. The same code also covers a `remove`'s workspace-delete or a `rename`'s project-file write failing — an I/O error, not git's, so the message and remediation are I/O-appropriate there instead | the failure is one of the specific refusals above (dirty tree, diverged, unrelated history) — those are refused before the failing command ever runs, with their own code | check git's output and try again |
| `interrupted` | two cases: (1) a project is still `preparing` when the daemon starts — an `add` whose clone never finished, turned `Failed{interrupted}` by the start-up sweep; (2) `job.cancel` (or a daemon shutdown, if the process survives long enough) trips a job's cancel flag after its work has already started — the job only notices at its next checkpoint (`check_cancelled` in `crates/willied/src/projects.rs`), and reports `interrupted` for any job kind | the cancel flag is already tripped *before* the job's work starts — that produces `cancelled`, not `interrupted`; an app close mid-`sync_to_windows`/`update_from_windows`/`relocate` whose process exits before the next checkpoint runs leaves no code at all — job records are never persisted, so that job simply vanishes and the project is left exactly as it was | per kind, from `check_cancelled`: for `add`, there is no retry — remove the project and add it again; for every other kind, just retry the operation |
| `cancelled` | `job.cancel` trips a job's cancel flag before or while it runs; the job ends `failed { code: "cancelled" }` | the job had already finished when cancel was called — a no-op, the job keeps its real outcome | start the operation again if it is still needed |

## Engine problem codes

These are produced on the Windows side, not by the daemon. They cross
the app's IPC boundary as `Problem { code, message, remediation }` and
the UI keys behaviour on them, so they are stable once released.

| Code | When | When not | Remediation |
| --- | --- | --- | --- |
| `wsl_not_installed` | `wsl.exe` could not be spawned at all | it ran and answered something unexpected — that is `wsl_unparseable_output` | enable WSL 2.4.4 or newer (administrator) and retry |
| `wsl_command_failed` | a `wsl.exe` management command exited non-zero | the command worked and only its output was surprising | two texts are special-cased: `0x80070569` asks an administrator for the "Log on as a service" right, a failed `--import` says the image is still on disk and to retry Install; otherwise run the same command in PowerShell, then click Run doctor again |
| `wsl_unparseable_output` | `wsl.exe` answered, but not with what the reading needs (no version in `--version`) | the command failed — that is `wsl_command_failed` | run `wsl --version` in PowerShell, update WSL if it is old, then click Run doctor |
| `wsl_io` | the pipe to `wsl.exe` broke while the command was running | the child process died — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_error` | the daemon answered with a well-formed RPC error: it is alive and refused | the daemon is silent, dead or off-protocol | the daemon's own remediation when it sent one, otherwise click Run doctor to retry |
| `daemon_timeout` | no reply within the call budget (60 s for `hello`, 10 s after) | the call was answered with an error — that is `daemon_error` | click Run doctor (it restarts the daemon) |
| `daemon_transport` | writing to or reading from the daemon's pipe failed | the child exited and the exit explains it — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_exited` | the child process is gone; the detail is its stderr, and when the daemon failed to start and stderr is silent, `wsl.exe`'s own stdout message instead — a daemon that dies after saying hello reports stderr only | the daemon is alive but slow or wrong | by detail: the service-logon right, Install distribution for a missing one, otherwise one more Run doctor |
| `protocol_violation` | a line was an envelope but not usable: closed stdout, or a result the method cannot hold | the line was no envelope at all (those are ignored) | click Run doctor (it restarts the daemon); if it repeats, click Install distribution |
| `version_mismatch` | `hello` reported a Willie version other than the app's | the protocol version differs — the daemon answers `protocol_version_mismatch` itself | click Install distribution: it reinstalls the distribution with binaries matching this app |
| `image_invalid` | the image beside the app is missing, empty, or its sha256 does not match the `.sha256` sidecar | there is no image at all — that is `image_not_found` | rebuild the image with `just distro-build`, then click Install distribution again |
| `image_not_found` | no image in any candidate location (`WILLIE_ROOTFS`, the resource dir, `target/distro`) | one was found and rejected — that is `image_invalid` | reinstall Willie; in development run `just distro-build` |
| `daemon_not_running` | a call was made while no daemon is live | the daemon was live and died — that is `daemon_exited` | click Run doctor (it starts the daemon) |
| `distro_not_registered` | the pre-flight before a start found no `willie` distribution | the distribution exists and its daemon failed — that is `daemon_exited` | click Install distribution |
