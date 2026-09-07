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
arrives later as a `state.event`. `rename` and `set_sandbox` are
synchronous.

| Method | Params | Result |
| --- | --- | --- |
| `project.list` | `{}` | `ProjectList { projects: [Project] }` |
| `project.add` | `AddParams { windows_path, name? }` | `AddResult { project_id, job_id }` |
| `project.remove` | `RemoveParams { id, delete_workspace?, force? }` | `JobRef { job_id }` |
| `project.sync_to_windows` | `{ id }` | `JobRef { job_id }` |
| `project.update_from_windows` | `{ id }` | `JobRef { job_id }` |
| `project.relocate` | `RelocateParams { id, windows_path }` | `JobRef { job_id }` |
| `project.rename` | `RenameParams { id, name }` | `Project` |
| `project.set_sandbox` | `SetSandboxParams { project_id, profile }` | `Project` |

A `Project` is `{ id, name, slug, source, workspace, branch, state,
source_present, created_at, sandbox, sandbox_problem? }`; `state` is
`preparing`, `ready` or `failed { code, message, remediation }`.
`source_present` is recomputed from the filesystem on every
`state.snapshot`, never trusted from disk. `sandbox` is layer 2 of the
sandbox policy (a `SandboxProfile` — see `sandbox.*` below);
`set_sandbox` replaces it outright rather than merging onto the stored
one, resolving the replacement against the harness defaults before
persisting it, the same fail-closed check `session.create` and
`sandbox.explain` apply.

`sandbox_problem` is present only when the record's `[sandbox]` table
could not be read; the project then loads with the default profile, and
`session.create`/`sandbox.explain` refuse with `sandbox_profile_invalid`
until the profile is replaced through `set_sandbox`. It is recomputed on
every load, never trusted from disk.

## `job.*`
| Method | Params | Result |
| --- | --- | --- |
| `job.list` | `{}` | `{ jobs: [Job] }` |
| `job.get` | `{ id }` | `Job` |
| `job.cancel` | `{ id }` | `null` — trips the cancel flag; a no-op once finished |

A `Job` is `{ id, kind, project_id?, state, started_at, finished_at?,
log_tail }`; `project_id` is absent for a job that belongs to no project
(a tool install or update). `kind` is `add`, `remove`, `sync_to_windows`,
`update_from_windows`, `relocate`, `install_harness` or `update_harness`;
`state` is `running`, `done` or `failed { code, message, remediation }`.

## `session.*`
| Method | Params | Result |
| --- | --- | --- |
| `session.create` | `CreateParams { project_id, git_identity? { name, email }, resume? }` | `CreateResult { session }` — the session, already `running`, or an error if it could not start |
| `session.stop` | `{ id }` | `null` — asks the supervisor to stop; the outcome arrives as a `session_changed` event |
| `session.list` | `{}` | `SessionList { sessions: [Session] }` |

A `Session` is `{ id, project_id, harness, workspace, state, created_at,
started_at?, finished_at?, pid?, clients, resumed_from?, sandbox }`;
`state` is `creating`, `running`, `stopping`, `exited { code?, signal? }`
or `failed { code, message, remediation }`. `sandbox` is `{ applied:
[string], unavailable: [string], denied: [{ class, name, count,
first_at, last_at }], degraded: [string] }` — the mechanisms the
session's sandbox applied and the required-optional ones the kernel did
not offer (e.g. `landlock` on a kernel with no Landlock ABI or only
ABI 1); what it refused, one row per (`class`, `name`), folded from
the session's `sandbox_denied { class, name, count }` events (`count`
accumulates, `first_at` stays at the first refusal, `last_at` advances
to the latest; `class` is `syscall` or `terminal` — for `syscall` the
`name` is the syscall's, for `terminal` it is one of `clipboard`,
`title`, `window` or `query_echo`, the output escape sequence the
terminal filter dropped (decision 0020)); and
the mechanisms that fell back to their closed direction while the
session ran, each named once, folded from its `sandbox_degraded
{ mechanism, message }` events. Every field is empty on a session from
a log written before it was recorded. Creating a session is
synchronous up to
the supervisor's readiness: the reply already carries a `running` session
or the coded failure. There is no `attach_command` in the reply — the
engine composes `wsl.exe … willie attach <id>` itself. An unknown
`project_id` fails with the existing `project_not_found` code (see
Project problem codes below), not a new one.

