# Sessions — a Claude Code terminal per project

## Problem

Willie registers projects and keeps their ext4 workspaces in sync, but
it cannot yet do the one thing it exists for: launch an agent. Today the
only way to run Claude Code inside the distribution is a shell opened by
hand, with no record of what ran where, no way to find a session again
from the app, and nothing that survives the app being closed.

Spike S2 (decision 0012) measured the shape that fixes this — a detached
supervisor per session owning a PTY and serving attach clients over a
Unix socket, with Windows Terminal as a plain client — and left a list of
gaps a real implementation must close: no control channel, a readiness
handshake that confirms the socket but not the harness, no clean
`SIGTERM`, replay that flashes stale bytes into a full-screen program, a
single unbounded client. This feature turns that prototype into the
sessions slice of `docs/ARCHITECTURE.md` §5.5: **F1 — sessions, without
the sandbox**.

## Goals

- From a project's row, open a Claude Code session in a Windows Terminal
  tab, running inside the distribution with the ext4 workspace as its
  working directory, in under two perceived seconds.
- Sessions survive closing the app, killing the daemon and closing the
  tab; a new tab can attach to a running session, several tabs at once.
- The daemon re-adopts live supervisors when it restarts and finalises
  dead ones from their event logs; the app always shows the truth.
- Fail closed with an actionable message: no harness installed, no git
  identity, an unreadable spec — the session does not start.
- Install Claude Code from its official installer on an explicit user
  action, with progress, and nothing else installed without asking.
- Every behaviour above is testable inside the distribution without the
  app, against a fake harness.

## Non-goals

- The sandbox (bubblewrap, seccomp, Landlock) and capabilities — the
  sandbox feature. This slice fixes the launcher's inputs (`argv`, `env`,
  cwd) so that feature only changes how the child is wrapped.
- Resuming a previous Claude Code conversation — needs the harness's own
  session id, matched from its JSONL files; arrives with the feature
  that reads them.
- An embedded terminal or read-only attach — decision 0006 left both
  undecided; a tab in Windows Terminal is the client for now.
- A Windows Terminal profile fragment — the spike opened tabs without
  touching `settings.json`; a profile is cosmetics until proven needed.
- Proxy and CA propagation — the network feature.
- SQLite — the session index is memory rebuilt from the files at start,
  like projects; the database arrives with the first query that needs it.
- A second harness — the trait grows only what this slice calls; only
  `ClaudeCode` implements it.

## Design

### Domain — `crates/willie-core/src/session.rs`, `id.rs`

```rust
pub struct Session {
    pub id: SessionId,              // sess_…
    pub project_id: ProjectId,
    pub harness: String,            // "claude-code"
    pub workspace: String,          // cwd handed to the harness
    pub state: SessionState,
    pub created_at: String,         // epoch seconds, like jobs
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub pid: Option<u32>,           // the harness, while running
    pub clients: u32,               // attached terminals
}

pub enum SessionState {
    Creating,
    Running,
    Stopping,
    Exited { code: Option<i32>, signal: Option<i32> },
    Failed { code: String, message: String, remediation: String },
}
```

`Failed` has the shape of `ProjectState::Failed` so the app reuses the
state chip and the inline remediation. Zero I/O, as the crate demands.

### Session files — `/var/lib/willie/sessions/<id>/`

The files are the truth; the daemon's memory is an index of them.

| File             | Written by | Content                                                                 |
| ---------------- | ---------- | ----------------------------------------------------------------------- |
| `spec.json`      | daemon     | immutable: id, project_id, harness, workspace, `argv`, `env`, created_at, Willie version — the whole launch, resolved; the supervisor executes it without knowing what a harness is |
| `events.jsonl`   | supervisor | append-only, one JSON object per line: `{"at","kind",…}` with kinds `created`, `started{pid}`, `attached{client}`, `detached{client}`, `resized{rows,cols}`, `stop_requested{by}`, `exited{code,signal}`, `failed{reason}` |
| `supervisor.log` | supervisor | its own stderr — diagnostics, never read for state                      |

The socket is `/run/willie/sessions/<id>.sock`, mode 0600. Nothing
under `/mnt/*`; nothing is removed in this slice — finished sessions are
history, two small files each.

### Wire protocol — `crates/willie-linux/src/wire.rs`

The spike framed one direction; both are framed now, so a client can
tell *why* it was closed and the daemon can ride the same socket. Every
frame is `type:u8 len:u16` big-endian plus payload (≤ 64 KiB; output is
chunked). Binary payloads where bytes flow, small JSON where they do not.

