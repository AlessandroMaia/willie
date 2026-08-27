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
| `willied`          | `willie` (1000) | Daemon. Owns projects, profiles, session index, plugins, SQLite. Listens on **stdio** (engine) and **`/run/willie/willied.sock`** (local clients). |
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
                     and willie-cli (well-known paths, doctor checks)
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
`proto` only; the UI knows only `proto` (TS mirrors of the types).

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
daemon keeps a manifest (`tools`) for detection, display and reinstall
after migration.

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
| Willie binaries (frequent)     | `/opt/willie/bin/*`, `/opt/willie/libexec/*` | engine streams a tar to `wsl.exe --user root --exec tar -x …` into a temp dir, atomic rename, daemon restart; running supervisors keep the old binary until they exit. |
| Base image (rare)              | packages, `/etc`                          | wizard: `wsl --export` backup → `tar` of `/var/lib/willie` and `/home/willie` to Windows → `--unregister` → `--import` → restore both zones → reinstall managed tools from the manifest → `doctor`. |
| Factory reset                  | everything                                | `--unregister` + `--import`, double confirmation, backup offered.                                                        |

Reinstalling validates the new image before unregistering the old
distribution; a data-preserving upgrade (export/restore of the two
data zones) is a later slice, so today a reinstall replaces the
distribution wholesale.

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

`Session { id: ULID (sess_…), project_id, harness: "claude-code",
capabilities: CapabilitySet, args, created_at, state }`.
States: `creating → running → exited(code) | failed(reason)`, with a
transient `stopping`. Truth: `/var/lib/willie/sessions/<id>/spec.json`
(immutable after creation) and `events.jsonl` (append-only: `created`,
`started{pid}`, `attached{client}`, `detached`, `resized{cols,rows}`,
`exited{code,signal}`, `failed{reason}`). SQLite is a rebuildable index
(`willie reindex`).

### 3.2 Flows

**Start.**
1. UI → engine → RPC `session.create { project_id, harness,
   capabilities_override?, args? }`.
2. `willied` resolves configuration layers (§3.4), validates
   **fail-closed**, writes `spec.json`, spawns `willie-sess <id>`
   **detached** (setsid; stdio to `sessions/<id>/supervisor.log`).
3. `willie-sess` opens the PTY, builds the sandbox, runs the harness on
   the PTY slave, listens on `/run/willie/sessions/<id>.sock` (0600),
   records `started`, notifies the daemon over `willied.sock`.
4. `willied` replies `{ session_id, attach_command }`; the engine runs
   `wt.exe -w 0 new-tab -p "Willie" --title "<project>" -- wsl.exe -d willie --exec /opt/willie/bin/willie attach <id>`.
5. `willie attach` connects to the session socket, receives the ring
   buffer, puts its tty in raw mode, relays bytes both ways and
   `SIGWINCH` → `resize`. Several attaches may coexist; all read-write.

**Stop.** `session.stop` → daemon → supervisor: `SIGINT` (5 s) →
`SIGTERM` (5 s) → `SIGKILL` to the process group; `exited` recorded; the
supervisor exits when the last client detaches.

**Daemon restart.** Scan `/run/willie/sessions/*.sock`, ping each
supervisor, remove dead sockets, rebuild `sessions` from the event logs.

**Resume.** A new session whose `args` come from
`Harness::resume_args(harness_session_id)`; the harness's own id is found
through the capability matrix (`projects/<slug>/*.jsonl`, matched by cwd
and time).

**App closed.** Daemon exits; supervisors and terminal tabs continue; on
reopen the restart flow restores supervision.

### 3.3 Sandbox

Applied by `willie-sess`: **bubblewrap** (namespaces and mounts) +
**seccomp-bpf** (program compiled by the supervisor, passed as an fd) +
**Landlock** (applied by re-executing `willie-sess --inner` inside the
namespace right before `exec` of the harness) + **rlimits**.

**Base — always on, not configurable:**
- `--unshare-user --unshare-pid --unshare-ipc --unshare-uts`,
  `--die-with-parent` (the supervisor); `TIOCSTI` blocked by seccomp
  instead of `--new-session`;
- `no_new_privs`; system paths (`/usr`, `/lib*`, `/bin`, `/sbin`, selected
  `/etc` files) read-only;
- `$HOME=/home/willie` as **tmpfs**; private `/tmp`; fresh `/proc`;
  minimal `/dev`;
- **no `/init`, no `/run/WSL`, no `WSL_INTEROP`/`WSL_DISTRO_NAME`** ⇒ no
  Windows executable is reachable (binfmt points at `/init`, absent in the
  namespace);
- `/mnt/*` **not mounted** except the project path and `extra.paths`;
- `sudo` masked; `/var/lib/willie` and `/run/willie` not mounted;
- environment **allowlist**: `PATH`, `HOME`, `USER`, `TERM`, `COLORTERM`,
  `LANG`, `LC_*`, `TZ`, plus the `machine.env` variables;