`CreateParams.resume` (default `false`) asks the daemon to continue the
project's most recent conversation instead of starting fresh: the harness
launches with its continue flag in the workspace, and the new session's
`resumed_from` names the finished session it continues. This is
continue-latest and project-scoped — there is no way yet to resume a
specific older session by id. See `harness_cannot_resume` and
`session_already_live` in the session codes below for its fail-closed
guards; the existing `project_not_found`/`project_not_ready`/
`project_busy`/`harness_not_installed` guards apply to a resume request
unchanged.

## `sandbox.*`
| Method | Params | Result |
| --- | --- | --- |
| `sandbox.explain` | `ExplainParams { project_id }` | `ExplainResult { entries: [Explained], capabilities }` |

Reports what a session for `project_id` would run under, without
starting one: the same two layers `session.create` resolves
(`willie_core::sandbox::resolve`), reported row by row. `entries` is
one `Explained { capability, enabled, source }` per `Capability`, in
`Capability::ALL` order; `source` is `default` (the harness decided
it), `profile` (the project's profile spoke) or `unavailable` (this
version cannot apply it at all). `capabilities` is the same resolved
`CapabilitySet` `session.create` would write into the spec. An unknown
`project_id` fails with `project_not_found`; a profile that cannot
resolve fails with the same `sandbox_capability_unsupported` /
`sandbox_profile_invalid` codes `session.create` uses (see Session and
tool codes below).

## `tool.*`
| Method | Params | Result |
| --- | --- | --- |
| `tool.install` | `InstallParams { harness }` | `JobRef { job_id }` — the install runs as a job; watch its `job_changed` events |
| `tool.list` | `{}` | `ToolList { tools: [ToolStatus] }` |
| `tool.update` | `UpdateParams { tool }` | `JobRef { job_id }` |

`tool.install` stays: `InstallParams { harness }` → `JobRef`. A
`ToolStatus` is `{ id, name, installed, version?, recorded_version? }`:
`version` is live detection (present iff installed), `recorded_version` is
what the manifest recorded. The catalogue is the harness registry today.
`tool.update` re-runs the installer for an installed tool and answers the
same `JobRef` as install; its job kind is `update_harness`.

`harness_already_installed` and `tool_busy` come back synchronously as
the call's own error, before any job starts; a job that starts and then
fails always carries `install_failed`. `tool.update` on a tool the daemon
does not detect refuses synchronously with `tool_not_installed`; an
unknown tool id is `invalid_params` instead (see Session and tool codes
below).

## `plugin.*`

Plugins are compiled into the daemon, run outside every session sandbox,
and each degrades only itself. The host keeps a registry, persists which
plugins are enabled (and, for a per-project plugin, in which projects) in
`/var/lib/willie/plugins/enabled.toml`, and routes calls to them.

| Method | Params | Result |
| --- | --- | --- |
| `plugin.list` | `{}` | `[PluginStatus]` |
| `plugin.enable` | `EnableParams { id, project_id? }` | `PluginStatus` |
| `plugin.disable` | `EnableParams { id, project_id? }` | `PluginStatus` |

A `PluginStatus` is `{ id, name, scope, enabled, degraded }`; `scope` is
`global` or `per_project`; `enabled` is `{ global: <bool> }` for a global
plugin or `{ per_project: [ProjectId] }` for a per-project one; `degraded`
is `true` once a call into the plugin has panicked or returned an internal
fault (`plugin_internal`) — a genuine malfunction. An ordinary coded refusal
(a legitimate "no", e.g. `profile_exists`) does not degrade it.
`EnableParams.project_id` picks the scope: absent enables (or disables) the
plugin globally, present enables (or disables) it for that project. A scope
the plugin's manifest forbids — a global one for a per-project plugin, or
the reverse — is refused with `plugin_scope_mismatch`. A missing or
unreadable `enabled.toml` reads as "nothing enabled" and the daemon still
runs.

A plugin's own methods carry no dispatch arm of their own: a method under a
plugin's namespace (today `profile.*`, the configuration-profiles plugin)
is routed to the host, which splits `<id>.<method>`, finds the plugin and
calls it. The plugin id doubles as its method namespace, so the profiles
plugin — methods `profile.*` — is identified as `profile`. An unknown id is
`plugin_not_found`; a call to a disabled plugin is `plugin_disabled`; a
plugin that panics is caught at the host boundary, marked `degraded`, and
answered with `plugin_panicked` — the daemon lives.

