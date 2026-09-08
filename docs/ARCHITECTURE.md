# Architecture

Willie is two Rust programs joined by a pipe.

- **`willie-app`** (Windows, Tauri 2 + React): the UI, the tray icon and
  the **engine** — the only component that knows Windows exists.
- **`willied`** (Linux, inside the WSL distribution `willie`): the source
  of truth — projects, sessions, sandbox, plugins and the SQLite index.

The engine starts the daemon with
`wsl.exe -d willie --user willie --exec /opt/willie/bin/willied --stdio`
and speaks JSON-RPC over the process pipes. Each agent session lives in a
**detached supervisor** (`willie-sess`) that owns the PTY and the sandbox
and outlives the app; Windows Terminal today, and an embedded terminal
later, are only **clients** of that PTY. All state lives in ext4; nothing
third-party ships in the package.

Decision records in `docs/decisions/` explain the *why*; this document is
the *what*.

## 1. Topology

```
┌─────────────────────────── Windows ────────────────────────────┐
│  willie-app (Tauri 2)                                          │
│  ├─ WebView2 · React UI  ◄── Tauri events ───┐                 │
│  ├─ tray                                     │                 │
│  └─ willie-engine ── JSON-RPC (ndjson) ──────┼──► stdin/stdout │
│       wsl.exe wrapper · proxy/CA · WT profile · supervision    │
│       state: %LOCALAPPDATA%\Willie\data\engine.toml            │
└────────────────────────────┬───────────────────────────────────┘
                             │  wsl.exe -d willie --user willie --exec willied --stdio
┌────────────────────────────▼──────────── distro "willie" ──────┐
│  willied (uid 1000, unprivileged)                              │
│  ├─ stdio  ◄─ engine                                           │
│  ├─ /run/willie/willied.sock  ◄─ willie (CLI)                  │
│  ├─ SQLite /var/lib/willie/willie.db (index)                   │
│  └─ plugins: profiles, usage                                   │
│                                                                │
│  willie-sess <id>  (one per session, detached)                 │
│  ├─ PTY master ── ring buffer ── /run/willie/sessions/<id>.sock│
│  ├─ events.jsonl (append-only, written without the daemon)     │
│  └─ bwrap + Landlock + seccomp ──► harness (claude)            │
│                                                                │
│  willie attach <id>  ◄── Windows Terminal via wsl.exe          │
└────────────────────────────────────────────────────────────────┘
```

### 1.1 Windows side — `willie-app` + `willie-engine`

| Area              | Responsibility                                                                                               |
| ----------------- | ------------------------------------------------------------------------------------------------------------ |
| Prerequisites     | `wsl.exe --version` / `--status`; requires WSL ≥ 2.4.4 with version 2 default; Windows Terminal recommended. |
| Provisioning      | `wsl --import willie %LOCALAPPDATA%\Willie\data\distro <rootfs.tar.gz>`; `--unregister`; `--export` for backups; binary and image updates (§2.4). |
| Daemon supervision| spawn with `CREATE_NO_WINDOW`; `hello`; liveness probe on every status read and one restart on a failed call (F0); periodic ping and backoff later. |
| Privileged steps  | one-shot `wsl.exe -d willie --user root --exec …` (base packages, `update-ca-certificates`, `/opt/willie`). The daemon never runs as root. |
| Windows facilities| proxy/PAC detection (WinHTTP), certificate export (`Root` and `CA` stores), Windows Terminal profile fragment, `wt.exe` launch, notifications, tray. |
| UI bridge         | translates webview actions into RPC; forwards daemon notifications as Tauri events; keeps the UI store fed by `state.snapshot` + `state.events`. |
| State             | `engine.toml` only: installed image version, machine profile (proxy/CA), UI preferences. No projects, no sessions, no database. |

### 1.2 Linux side — inside the distribution

| Process            | User            | Role                                                                                                   |
| ------------------ | --------------- | ------------------------------------------------------------------------------------------------------ |
| `willied`          | `willie` (1000) | Daemon. Owns projects, profiles, session index, plugins, SQLite. Listens on **stdio** (engine); **`/run/willie/willied.sock`** (`0600`) is served for local clients — request and reply only, no `state.event` notifications, no `daemon.shutdown`. |
| `willie-sess <id>` | `willie`        | Detached supervisor of one session: PTY, sandbox, `/run/willie/sessions/<id>.sock`, `events.jsonl`, ring buffer. Lives as long as the harness, independent of daemon and app. |
| `willie`           | `willie`        | Stateless CLI: `attach <id>`, `doctor`, `sandbox explain <project>`, `reindex`, `dev test`.            |
| harness            | `willie` in a user namespace | The agent CLI, child of the supervisor, inside the sandbox (§3.3).                        |

State in ext4 (never under `/mnt/*`):

| Zone         | Path                                                                                           | Written by                  | Survives                     |
| ------------ | ---------------------------------------------------------------------------------------------- | --------------------------- | ---------------------------- |
| System       | base packages · `/opt/willie/bin/{willied,willie,willie-sess}` · `/etc/wsl.conf` · `/etc/wsl-distribution.conf` · `/etc/willie/` | Willie only (via engine) | nothing — replaced on update |
| Willie data  | `/var/lib/willie/` → `willie.db`, `sessions/<id>/{spec.json,events.jsonl}`, `projects/<id>.toml`, `profiles/<name>/` (git), `plugins/<id>/` | `willied`, `willie-sess` | binary and image updates |
| User data    | `/home/willie/` → `.willie/agent-state/<harness>/`, `projects/<slug>/`, managed tools (`~/.local`, `~/.dotnet`, …), caches | user, tools, harness (via sandbox) | idem              |
| Ephemeral    | `/run/willie/` (sockets), `/tmp`                                                               | processes                   | distribution restart         |

**Agent-state path convention.** On the host the harness's persistent
state lives in `/home/willie/.willie/agent-state/claude/dot-claude/`
(directory) and `…/claude.json` (file). Inside the sandbox they appear as
`$HOME/.claude` and `$HOME/.claude.json` (capability `agent.state`). The
image also symlinks `~/.claude` → that directory so running the CLI from a
plain shell in the distribution uses the same login. Throughout this
document `~/.claude/…` means the path *as the harness sees it*; daemon and
plugins resolve host paths through `Harness::state_paths()`.

### 1.3 Workspace

```
crates/
  willie-core        domain: ids (ULID), Project, Session, CapabilitySet,
                     config — ZERO I/O
  willie-linux       Linux-side helpers shared by willied, willie-sess
                     and willie-cli (well-known paths, doctor checks,
                     the sandbox plan and argument vector as data)
  willie-proto       JSON-RPC messages (serde), protocol version,
                     snapshot/events
  willie-engine      Windows: wsl.exe wrapper, provisioning, proxy/CA,
                     WT, supervision
  willied            Linux daemon: RPC server, projects, session index,
                     plugin host, storage
  willie-sess        Linux supervisor: PTY, sandbox
                     (bwrap/Landlock/seccomp), socket, events
  willie-cli         Linux CLI: attach, doctor, sandbox explain,
                     reindex, dev test
  willie-harness     Harness trait + capability matrix + ClaudeCode
  willie-plugin-api  Plugin trait, PluginCtx, manifest
  willie-plugins/{profiles,usage}
apps/willie-app/     Tauri 2 (Rust) + React/TS/Vite; UI plugins in
                     src/plugins/<id>/
xtask/               dev tasks (cargo xtask …)
distro/              reproducible rootfs recipe, wsl*.conf, oobe, sha256
```