- seccomp blocks `ptrace`, `process_vm_*`, `bpf`, `io_uring_*`,
  `perf_event_open`, `userfaultfd`, the mount family, `unshare`/`setns`,
  module loading, `kexec_*`, `keyctl`/`add_key`, `ioctl(TIOCSTI)`, packet
  and raw sockets;
- rlimits `NPROC`, `NOFILE`, `CORE=0`;
- **network on** (no `--unshare-net`): the harness needs it; Landlock at
  this kernel version has no network rules; fine-grained egress is a
  growth item.

**Named capabilities** (positive names; each carries one consequence
sentence shown in the UI):

| Capability         | Default          | Effect                                                                                                 |
| ------------------ | ---------------- | ------------------------------------------------------------------------------------------------------ |
| `project.rw`       | always           | bind **rw** of the project directory **at the same path** (`/mnt/c/...` or ext4)                      |
| `agent.state`      | on (Claude Code) | bind rw of the harness state dir/file → `$HOME/.claude`, `$HOME/.claude.json`; without it no login    |
| `tools.ro`         | on               | binds **ro** of managed tools (`~/.local`, `~/.dotnet`, Node manager) — the agent cannot alter them   |
| `caches.rw`        | on               | binds rw of `~/.nuget`, `~/.npm`, `~/.cache` (shared across sessions)                                  |
| `git.identity`     | on               | `~/.gitconfig` ro                                                                                      |
| `home.persistent`  | off              | `$HOME` = `~/.willie/homes/<project_id>` instead of tmpfs                                              |
| `extra.paths`      | empty            | additional `ro`/`rw` binds declared in the profile                                                     |
| `ssh`              | off              | `~/.ssh` ro + agent socket                                                                             |
| `mnt.all`          | off ⚠            | mounts all of `/mnt/*`                                                                                 |
| `windows.interop`  | off ⚠⚠           | mounts `/init`, keeps `WSL_INTEROP` — equivalent to no sandbox towards Windows                        |

`willie sandbox explain <project>` prints the exact bubblewrap argument
vector, the seccomp summary and the Landlock rules.

### 3.4 Configuration layers (increasing authority, monotonic)

1. Willie defaults (per harness: `Harness::default_capabilities()`).
2. Project profile — `/var/lib/willie/projects/<id>.toml`, edited by the
   UI; may enable or disable any capability.
3. `.willie/sandbox.toml` **inside the repository** — may only
   **tighten**: remove capabilities, add denied paths. It never opens
   anything because the agent can write it. Masked inside the sandbox.