The `plugin_changed` `state.event` kind and a plugin's own `plugin.emitted`
notification are reserved in the protocol but **not yet emitted**: an
enable/disable returns the new `PluginStatus` synchronously and a client
sees the change on its next `state.snapshot` (whose `plugins` field the host
fills). Forwarding live plugin changes and emissions is a follow-up.

## `profile.*`

The configuration-profiles plugin (id `profile`, scope `per_project`).
Routed through `plugin.*` above: disabled or an unknown method answers
with the plugin codes there, not the ones below. A profile is
`<store_dir>/<name>/`, a git repository the plugin manages with its own
`git` (never the daemon's); every write is its own commit.

| Method | Params | Result |
| --- | --- | --- |
| `profile.list` | `{}` | `[ProfileSummary { name, fragments_active }]` |
| `profile.create` | `{ name }` | `ProfileSummary` |
| `profile.read_fragment` | `{ name, fragment }` | `{ content }` |
| `profile.write_fragment` | `{ name, fragment, content }` | `{ content }` |
| `profile.check` | `{ name, project_id }` | `{ changes: [Change] }` |
| `profile.apply` | `{ name, project_id }` | `{ changes: [Change], backup_path }` |

`fragment` is one of `settings`, `instructions`, `mcp` (booleans in
`profile.toml`'s `[fragments]` table), or `rules/<file>` / `hooks/<file>`
(a specific file, active once its name is in that family's list — the
list is the on/off switch, there is no separate flag). `profile.create`
scaffolds `profile.toml`, an empty `settings.json` and `CLAUDE.md`, `git
init`s the directory and commits the scaffold. `profile.write_fragment`
writes the fragment file, marks it active in `profile.toml`, and commits
both in one commit; `profile.read_fragment` on a fragment never written
returns `{ content: "" }` rather than refusing. Turning a fragment back
off is done by editing `profile.toml` directly (the supported path,
per the design) — there is no Phase-1 method for it.

`profile.check`/`profile.apply` (Phase 2, the format-preserving merge
into a project) never resolve `project_id` themselves: before either
reaches the plugin, the daemon's `profile.*` route looks the project up
and injects the resolved ext4 `workspace` path and the harness-state
settings path into the request as `_workspace`/`_harness_settings` — a
caller only ever sends `name`/`project_id`, and an unknown `project_id`
is refused `project_not_found` at this resolution step, before the
plugin runs at all (so it is never masked by `plugin_disabled`, even if
the plugin also happens to be disabled). `profile.check` plans every
active fragment's change — a JSON merge into `.claude/settings.json`
(`settings`, `mcp`), a Markdown merge into `CLAUDE.md` between the
`willie` markers (`instructions`), or a file copy into `.claude/rules/`
/ `.claude/hooks/` — against the project's current files, without
writing. `profile.apply` performs the same plan, first backing up every
file it will change into `<workspace>/.willie-bak/<nanosecond
timestamp>/` at its relative path (a `Change` that only creates a file
has nothing to back up), then writes; `backup_path` names the backup
directory even when nothing needed copying. A `settings` fragment
marked `settings_scope = "global"` in `profile.toml`'s `[fragments]`
table (default: `"project"`) plans and applies a *second* change,
merged independently into the harness state's own `settings.json` (an
absolute path in `Change.path`, distinguishing it from the project's
workspace-relative ones) — every other project's sessions read that
file, so a profile opts into touching it explicitly rather than by
surprise.

Phase 3 (`profile.set_remote`/`push`/`pull`, the minimal sync) is a
later slice of this same plugin and not yet built.

## `state.*`
| Method | Params | Result |
| --- | --- | --- |
| `state.snapshot` | `{}` | `Snapshot { seq, projects: [Project], jobs: [Job], sessions: [Session], plugins: [PluginStatus] }` |

`state.event` is a notification (daemon → client), never a request. Its
params are `Event { seq, kind }` where `kind` is `project_changed
{ project }`, `project_removed { id }`, `job_changed { job }` or
`session_changed { session }`. (`plugin_changed { plugin }` is reserved but
not yet emitted — see `plugin.*` above.) `seq` is a
monotonic counter shared by the snapshot and every event: a client that
holds a snapshot at `seq = N` applies every event with `seq > N` in order.
A single writer owns stdout, so events never interleave and their `seq`
values always arrive strictly increasing.

There is no replay buffer. If an event's `seq` is not exactly one past
the client's own — a gap, however it happened — or the daemon restarts
(a fresh process starts its `seq` back at zero), the client discards
what it has and calls `state.snapshot` again instead of trying to
reconcile the hole.

## Transports

The same messages travel two ways. The **engine** speaks over the
daemon's **stdio** pipe: it may call every method, receives every
`state.event` notification, and ends the daemon by closing stdin or
calling `daemon.shutdown`. A **local client** (the `willie` CLI) speaks
over the daemon's Unix socket, `/run/willie/willied.sock` (`0600`, the
distro user's own): request and reply only, no notifications — a client
that wants state calls `state.snapshot`. `daemon.shutdown` over the
socket is refused with `method_not_served`; the daemon's lifecycle is the
engine's. A session never reaches the socket: the sandbox does not mount
`/run/willie`.

## Error codes (daemon)
| Code | Meaning |
| --- | --- |
| `method_not_found` | unknown method |
| `method_not_served` | the method exists but this transport does not serve it (`daemon.shutdown` over the local socket); not `method_not_found`, which is an unknown method |
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
start. A tool install job fails with `install_failed`.

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
| `project_not_found` | any `project.*` method (`remove`, `sync_to_windows`, `update_from_windows`, `relocate`, `rename`, `set_sandbox`), `session.create`/`sandbox.explain`, or a `profile.check`/`profile.apply` whose `project_id` the daemon's `profile.*` route cannot resolve (before the profiles plugin ever runs), names an id no longer in the daemon's state | the id is valid but a job is already running for it — that is `project_busy` | check the project id and try again; a stale UI should re-snapshot first |
| `project_busy` | a `project.*` operation that starts a job is called while that project already has one job running — one job per project at a time | the daemon's 3-job pool is full but this project is idle — that job is queued, not refused; `project_busy` is per project | wait for the current job to finish, or cancel it with `job.cancel` |
| `sessions_running` | `project.remove` is called while the project has at least one session in `running` or `stopping` | no session of the project is live — the remove job is submitted as usual | stop the project's sessions first |
| `source_missing` | `sync_to_windows` or `update_from_windows` runs and the project's Windows source is gone — the directory no longer exists, or it exists but its `.git` does not (the same `source_present` check the project row uses) | the source exists but is dirty or on the wrong branch — that is `windows_tree_dirty`/`windows_branch_mismatch`, only checked once the source is confirmed present | relocate the project to a checkout that still exists |
| `windows_tree_dirty` | `sync_to_windows` runs `git status --porcelain` on the Windows checkout before pushing and finds it non-empty | the tree is clean but on the wrong branch — that is `windows_branch_mismatch`, checked right after | commit or discard the changes in the Windows checkout, then try again — nothing was touched |
| `windows_branch_mismatch` | `sync_to_windows`'s Windows checkout is clean but checked out on a branch other than the project's recorded one | the checkout is on the right branch but missing entirely — that is `source_missing`, checked first | check out the project's branch in the Windows checkout, then try again |
| `windows_diverged` | `sync_to_windows`'s Windows checkout is clean and on the right branch, but after a `fetch` its `windows/<branch>` holds commits the workspace's `HEAD` does not — pushing would be rejected non-fast-forward | the workspace is simply ahead, or the two sides hold the same commits — either way the push proceeds and the job ends `done` | click Update from Windows first, then Send to Windows again |
| `workspace_diverged` | `update_from_windows` fetches `windows` and neither the workspace's `HEAD` nor `windows/<branch>` is an ancestor of the other — both sides moved, each holding commits the other does not | the workspace is simply behind — that fast-forwards silently and the job ends `done` — or simply ahead of, or equal to, `windows/<branch>`: there is nothing to update, so the job ends `done` without touching the workspace | the workspace and the Windows checkout have both moved; reconcile them (rebase or merge in a terminal), then sync |
| `workspace_dirty` | `remove` is called with `delete_workspace: true` and `force: false`, and the workspace has uncommitted changes | the same case with `force: true` — the workspace is deleted regardless | commit or discard the workspace's changes, or remove with force (a one-click "Remove anyway" on the row) |
| `source_unrelated` | `relocate`'s new source is a git repository whose history does not contain the workspace's current `HEAD` commit | the new source is not a git repository at all — that is `not_a_git_repository`, checked first | point relocate at a checkout that shares history with the workspace |
| `git_failed` | any `git` invocation inside a job exits non-zero, or `git` itself cannot be spawned — the message carries git's own error output: the last few lines for an ordinary git command, and the whole clone log when `add`'s clone itself fails. The same code also covers a `remove`'s workspace-delete failing — an I/O error, not git's, so the message and remediation are I/O-appropriate there instead | the failure is one of the specific refusals above (dirty tree, diverged, unrelated history) — those are refused before the failing command ever runs, with their own code; a project record write failing (`rename`, `set_sandbox`) is `state_write_failed`, not this code | check git's output and try again |
| `state_write_failed` | a project record could not be written to the daemon's state directory (a `rename` or `set_sandbox` persisting) — an I/O failure, not git's, so it is no longer reported as `git_failed` | the write succeeds, or the failure is git's own — an `add`'s clone or a `sync_to_windows`/`update_from_windows` push — which stays `git_failed` | check the daemon's state directory permissions and try again |
| `interrupted` | two cases: (1) a project is still `preparing` when the daemon starts — an `add` whose clone never finished, turned `Failed{interrupted}` by the start-up sweep; (2) `job.cancel` (or a daemon shutdown, if the process survives long enough) trips a job's cancel flag after its work has already started — the job only notices at its next cancellation checkpoint, and reports `interrupted` for any job kind | the cancel flag is already tripped *before* the job's work starts — that produces `cancelled`, not `interrupted`; an app close mid-`sync_to_windows`/`update_from_windows`/`relocate` whose process exits before the next checkpoint runs leaves no code at all — job records are never persisted, so that job simply vanishes and the project is left exactly as it was | per kind: for `add`, there is no retry — remove the project and add it again; for every other kind, just retry the operation |
| `cancelled` | the job's cancel flag was already tripped when its work was about to start, so the runner ended it `failed { code: "cancelled" }` without ever running it | the flag trips after the work has begun — that is `interrupted`, reported at the job's next cancellation checkpoint; the job had already finished when cancel was called — a no-op, the job keeps its real outcome | start the operation again if it is still needed |

## Session and tool codes

These are the codes a `session.*`/`tool.*` refusal carries, whether they
come back synchronously as the `error` of the reply, or later inside a
`session_changed` event's `Session.state: failed { code }` or a
`job_changed` event's `state: failed { code }`. `session_not_found` is
reserved: no session method today distinguishes an unknown id from one
whose supervisor cannot be reached, so `session.stop` reports
`session_not_running` for both. Remediations match
`willie_core::session::remediation_for`, the one table the daemon
reads from (and the app will, once its Plan B session UI lands). The
two sandbox codes are the exception: theirs are built by
`willie_core::sandbox::CapabilityError` so the text can name the
capability or the path at fault, which a static table cannot;
`sandbox.explain` reports the same two, for the same reason, since it
resolves the same policy without starting a session, and so does
`project.set_sandbox`, which validates by resolving before it persists
a profile the UI edits.

| Code | When | Remediation |
| --- | --- | --- |
| `project_not_ready` | `session.create` on a project that is `preparing` or `failed` | wait for the project to be ready, or fix its failure first |
| `harness_cannot_resume` | `session.create { resume: true }` and the harness's `Resume` capability is `None` | open a fresh session instead; this harness cannot continue a conversation |
| `session_already_live` | `session.create { resume: true }` while the project already has a live session | use the running session, or stop it first, then resume |
| `harness_not_installed` | no harness binary on the session `PATH` (or `--version` fails) | click Install on the Dashboard |
| `git_identity_missing` | none of the identity sources — an existing `~/.gitconfig`, the Windows identity, the source checkout's — yields a name and e-mail | set `git config --global user.name` and `user.email` on Windows, then open the session again |
| `sandbox_capability_unsupported` | `session.create` on a project whose sandbox profile enables a capability this version cannot apply | remove it from the project's sandbox settings; the message names it |
| `sandbox_profile_invalid` | `session.create` on a project whose sandbox profile lists an `extra_paths` entry that is not absolute, contains a `..` component (refused outright, never resolved), or — compared textually, after collapsing repeated separators and `.` components — names, reaches into, or is an ancestor of what the base closes or a deferred capability grants: the whole filesystem; `/mnt` itself, a bare drive letter under it (a whole drive is `mnt.all`), or anything under it whose first component is not a drive letter, the same family `/run` closes; `/init`; `/run`; the kernel's interfaces; the system directories; `/opt/willie`; `/var/lib/willie`; the managed tool roots and package caches; the home directory and the private temporary directory themselves, though a path inside either is still grantable; `~/.willie`; `~/.ssh`; `~/.claude`; `~/.claude.json`; `~/.gitconfig`. An ancestor of any of these — `/home`, `/var`, `/opt` among them, none of which is itself on the list — is refused too, because it would contain what it is an ancestor of; the message names the path and the reason. An unknown key or a wrong type in the `[sandbox]` table also carries this code, set once at load: the daemon parses the record's other fields separately from its `sandbox` sub-table, so a `[sandbox]` that fails to become a `SandboxProfile` no longer takes the whole record down with it — the project loads with the default profile and this code recorded as its `sandbox_problem` (shown on the Projects screen), and `session.create`/`sandbox.explain` refuse with it until the profile is replaced through `set_sandbox`, which clears it; a record that fails to parse at all, or whose *other* fields do not match `Project`, is still skipped at start-up (`willied: skipping unreadable project …` on stderr) with no coded error. Also: at launch, an `extra_paths` entry that cannot be resolved on disk, or that resolves through a symbolic link into a location the guard refuses — the daemon's guard is textual, so this is the same rule applied to the path that will actually be mounted | the message names the path; correct it in the project's sandbox settings |
| `supervisor_spawn_failed` | `willie-sess` could not be executed, or its launcher's readiness line could not be parsed | run `willie doctor`; reinstall the distribution if the supervisor binary is missing |
| `supervisor_timeout` | no readiness reply from the supervisor within ten seconds | open the session again; run `willie doctor` if it repeats |
| `harness_exec_failed` | the harness binary is not an executable file, or the workspace is not a directory — checked by the supervisor before the helper is spawned (binary gone, workspace deleted by hand), and again at the spawn itself when the child cannot enter the working directory, which is the same condition a moment later and not the helper failing to start | reinstall Claude Code, or remove the project and add it again |
| `sandbox_backend_missing` | the supervisor found no namespace helper at `/usr/bin/bwrap`; nothing was created and no PTY exists; or the stage's report omits a required mechanism; or the kernel lacks seccomp user notification, so the stage could not install the syscall filter. Not for a helper that exists and fails to start — that is `sandbox_apply_failed` | rebuild and reinstall the distribution (`just distro-build`, `just distro-install`) |
| `sandbox_apply_failed` | the helper could not be executed; a per-project cache directory could not be created; or a path the plan binds without tolerance is not on this machine — the harness's state directory, the git configuration — checked by the supervisor before the helper is spawned, because inside it the same failure is a message on the session's terminal and a bare exit 1; the in-namespace stage found it was not inside a namespace, could not set a resource limit, had the syscall filter rejected by the kernel for any reason other than missing user notification, or reported nothing within the deadline; or a Landlock rule path could not be opened, or the kernel offered a Landlock ABI and then refused to restrict the stage; or the report named seccomp without a listener, or its control message was truncated or carried more than one descriptor — a descriptor that cannot be trusted is closed, never kept. A bind the plan marks as tolerant may be absent. Not for a missing helper (`sandbox_backend_missing`), nor for a missing harness or workspace (`harness_exec_failed`), nor for a kernel with no usable Landlock ABI, which the session reports as `unavailable` and runs without | the message names the path; the helper writes its own complaint to the session's terminal, so attach to see it, then run `willie doctor` |
| `session_not_found` | reserved for an unknown session id; not produced today (see above) | refresh the Sessions screen |
| `session_not_running` | `session.stop` on a session with no live control connection | nothing to stop; open a new session |
| `sessions_running` | `project.remove` while the project has a `running` or `stopping` session | stop the project's sessions first |
| `supervisor_lost` | a session's control connection ended and a follow-up probe of its socket got no answer, with no terminal event in its log — set only inside the session's own `Failed` state, never as a call's synchronous error | open a new session |
| `harness_already_installed` | `tool.install` when detection already finds the harness | nothing to install |
| `tool_not_installed` | `tool.update` on a tool the daemon does not detect. Not for an unknown tool id — that is `invalid_params` | install it first, then update |
| `tool_busy` | a tool job is already running | wait for the running install to finish |
| `install_failed` | the installer exited non-zero, or could not be spawned | read the installer output, check the network, then try again |

## Plugin codes

These come back as the `error` of a `plugin.*` call, or of a plugin-routed
`profile.*` call. A plugin's own coded failures (e.g. `profile_exists`
below) travel through unchanged, carrying the plugin's own code and
remediation.