Dependency rules: `core` and `proto` depend on nothing internal; `engine`
and `willied` never see each other (only `proto`); `willie-sess` depends
on `core` + `harness`, not on the daemon; plugins depend on `plugin-api`
+ `core` + `harness`, never on `willied`; `willie-linux` depends on
`proto`, `core` and `harness`, never on the daemon or the supervisor;
the UI knows only `proto` (TS mirrors of the types).

## 2. The distribution

### 2.1 Base image

Debian stable *slim* (glibc), about 70 MB compressed (F0 measured
70 164 480 bytes), built reproducibly from `distro/` and versioned with
the app (embedded sha256).

System content: `ca-certificates`, `git`, `curl`, `bubblewrap`, `sudo`,
`procps`, `iproute2`, `less`; `/opt/willie/bin/*` as **static musl**
binaries; users `root` (no password hash) and `willie` (uid 1000, home
`/home/willie`, passwordless `sudo` like stock WSL distributions — the
*sandbox*, not the OS, denies privilege to the agent).

`/etc/wsl-distribution.conf`:

```ini
[oobe]
command = /opt/willie/libexec/oobe.sh   # non-interactive: validate uid 1000, create /run/willie, exit 0
defaultUid = 1000
defaultName = willie
[shortcut]
enabled = false
[windowsterminal]
enabled = false                          # the engine manages the profile
```

`/etc/wsl.conf`:

```ini
[boot]
systemd = false
# /run is a fresh tmpfs; the unprivileged daemon needs its socket dir
command = install -d -o willie -g willie -m 0750 /run/willie
[automount]
enabled = true
options = "metadata"
[interop]
enabled = true                 # the USER may call .exe from a shell; the SANDBOX denies it
appendWindowsPath = false      # no Windows PATH in Linux: no shell collisions, no slow 9P lookups
[network]
generateResolvConf = true
[user]
default = willie
[gpu]
enabled = false
[time]
useWindowsTimezone = true
```

No `/etc/resolv.conf`, no kernel or initramfs in the tar.

### 2.2 Managed tools never go through `apt`

The agent CLI (native installer → `~/.local/bin`), .NET (`~/.dotnet`),
Node (a version manager in the home) install **into the home**. The system
zone stays pure and data migration is a `tar` of two directories. The
daemon keeps a manifest, `/var/lib/willie/tools.toml`, for display and
reinstall after migration: one TOML table per tool id, `{ version,
installed_at, installer }`, written after `tool.install`/`tool.update`
runs its installer and re-detects the tool, and read fresh on every
`tool.list`; an absent or unreadable file loads empty, so a lost manifest
degrades to a re-detect, never an error. The manifest is not the truth
the Tools screen shows — live detection is, because a user can update a
tool outside Willie (decision 0022).

### 2.3 Lifecycle

The distribution starts implicitly when the engine spawns
`willied --stdio`. App open ⇒ daemon alive ⇒ distribution alive. App
closed ⇒ daemon exits; with no supervisors left, the VM shuts down after
WSL's idle timeout — zero cost while Willie is closed. Sessions open in
Windows Terminal keep their supervisors (and the distribution) alive; on
return the daemon re-adopts them (§3.2). No systemd; the only
`[boot] command` is the one-shot `mkdir` of §2.1.
Willie **never writes the user's `.wslconfig`** (it is global).

### 2.4 Two update rhythms

| Rhythm                         | What changes                              | How                                                                                                                      |
| ------------------------------ | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Willie binaries (frequent)     | `/opt/willie/bin/*`, `/opt/willie/libexec/*` | **Not implemented in the app.** Designed as: engine streams a tar to `wsl.exe --user root --exec tar -x …` into a temp dir, atomic rename, daemon restart; running supervisors keep the old binary until they exit. Today only the development recipe `just distro-push` does this, staging each binary beside its target and renaming over it as root. |
| Base image (rare)              | packages, `/etc`                          | wizard: `wsl --export` backup → `tar` of `/var/lib/willie` and `/home/willie` to Windows → `--unregister` → `--import` → restore both zones → reinstall managed tools from the manifest → `doctor`. |
| Factory reset                  | everything                                | `--unregister` + `--import`, double confirmation, backup offered.                                                        |

Reinstalling validates the new image before unregistering the old
distribution; a data-preserving upgrade (export/restore of the two
data zones) is a later slice, so today a reinstall replaces the
distribution wholesale.

**Nothing detects a stale distribution.** The engine compares the
version the daemon reports at `hello` against its own, and both are the
workspace's crate version, which does not move between releases — so a
distribution whose binaries are many commits behind answers the check
and passes. `/etc/willie/image-version` carries the commit and would
tell them apart, but no check reads it. Until one does, a build being
tested is only as current as the last `distro-push` or `distro-install`,
and an acceptance walk must say so.

Never `wsl --mount` (administrator) and never a second distribution.

### 2.5 Corporate networks