| Direction           | Frame     | Payload                                                          |
| ------------------- | --------- | ---------------------------------------------------------------- |
| client → supervisor | `hello`   | JSON `{ "role": "terminal" \| "control", "rows", "cols" }` — mandatory first frame |
|                     | `input`   | bytes for the PTY master                                         |
|                     | `resize`  | `rows:u16 cols:u16`                                              |
|                     | `detach`  | empty; leave, session keeps running                              |
|                     | `stop`    | empty; `control` only                                            |
|                     | `status`  | empty; `control` only                                            |
| supervisor → client | `output`  | bytes from the PTY                                               |
|                     | `status`  | JSON `{ "pid", "state", "clients", "started_at" }`               |
|                     | `event`   | the exact JSON line just appended to `events.jsonl`; `control` only |
|                     | `closed`  | JSON `{ "reason": "exited" \| "stopped" \| "shutdown" \| "too_slow" \| "protocol", "code"?, "signal"? }` |

A malformed frame, or a first frame that is not `hello`, closes that
client with `protocol`. The codec stays pure (no I/O) so both binaries
share one definition and the tests run on any host.

### Supervisor — `crates/willie-sess/src/{main,detach,pty,child,server}.rs`

`willie-sess run --spec <path>`. Kept from the spike, as decision 0012
requires: double fork with `setsid` in the intermediate (the supervisor
never leads a session), a readiness pipe back to the launcher, the
launcher's descriptors dropped, stdio redirected to `supervisor.log`,
`/dev/ptmx` plus ioctls, fail-closed socket path handling (a live path
is refused, a dead one is unlinked), `chdir("/")` with the workspace
passed to the harness as its explicit cwd.

**Readiness confirms the harness.** The harness child holds a
close-on-exec pipe: a successful `execvp` closes it (EOF ⇒ `started`,
the pipe reply is `ok <pid>`); a failed one writes `errno` (the reply is
`fail harness_exec_failed: <text>`). PTY or bind failures reply
`fail <code>: <text>` the same way. The daemon never sees "started"
followed by an immediate exit.

**Launch (no sandbox).** `argv`, `env` and cwd come from the spec. The
child does `setsid`, `TIOCSCTTY`, dups the slave onto 0/1/2 and execs;
the PTY starts at 80×24 until the first `hello`. The environment is an
explicit allowlist even without a sandbox, so the sandbox feature only
replaces the wrapper:
`PATH=/home/willie/.local/bin:/usr/local/bin:/usr/bin:/bin`, `HOME`,
`USER`, `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG=C.UTF-8`,
`TZ` (when set), `DISABLE_AUTOUPDATER=1`. Nothing else leaks from the
daemon's environment.

**Clients.** Several, all read-write (decision 0006). Each has a reader
thread (frames → PTY, resize, detach) and a writer thread fed by a
**bounded queue** of 1 MiB; a full queue means a wedged terminal and the
client is closed with `too_slow` — it reattaches and gets the replay.
The last resize wins. `attached`/`detached` are logged with a client id.

**Replay.** A 256 KiB ring of PTY output. On a terminal's `hello`: if the
harness is not in the alternate screen, the ring is sent; if it is (an
`ESC[?1049h`, `?1047h` or `?47h` seen without its `l`), nothing is sent
and a redraw is forced by applying the client's size — equal to the
current one, the supervisor nudges (cols−1, then cols) so the child gets
`SIGWINCH`. Claude Code draws on the main screen, so the common path is
the replay the spike validated.

**Lifecycle.** Harness exit → `exited{code,signal}` logged, `closed
{exited}` to every client, socket unlinked, supervisor exits when the
last client disconnects or after two seconds. A stop — the `stop` frame
from the daemon **or** `SIGTERM`/`SIGHUP` to the supervisor — logs
`stop_requested{by}`, then `SIGINT` to the harness's process group, five
seconds, `SIGTERM`, five seconds, `SIGKILL`, then the normal exit path.
No orphan socket and no log without a terminal event, on any path. The
grace periods are `WILLIE_SESS_STOP_GRACE_MS` for the tests only. The
supervisor never depends on the daemon: with no control client, events
go to the log alone.

### Attach client — `crates/willie-cli/src/attach.rs`