| Code | When | When not | Remediation |
| --- | --- | --- | --- |
| `plugin_not_found` | `plugin.enable`/`disable`, or a `profile.*` call, names a plugin id no plugin in the registry answers to | the id is known but disabled — that is `plugin_disabled` | check `plugin.list` for the available plugin ids |
| `plugin_disabled` | a `profile.*` (plugin-routed) call while the plugin is not enabled — a global plugin whose flag is off, or a per-project plugin enabled in no project | the plugin is enabled — the call reaches it and returns the plugin's own result or coded error | enable it with `plugin.enable` before calling its methods |
| `plugin_scope_mismatch` | `plugin.enable`/`disable` in a scope the manifest forbids: a global scope (no `project_id`) for a per-project plugin, or a per-project scope (a `project_id`) for a global plugin | the scope matches the manifest — the enable/disable proceeds | enable it in the scope its manifest declares (a `project_id` for a per-project plugin, none for a global one) |
| `plugin_panicked` | a plugin's `on_enable`/`on_disable`/`handle` panicked; the panic is caught at the host boundary and the plugin is marked `degraded`, the daemon lives | the plugin returned an ordinary coded refusal — that carries the plugin's own code and does not degrade it; only a panic or an internal fault (`plugin_internal`) does | check the daemon log; the plugin stays degraded until a later call succeeds or it is re-enabled |
| `plugin_internal` | a plugin returned `PluginError::Internal` — a genuine fault (not a `Coded` refusal it can name); the plugin is marked `degraded` | the plugin returned a coded refusal (e.g. `profile_exists`) — a legitimate "no" that carries its own code and leaves the plugin healthy | retry; if it repeats, check the daemon log — the plugin stays degraded until a later call succeeds or it is re-enabled |

