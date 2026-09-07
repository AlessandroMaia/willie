# Sandbox follow-ups — the daemon's local socket, the unreadable profile, and three small fixes

The sandbox's two halves (`designs/sandbox.md`, decisions 0016–0020) left
a short list of things the reviews named and no phase owned. This design
closes the ones that change what a person can do or see; the rest stay
listed under Non-goals with the moment that will pick them up.

## Problem

### `willie sandbox explain` has nowhere to send its request

`docs/ARCHITECTURE.md` promises `/run/willie/willied.sock` for local
clients in three places and nothing serves it: `willied` answers only the
engine's stdio pipe. The CLI's `fetch_explain` therefore fails closed with
`daemon_unreachable` on every invocation, and the contract's own words are
"no release builds it yet". The parsing and rendering are done; the
transport is the missing piece.

### An unreadable `[sandbox]` erases the project

`SandboxProfile` is `deny_unknown_fields`, as it must be: a key this
version does not know must not be silently ignored when it may name a
capability. But the profile is a table inside the project's record, and
`store::load_all` parses the record whole, so one mistyped key in a
hand-edited `[sandbox]` fails the whole file. The daemon prints
`skipping unreadable project` to stderr and the project vanishes from the
interface, with its workspace and its sessions' history still on disk and
no way to reach them from the app.

### Three things the reviews named