Invalid configuration (unknown key, wrong type, an attempt to open at
layer 3) ⇒ the session does not start, with an actionable error. A project
under `/mnt/*` gets `slow_fs = true` (badge in the UI).

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
    fn usage_sources(&self) -> UsageSources;               // OAuth credentials, JSONL glob (usage plugin)
    fn resume_args(&self, harness_session_id: &str) -> Vec<String>;
}
```

`HarnessCapabilities { interactive_tui, resume: None|ById|Continue,
headless_stream, hooks, settings_format, mcp_config_format }` is **data**,
validated by a mini-bench (`willie dev bench-harness`) against the
installed binary. No consumer branches on the harness id.

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

`PluginCtx` provides: private storage (`plugin_kv` namespace or
`/var/lib/willie/plugins/<id>/`), the harness registry, the user's
filesystem (**outside** the sandbox — plugins run in the daemon), a
scheduler, an HTTP client already configured with proxy/CA, and
`emit(PluginEvent)`.

Rules: plugins never touch daemon internals; never run inside a session;
a plugin error ⇒ that plugin is `degraded`, the daemon continues. In the
UI each plugin is a statically registered React module under
`apps/willie-app/src/plugins/<id>/` that only uses core RPC/events.
**Not plugins:** sessions, sandbox, managed tools, WT profile, corporate
network.

### 4.2 `profiles`

A profile is a **git-versioned** directory in
`/var/lib/willie/profiles/<name>/`: `profile.toml` (metadata; lists
`mcp.enabled`, `hooks.enabled`, `rules.enabled`) and fragments
`settings.json`, `CLAUDE.md`, `rules/*.md`, `hooks/*`, `mcp.json`.
`profile.check` shows the diff; `profile.apply` writes into the repository
(`.claude/settings.json`, `CLAUDE.md`, `.claude/rules/`, hooks) and into
the harness state (`~/.claude/settings.json`) with a **format-preserving
merge**: JSON with key order kept, TOML edited in place, Markdown only
between `<!-- willie:begin --> … <!-- willie:end -->`; differential backup
in `.willie-bak/<timestamp>/`. Toggling an MCP/hook/rule = re-apply. Token
cost of an MCP server (best effort): `tools/list` over stdio JSON-RPC,
estimated at characters/4. Being git, profiles can sync between the two
machines through a private remote (later slice).

### 4.3 `usage`

Sources (Anthropic first; one **enum variant per provider**, a common
projection only at the output):
1. OAuth credentials in the harness state → the usage endpoint with the
   CLI's `User-Agent` — a **fragile** source: 60 s cache, `stale`
   flagged, backoff on 429, never a visible failure;
2. the harness's session JSONL (`state_paths()` → host path under the
   agent-state directory) → tokens per session/project and context
   percentage (`input + cache_creation + cache_read`), reading bounded
   tails, skipping sidechains and corrupt records;
3. Willie's `events.jsonl` to match a Willie session with a harness
   session (cwd + time window).

Output `usage.snapshot` (**additive** JSON):
`{ providers: [{ id, windows: [{ kind: "5h"|"7d", pct, resets_at }], credits?, stale, fetched_at }], sessions: [{ id, tokens, context_pct }] }`
plus notification `usage.updated`. Configurable thresholds emit
`usage.alert` → engine → Windows notification; the **tray** shows the main
window's percentage.

### 4.4 Persistence

- **Daemon:** SQLite `/var/lib/willie/willie.db` (WAL, ext4), **one
  writer actor** + read pool, versioned migrations. Tables `projects`,
  `sessions` (index), `tools`, `profiles_applied`, `plugin_kv`,
  `schema_version`.
- **Files are the truth** where they must survive without the daemon:
  `sessions/<id>/events.jsonl` (supervisor) and profiles in git.
  `willie reindex` rebuilds SQLite.
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
  (list, add, remove, update) · `session.*` (create, stop, list, get,
  attach_info) · `tool.*` (list, install, update) · `profile.*` (list,
  check, apply) · `usage.*` (snapshot, config) · `plugin.*` (list, enable,
  disable).
- Errors: `{ code, message, remediation }` — actionable message, named
  remediation.

## 5. UI, build, installer and tests

### 5.1 UI (Tauri 2 + React/TS/Vite)

| Screen        | Contents                                                                                                   |
| ------------- | ---------------------------------------------------------------------------------------------------------- |
| Dashboard     | engine/distro/daemon traffic light with expandable `doctor`; active sessions; usage summary                |
| Projects      | list with `slow_fs` badge; add via Windows folder picker (→ `/mnt/c/...`); project page: capabilities (with consequences), enabled plugins, applied profile |
| Sessions      | active/history (state, project, duration); *stop*; *open in Windows Terminal* (re-attach); *explain sandbox* |
| Tools         | detected in the distribution, version; install/update from the official source with confirmation and log  |
| Plugins       | enable/disable globally and per project; **Usage** and **Profiles** panels                                 |
| Settings      | machine (detected proxy/CA, re-sync), updates, tray                                                        |
| Tray          | icon with the main window's percentage; menu: open, new session in a recent project, mute alerts, quit     |

Closing the window minimises to the tray; *quit* stops the daemon
(sessions in Windows Terminal continue). One store fed only by
`state.snapshot` + `state.events` — the UI **never computes truth**.

### 5.2 Build and development (all from Windows)

Rust MSVC via `rustup`, Node 22 + pnpm, Tauri CLI as a dev dependency,
WebView2 (native on Windows 11). `willied`, `willie`, `willie-sess` are
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
| S3    | F3     | sandbox on the real kernel: `landlock` in `/sys/kernel/security/lsm`; Landlock ABI; `unshare -U`; `.exe` denied inside; CLI logs in with `agent.state` + tmpfs home | matrix of what works, recorded as a decision |
| S4    | F2     | corporate network: what does `autoProxy` inject (PAC or static)? does `curl` to the API work with the imported CA? | exact list of variables/files to propagate |
| S5    | F1     | `/mnt/c` performance: `git status`, `rg`, CLI startup on a real repository                                 | measured (0011): warm `git status` 511 ms on DrvFs vs 4.9 ms on ext4 (~100×), `rg` 19×, traversal 27× — F1 ships the ext4 workspace |

### 5.5 Slices ↔ components

| Slice | Delivers                                                        | Components                                                                                           | Spike |
| ----- | --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----- |
| F0    | distribution registered; UI shows health; `doctor` — **delivered 2026-08-26** | `distro/`, engine (import, supervision), `willied` (hello/health/doctor), `willie doctor`, Dashboard, NSIS | S1 |
| F1    | project in an ext4 workspace (`C:\` warned, 0011) + Claude Code session in WT, no sandbox | `project.*`, `session.*`, `willie-sess` (PTY, socket, events — sandbox off), `willie attach`, WT profile, Projects/Sessions screens | S2, S5 |
| F2    | proxy/CA propagated                                             | engine (WinHTTP, cert stores), `machine.env`, network `doctor`                                      | S4    |
| F3    | sandbox with capabilities and layers                            | `willie-sess` (bwrap/seccomp/Landlock, `--inner`), `willie-core` (CapabilitySet, layers), `sandbox explain`, capability UI | S3 |
| F4    | managed tools                                                   | `tool.*`, manifest, `Harness::detect`, Tools screen                                                  | —     |
| F5    | usage plugin + tray                                             | `willie-plugin-api`, `plugins/usage`, tray/notifications, Usage panel                               | —     |
| F6    | profiles plugin                                                 | `plugins/profiles` (git, merge), Profiles panel                                                      | —     |

F5/F6 order is the user's choice; F2 moves up if the first real use is on
the corporate machine.

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