## Profile codes

The configuration-profiles plugin's own coded refusals (see `profile.*`
above); none of them mark the plugin `degraded` — each is a legitimate
"no" the caller can act on, not a fault.

| Code | When | Remediation |
| --- | --- | --- |
| `profile_exists` | `profile.create` names a profile that already has a directory under `store_dir` | pick a different name, or edit the existing profile |
| `profile_not_found` | `profile.read_fragment`/`write_fragment` names a **valid-shaped** profile name with no `profile.toml` under it | check `profile.list` for the available profile names |
| `profile_fragment_unknown` | `profile.read_fragment`/`write_fragment`'s `fragment` is not `settings`, `instructions`, `mcp`, or a `rules/<file>`/`hooks/<file>` naming a single, safe file name | use one of `settings`, `instructions`, `mcp`, `rules/<file>`, `hooks/<file>` |
| `profile_name_invalid` | any of the methods' `name` is empty, contains a path separator, is `.`/`..`, or opens with a Windows drive-letter pattern (`C:foo`, `a:bar`) — checked before any path is built from it, and re-checked after joining it onto `store_dir` in case the join itself produced something outside it (belt and suspenders, since this crate has no `cfg(target_os = "linux")` of its own and so also builds and runs under Windows path semantics) | use a name with no path separators, not `.` or `..`, and not shaped like a drive letter |
| `profile_target_missing` | `profile.check`/`profile.apply` against a project whose ext4 `workspace` directory does not exist on disk — checked first, before any fragment is read | re-add the project (its ext4 clone is gone) before checking or applying a profile |
| `profile_fragment_invalid` | an active `settings`/`mcp` fragment, or the project's own existing `.claude/settings.json` (or the harness state's), is not valid JSON — from the pure merge in `apply.rs`, surfaced before any write | fix the fragment's or the target's content so it parses as JSON, then check or apply again |
| `profile_markers_malformed` | the project's existing `CLAUDE.md` has a `<!-- willie:begin -->` marker with no matching `<!-- willie:end -->` after it — refused rather than appending a second block that would never converge on a later apply | fix or remove the stray `<!-- willie:begin -->` marker in `CLAUDE.md`, then apply again |
| `profile_fragment_missing` | an internal wiring fault: an active fragment for which `check`/`apply` did not supply target content to the pure planner — not expected to occur, since every active fragment's target is always read before planning | check the daemon log; this is a bug in Willie, not something to fix in the profile |

