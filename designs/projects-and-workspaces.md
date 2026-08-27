# Projects and ext4 workspaces

## Problem

Agents work on files, and every file operation on a repository that
lives on the Windows drive pays a 9p round trip inside the distribution:
`git status` on a 129-file tree costs 511 ms on DrvFs against 4.9 ms on
ext4, and a first run after a pause 3.1 s against 0.1 s (decision 0011).
A realistic checkout would put every `git status` an agent runs at
several seconds. Today Willie has no notion of a project at all: the
first slice delivered the distribution, the daemon and health; the
user's code is still only under `C:\`.

This feature gives Willie **projects**: a registered Windows checkout
paired with a clone of it inside the distribution's ext4 disk, kept in
sync through git, with the Projects screen to add, watch, sync and
remove them. The sessions feature that follows it (slices table in
`docs/ARCHITECTURE.md` §5.5) starts agents inside those workspaces.

## Goals

- Register a Windows checkout as a project and get an ext4 workspace
  that is a faithful git clone of it, with the original remotes kept
  and the Windows checkout reachable as the remote `windows`.
- Send the agent's commits back to the Windows checkout, and bring the
  Windows checkout's commits into the workspace, from the UI — with
  refusals instead of surprises when either tree is dirty or diverged.
- Show every long operation as a job with progress, a failure code and
  a remediation; never block the UI or the engine on a clone.
- Feed the UI from one daemon snapshot plus an event stream, the data
  path every later slice uses.
- Run the daemon's Linux tests for real, inside the distribution, from
  the quality gate.

## Non-goals

- Sessions, `willie-sess`, `willie attach`, Windows Terminal profile,
  harness detection — the sessions feature, next in the slices table.
- Sandbox and capabilities — the sandbox feature (a project's
  capabilities are not even recorded yet).
- Installing or updating tools — the managed-tools feature.
- Submodules, git-lfs, worktrees per branch, a git client in the UI
  (branch switching stays in the terminal).
- SQLite: the daemon indexes projects in memory from their TOML files;
  the database arrives with the first query that needs it.