- **`prctl(PR_SET_SECCOMP)` stays allowed** (phase 2's residual). It cannot
  bypass the filter — a filter installed through `prctl` has no listener,
  so it can only tighten — but a shadowing filter answering `ENOSYS` first
  would mute the denial line the supervisor would otherwise log.
  Observability only; a small deny.
- **The refused-path remediation repeats the reason.** The message says
  "`/mnt/c` cannot be an extra path: a whole Windows drive is `mnt.all`" and
  the remediation says "this version will not grant it: a whole Windows
  drive is `mnt.all`". The walk noted the same sentence read twice.
- **Two step strings are literals.** `crates/willie-sess/src/sandbox/mod.rs`
  spells "cannot enter the workspace" and "cannot execute the harness" while
  `pty.rs` exports them as `WORKSPACE_STEP` and `EXEC_STEP`; `main.rs`
  matches on the constants, so the two pairs can drift apart.

## Goals

- `willie sandbox explain <id|slug>` answers from inside the distribution
  while the app is open, through the daemon's local socket, with the same
  reply the app would get.
- The socket serves every method the stdio transport serves except the
  daemon's own shutdown, and only to the distro user; a session can never
  reach it.
- A project whose `[sandbox]` table cannot be read still loads, shows why
  on its row, cannot open a session until it is fixed, and is fixed from
  the Sandbox dialog without editing a file. What the person wrote survives
  until they choose to replace it.
- A session's attempt to install its own filter through `prctl` is refused
  and recorded like every other denial.
- A refused extra path reads once as the reason and once as what to do.
- The supervisor's step strings have one spelling.

## Non-goals

- **`willie reindex`.** ARCHITECTURE names it beside `sandbox explain`, but
  the SQLite index it would rebuild does not exist yet; the socket is what
  it will need, and the command arrives with the index.
- **Notifications over the socket.** `state.event` is the engine's view;
  a CLI is request/reply. A socket client that wants state calls
  `state.snapshot`.
- **`daemon.shutdown` over the socket.** The daemon's lifecycle is the
  engine's, which supervises it; a CLI must not end it from under the app.
- **Wiring `sandbox.explain` into the app.** The dialog already receives,
  per row, the harness default and whether the capability is implemented,
  which is every fact `Source` would carry. A round trip would add nothing;
  a per-row provenance label, if wanted, is a line of TypeScript for the
  visual pass.
- **Validating the profile's meaning at load.** A profile that parses but
  cannot resolve (a deferred capability enabled, a guarded extra path)
  loads as it does today and is refused by `resolve` at `session.create`,
  `sandbox.explain` and `project.set_sandbox`, each with its own code.
  Only the parse failure is new here.
- **The supervisor's start-up path**: folding its nine refusal blocks,
  `prepare` checking the base vector's sources, the signal window in the
  spawn helper, the two guard tests. All named in the part-1 ledger; they
  belong to whichever change next touches that path.

## Design

### The socket path — `crates/willie-linux/src/paths.rs`

```rust
/// `<run_dir>/willied.sock`: the daemon's local socket.
pub fn daemon_socket(run_dir: &Path) -> PathBuf
```

Beside `session_socket`. `run_dir` is `WILLIE_RUN_DIR` when set, else
`RUN_DIR` (`/run/willie`), so the daemon, the CLI and the tests agree the
way they already do for session sockets.

### The daemon's inbound loop — `crates/willied/src/server.rs`, `main.rs`

Today `Server::serve(reader)` reads stdin lines in place and sends every
reply through `Outbound`, the single writer that owns stdout. It becomes
a consumer of one channel with two kinds of origin:

```rust
enum Reply { Stdio, Socket(mpsc::Sender<Response>) }
enum Inbound { Line { text: String, reply: Reply }, StdinClosed }
```

- A **stdin reader thread** sends each line as `Line { reply: Stdio }`
  and `StdinClosed` at EOF or read error.
- An **accept thread** owns the `UnixListener` and spawns one thread per
  connection. A **connection thread** reads a line, sends
  `Line { reply: Socket(tx) }`, blocks on `rx` for the response, writes
  it, and reads the next line: one request in flight per connection. EOF
  from the client ends the thread; a write failure ends it too, and the
  dispatched effect stands, as it does today when the stdio pipe breaks.
- The **dispatcher** — `serve` — parses the line (so `invalid_request`,
  id 0, has one code path whatever the origin), dispatches, and routes:
  `Stdio` → `Outbound::send_response`, `Socket(tx)` → `tx.send`, ignored
  if the connection has gone. Messages are handled in arrival order across
  origins. The daemon keeps its invariant of one request at a time; the
  engine's ordering is unchanged because everything was serialised on the
  stdin loop already.

```
engine ──stdin──▶ reader thread ─┐
                                 ├─▶ Inbound ─▶ serve: parse, dispatch ─▶ Outbound ─▶ stdout
willie ──sock───▶ conn thread ───┘                     └─▶ tx ─▶ conn thread ─▶ sock
```

**Method policy.** The dispatch table is the same. `daemon.shutdown`
arriving with `Reply::Socket` answers the error `method_not_served`,
message "`daemon.shutdown` is the engine's; the daemon stops with the app",
remediation "restart the daemon from the Dashboard". With `Reply::Stdio` it
behaves as today. `daemon.hello` first is the client's duty on both
transports; the daemon does not enforce order on either, as today.

**Bind.** `main.rs` binds after `session_ops.scan()` and before serving,
mirroring the supervisor's `server::bind`: refuse to steal a live socket
(`connect` succeeds → `AddrInUse`, "another daemon is listening"), remove
a dead one, bind, `0600`. A bind failure ends the daemon with the error on
stderr before it serves anything — fail closed. The run directory is the
doctor's `run dir` check with its `chown` remediation, and a live socket
means a second daemon, which must not exist. The engine reports the exit
as `daemon_exited` with that stderr.

**Shutdown.** `serve` returns on `StdinClosed` or a stdio
`daemon.shutdown`, as today. `main` then trips the jobs, joins the writer,
removes the socket file, and returns; the process exit ends the accept and
connection threads, whose clients read EOF. No thread is joined that would
block on `accept`.

**Reach.** `/run/willie` is `0750 willie:willie`, the socket `0600`, and
the sandbox never mounts `/run/willie` (decision 0016, the base vector).
A session cannot name the socket, let alone connect to it. Nothing in the
daemon relies on who is calling, because nothing else can.

**Platform.** `UnixListener` and the accept path are
`#[cfg(target_os = "linux")]`; the `Inbound` loop itself is portable and
unit-tested on the host with fake origins.

### The CLI client — `crates/willie-cli/src/main.rs`

`fetch_explain(project)` stops refusing and becomes:

1. `connect(daemon_socket(run_dir))`, read timeout 10 s (the protocol's
   call budget after `hello`). Connection refused or no file →
   `daemon_unreachable`, remediation "open the Willie app; the daemon runs
   while it is open".
2. `daemon.hello` with `Hello::for_client("willie")`. A daemon error
   passes through as it is (`protocol_version_mismatch` and its own
   remediation).
3. If the argument does not start with `proj_`, `project.list` and match
   `slug`; none → `project_not_found`, remediation "the slug is the
   workspace's directory name under ~/projects; the id is on the project's
   record".
4. `sandbox.explain { project_id }`; the reply is rendered exactly as
   today.

A read timeout is `daemon_timeout`; any other I/O failure is
`daemon_transport`. Ids count up from 1 per connection. The client is a
private `mod daemon` in the CLI: `connect`, `call(method, params)`, nothing
else. Exit codes follow the contract's table: 1 for every failure above.

### The unreadable profile — `crates/willie-core/src/project.rs`, `crates/willied/src/store.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProblem { pub code: String, pub message: String, pub remediation: String }

pub struct Project {
    …
    #[serde(default)]
    pub sandbox: SandboxProfile,
    /// Set by the daemon when the record's `[sandbox]` table could not be
    /// read; recomputed on every load and never trusted from disk, like
    /// `source_present`. `sandbox` is the default while it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_problem: Option<SandboxProblem>,
}
```

The same inline `code / message / remediation` shape every failed state
already carries, so the app renders it with what it has.

**Load** (`store::load_all`) parses in two stages. First the file as a
`toml::Table`; a failure here is the record being unreadable, and it is
skipped as today. Then the `sandbox` value is taken out of the table and
tried as `SandboxProfile`:

- it parses → the project loads as today, `sandbox_problem: None`;
- it does not → the rest of the table becomes the `Project` with the
  default profile and `sandbox_problem = Some { code:
  "sandbox_profile_invalid", message: "the [sandbox] table could not be
  read: <toml error>", remediation: "open Sandbox… for this project and
  save it to replace the table, or fix the file" }`.

Whatever the file held in `sandbox_problem` is overwritten by this
computation; the field is written to disk only because the record and the
wire share one type, and it is never read back as truth.

**Refusals.** `session.create` (in `SessionOps::create`, before
`session_capabilities`) and `handlers::sandbox_explain` (before `resolve`)
return the stored problem as the `OpError` when it is set. Fail closed:
the default profile is in memory, and nothing runs on it while the person
has not seen the problem.

**Saves.** `project.set_sandbox` resolves the new profile, sets
`sandbox_problem = None`, saves — the table is replaced. Every other save
of a project whose `sandbox_problem` is set — `rename`, `relocate`, the
`preparing → failed` transition at start-up, a job's completion — goes through
`store::save`, which reads the current file's `sandbox` table and puts it
back into the document it is about to write. The hand-written table is
lost only when its owner replaces it. When the current file is missing or
unparseable there is nothing to preserve and the record is written as it
stands.

**One code corrected on the way.** `set_sandbox` reports a failed write as
`git_failed`, which it is not; it becomes `state_write_failed`, "the
project's record could not be written", with the existing remediation.

### The app — `apps/willie-app/src/features/projects/*`, `lib/proto.ts`

- `lib/proto.ts`: `Project.sandbox_problem?: Problem` (the `Problem` shape
  `lib/ipc.ts` already has).
- `components/failure-chip.tsx` gains `tone?: "error" | "warning"`
  (default `error`), coloured through `TONE_SURFACE`; it stays the one
  chip for a stacked code / message / remediation.
- `project-state-chip.tsx`: a ready project with `sandbox_problem` renders
  the chip with `tone="warning"`, after the failed-state branch and before
  the job branches. A failed project keeps showing its failure.
- `project-row.tsx`: Open session and Resume are disabled while
  `sandbox_problem` is set, with the problem's message as the button
  `title`; Send to Windows and Update from Windows stay enabled — the
  workspace is fine, the policy is not.
- `sandbox-dialog.tsx`: when the project carries `sandbox_problem`, a
  `ProblemAlert` at the top of the content says the saved table could not
  be read and that Save replaces it; the controls start from `{}`, that is
  the harness defaults, which is also what the dialog would show for a
  project that never set anything. The save path is unchanged, so the
  existing `problem` prop (a refused save) keeps its place below.

### The `prctl` deny — `crates/willie-linux/src/sandbox/seccomp.rs`

`prctl` joins the argument-judged block after `socket`:

```rust
asm.jump(JEQ_K, SYS_prctl, To::Prctl, To::Next);
…
asm.mark(To::Prctl);
asm.stmt(LD_W_ABS, DATA_ARG0);
asm.jump(JEQ_K, PR_SET_SECCOMP, To::Notify, To::Allow);
```

`SYS_prctl = 157`, `PR_SET_SECCOMP = 22`; `name(157)` answers `"prctl"` so
the denial log names it. Every other `prctl` passes: the harness and its
children set names, death signals and no-new-privs through it.
`docs/decisions/0018` and `designs/sandbox-seccomp.md` stop listing the
residual; the acceptance checklist gains one row.

### The remediation — `crates/willie-core/src/sandbox.rs`

`CapabilityError::ExtraPathGuarded`'s message keeps "`<path>` cannot be an
extra path: <reason>"; its `remediation()` becomes "name a narrower path
outside it, or remove the entry". The test
`each_flavour_of_guard_answers_with_the_reason_a_user_reads` asserts the
reason in the message and asserts the remediation does not contain it.

### The step constants — `crates/willie-sess/src/sandbox/mod.rs`

The two `step:` literals become `pty::WORKSPACE_STEP` and `pty::EXEC_STEP`.
No behaviour change; the existing assertion on the text stays green.

### Errors and edge cases

| Condition | Behaviour | Code |
| --- | --- | --- |
| socket bind fails (dir missing, not writable, another daemon live) | the daemon exits before serving, reason on stderr; the engine shows `daemon_exited` | — |
| `daemon.shutdown` over the socket | refused; the connection stays open | `method_not_served` |
| unknown method over the socket | as on stdio | `method_not_found` |
| malformed line over the socket | error with id 0, the connection goes on | `invalid_request` |
| a client connects and sends nothing | one parked thread until it disconnects; nothing else waits on it | — |
| a client disconnects while its request is dispatched | the effect stands, the reply is dropped | — |
| CLI: no daemon | `daemon_unreachable`, "open the Willie app" | exit 1 |
| CLI: no reply in 10 s | `daemon_timeout` | exit 1 |
| CLI: slug matches no project | `project_not_found` | exit 1 |
| `[sandbox]` does not parse | the project loads with the problem; Open session and Resume disabled; the chip says why | `sandbox_profile_invalid` |
| `session.create` / `sandbox.explain` on such a project | refused with the stored problem | `sandbox_profile_invalid` |
| the whole record does not parse | skipped with the stderr line, as today | — |
| a save of a project with the problem, other than `set_sandbox` | the file's `[sandbox]` table is preserved | — |
| `set_sandbox` cannot write the record | refused | `state_write_failed` |
| a session calls `prctl(PR_SET_SECCOMP, …)` | `EPERM`, one `sandbox_denied { class: "syscall", name: "prctl" }` | — |

When not to use `method_not_served`: an unknown method is
`method_not_found`; a method refused by its own precondition keeps its own
code. When not to use `sandbox_profile_invalid` for the load problem: a
profile that parses but names a deferred capability is
`sandbox_capability_unsupported`, from `resolve`, as before.

## Testing

**Host (`cargo test`, part of `just check`):**

- `server.rs`: the `Inbound` loop with fake origins — a stdio line is
  answered through `Outbound`; a socket line is answered through its
  sender; `daemon.shutdown` from a socket origin is `method_not_served`
  and the loop goes on; from stdio it still returns `Shutdown`;
  `StdinClosed` returns `Eof`; a dropped reply sender is not an error.
- `store.rs`: a record with an unknown `[sandbox]` key loads with
  `sandbox_problem` set and the default profile; a valid table loads
  without; a save of that project through `save` preserves the table
  byte-for-byte as TOML; `set_sandbox`'s save replaces it; a record whose
  body is invalid is still skipped.
- `projects.rs` / `sessions.rs` / `handlers.rs`: `session.create` and
  `sandbox.explain` on a project with the problem answer
  `sandbox_profile_invalid` with the stored message; `set_sandbox` clears
  it and returns the project without it.
- `seccomp.rs`: the BPF interpreter — `prctl(PR_SET_SECCOMP)` →
  `USER_NOTIF`, `prctl(PR_SET_NAME)` → `ALLOW`; the libc-gated test adds
  `prctl` to the encodings it checks.
- `sandbox.rs` (core): the remediation no longer contains the reason.
- `willie-sess`: the existing text assertion.
- Frontend (vitest): the row chip appears with the warning tone; Open
  session and Resume are disabled while it is set and the sync buttons are
  not; the dialog shows the alert and starts from `{}`; a project without
  the field renders as before.

**Distribution (`just test-linux`):**

- `willied/tests/socket.rs`: the real daemon with a temporary
  `WILLIE_RUN_DIR`: connect, `hello` + `daemon.health` answered;
  `daemon.shutdown` → `method_not_served`; a second connection is served
  while the first is open; closing stdin ends the daemon and the socket
  file is gone.
- `willie-cli/tests/cli.rs`: a fake daemon on a temporary socket answering
  `hello`, `project.list` and `sandbox.explain`: `sandbox explain <slug>`
  prints the rows, `--json` the reply verbatim; with no socket,
  `daemon_unreachable` on stderr, exit 1, empty stdout.

**Manual:** the acceptance checklist gains a "Follow-ups" section: row 19
(`!perl -e 'syscall(157, 22, 2, 0) or die $!'` → `Operation not permitted`;
the event log names `prctl`), row 20 (`wsl -d willie --user willie --
/opt/willie/bin/willie sandbox explain <slug>` with the app open prints the
capability rows; with the app closed, `daemon_unreachable`), and row 21
(edit a project's record to add `nonsense = true` under `[sandbox]`,
restart the daemon: the row shows the warning chip, Open session is
disabled, Sandbox… shows the alert, Save clears it and the file's table is
replaced).

## Rollout / compatibility

- `PROTOCOL_VERSION` unchanged: `sandbox_problem` is additive and
  defaulted; `method_not_served` and `state_write_failed` are new codes in
  the tables; a "Transports" section states what the socket serves and does
  not.
- `docs/CLI_CONTRACT.md` drops the "no release builds it yet" paragraph and
  lists the CLI's three transport codes.
- `docs/ARCHITECTURE.md`: the two "not served yet" phrases go; the project
  record section names the deferred `[sandbox]` parse; §3.3's list of what
  `sandbox explain` prints is unchanged.
- Decision **0021 — the daemon's local socket: one dispatcher, request and
  reply only, no shutdown**. Options: a second dispatcher per transport, or
  a shared server behind a lock, against the single inbound channel; why
  notifications and shutdown stay with the engine.
- Release note: `willie sandbox explain` works from inside the
  distribution while the app is open; a project whose sandbox settings
  cannot be read is shown with the reason instead of disappearing, and is
  repaired from the Sandbox dialog; a session can no longer install a
  syscall filter of its own through `prctl`.
- Older records: a project written by any earlier version has a valid or
  absent `[sandbox]` table and loads exactly as before.

## Open questions

None. The connection count is unbounded and idle connections park a thread
each; both are the local user's own CLI processes, and a cap would need a
code and a story for what the refused caller does. Revisit if a socket
client that is not a person appears.