## Engine problem codes

These are produced on the Windows side, not by the daemon. They cross
the app's IPC boundary as `Problem { code, message, remediation }` and
the UI keys behaviour on them, so they are stable once released.

| Code | When | When not | Remediation |
| --- | --- | --- | --- |
| `wsl_not_installed` | `wsl.exe` could not be spawned at all | it ran and answered something unexpected — that is `wsl_unparseable_output` | enable WSL 2.4.4 or newer (administrator) and retry |
| `wsl_command_failed` | a `wsl.exe` management command exited non-zero | the command worked and only its output was surprising; a failure naming HCS `0x80070569` is `service_logon_right_missing` | one text is special-cased: a failed `--import` says the image is still on disk and to retry Install; otherwise run the same command in PowerShell, then click Run doctor again |
| `service_logon_right_missing` | a `wsl.exe` command or a daemon start failed with HCS `0x80070569`: the virtual-machine account lacks "Log on as a service", so no WSL 2 VM can be created. The one code two different failures share, because the remedy is the same one | the failure has any other cause — those keep `wsl_command_failed` or `daemon_exited` | an administrator must grant that right to `NT VIRTUAL MACHINE\Virtual Machines` (S-1-5-83-0), then sign in again; the Dashboard offers the exact commands to copy |
| `wsl_unparseable_output` | `wsl.exe` answered, but not with what the reading needs (no version in `--version`) | the command failed — that is `wsl_command_failed` | run `wsl --version` in PowerShell, update WSL if it is old, then click Run doctor |
| `wsl_io` | the pipe to `wsl.exe` broke while the command was running | the child process died — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_error` | the daemon answered with a well-formed RPC error: it is alive and refused | the daemon is silent, dead or off-protocol | the daemon's own remediation when it sent one, otherwise click Run doctor to retry |
| `daemon_timeout` | no reply within the call budget (60 s for `hello`, 10 s after) | the call was answered with an error — that is `daemon_error` | click Run doctor (it restarts the daemon) |
| `daemon_transport` | writing to or reading from the daemon's pipe failed | the child exited and the exit explains it — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_exited` | the child process is gone; the detail is its stderr, and when the daemon failed to start and stderr is silent, `wsl.exe`'s own stdout message instead — a daemon that dies after saying hello reports stderr only | the daemon is alive but slow or wrong; a start that failed on the host right is `service_logon_right_missing` | by detail: Install distribution for a missing one, otherwise one more Run doctor |
| `protocol_violation` | a line was an envelope but not usable: closed stdout, or a result the method cannot hold | the line was no envelope at all (those are ignored) | click Run doctor (it restarts the daemon); if it repeats, click Install distribution |
| `version_mismatch` | `hello` reported a Willie version other than the app's | the protocol version differs — the daemon answers `protocol_version_mismatch` itself | click Install distribution: it reinstalls the distribution with binaries matching this app |
| `image_invalid` | the image beside the app is missing, empty, or its sha256 does not match the `.sha256` sidecar | there is no image at all — that is `image_not_found` | rebuild the image with `just distro-build`, then click Install distribution again |
| `image_not_found` | no image in any candidate location (`WILLIE_ROOTFS`, the resource dir, `target/distro`) | one was found and rejected — that is `image_invalid` | reinstall Willie; in development run `just distro-build` |
| `daemon_not_running` | a call was made while no daemon is live | the daemon was live and died — that is `daemon_exited` | click Run doctor (it starts the daemon) |
| `distro_not_registered` | the pre-flight before a start found no `willie` distribution | the distribution exists and its daemon failed — that is `daemon_exited` | click Install distribution |
| `terminal_launch_failed` | `session_open`/`session_attach` could open neither a Windows Terminal tab nor a fallback console, for a session that is already `running` (`EngineError::TerminalLaunch`, `crates/willie-engine/src/error.rs`) — produced only on the Windows side, never by the daemon | the session itself failed to start — that is the daemon's own `Session.state: failed` code from `session.create`, unrelated to this one | **non-fatal**: the session keeps running; open any terminal and paste the given `wsl -d willie --user willie -- willie attach <id>` line |
| `embedded_terminal_failed` | the engine could not spawn `wsl.exe`/`willie attach --host` to open a session in the in-app embedded terminal (`EngineError::EmbeddedTerminal`, `crates/willie-engine/src/embed.rs`) — produced only on the Windows side, never by the daemon | the Windows-Terminal-tab path failed instead — that is `terminal_launch_failed`, a separate, unrelated code | **non-fatal**: the session keeps running; open it in a Windows Terminal tab instead (Open session), or click Run doctor |