- **Proxy.** WSL's `autoProxy` (on by default) injects `HTTP(S)_PROXY` or
  a PAC URL. The engine additionally reads the Windows configuration
  (WinHTTP + Internet Settings, PAC included), resolves a **static** proxy
  for the hosts that matter (`WinHttpGetProxyForUrl`), and writes
  `/etc/willie/machine.env` (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`) plus
  `~/.gitconfig` (`http.proxy`) and `~/.npmrc` when those tools exist.
- **TLS-inspection CA.** The engine exports the `Root` and `CA` stores
  (current user + local machine) as PEM →
  `/usr/local/share/ca-certificates/willie/*.crt` →
  `update-ca-certificates` (as root) → `SSL_CERT_FILE`,
  `NODE_EXTRA_CA_CERTS`, `GIT_SSL_CAINFO` in `machine.env`. Idempotent
  and diff-based on every engine start.
- **Consumption.** `machine.env` is loaded by the daemon and reaches
  sessions through the sandbox's environment allowlist (§3.3).
- **Validation.** `doctor` runs `curl -sI https://api.anthropic.com` from
  inside the distribution and reports `[ok]`/`[FAIL]` with a named
  remediation.
- On a personal machine none of this exists: empty detection ⇒ empty
  `machine.env` ⇒ nothing to configure.

## 3. Sessions, supervisor and sandbox

### 3.1 Model

`Session { id: ULID (sess_…), project_id, harness: "claude-code" (or
"zsh" for a shell session), kind: agent | shell, capabilities:
CapabilitySet, args, created_at, state, label?, title? }`.
States: `creating → running → exited(code) | failed(reason)`, with a
transient `stopping`. Truth: `/var/lib/willie/sessions/<id>/spec.json`
(immutable after creation) and `events.jsonl` (append-only: `created`,
`sandbox_applied{mechanisms,unavailable}`, `started{pid}`,
`renamed{label}`, `attached{client}`, `detached`, `resized{cols,rows}`,
`sandbox_denied{class,name,count}`, `sandbox_degraded{mechanism,message}`,
`stop_requested{by}`, `exited{code,signal}`, `failed{reason}`). SQLite is
a rebuildable index (`willie reindex`).

### 3.2 Flows

**Start.**
1. UI → engine's `session_open(project_id)`: the engine reads the
   Windows git identity (`git.exe config --global user.name`/
   `user.email`, `CREATE_NO_WINDOW`; no `git.exe` ⇒ none sent), then
   calls RPC `session.create { project_id, git_identity? }`. The
   `harness`, `capabilities_override?` and `args?` fields arrive with
   the sandbox and resume slices; they are not on the wire yet.
2. `willied` resolves configuration layers (§3.4), validates
   **fail-closed**, writes `spec.json`, spawns `willie-sess <id>`
   **detached** (setsid; stdio to `sessions/<id>/supervisor.log`).
3. `willie-sess` opens the PTY, builds the sandbox, runs the harness on
   the PTY slave, listens on `/run/willie/sessions/<id>.sock` (0600) and
   records `started`. The harness's output crosses one filter on its way
   out: a VT/ANSI byte parser between the PTY and the attach fan-out,
   before the ring buffer, drops the escape sequences that would act on
   whoever is attached — writing the host clipboard, changing Willie's
   tab or window title, or echoing attacker-controlled text back into the
   input — and records each as `sandbox_denied { class: "terminal" }`
   (decision 0020); everything that draws passes byte-for-byte, and
   because the filter is before the ring a late attach replays already
   filtered bytes. The supervisor runs the harness by re-executing
   itself as `willie-sess --inner` inside the namespace, which applies
   the resource limits, applies Landlock, installs the syscall filter
   and reports which mechanisms took effect before the harness's `exec`
   (decisions 0017, 0018, 0019); the supervisor records that report as
   `sandbox_applied`, refuses the session if it names less than the
   required subset, and answers the filter's notifications for the
   session's life. The
   daemon rides the session socket as a
   control client (decision 0014) — the supervisor never calls the
   daemon and never depends on it.
4. `willied` replies `{ session }` once the supervisor reports ready;
   the engine opens a Windows Terminal tab running `wsl.exe -d willie
   --user willie --exec /opt/willie/bin/willie attach <id>` — there is
   no `attach_command` on the wire. `wt.exe` is looked up on `PATH`,
   then under `%LOCALAPPDATA%\Microsoft\WindowsApps`; when neither
   exists, the same `wsl.exe …` line runs in a new console instead. A
   tab that fails to open does not undo the session: `session_open`
   returns it as a non-fatal `terminal_problem` (`terminal_launch_failed`,
   remediation the paste-able attach line) alongside the running
   session. `session_attach(id, title)` opens one more tab for an
   already-running session the same way, except a failure there comes
   back as a plain error — there is no session outcome left to protect.
5. `willie attach` connects to the session socket, receives the ring
   buffer, puts its tty in raw mode, relays bytes both ways and
   `SIGWINCH` → `resize`. Several attaches may coexist; all read-write.

**Stop.** `session.stop` → daemon → supervisor: `SIGINT` (5 s) →
`SIGTERM` (5 s) to the harness process, resolved through the helper's
reaper, then `SIGKILL` to the process group (decision 0016); `exited`
recorded; the supervisor exits when the last client detaches.

**Daemon restart.** It scans the session directories under
`/var/lib/willie/sessions/`, reading each one's spec and event log, and
adopts the supervisors whose sockets answer `status` — pid and client
count come from that reply — while finalising the rest from the
event log alone, marking a still-live-looking session `failed
{ supervisor_lost }` when even the log has no terminal event. The session
index lives in memory, rebuilt this way on every start; SQLite indexing
is still deferred.

**Resume.** `session.create { resume: true }` builds the harness launch
as `LaunchMode::Continue` — the same binary, `--continue` appended, still
in the project's workspace — instead of `LaunchMode::Fresh`; the new
session records which finished session it continues in `resumed_from`.
It reuses this same create path end to end, including the open/terminal
flow above, so a resumed session attaches, stops and streams to the
embedded terminal like any other. `resume_from` optionally names which
finished session to continue; absent, the daemon targets the project's
most recent terminal session (continue-latest, the original behaviour,
unchanged). Fail-closed: `harness_cannot_resume` when the target
harness's `Resume` capability is `None` (always true of a `Shell`
session, which has no conversation to continue); `resume_target_not_found`
when a named target does not exist; `resume_target_live` when it is
still running or stopping — it already has a tab, so continuing it
elsewhere would double-drive the same transcript.

**Several live sessions.** A project may now have more than one live
session at once: a fresh `session.create` no longer looks at the
project's other sessions at all, so `session_already_live` is retired —
kept in `docs/PROTOCOL.md`'s code table only so a client that matched on
it knows why it stopped appearing. This is safe because each session
runs in its own private sandbox home while the shared `agent.state` bind
(§3.3) carries the login and the harness's own per-session logs — one
file per conversation — so two conversations in one workspace never
collide; the usage plugin already matches a session to its log by
workspace and time window (decision 0025).

**Shell sessions.** `SessionKind` (`willie_core::session`) distinguishes
an agent conversation from an interactive shell, `#[serde(default)]` so
a spec written before this existed re-adopts as `Agent`. A `Shell`
session runs the same project checks, the same resolved
`CapabilitySet`, the same supervisor and the same sandbox as an agent
one, but its launch is built by the daemon (`willied::shell`) rather
than by a harness: `/usr/bin/zsh -l` in the project's workspace,
`ZDOTDIR=/etc/willie/zsh` pointing at Willie's own prompt
(`distro/zsh/.zshrc`: history in `$HOME/.zsh_history`, a two-line prompt
naming the branch) and the same environment allowlist a harness gets.
Its `harness` field reads `"zsh"`, never a registry harness id; it
cannot resume (`harness_cannot_resume`) and is excluded from the usage
plugin's session enrichment (§4.3) — it keeps no harness-readable log to
match. `willie-sess`'s `prepare()`
(`crates/willie-sess/src/sandbox/mod.rs`) branches on `SessionKind` only
to pick whose rules govern the one bind the plan actually consults for
a shell, the agent's own state directory: a shell is not an installable
harness and names none in `willie_harness::registry()`, so it borrows
`ClaudeCode`'s for that lookup alone. `/etc/willie/zsh` joins the rest
of `/etc` as a read-only, tolerated-absent bind (`ETC_OPTIONAL` in
`willie-linux::sandbox::bwrap`, `--ro-bind-try`), so an image built
before this feature — with no such directory — still starts every
other session normally; on such an image a shell create is refused
fail-closed with `shell_unavailable`, checked
against `/usr/bin/zsh` itself, before anything is spawned. Because
`$HOME` is the same private tmpfs every session gets by default (§3.3,
`home.persistent` off), a shell's own history does not survive past its
session — a product follow-up, not an oversight.

**Names.** A session gets a display name from two independent sources.
`title` is its first prompt, read lazily and best-effort: `willied`'s
`session_title` module (`crates/willied/src/session_title.rs`) looks for
the newest `*.jsonl` under the matching harness's session-logs directory
modified at or after the session started, reads its first user-turn
record and trims the text to 80 characters; a title that cannot be
found stays absent rather than guessed, and the UI shows the session's
short id instead. The read is deliberately lazy — from `session.list`
and from the state-snapshot path, rather than once at `Started` — since
the harness usually has not written its log yet that early; it is
bounded to the first 64 KiB of one file per untitled session per call,
and never depends on a plugin being enabled. `label` is what the user
typed: `session.rename { id, label }` appends a `Renamed { label }`
event to the session's own log before updating the in-memory record and
emitting `session_changed`, so a crash between the two never loses it;
an empty or all-whitespace label clears it, one over 120 characters is
refused. The UI shows `label ?? title ?? short id` everywhere a session
is named.

**App closed.** Daemon exits; supervisors and terminal tabs continue; on
reopen the restart flow restores supervision.

**App (engine + UI).** `session_attach` opens one more terminal for an
existing session — no daemon call involved; `session_stop(id)` and
`tool_install(harness)` are the thin engine wrappers around
`session.stop`/`tool.install`. The app never talks to a session socket
itself. Like every other change, `session_changed` reaches the webview
over `daemon://event`, the same stream as `project_changed` and
`job_changed` — in the same spirit as decision 0014 (the daemon rides
the session socket as a control client), the app in turn only ever
talks to the daemon over its own RPC pipe. The system-scoped **Session
screen** (§5.1) replaced the old Sessions screen: a tab per live agent
session and per live shell, `+` to start either, and a **Sessions
panel** (a Sheet) listing every live session with *Open* (focuses its
tab) and recently finished ones with *Resume* (`resume_from` names the
exact one, adding a new live tab). The old gating that required *no*
live session before a project's row offered Resume is gone along with
the one-live-session rule itself (see Several live sessions above) —
Resume and a fresh session now coexist freely; stopping a session still
asks the daemon with no confirmation. Double-clicking a tab's name
turns it into an inline field that calls `session.rename` on Enter, Esc
cancels, blur commits. The Setup → Engine screen (the former Dashboard)
keeps the harness doctor check's **Install** button, showing the
install job's last log line while it runs. A session's terminal is a
Windows Terminal tab Willie composes and hands off by default
(`session_open`/`session_resume`, unchanged), and every live tab on the
Session screen also renders inside the Willie window itself — no longer
an opt-in "Open in app" action on a Sessions row, since every Session-
screen tab is the embedded terminal: the engine spawns
`willie attach <id> --host` and bridges its stdio to an `xterm.js`
terminal (`crates/willie-engine/src/embed.rs`). The engine keeps **one
bridge per session**, keyed by session id
(`embed::Bridges<Embedded>`), because the screen mounts every live tab
at once: opening a second session never detaches the first, re-opening
one replaces only its own child, a tab's unmount closes just that
bridge, and the engine closes all of them when it drops so quitting
leaves no `wsl.exe` child behind. Input or resize for a session with no
bridge is refused as `embedded_terminal_not_open` rather than answered
`Ok` — a dead tab says so instead of swallowing keystrokes. The bridge
writes encoded `input`/`resize` frames to the child's stdin (a small
`willie-proto::hostterm` dialect) and reads raw session output from its
stdout, tagged with the session id so each tab renders only its own;
`attach --host` re-frames the input for the wire protocol the same way
the tty-mode client does. This embedded path reuses the same
session socket as the Windows-Terminal-tab path — consistent with
decision 0014. Shipping it also fixed `terminal::locate_wt`, which used
to *execute* `wt.exe --version` to detect Windows Terminal and flashed a
stray window on every open; it now resolves `wt.exe` by file presence
only, on `PATH` and under `%LOCALAPPDATA%\Microsoft\WindowsApps`, and
never runs it.

### 3.3 Sandbox

Applied by `willie-sess`: **bubblewrap** (namespaces and mounts) +
**seccomp-bpf** (the program built as data and installed by the
re-executed `willie-sess --inner` inside the namespace, with a
user-notification listener the supervisor answers) + **Landlock**
(applied by the same stage, after the limits and before the filter) +
**rlimits**.

**Base — always on, not configurable:**
- `--unshare-user --unshare-pid --unshare-ipc --unshare-uts`,
  `--disable-userns` (no nested user namespace: the filter refuses
  `unshare`/`setns`, this closes the `clone` flags — decision 0018),
  `--die-with-parent` (the supervisor); `TIOCSTI` blocked by seccomp
  instead of `--new-session`;
- `no_new_privs`; system paths (`/usr`, `/lib*`, `/bin`, `/sbin`, selected
  `/etc` files) read-only;
- `$HOME=/home/willie` as **tmpfs**; private `/tmp`; fresh `/proc`;
  minimal `/dev`;
- **no `/init`, no `/run/WSL`, no `WSL_INTEROP`/`WSL_DISTRO_NAME`** ⇒ no
  Windows executable runs. Measured (0016): the binfmt entry carries the
  *fix binary* flag, so the kernel holds the interpreter open and an
  absent `/init` does not stop it — what stops it is the interop socket
  directory missing from the namespace. `/run` is refused as an
  `extra.paths` entry for exactly this reason;
- `/mnt/*` **not mounted** except the project path and `extra.paths`;
- `sudo` masked; `/var/lib/willie` and `/run/willie` not mounted;
- the harness binary (`argv[0]`) bound **ro** at its own path whatever
  the policy says — a session that cannot start is no session;
- environment **allowlist**, applied by the vector itself (`--clearenv`
  then one `--setenv` per variable), not by whoever spawns it: `PATH`,
  `HOME`, `USER`, `TERM`, `COLORTERM`, `LANG`, `LC_*`, `TZ`, plus the
  `machine.env` variables;
- seccomp denies `ptrace`, `process_vm_*`, `pidfd_getfd`, `bpf`,
  `io_uring_*`, `perf_event_open`, `userfaultfd`, `seccomp` itself (a
  nested filter's listener would pre-empt the supervisor's), the mount
  family, `unshare`/`setns`, module loading, `kexec_*`, the key
  management calls, `ioctl(TIOCSTI)`, packet sockets (by family or by
  the obsolete `SOCK_PACKET` type), raw sockets and every netlink
  protocol but route; the filter is installed by the in-namespace stage
  with a user-notification listener, and the supervisor answers each
  intercepted call `EPERM` and records it as `sandbox_denied`, coalesced
  per syscall (decision 0018);
- rlimits `NPROC`, `NOFILE`, `CORE=0`, applied by the re-executed
  supervisor inside the session's own user namespace;
- **Landlock** (ABI ≥ 2): read and execute under `/`, write only at the
  plan's read-write mounts plus `/tmp` and `/dev`, so a mount the plan
  never granted read-write is read-only whatever bound it; ABI 1 or none
  is reported `unavailable` and the session runs on the mounts
  (decision 0019);
- **network on** (no `--unshare-net`): the harness needs it; Landlock at
  this kernel version has no network rules; fine-grained egress is a
  growth item.

**Named capabilities** (positive names; each carries one consequence
sentence shown in the UI):

| Capability         | Default          | Effect                                                                                                 |
| ------------------ | ---------------- | ------------------------------------------------------------------------------------------------------ |
| `project.rw`       | always           | bind **rw** of the project directory **at the same path** (`/mnt/c/...` or ext4)                      |
| `agent.state`      | on (Claude Code) | bind rw of the harness's state directory under `~/.willie/agent-state/`, plus the `~/.claude` and `~/.claude.json` links into it; without it no login |
| `tools.ro`         | on               | binds **ro** of the managed tool roots (`~/.local`, `~/.dotnet`; a Node manager when F4 adds one) — the agent cannot alter them |
| `caches.rw`        | on               | binds rw a **per-project** directory (`~/.willie/caches/<project_id>/…`) over `~/.npm`, `~/.nuget`, `~/.cache` |
| `git.identity`     | on               | `~/.gitconfig` ro                                                                                      |
| `home.persistent`  | off              | `$HOME` = `~/.willie/homes/<project_id>` instead of tmpfs                                              |
| `extra.paths`      | empty            | additional `ro`/`rw` binds declared in the profile                                                     |
| `ssh`              | off              | `~/.ssh` ro + agent socket                                                                             |
| `mnt.all`          | off ⚠            | mounts all of `/mnt/*`                                                                                 |
| `windows.interop`  | off ⚠⚠           | mounts the interop socket directory (`/run/WSL`) and keeps `WSL_INTEROP`, which names the socket in it — equivalent to no sandbox towards Windows. The interpreter itself is never the question: the kernel holds it open through its binfmt entry (0016) |

`willie sandbox explain <project>` prints the resolved capabilities.
The mounts and the exact bubblewrap argument vector are built **as
data** by `willie-linux::sandbox` (`plan`, then `bwrap::argv`), so the
supervisor that applies them and the daemon that explains them share
one builder and its tests run on any host; the supervisor launches
every session through them and records `sandbox_applied { mechanisms,
unavailable }`, measured by the in-namespace stage in the event log
before `started`; what the syscall filter refuses follows as
`sandbox_denied { class, name, count }`, and a filter no longer served
as `sandbox_degraded { mechanism, message }`. The Landlock rules are
derived from the same plan (decision 0019); the filter summary and the
path rules in `sandbox explain` are a named follow-up of the
enforcement slice.

### 3.4 Configuration layers (increasing authority, monotonic)

1. Willie defaults (per harness: `Harness::default_capabilities()`).
2. Project profile — the `sandbox` table of the project's own record at
   `/var/lib/willie/projects/<id>.toml`; may enable or disable any
   capability this version implements. Edited from the Projects
   screen: a project row's `⋯` menu → *Sandbox…* opens a dialog listing
   every capability with the sentence that says what enabling costs,
   saved through `project.set_sandbox`, which resolves the profile
   before persisting it — the same two refusals `session.create` gives.
   A `[sandbox]` table that cannot be parsed (an unknown key, a wrong
   type) no longer removes the project from the daemon's state: it
   loads with the default profile and a problem the Projects screen
   shows, and `session.create`/`sandbox.explain` refuse until the
   profile is replaced through `project.set_sandbox`.
3. `.willie/sandbox.toml` **inside the repository** — may only
   **tighten**: remove capabilities, add denied paths. It never opens
   anything because the agent can write it. Masked inside the sandbox.
   **Not yet implemented**: no code reads or writes this file, and the
   sandbox dialog has no control for it.

Invalid configuration (unknown key, wrong type, an attempt to open at
layer 3) ⇒ the session does not start, with an actionable error.

### 3.5 Harness trait and capability matrix

```rust
trait Harness {
    fn id(&self) -> &'static str;                         // "claude-code"
    fn capabilities(&self) -> HarnessCapabilities;         // data record
    fn detect(&self, env: &UserEnv) -> Option<Installed>;  // version + path
    fn command(&self, spec: &SessionSpec) -> Command;      // argv, env, cwd
    fn default_capabilities(&self) -> CapabilitySet;
    fn state_paths(&self) -> Vec<StatePath>;               // ~/.claude, ~/.claude.json
    fn settings_paths(&self) -> SettingsPaths;             // user + project (profiles plugin)
    fn resume_args(&self, harness_session_id: &str) -> Vec<String>;
}
```

`HarnessCapabilities { interactive_tui, resume: None|ById|Continue,
headless_stream, hooks, settings_format, mcp_config_format }` is **data**,
validated by a mini-bench (`willie dev bench-harness`) against the
installed binary. No consumer branches on the harness id.

A `usage_sources` method was sketched here when this trait was first
designed, for a usage plugin that would read both a provider's OAuth
credentials and a JSONL glob through one call. It was never built: the
usage plugin's as-built read path (§4.3) needs only two methods the trait
already carries for other reasons — `session_logs_dir(home)`, where a
harness keeps its session transcripts, and `escape_workspace`, how it
names a workspace's own log directory — so no usage-specific trait member
exists. A method for source 1 (the OAuth-backed usage endpoint, still
deferred) would be added to the trait when that source is built, not
before.

Known particulars of `ClaudeCode`: `DISABLE_AUTOUPDATER=1` in the session
environment (the binary is read-only under `tools.ro`; updates belong to
the managed-tools feature); the project is mounted at the same path so the
CLI's project slug matches what the user sees outside Willie;
`resume_args(id)` → `["--resume", id]`; headless mode →
`["-p", "--output-format", "stream-json"]` (reserved for later).

## 4. Plugins, persistence and protocol

### 4.1 Plugin contract (internal)

```rust
trait Plugin {
    fn manifest(&self) -> PluginManifest;   // id, scope Global|PerProject, config schema, UI panels
    fn on_enable(&mut self, ctx: &PluginCtx, scope: Scope) -> Result<()>;
    fn on_disable(&mut self, ctx: &PluginCtx, scope: Scope) -> Result<()>;
    fn handle(&mut self, ctx: &PluginCtx, req: PluginRequest) -> Result<PluginResponse>; // "<id>.<method>"
    fn on_event(&mut self, ctx: &PluginCtx, ev: &CoreEvent);   // session started/exited, project registered, tick
}
```

**Built for the first cut** (the plugin host, decision 0023): the trait
above matches what ships, `on_enable`/`on_disable`/`on_event` defaulting
to no-ops so a plugin with nothing to do there (`profile`, and the
`usage` stub) implements only `manifest` and `handle`. `PluginCtx` today
carries only what this cut needs — `store_dir()`, a private tree of
plain files under `/var/lib/willie/plugins/<id>/` (not a SQLite
`plugin_kv` table; see §4.4), and `emit()`, which the host currently
swallows rather than forwarding (`plugin.emitted` is reserved on the
wire but not yet sent, see §4.5). The harness registry, a scheduler and
an HTTP client already configured with proxy/CA are documented seams
`usage` (F5) grows next, deliberately not built here.

Rules: plugins never touch daemon internals; never run inside a session;
a plugin error ⇒ that plugin is `degraded`, the daemon continues — except
a plugin's own coded refusal (e.g. `profile_exists`), which is a
legitimate "no" and leaves it healthy. In the UI each plugin is a
statically registered React module under `apps/willie-app/src/plugins/<id>/`
that only uses core RPC/events. **Not plugins:** sessions, sandbox,
managed tools, WT profile, corporate network.

### 4.2 `profiles`

**Built for the first cut** (decisions 0023, 0024): a profile is a
**git-versioned** directory at `/var/lib/willie/plugins/profile/<name>/`
— the profiles plugin's own store under §4.1's per-plugin tree, not a
bespoke top-level `profiles/` directory — holding `profile.toml`
(metadata: `settings`/`instructions`/`mcp` booleans plus `rules`/`hooks`
as lists of active file names, not flat per-family booleans) and
fragments `settings.json`, `CLAUDE.md`, `rules/*`, `hooks/*`, `mcp.json`.
`profile.check` plans without writing; `profile.apply` writes into the
project's workspace (`.claude/settings.json`, `CLAUDE.md`,
`.claude/rules/`, `.claude/hooks/`) and, only when a `settings` fragment
opts in (`settings_scope = "global"` in `profile.toml`), also into the
harness state (`~/.claude/settings.json`) that every project's sessions
read — a **format-preserving merge** for the JSON and Markdown
fragments (key order kept; Markdown only between `<!-- willie:begin -->
… <!-- willie:end -->`) and a plain file copy for `rules`/`hooks`;
differential backup of everything about to change into
`<workspace>/.willie-bak/<nanosecond timestamp>/`. Toggling a fragment on
is `profile.write_fragment` then a re-apply; turning one off is done by
editing `profile.toml` directly (no Phase-1 method for it).
`profile.set_remote`/`push`/`pull` carry a profile between the user's two
machines over the plugin's own `git` wrapper (never `willied`'s) — `git
push -u origin HEAD`, `git pull --ff-only` refusing a divergent history
as `profile_sync_conflict` rather than attempting a merge inside the
profile's own tracked files. **Not built:** MCP server token-cost
estimation (the design's `tools/list`-over-stdio, characters/4 estimate)
— the `mcp` fragment merges its JSON into `mcpServers` verbatim, nothing
sizes what it costs a session's context budget — and any UI to resolve a
sync conflict beyond surfacing `profile_sync_conflict`'s own remediation
(open a terminal inside the distribution).

### 4.3 `usage`

**Partially delivered.** Of the design's three sources, two are built:
2. the harness's own session JSONL → tokens per session and a context
   percentage; 3. matching a Willie session to that log by workspace and
   time window. Source 1 — a provider's OAuth-backed usage endpoint
   (credits, limit windows) — is **not built**, and neither is the tray or
   any notification; see "Not built" below.

`usage.snapshot` (the `usage` plugin, global scope) is recomputed on
every call, never scheduled: the daemon's `usage.*` route
(`willied::handlers::usage_handle`/`enrich_usage_targets`) fills the
plugin's params with every session it already knows — `_sessions: [{ id,
project_id, workspace, window }]`, a still-live session's window left
open — and `_home`, the distro home directory, the same daemon-fills-
targets seam `profile.*` uses (decision 0024) so the plugin stays
daemon-ignorant. For each session the plugin resolves the first
registry harness that keeps logs under `_home`
(`Harness::session_logs_dir`), turns the session's `workspace` into that
harness's log directory name (`Harness::escape_workspace`), lists its
`*.jsonl` files, and picks the one whose modified time falls inside the
session's window — this *is* sources 2 and 3 together: the daemon's own
session index already carries the workspace and time window a separate
`events.jsonl` match would otherwise have to recover. A bounded tail (64
KiB) of the picked file is read; the newest line carrying a usage block
(top-level `usage`, or `message.usage` for an assistant turn; a
`isSidechain: true` record skipped) sums `input + cache_creation_input +
cache_read_input + output` into that session's token count, and a small
model-prefix table turns the input-side fields into `context_pct` —
`None` rather than a guessed denominator when the model is unrecognised.
Every step degrades to "no usage data" instead of a fault: no matching
harness, no log directory, an empty listing, no file overlapping the
window, or a line that fails to parse are all the same zero-token outcome,
never a panic and never a call failure. Storage is the harness's own
files plus this in-memory projection, recomputed fresh each call; no
SQLite index and no scheduler back it. `on_event` emits the plugin's own
`usage.updated` on every `SessionStarted`/`SessionExited` (§4.1), but the
host does not yet forward plugin emissions to clients (see `plugin.*`
there and `docs/PROTOCOL.md`'s `usage.*` section) — the Usage panel polls
`usage.snapshot` instead of reacting to a push. `providers` is present
and always empty this cut, keeping source 1 additive whenever it lands.

**Not built:** source 1 (a provider's OAuth credentials in the harness
state → its usage endpoint — a fragile source needing its own cache,
staleness flag and backoff, deliberately kept out of this cut so
`usage.snapshot` never makes a network call); the tray icon showing a
percentage; a configurable-threshold notification; and forwarding
`usage.updated` to clients in real time (poll-only this cut). See
decision 0025.

### 4.4 Persistence

- **Daemon (planned):** SQLite `/var/lib/willie/willie.db` (WAL, ext4),
  **one writer actor** + read pool, versioned migrations. Tables
  `projects`, `sessions` (index), `tools`, `profiles_applied`,
  `plugin_kv`, `schema_version`. **Still deferred** (§3.2): today the
  daemon has no SQLite at all. Project and tool records are TOML files
  under `/var/lib/willie/`, the session index lives in memory and is
  rebuilt from each session's own directory on every start, and a
  plugin's persistence — enablement (`plugins/enabled.toml`) and its
  private store (`plugins/<id>/`, profiles' own profiles among them,
  §4.1–4.2) — is plain files, not a `plugin_kv` table. `willie reindex`
  and the SQLite tables above land together, in a later slice.
- **Files are the truth** where they must survive without the daemon:
  `sessions/<id>/events.jsonl` (supervisor) and profiles in git — true
  today exactly as planned, ahead of SQLite existing at all.
- **Engine:** `engine.toml` only.

### 4.5 Protocol (`willie-proto`)

- **Framing** ndjson; **JSON-RPC 2.0** requests/responses plus
  daemon→client **notifications**.
- `daemon.hello { client, willie_version, protocol_version }` →
  `{ willie_version, protocol_version, distro_image_version }`. Engine and
  daemon must be the **same Willie version**; on mismatch the engine
  applies the binary fast path (§2.4) and reconnects.
- **Snapshot + stream:** `state.snapshot` (projects, sessions, tools,
  plugins, health, pending decisions) and `state.event { seq, kind,
  payload }` with a monotonic sequence; reconnection = new snapshot (no
  replay buffer). Pending human decisions are part of the snapshot.
- The **same** protocol runs on stdio (engine), on the Unix socket (CLI)
  and, later, on TCP loopback.
- Namespaces: `daemon.*` (hello, health, doctor, shutdown) · `project.*`
  (list, add, remove, sync_to_windows, update_from_windows, relocate,
  rename, set_sandbox, tree, read_file) · `session.*` (create, stop,
  list, rename) · `sandbox.*` (explain) · `tool.*` (list, install,
  update) · `profile.*` (list, create, read_fragment, write_fragment,
  check, apply, set_remote, push, pull) · `usage.*` (snapshot) ·
  `plugin.*` (list, enable, disable).
- Errors: `{ code, message, remediation }` — actionable message, named
  remediation.

## 5. UI, build, installer and tests

### 5.1 UI (Tauri 2 + React/TS/Vite)

The window is frameless (`"decorations": false`, `"shadow": true` —
Windows 11 still draws the rounded corners and drop shadow of a
decorated window): a 36 px header replaces the native title bar,
composed entirely by Willie (`app/shell/header.tsx`). Left to right: a
sidebar-toggle button (also `Ctrl+B`), a settings button that opens the
setup drawer, a centre `div` marked `data-tauri-drag-region` showing
"Willie · &lt;system&gt;", and three window controls
(`getCurrentWindow().minimize()`/`.toggleMaximize()`/`.close()`, the
`core:window:allow-*` capabilities) reflecting `isMaximized()` on mount
and on every resize.

**The sidebar is the work context, not a menu of destinations.** On
top, a system selector (glyph, name, `workspace · branch` once the tree
has loaded the branch, a live dot when any of the system's sessions is
live) opens a searchable list of every system plus "Add system…";
beside it, a "…" menu holds one system's actions — Open in VS Code
(WSL), Open in Explorer, Update from Windows, Rename, Relocate, Remove.
Below that, every system carries the same four screens, `Ctrl+1`–
`Ctrl+4` (inside the embedded terminal only these reach the shell;
`Ctrl+B` stays with the session):

| Screen   | Contents                                                                                            |
| -------- | ----------------------------------------------------------------------------------------------------- |
| Session  | one tab per live agent session and per live shell, `+` to start either, a Sessions panel to resume a finished session or focus a live tab; the workspace tree as a drawer with a read-only file preview beside it |
| Sandbox  | the system's posture aggregated over its sessions, three counts, the chronological denial history filterable by session; capabilities are edited in a drawer |
| Profiles | the existing profiles panel (§4.2), scoped to the current system                                   |
| Usage    | the existing usage panel (§4.3), scoped to the current system                                       |

The workspace tree and file preview read through one seam:
`project.tree`/`project.read_file` resolve every path via
`workspace::resolve_within` (`crates/willied/src/workspace.rs`) — the
single containment check a workspace-relative path passes through
anywhere in the daemon, canonicalising both the workspace and the
target so a symlink cannot walk out of it — refusing an outside path
(`path_outside_workspace`) and a non-regular file (`file_not_text`; see
`docs/PROTOCOL.md`). A file's preview is read-only and capped at 512
KiB; editing is VS Code's job, one click away.

**Everything machine-wide lives behind the header's settings button**,
a drawer (`app/shell/setup-drawer.tsx`) listing six entries that each
navigate to a `/setup/*` route and keep their existing content: Engine
(today's health view, formerly Dashboard), Tools, Plugins (no longer
mounting the Profiles/Usage panels — those are the system screens
above), Profile store, Systems (the project registry: add, discover,
roots — its per-row session and sandbox actions moved to the sidebar's
"…" menu and the system screens), and Settings (theme). The
pre-redesign paths (`/dashboard`, `/projects`, `/sessions`, `/tools`,
`/plugins`) all redirect to their new homes, so a saved location keeps
working. The tray keeps its existing icon and menu (open, new session
in a recent project, mute alerts, quit), unaffected by this redesign.

The selected system is a UI preference, not daemon state: `engine.toml`
carries an optional `[ui] current_project`
(`crates/willie-engine/src/config.rs`), read and written through two
engine methods (`ui_prefs`/`set_ui_prefs`); a preference naming a
project that no longer exists falls back to the first project, and the
preference is rewritten. Every system screen reads the current system
from this preference, never from a route parameter, so switching
systems keeps the screen in place.

A thin footer keeps engine health, the first problem, the daemon
version and the live-session count, and — while a session tab is
focused on the Session screen — a governance segment: that session's
sandbox posture, denied count and a context meter, linking to the
Sandbox screen filtered to it. A shell session carries no sandbox
report worth narrating this way (it is not a harness conversation), so
the segment is hidden for one; on the Sandbox screen itself the segment
instead shows the system's own aggregate posture and denied count.

**Not persisted:** the theme choice — `store/use-theme.ts` is a
module-level singleton that resets to "system" on every launch — a
named follow-up, not an oversight. **Not scoped to one system yet:**
the Profiles screen's daemon-side gate (`plugin_disabled` until *any*
project has enabled the plugin, not specifically the one showing) — the
screen renders that as "Profiles are off for this system" plus an
Enable button, a frontend-only accommodation; the daemon-side semantics
change (scoping the gate to the shown system) is a named follow-up, so
`docs/PROTOCOL.md`'s wording about a disabled plugin refusing every call
stays true today.

Closing the window minimises to the tray; *quit* stops the daemon
(sessions in Windows Terminal continue). Daemon truth flows through one
store fed only by `state.snapshot` + `state.events`; a second,
read-only store mirrors `engine_status` + `engine://status`. The UI
**never computes truth**.

The frontend source (`apps/willie-app/src/`) gives every kind of file
one home: `app/` composes the shell — the route registry, the router,
the sidebar and status bar, the theme effect and the shortcuts;
`features/<domain>/` holds one directory per screen; `components/`
holds the design system's own pieces (`tone.ts`, `StatusDot`,
`StatusBadge`, `FailureChip`, `ProblemAlert`) over the generated
primitives in `components/ui/`; `store/` binds the two stores (daemon
snapshot, engine status) to React; `lib/` has no React — the wire-type
mirrors, the Tauri bridge, the pure domain modules and both stores'
state machines; `plugins/<id>/` holds the plugin panels. Imports only
point down: `app → features → components`, `app` and `features` may
read `store`, everything may read `lib`, and a feature never imports
another feature — a dialog more than one feature reuses lives in
`components/` instead; `lib` returns facts, never a tone. The linter
fails `just check` on a violation. Tokens live in `styles/globals.css`
and nowhere else; the theme follows the system preference through
`.dark` on the root by default, or a pinned light/dark chosen on the
Settings screen. Rationale: `designs/frontend-foundations.md`,
`designs/frontend-visual-system.md` and `designs/system-scoped-shell.md`.

### 5.2 Build and development (all from Windows)

Rust MSVC via `rustup`, Node 22 or newer + pnpm (the pnpm workspace sits
at the repository root), Tauri CLI as a dev dependency, WebView2 (native
on Windows 11). `just setup` prepares a fresh clone: hooks,
`pnpm install`, `cargo fetch`, then `just ensure` as the verification. `willied`, `willie`, `willie-sess` are
**cross-compiled** to `x86_64-unknown-linux-musl` with `cargo-zigbuild`
(static; no Linux toolchain on the machine). Unit tests of portable crates
run natively on Windows; Linux integration tests run **inside the Willie
distribution** (`willie dev test` copies the binaries and runs them).
Image built from `distro/` with an embedded sha256. Quality gate:
`just check` (see `docs/TESTING.md`); no remote CI.

### 5.3 Installer and updates

Tauri bundler, **NSIS per-user** → `%LOCALAPPDATA%\Willie` (no
administrator; a per-user Tauri install cannot choose `Programs\`).
Engine state lives inside it at `%LOCALAPPDATA%\Willie\data\`
(`engine.toml`, `distro\`, `logs\`), a subdirectory the uninstaller
never removes: its `RMDir` of the install root is non-recursive, and
its optional app-data purge targets the bundle-identifier directory,
not this one. The `willie-rootfs.tar.gz` ships as a bundle resource.
First run = wizard (prerequisites → `--import` → `doctor`). App updates
through the Tauri updater (own signing key); the engine then compares
`hello` and applies the binary fast path. Image updates (rare) use the
wizard of §2.4. Code signing is out of scope for now.

### 5.4 Spikes — before the slice that depends on them

| Spike | Before | Question                                                                                                   | Exit criterion                                                        |
| ----- | ------ | ---------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| S1    | F0     | stdio through `wsl.exe`: latency; UTF-16LE of `wsl.exe`'s own messages vs raw bytes of the Linux process; `CREATE_NO_WINDOW`; does the daemon die with its parent? | steady-state round trip < 50 ms (measured ≈0.5 ms); the first request after a spawn is budgeted in seconds (measured 0.2–0.9 s); no console window; a `--exec` child dies with its Windows parent — supervisors detach and confirm it |
| S2    | F1     | PTY + supervisor + attach in Windows Terminal: resize, 24-bit colour, keys, faithful TUI; supervisor survives the daemon | measured (0012): Claude Code session in a WT tab through `willie attach`, redrawn on reattach; keystroke echo 165 µs median; the session survives its launcher and `willied` — F1 adds a control channel |
| S3    | F3     | sandbox on the real kernel: `landlock` in `/sys/kernel/security/lsm`; Landlock ABI; `unshare -U`; `.exe` denied inside; CLI logs in with `agent.state` + tmpfs home | measured (0016): Landlock ABI 3, probed through the syscall because securityfs is not mounted; unprivileged user namespaces, seccomp and its user notification all present; a Windows executable refused because `/run/WSL` is outside the namespace, not because `/init` is; the CLI logged in and answered a prompt with `agent.state` and a tmpfs home — F3 ships the required subset |
| S4    | F2     | corporate network: what does `autoProxy` inject (PAC or static)? does `curl` to the API work with the imported CA? | exact list of variables/files to propagate |
| S5    | F1     | `/mnt/c` performance: `git status`, `rg`, CLI startup on a real repository                                 | measured (0011): warm `git status` 511 ms on DrvFs vs 4.9 ms on ext4 (~100×), `rg` 19×, traversal 27× — F1 ships the ext4 workspace |

### 5.5 Slices ↔ components

| Slice | Delivers                                                        | Components                                                                                           | Spike |
| ----- | --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----- |
| F0    | distribution registered; UI shows health; `doctor` — **delivered 2026-08-26** | `distro/`, engine (import, supervision), `willied` (hello/health/doctor), `willie doctor`, Dashboard, NSIS | S1 |
| F1    | projects: register a Windows checkout, ext4 workspace synced through git (0011, 0013) | `project.*`, `job.*`, `state.*`, engine project methods and `engine.toml` roots, Projects screen, `just test-linux` | S5 |
| F1    | sessions: Claude Code session in a WT tab, no sandbox — **delivered 2026-08-28** | `session.*`, `willie-sess` (PTY, socket, events — sandbox off), `willie attach`, engine (`session_open`/`session_attach`/`session_stop`/`tool_install`), Sessions screen, Projects-row Open session + badge, Dashboard Install | S2 |
| F2    | proxy/CA propagated                                             | engine (WinHTTP, cert stores), `machine.env`, network `doctor`                                      | S4    |
| F3    | sandbox with capabilities and layers                            | `willie-sess` (bwrap/seccomp/Landlock, `--inner`), `willie-core` (CapabilitySet, layers), `sandbox explain`, capability UI | S3 |
| F4    | managed tools: the harness, install and update — **delivered 2026-09-07** | `tool.*`, manifest, `Harness::detect`, Tools screen                                                  | —     |
| F5    | usage plugin + tray                                             | `willie-plugin-api`, `plugins/usage`, tray/notifications, Usage panel                               | —     |
| F6    | profiles plugin                                                 | `plugins/profiles` (git, merge), Profiles panel                                                      | —     |

F5/F6 order is the user's choice; F2 moves up if the first real use is on
the corporate machine. F4's first cut is the harness only (Claude Code);
.NET and a Node version manager (§2.2) remain future catalogue entries,
added through the `ManagedTool` extension point decision 0022 names.

## 6. Architectural risks

| Risk                                                             | Mitigation                                                                                                         |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `wsl.exe` mixes its own UTF-16LE messages with raw process bytes | Measured and handled (0010): the child's bytes arrive as raw UTF-8, `wsl.exe`'s own messages as UTF-16LE, and `text::decode_wsl_output` decodes them without replacement characters. The protocol is UTF-8 ndjson and ignores non-JSON lines; `wsl.exe` errors are read from the exit code plus decoded stderr, or `wsl.exe`'s own stdout message when stderr is empty. |
| Daemon dies ⇒ engine misses events until reconnection            | Snapshot on reconnect replaces replay; supervisors log events on their own.                                        |
| Landlock has no network rules at this kernel version             | Network is an always-on capability for now; egress by nftables/uid is a growth item; the UI says "the agent has network". |
| Orphaned supervisors after a supervisor crash                    | `willied` removes dead sockets and marks the session `failed`; `spec.json` + `events.jsonl` keep the history.      |
| Concurrent writers on multiple attaches                          | Accepted (multiplexer semantics); the UI shows how many clients are attached.                                       |
| `wsl --import` refused by corporate policy                       | Confirmed once on the corporate machine; cause and remediation known (0010): the virtual-machine account lacked the "Log on as a service" right, so VM creation failed with HCS `0x80070569`. `doctor` reports that code with its remediation; there is no administrator-free alternative. |
| Undocumented usage endpoints change                              | `stale` + cache + JSONL fallback; never a visible failure; plugin isolated.                                          |
| Windows Terminal absent or fragment format changes               | The JSON fragment is the official mechanism; fallback is `wsl.exe` in a plain console window.                       |
| Scope creep during implementation                                | Slices are closed and tested one at a time; §5.5 is the contract.                                                   |