`willie attach <id>` resolves `/run/willie/sessions/<id>.sock` (a path
is accepted too, for tests). No socket: `willie attach: session <id> is
not running` on stderr, exit 1. Otherwise: connect, `hello{terminal,
TIOCGWINSZ}`, raw mode on the tty with a guard that restores it on every
exit path, one loop stdin → `input`, one loop socket → `output` written
straight to descriptor 1 (never a line-buffered handle). `SIGWINCH` is
installed without `SA_RESTART` so the interrupted read is the resize
notification. `Ctrl-]` detaches (exit 0); a one-line `detach: Ctrl-]`
hint goes to stderr on attach. `closed{reason}` restores the terminal,
prints one line (`session exited with code 7`, `session stopped`,
`supervisor shutting down`, `connection too slow, reattach`) and exits
**with the harness's exit code** when the session ended — 0 closes the
Windows Terminal tab gracefully, anything else keeps it open with the
message, which is the behaviour wanted for failures. Exit 1 for
connection errors, 2 for usage. `attach` is the only new subcommand.

### Harness — `crates/willie-harness/src/lib.rs`

The trait grows exactly what this slice calls:

```rust
fn parse_version(&self, output: &str) -> Option<String>;                 // pure
fn detect(&self, binary: &Path) -> Option<Installed>;                    // runs `--version`, 5 s timeout
fn launch(&self, binary: &Path, workspace: &Path, home: &Path) -> Launch; // pure: the allowlist above
fn installer(&self) -> &'static str;                                     // one `sh -c` command line, the official installer
```

The daemon locates the binary on the session `PATH` (or at
`WILLIE_HARNESS_BIN`, for a custom install location and the tests).
`HarnessCapabilities` is unchanged; `state_paths`, `resume_args` and
`usage_sources` stay out until a consumer exists.

### Daemon — `crates/willied/src/{sessions,control,identity,tools}.rs`

**`session.create { project_id, git_identity? }` → `{ session }`**, in
this fail-closed order:

1. the project exists and is `ready` — else `project_not_ready`;
2. the harness is detected — else `harness_not_installed`;
3. the git identity is ensured (below) — else `git_identity_missing`;
4. the spec is built from `Harness::launch` and written;
5. `willie-sess run --spec …` is spawned and the readiness pipe read
   with a ten-second budget: `ok <pid>` → connect as `control`, the
   session is `running`; `fail <code>` → `Failed{code}`; no reply →
   `SIGTERM` to the pid if the pipe named one, `supervisor_timeout`.

Several sessions per project are allowed. `session.stop { id }` sends
`stop` on the control connection and moves the session to `stopping`
(`session_not_running` without a connection). `session.list` mirrors
`job.list`; the app reads the snapshot. There is no `attach_command` in
the reply: the engine composes it from the distro name and the binary
path it already knows.

**Control connection.** One thread per live session reads `event`,
`status` and `closed` frames and translates them into `Session` changes
plus `session_changed` events: `attached`/`detached` move `clients`,
`stop_requested` → `stopping`, `exited`/`failed` → the final state and
the thread ends. A read that fails without a terminal event re-scans
that one session: socket gone → finalised from the log,
`Failed{supervisor_lost}` when the log has no terminal event either.

**Start-up scan.** For every `sessions/<id>/`: read the spec and the log
(`started_at` already folds in from a logged `started` event); a socket
that answers `status` is adopted as `running` (pid and clients taken from
the reply) and gets a control thread; a socket that does not answer is
unlinked and the session finalised from the log. Daemon shutdown closes
the control connections; supervisors do not notice.

**Git identity** (`identity.rs`), resolved in order and written with
`git config --global` as the daemon's user: (1) `/home/willie/.gitconfig`
already holds `user.name` and `user.email` — nothing to do; (2) the
`git_identity` in the params, read by the engine from the Windows global
configuration; (3) `git -C <source> config user.name` / `user.email` on
the Windows checkout; (4) `git_identity_missing`. Decision 0015 requires
the stopgap `add` applies today — `projects.rs::copy_source_identity`
copying the source `HEAD` author into the clone's *local* configuration
— to be **removed**: a local identity outranks the global one, so the
copied author (possibly another person) would win over the user's own
whenever a commit is made directly in the workspace. That removal has
**not landed**: `copy_source_identity` still runs on every `add`, so
every workspace — not only ones from before this slice — keeps a local
override until `git config --unset user.name` / `user.email` is run in
it by hand; the acceptance checklist says so. Removing the call is
tracked as a follow-up in decision 0015, not closed by this slice.

**`project.remove`** is refused with `sessions_running` while a session
of that project is `running` or `stopping`: unregistering or deleting the
cwd under a working agent is never an option.