- A data-preserving image upgrade: this slice changes the image, so a
  reinstall (which discards the distribution's data) is still the way.

## Design

### Domain — `crates/willie-core/src/project.rs`, `id.rs`, `paths.rs`

```rust
pub struct Project {
    pub id: ProjectId,              // proj_…
    pub name: String,               // display only, editable
    pub slug: String,               // fixed at creation: workspace dir
    pub source: WindowsPath,        // "C:\github\x", as registered
    pub workspace: String,          // "/home/willie/projects/<slug>"
    pub branch: String,             // branch checked out at add time
    pub state: ProjectState,        // Preparing | Ready | Failed { … }
    pub source_present: bool,       // computed per snapshot, not stored
    pub created_at: String,         // RFC 3339
}
pub enum ProjectState {
    Preparing, Ready, Failed { code, message, remediation },
}
pub type JobId = Id<Job>;           // job_… (new id kind)
pub fn windows_to_drvfs(path: &str) -> Option<String>;  // C:\x → /mnt/c/x
pub fn slug_for(folder_name: &str, taken: &[String]) -> String;
pub fn source_key(path: &str) -> String;  // lower-case, `\`, no trailing sep
```

`windows_to_drvfs` is pure and shared by the engine (validation) and the
daemon (operations); the engine's `paths::to_wsl_path` delegates to it.
`slug_for` produces kebab-case ASCII from the folder name and appends
`-2`, `-3`… while the slug is taken. `source_key` is the identity used
to refuse registering the same checkout twice (`project_exists`).

Truth on disk: `/var/lib/willie/projects/<id>.toml`, one file per
project, rewritten on every state change. Workspaces live under
`/home/willie/projects/` (user-data zone: a checkout is the user's;
the sandbox feature mounts it rw as `project.rw`).

### Daemon — `crates/willied/src/{state,projects,jobs,git}.rs`

`State { projects: BTreeMap<ProjectId, Project>, jobs: BTreeMap<JobId,
Job>, seq: u64 }` behind a `Mutex`. Handlers answer in milliseconds;
everything that runs `git` is a **job**.

```
request thread (stdin) ─► handler ─► State ─► reply ─────┐
                                                          ├─► writer (stdout)
job threads (git) ──────► State ─► state.event ──────────┘
```

- One writer thread owns stdout and receives responses and
  notifications over a channel, so lines never interleave.
- Jobs run `git` through `std::process::Command` with a 4 KiB log tail
  and are limited to **3 at a time**, the rest queued; a project accepts
  **one job at a time** (`project_busy`).
- On `daemon.shutdown` or stdin EOF the daemon kills running `git`
  children, records `Failed { code: "interrupted" }` in the affected
  TOMLs and exits; the UI offers *retry*. At start-up a scan turns any
  `Preparing` project found on disk into the same `interrupted` failure.

Operations (`projects.rs`), all as jobs:

| Operation | Steps | Refusals |
| --- | --- | --- |
| `add` | validate source is under `/mnt/*`, is a git repository with an attached HEAD; `git clone <drvfs> <workspace>`; rename `origin` → `windows`; copy the source's remotes into the clone; `core.autocrlf=false` in the clone; `receive.denyCurrentBranch=updateInstead` in the **Windows** repository; state `Ready` | `path_not_windows`, `not_a_git_repository`, `source_detached_head`, `project_exists`, `workspace_exists` |
| `sync_to_windows` | source present; Windows tree clean and on the project's branch; `git -C <workspace> push windows HEAD:<branch>` | `source_missing`, `windows_tree_dirty`, `windows_branch_mismatch`, `git_failed` |
| `update_from_windows` | source present; `git fetch windows`; fast-forward `<branch>` only | `source_missing`, `workspace_diverged`, `git_failed` |
| `remove` | delete the TOML; when `delete_workspace`, `rm -rf <workspace>` — refused if the workspace has uncommitted changes and `force` is false; never touches `C:\` | `workspace_dirty` |
| `relocate` | new source must be a git repository whose history contains the workspace's `HEAD`; rewrite `source` and the `windows` remote URL | `path_not_windows`, `not_a_git_repository`, `source_unrelated` |
| `rename` | changes `name` only; synchronous (no job) | `project_not_found` |

`source_missing` is computed by the daemon when it builds a snapshot and
before every job: the source directory no longer exists or has no
`.git`. It is a flag on the project (`source_present: bool`), not a
state — the workspace stays usable throughout.

Submodules are not recursed (`submodules_skipped` in the job log); a
`.gitattributes` mentioning lfs adds `lfs_not_supported` to the log.

### Protocol — `crates/willie-proto/src/{project,job,state}.rs`

Documented in `docs/PROTOCOL.md`. `PROTOCOL_VERSION` stays 1: everything
is additive.

| Method | Params → Result |
| --- | --- |
| `project.list` | `{}` → `{ projects: [Project] }` |
| `project.add` | `{ windows_path, name? }` → `{ project_id, job_id }` |
| `project.remove` | `{ id, delete_workspace, force }` → `{ job_id }` |
| `project.sync_to_windows` | `{ id }` → `{ job_id }` |
| `project.update_from_windows` | `{ id }` → `{ job_id }` |
| `project.relocate` | `{ id, windows_path }` → `{ job_id }` |
| `project.rename` | `{ id, name }` → `{ project }` |
| `job.list` / `job.get` | `{}` / `{ id }` → `Job { id, kind, project_id, state: running \| done \| failed { code, message, remediation }, started_at, finished_at?, log_tail }` |
| `job.cancel` | `{ id }` → `null` (kills the job's `git`; the job ends `failed { code: "cancelled" }`) |
| `state.snapshot` | `{}` → `{ seq, projects, jobs }` |
| `state.event` (notification) | `{ seq, kind, payload }` — `project.changed { project }`, `project.removed { id }`, `job.changed { job }` |

`seq` is monotonic for the daemon's lifetime. A client applies events in
order; on a gap, or after the daemon restarts, it asks for a new
snapshot. There is no replay buffer.

### Engine — `crates/willie-engine/src/{rpc,daemon,engine,config}.rs`

The RPC client becomes a **reader thread** that owns the daemon's stdout:
envelopes with an `id` are routed to the waiting caller through a
per-call channel (same deadlines as today), notifications go to a
subscriber channel, non-envelope lines are still kept for the
`daemon_exited` detail. End of stdout fails every pending call with the
existing classification.

- `DaemonSupervisor::subscribe() -> Receiver<Notification>`; the engine
  forwards to the app, which emits the Tauri event `daemon://event`.
- New `Engine` methods, each with the distro pre-flight and the
  start-on-demand `run_doctor` already has: `project_list`,
  `project_add`, `project_remove`, `project_sync_to_windows`,
  `project_update_from_windows`, `project_relocate`, `project_rename`,
  `job_cancel`, `state_snapshot`.
- `engine.toml` appears: `[projects] roots = ["C:\\path\\to\\repos"]`
  — user-entered, **no default value in the code**. Read and written by
  `config.rs` with `toml`.
- `discover(roots) -> Vec<Candidate { path, name, registered: bool }>`
  walks each root up to two levels looking for `.git` (directory or
  file), on the Windows side.
- The engine validates only that a Windows path exists and is a
  directory (`path_not_found`) before sending it; mapping to `/mnt/c` is
  the daemon's.

### App — `apps/willie-app/src-tauri/src/lib.rs`, `apps/willie-app/src/`

Tauri commands mirror the engine methods (`async`), plus
`projects_roots_get/set`, `projects_discover` and
`open_in_explorer(path)` (runs `explorer.exe`; no plugin). One new
dependency, `tauri-plugin-dialog`, for the Windows folder picker.

UI store `useDaemonState()` (`src/lib/state.ts`): replaced by a
snapshot, updated by `project.changed`/`project.removed`/`job.changed`,
re-snapshots on a `seq` gap or when `engine_status` shows the daemon
restarted. `App.tsx` gains a two-tab navigation: **Dashboard | Projects**.

Projects screen (`src/screens/Projects.tsx`):

| Element | Behaviour |
| --- | --- |
| Roots | list of base folders (picker or typed), saved to `engine.toml`; **Discover** scans them and lists repositories with checkboxes → one `project.add` per selected repo, queued |
| Add by path | picker or typed full path; name suggested from the folder, editable |
| Row | name (editable inline → `project.rename`) · Windows path · `\\wsl.localhost\willie\home\willie\projects\<slug>` with *copy* and *open in Explorer* · branch of both sides, warning when they differ · state chip (`preparing` with spinner and log tail, `ready`, `failed` with remediation and *retry*) · `source_missing` badge with *Relocate* |
| Actions | *Send to Windows*, *Update from Windows*, *Remove* (dialog: delete workspace? — a `workspace_dirty` refusal explains and offers force), *Cancel* on a running job |

The Dashboard shows the project count. The UI never computes truth:
states, flags and errors come from the daemon.

### Image — `distro/provision.sh`

- `/etc/default/locale` with `LANG=C.UTF-8`; daemon and, later, the
  session supervisor also pass `LANG` explicitly to children.
- `~/.willie/agent-state/claude/claude.json` placeholder is `{}`
  (0600), not empty — an empty file is rejected as corrupted.
- `/home/willie/projects` created `0700 willie:willie`.

### Linux tests — `xtask/src/test_linux.rs`, `justfile`

`just test-linux` (`cargo xtask test-linux`) builds the test binaries of
the Linux crates for `x86_64-unknown-linux-musl` with the same
cross-compiler as the release binaries (`--no-run`,
`--message-format=json` to find the executables) and runs each inside
the distribution through `wsl.exe --exec` straight from
`/mnt/c/…/target/…` (static binaries). Opt-in through
`WILLIE_TEST_DISTRO=willie`; `just check` includes it when the variable
is set. The existing `willied` and `willie-linux` tests start running on
Linux for real.

### Errors and edge cases

Daemon RPC / job codes (documented with "when not" in `PROTOCOL.md`):
`path_not_windows`, `path_not_found` (engine), `not_a_git_repository`,
`source_detached_head`, `project_exists`, `workspace_exists`,
`project_not_found`, `project_busy`, `source_missing`,
`windows_tree_dirty`, `windows_branch_mismatch`, `workspace_diverged`,
`workspace_dirty`, `source_unrelated`, `git_failed` (with git's stderr
tail), `interrupted`, `cancelled`.

| Case | Behaviour |
| --- | --- |
| Other drive (`D:\…`) | `/mnt/d/…`, normal |
| UNC or network path | `path_not_windows` |
| Spaces and accents in the path | supported; paths are always passed as arguments, never through a shell |
| Same checkout registered twice (case-insensitive, trailing separator ignored) | `project_exists` |
| Windows checkout on a detached HEAD | `source_detached_head` at add; `windows_branch_mismatch` at sync |
| Non-git folder | `not_a_git_repository`, remediation: `git init` and a first commit in the folder, then add again — Willie never creates history for the user |
| Source removed or moved | `source_present = false`, sync refused with `source_missing`, *Relocate* fixes it |
| App closed during a clone | job killed, project `Failed { interrupted }`, *retry* |
| Daemon restarted with a `Preparing` project | same `interrupted` failure at start-up |
| Windows tree dirty at *Send* | `windows_tree_dirty`; nothing touched (`updateInstead` only updates a clean tree, and the daemon checks first to give the better message) |

## Testing

| Goal | Tests | Where |
| --- | --- | --- |
| Pure domain | slug derivation (kebab, collisions), `windows_to_drvfs` (drives, UNC, relative), `source_key`, `Project` TOML round-trip | `#[cfg(test)]` in `willie-core`, `willied` |
| Protocol | serde round-trips for every new type, unknown fields ignored, `Job` state tag | `willie-proto` |
| Daemon operations | temp git repository under `/mnt/c/Users/<user>/AppData/Local/Temp` standing in for the Windows checkout: add copies remotes and renames `origin`; send updates a clean tree and refuses a dirty one without touching it; update fast-forwards and refuses divergence; remove refuses a dirty workspace without `force`; a killed job leaves `interrupted`; `job.cancel` ends `cancelled`; second job on a busy project refused | `crates/willied/tests/projects.rs`, run by `just test-linux` |
| Daemon streaming | snapshot then events with increasing `seq`; writer never interleaves under concurrent jobs | `crates/willied/tests/stream.rs` |
| Engine RPC client | responses routed by id while notifications arrive; pending calls fail on EOF; deadlines kept | peer-process tests in `willie-engine` (existing `test_support`) |
| Engine config | `engine.toml` round-trip, roots absent ⇒ empty, unknown keys ignored | `willie-engine` |
| Discover | temp tree with nested `.git` dirs and a `.git` file; depth limit | `willie-engine` |
| UI store | snapshot replace, event apply, gap ⇒ resnapshot | Vitest `state.test.ts` |
| Acceptance | `docs/checklists/projects-and-workspaces-acceptance.md` walked by the user | manual |

## Rollout / compatibility

- `PROTOCOL_VERSION` unchanged (additive namespaces).
- New files only: `/var/lib/willie/projects/*.toml`,
  `/home/willie/projects/*`, `%LOCALAPPDATA%\Willie\data\engine.toml`.
- Image rebuilt (`just distro-build`) and reinstalled; the Claude Code
  login inside the distribution is redone once.
- Documents in the same commits: `docs/PROTOCOL.md` (namespaces and
  codes), `docs/ARCHITECTURE.md` (§1.2 zone `/home/willie/projects`,
  §3.4 and §5.1 Projects row without `/mnt/c` and `slow_fs`, the F1
  row of §5.5 split into projects and sessions), `AGENTS.md`
  (`just test-linux`), `releases/v0.1.0.md`, decision **0013** — sync
  through git remotes
  with `updateInstead` (alternatives rejected: rsync or two-way file
  sync, mounting the workspace back under `C:\`).

## Open questions

- Concurrency limit of 3 jobs: a guess; revisit with real repositories.
- Whether *Discover* should also list repositories already registered as
  greyed rows (favoured) or hide them.

## Follow-ups for the next slice

- **Git identity for workspace commits.** The distribution's `willie`
  user has no git identity of its own. As a stopgap, `add` now copies the
  source checkout's `HEAD` commit author into the clone's local
  `user.name`/`user.email`, so a commit made in a workspace (by a person
  in a plain shell, or by the agent) works without a manual step in the
  common case. It does not cover a checkout whose history has no
  readable author, or a workspace commit that should carry the *current*
  user's identity rather than the source's original author — the
  sessions/sandbox slice still owns that full story: mount the user's
  Windows `~/.gitconfig` (the `git.identity` capability in ARCHITECTURE
  §3.3), or provision a default one.