**`tool.install { harness }` → `{ job_id }`** (`tools.rs`): a job of kind
`install_harness` running the harness's official installer through
`sh -c` as `willie`, with the existing `log_tail` streaming; on success
the harness is re-detected. Refused with `harness_already_installed` or
`tool_busy`; a failed run ends `install_failed` with the tail as the
message. `WILLIE_HARNESS_INSTALLER` replaces the command for the tests
only. To fit the job model, `Job.project_id` becomes `Option<ProjectId>`
(omitted from the JSON when absent).

### Protocol — `crates/willie-proto/src/{session,tool,job,state}.rs`

`session.create`, `session.stop`, `session.list`, `tool.install`;
`Snapshot.sessions: Vec<Session>`; `EventKind::SessionChanged { session }`.
`Job.project_id` optional and a new `JobKind::InstallHarness`. Every
code below gets a row in `docs/PROTOCOL.md`.

### Engine — `crates/willie-engine/src/{engine,terminal,identity}.rs`

- `session_open(project_id) -> SessionOpened { session, terminal_problem? }`:
  reads the Windows identity (`git.exe config --global user.name` /
  `user.email`, `CREATE_NO_WINDOW`; no `git.exe` ⇒ none sent), calls
  `session.create`, then opens the tab with

  ```
  wt.exe -w 0 new-tab --title "<project>" -- wsl.exe -d willie \
    --user willie --exec /opt/willie/bin/willie attach <id>
  ```

  `wt.exe` is looked up on `PATH` and under
  `%LOCALAPPDATA%\Microsoft\WindowsApps`; absent, the fallback is the
  same `wsl.exe …` line in a new console (`CREATE_NEW_CONSOLE`). A
  terminal that fails to open does **not** undo the session: it comes
  back as `terminal_problem` (`terminal_launch_failed`, remediation:
  the `wsl -d willie --user willie -- willie attach <id>` line to paste).
- `session_attach(id)` opens the tab only; `session_stop(id)`;
  `tool_install(harness)`.
- The command lines are composed by pure functions, tested on the host.

### App — `apps/willie-app/src-tauri/src/lib.rs`, `apps/willie-app/src/`

Tauri commands `session_open`, `session_attach`, `session_stop`,
`tool_install`; events keep flowing through `daemon://event`.

- `lib/proto.ts`: `Session`, `SessionState`, `Snapshot.sessions`,
  `session_changed`; `lib/state.ts` upserts sessions by id.
- Projects row: **Open session** (enabled on `ready`; a rejection is a
  row problem — `harness_not_installed` points at the Dashboard) and a
  badge with the count of live sessions.
- **Sessions** screen (new tab): project name (or "removed project"),
  state chip reusing the projects' chip (`running`, `stopping`,
  `exited 0`, `failed` with inline remediation), relative start time,
  attached clients, **Attach** and **Stop** for `running` (Stop asks no
  confirmation: it begins with `SIGINT`). Finished sessions: the twenty
  most recent. Pure derivations (sorting, recency) in `lib/sessions.ts`
  with tests.
- Dashboard: the doctor's "Claude Code" check gains **Install** when the
  harness is absent; while the job runs the button is disabled and shows
  the last `log_tail` line.

### Errors and edge cases

| Code                        | When                                                                    | Remediation                                                        |
| --------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `project_not_ready`         | `session.create` on a project that is `preparing` or `failed`           | wait for the project to be ready, or fix its failure first        |
| `harness_not_installed`     | no harness binary on the session `PATH` (or `--version` fails)          | click Install on the Dashboard                                     |
| `git_identity_missing`      | none of the four identity sources yields name and e-mail                | set `git config --global user.name` / `user.email` on Windows, open the session again |
| `supervisor_spawn_failed`   | `willie-sess` could not be executed                                     | run `willie doctor`; reinstall the distribution if the binary is missing |
| `supervisor_timeout`        | no readiness reply within ten seconds                                   | open the session again; run `willie doctor` if it repeats          |
| `harness_exec_failed`       | the harness child's `execvp` or `chdir` failed (binary gone, workspace deleted by hand) | reinstall Claude Code, or remove the project and add it again |
| `session_not_found`         | reserved for an unknown session id; not produced today — `session.stop` reports `session_not_running` for that case too | refresh the Sessions screen                                        |
| `session_not_running`       | `session.stop` on a session with no control connection                  | nothing to stop; open a new session                                |
| `sessions_running`          | `project.remove` while the project has a live session                   | stop the project's sessions first                                  |
| `supervisor_lost` (state)   | a socket that stopped answering with no terminal event in the log       | open a new session                                                 |
| `harness_already_installed` | `tool.install` when detection succeeds                                  | nothing to install                                                 |
| `tool_busy`                 | a tool job is already running                                           | wait for it to finish                                              |
| `install_failed`            | the installer exited non-zero                                           | read the log tail; check the network; try again                    |
| `terminal_launch_failed`    | engine: neither `wt.exe` nor a console could be opened                  | paste the attach command into any terminal                         |

Edge cases: the daemon dies mid-`create` after spawning — the supervisor
keeps running and the next start adopts it through the scan; a readiness
timeout after the pipe named a pid — the daemon sends `SIGTERM` to that
pid; `wsl --shutdown` kills every supervisor without a terminal event —
the next start marks them `supervisor_lost`; two `create`s for one
project — two sessions, distinct ids and sockets; a session whose project
was removed keeping the workspace — listed under "removed project" until
it ends. Timestamps are epoch-second strings, like jobs.

## Testing

**Host (`cargo test`)**: the wire codec in both directions and its
decoder across split reads; `parse_version`; the `launch` allowlist;
`Session` transitions from event lines (a pure function); the
alternate-screen tracker; the ring; `wt`/fallback command composition
and Windows identity parsing in the engine; `state.ts` and
`lib/sessions.ts` (Vitest).

**Distribution (`just test-linux`)**: `crates/willie-sess/tests/` spawns
the real binary against a fake harness (`sh -c` scripts: exit 7; ignore
`SIGINT` so `SIGTERM`/`SIGKILL` run; print `ESC[?1049h`; write without
pause for `too_slow`) and proves readiness EOF and errno paths, replay
and its suppression, two clients, `too_slow`, the stop ladder with
`WILLIE_SESS_STOP_GRACE_MS`, and `SIGTERM` as a clean shutdown (final
event present, socket unlinked). `willied`'s Linux-gated tests prove
`session.create` end to end with `WILLIE_HARNESS_BIN` at the fake
harness, the identity resolution order, re-adoption (create, drop the
`Ops`, build a new one over the same state dir — the session is
`running`; kill the supervisor — `supervisor_lost`), the `remove`
refusal, and `tool.install` with `WILLIE_HARNESS_INSTALLER`.

**Acceptance**: `docs/checklists/sessions-acceptance.md`, one action per
row with the exact command or observation.

## Rollout / compatibility

- `docs/PROTOCOL.md` gains `session.*`, `tool.*`, `session_changed`, the
  codes above, and `Job.project_id` marked optional. The app is the only
  client; nothing is released yet, so no compatibility shim.
- `docs/ARCHITECTURE.md`: `stop_requested` joins §3.1; §3.2 loses
  `attach_command` and notes the in-memory index; §5.5's sessions row is
  marked delivered when it is.
- Decision 0014 — one framed socket per session, the daemon as a control
  client (evolves the raw-downstream shape of 0012). Decision 0015 — git
  identity resolution and the stopgap's removal.
- A line in `releases/v0.1.0.md`.
- Environment overrides, documented in `docs/TESTING.md`:
  `WILLIE_HARNESS_BIN`, `WILLIE_HARNESS_INSTALLER`,
  `WILLIE_SESS_STOP_GRACE_MS`, `WILLIE_SESS_BIN`, `WILLIE_HOME` and
  `WILLIE_RUN_DIR`.

## Open questions

- Retention of finished sessions: keep everything (favoured — two small
  files each) or prune after N days once SQLite indexes them.
- A Windows Terminal profile fragment (icon, name, a plain Willie shell):
  favoured no, until a user asks.

## Plans

- **Plan A — Linux core**: `willie-core` `Session`; `willie-proto`
  types; `wire`; `willie-sess`; `willie attach`; `willie-harness`;
  `willied` (`session.*`, control, scan, identity, `remove` guard,
  `tool.install`); PROTOCOL rows; decisions 0014 and 0015.
- **Plan B — Windows and app**: engine `session_*`/`tool_install`,
  terminal launch and fallback, Windows identity; Tauri commands;
  `proto.ts`/`state.ts`; project row; Sessions screen; Dashboard
  Install; ARCHITECTURE updates; acceptance checklist; release note.

## Follow-ups for the next slices

- The sandbox wraps the same `argv`/`env`/cwd — `willie-sess` gains the
  wrapper, nothing upstream changes.
- Resume, once the harness's JSONL files are read.
- An embedded terminal is one more `terminal` client of the socket.
- SQLite for session history and pruning.
