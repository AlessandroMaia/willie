# The plugin host, folded into the profiles plugin (F6, first cut)

Willie's plugins are compiled into the daemon, run outside every session
sandbox, and each degrades only itself (§4.1). Today the `Plugin` trait is
`manifest()` alone, no daemon registers a plugin, the snapshot carries no
plugin state, and there is no `plugin.*` protocol or `PluginCtx`. This
slice builds the host contract and proves it with its first real consumer,
the **profiles** plugin: a git-versioned bundle of configuration a person
creates and edits in the app, applies into a project with a
format-preserving merge, and syncs between machines through a private git
remote. The **usage** plugin (F5) lands next on the same host.

Per the user's decisions: the host is folded into this slice (not a
standalone platform slice with no plugin to run); SQLite is deferred
(plugin storage is files, which §4.1 permits); the MCP token-cost estimate
is out of the first cut; a minimal git sync is in.

## Problem

- **No plugin host.** The `Plugin` trait exposes only `manifest()`;
  §4.1's `on_enable`/`on_disable`/`handle`/`on_event` and `PluginCtx`
  (storage, filesystem, `emit`) do not exist. Nothing can be enabled,
  configured, or run.
- **No configuration portability.** A person developing the same systems
  on two machines (a corporate and a personal one) has no way to carry
  the agent's configuration — `settings.json`, `CLAUDE.md`, rules, hooks,
  MCP servers — between them, or to apply a consistent set across projects
  on one machine.

## Goals

- The `Plugin` trait carries the real contract; the daemon hosts a
  registry, persists which plugins are enabled (and, for a per-project
  plugin, in which projects), and exposes it over `plugin.*` and in the
  snapshot.
- A `PluginCtx` gives a plugin file storage under
  `/var/lib/willie/plugins/<id>/`, filesystem and git access, and `emit`.
  The HTTP client and scheduler (usage's) are named seams, not built.
- The profiles plugin: create and edit a profile's fragments in the app;
  `profile.check` shows what applying would change; `profile.apply` writes
  the fragments into a project (and the harness state) with a
  format-preserving merge and a differential backup; a minimal git sync
  (`set_remote`, `push`, `pull`) carries a profile between machines.
- Each phase is independently green and reviewable.

## Non-goals

- **SQLite / `willie.db`.** Deferred by decision; plugin storage and the
  enablement record are files. Introduced when a plugin needs an index or
  query (usage's session index, perhaps), not here.
- **The MCP token-cost estimate.** §4.2's best-effort `tools/list` cost is
  a separable concern; deferred.
- **The HTTP client and the scheduler in `PluginCtx`.** Usage's; named as
  seams so usage adds them without reshaping the host.
- **Conflict resolution beyond git's.** The minimal sync pushes and pulls;
  a real conflict surfaces as a coded error the user resolves in a
  terminal. No three-way merge UI.
- **A general plugin-marketplace or dynamic loading.** Plugins are
  statically compiled in (§4.1); the registry is a Rust `Vec`.
- **Usage (F5).** Registered as a stub so the host lists it, but it does
  nothing until its own slice.

## Design

The slice is four phases. One design; the plan groups tasks by phase.
Phases 1–3 are backend (provable by tests and, once the CLI socket from
the sandbox follow-ups lands, by `willie`); phase 4 is the UI.

### Phase 1 — the host and the profile model

#### The grown trait — `crates/willie-plugin-api/src/lib.rs`

```rust
pub enum PluginError { … }              // typed, snake_case code + remediation
pub struct PluginRequest { pub method: String, pub params: serde_json::Value } // "<id>.<method>"
pub enum PluginResponse { Json(serde_json::Value) }

pub enum CoreEvent { SessionStarted{..}, SessionExited{..}, ProjectRegistered{..}, Tick }

pub trait Plugin: std::fmt::Debug {
    fn manifest(&self) -> PluginManifest;                       // unchanged
    fn on_enable(&mut self, ctx: &PluginCtx, scope: Scope) -> Result<(), PluginError> { Ok(()) }
    fn on_disable(&mut self, ctx: &PluginCtx, scope: Scope) -> Result<(), PluginError> { Ok(()) }
    fn handle(&mut self, ctx: &PluginCtx, req: PluginRequest) -> Result<PluginResponse, PluginError>;
    fn on_event(&mut self, ctx: &PluginCtx, ev: &CoreEvent) {}  // default: ignore
}
```

Default bodies keep a plugin that needs none (the usage stub) minimal.

#### `PluginCtx` — `crates/willie-plugin-api/src/lib.rs`

The first cut carries what profiles needs, no more:

```rust
pub struct PluginCtx<'a> {
    /// `/var/lib/willie/plugins/<id>/`, created on first use. The plugin's
    /// own private tree; nothing else reads it.
    pub store_dir: &'a Path,
    /// Emit a plugin event the daemon forwards as a notification.
    emit: &'a dyn Fn(PluginEmission),
}
impl PluginCtx<'_> {
    pub fn store_dir(&self) -> &Path;
    pub fn emit(&self, kind: &str, payload: serde_json::Value);
}
```

Filesystem and git access are plain `std::fs` and the daemon's `git`
helper used from inside the plugin, not wrapped here — the plugin runs in
the daemon, so it already has them; wrapping is deferred until a second
plugin shows a shared need. The HTTP client and scheduler are documented
in the trait's rustdoc as the next `PluginCtx` fields, not added.

#### The host — `crates/willied/src/plugins.rs` (new)

```rust
pub struct PluginHost {
    plugins: Vec<Box<dyn Plugin>>,      // willie_plugins::registry(): [profiles, usage-stub]
    enabled: EnabledState,              // loaded from plugins/enabled.toml
    state_dir: PathBuf,
}
```

- `list() -> Vec<PluginStatus>`: each plugin's manifest merged with its
  enabled state.
- `enable(id, scope)` / `disable(id, scope)`: validate the scope against
  the manifest's (`Global` vs `PerProject`), call `on_enable`/`on_disable`
  with a `PluginCtx` rooted at that plugin's store dir, persist
  `enabled.toml`, return the new status. A per-project enable records the
  project id; a global one a bare flag.
- `handle(method, params)`: split `method` at the first `.` into
  `<id>.<rest>`, find the plugin, call `handle`; an unknown id is
  `plugin_not_found`, a disabled plugin `plugin_disabled`.
- `on_event(ev)`: fan a `CoreEvent` to every enabled plugin; a plugin that
  returns `Err` from any call is marked `degraded` (surfaced in its
  status), the daemon continues.

`EnabledState` persists as `/var/lib/willie/plugins/enabled.toml`:

```toml
[profiles]
projects = ["proj_01J…"]   # per-project: the ids it is enabled in
[usage]
global = false
```

Loaded on start, rewritten on every enable/disable; a missing or
unreadable file is "nothing enabled" (fail safe, not closed — a plugin
being off is the safe default).

#### The protocol — `crates/willie-proto/src/plugin.rs` (new), `state.rs`

```rust
pub mod method { pub const LIST; pub const ENABLE; pub const DISABLE; }
pub struct PluginStatus { pub id, pub name: String, pub scope: Scope, pub enabled: Enablement, pub degraded: bool }
pub enum Enablement { Global(bool), PerProject(Vec<ProjectId>) }
pub struct EnableParams { pub id: String, pub project_id: Option<ProjectId> } // None = global
```

`Snapshot` gains `#[serde(default)] plugins: Vec<PluginStatus>`. A new
`EventKind::PluginChanged { plugin: PluginStatus }` and the plugin's own
`plugin.emitted { id, kind, payload }` notification carry live changes.
`profile.*` (below) is routed through `plugin.handle`, so it needs no
top-level dispatch arm — `server.rs` routes `plugin::LIST/ENABLE/DISABLE`
and a single `profile::*` catch to `host.handle`.

#### The profile model — `crates/willie-plugins/profiles/src/lib.rs`

A profile is `<store_dir>/<name>/`, a git repository:

```
profile.toml     # metadata: which fragments are active
settings.json    # merged into .claude/settings.json (JSON, key order kept)
CLAUDE.md        # merged into the project's CLAUDE.md (between willie markers)
rules/*.md       # copied into .claude/rules/
hooks/*          # copied into .claude/hooks/
mcp.json         # merged into .claude/settings.json's mcpServers (JSON)
```

`profile.toml`:

```toml
name = "funcef-auth"
[fragments]
settings = true
instructions = true       # CLAUDE.md
rules = ["no-force-push.md"]
hooks = []
mcp = true
```

Phase-1 methods (through `handle`):
- `profile.list` → `[ProfileSummary { name, fragments_active }]`.
- `profile.create { name }` → scaffold the dir, a default `profile.toml`,
  `git init`, an initial commit; refuses `profile_exists`.
- `profile.read_fragment { name, fragment }` / `profile.write_fragment { name, fragment, content }`
  → read/write a fragment file; each write is a git commit
  (`fragment: settings|instructions|mcp|rules/<f>|hooks/<f>`).

### Phase 2 — check and apply (the format-preserving merge)

`crates/willie-plugins/profiles/src/apply.rs` (new), pure over its inputs:

- **JSON merge** (`settings.json`, `mcp.json` → `.claude/settings.json`):
  parse the target and the fragment as ordered maps (`serde_json` with
  `preserve_order`), deep-merge the fragment onto the target keeping the
  target's existing key order and appending new keys, re-serialise with
  the same 2-space shape. `mcp.json` merges under `mcpServers`.
- **Markdown merge** (`CLAUDE.md`): replace only the region between
  `<!-- willie:begin --> … <!-- willie:end -->`, inserting the markers at
  the end if absent; the person's own prose outside is untouched.
- **File copy** (`rules/*.md`, `hooks/*`): write each active fragment file
  into `.claude/rules/` / `.claude/hooks/`, preserving mode for hooks.
- **Backup**: before any write, copy each target file that will change
  into `<workspace>/.willie-bak/<timestamp>/` at its relative path.

Methods:
- `profile.check { name, project_id }` → `[Change { path, kind: create|merge|overwrite, before?, after }]`,
  computed without writing — the same merge run against copies.
- `profile.apply { name, project_id }` → performs the writes into the
  project's ext4 workspace (`Project.workspace`) and, for `settings.json`,
  also into the harness state
  (`~/.willie/agent-state/claude/dot-claude/settings.json`); returns the
  applied `[Change]` and the backup path. Toggling a fragment in
  `profile.toml` and re-applying is the supported "turn it off" path.

The project is looked up through the daemon (the host passes the resolved
`Project` into the plugin via the request, or the plugin asks the ctx —
decided in the plan; the plugin must not reach into daemon internals, so
the resolved workspace path travels in the request params the daemon
fills, not a project handle).

### Phase 3 — the minimal sync

`crates/willie-plugins/profiles/src/lib.rs`, over the daemon's `git`
helper run in the profile's own directory:

- `profile.set_remote { name, url }` → `git remote add`/`set-url origin`.
- `profile.push { name }` → `git push -u origin HEAD`.
- `profile.pull { name }` → `git pull --ff-only`; a non-fast-forward or a
  conflict returns `profile_sync_conflict` naming the profile, remediation
  "resolve it in a terminal inside the distribution, then pull again".

No credential handling beyond what the distro's git already has; a private
remote the user configures (an SSH URL uses the session's keys story,
which is out of scope — the user configures a reachable remote). The sync
is what carries a profile between the two machines.

### Phase 4 — the UI

#### Plugins — `apps/willie-app/src/app/routes.ts`, `features/plugins/`

The sidebar's "Plugins" placeholder becomes an `AvailableEntry`
(`/plugins`, `Ctrl+5`). A Plugins screen lists `plugin.list`: each plugin
with its name, scope, and an enable/disable control — a global plugin a
toggle, a per-project plugin a note that it is enabled per project (from
the project's own surface). A `degraded` plugin shows an `error` chip.

#### The profiles panel — `apps/willie-app/src/plugins/profiles/`

Per §4.1, a plugin's UI is a statically registered React module under
`src/plugins/<id>/` that uses only core RPC/events. `plugins/profiles/`
exports a panel the Plugins screen mounts when profiles is enabled:

- a profile list with **New profile** (`profile.create`);
- a selected profile's fragments, each editable in a textarea
  (`read_fragment`/`write_fragment`), a save committing;
- **Apply to project…**: pick a registered project, show `profile.check`'s
  changes, confirm to `profile.apply`, then the backup path;
- **Sync**: set the remote, Push, Pull, with the conflict error surfaced.

Per-project enablement is offered on the project row (a "Profiles…" action
like "Sandbox…") or in the panel's Apply step; the plan picks one. The
bridge is `pluginsApi` (`list`, `enable`, `disable`) and `profilesApi`
(the `profile.*` methods) in `lib/ipc.ts`; the domain facts (a profile's
active fragments, a change list) live in `lib/domain/`.

### Errors and edge cases

| Condition | Behaviour | Code |
| --- | --- | --- |
| `plugin.enable` with a scope the manifest forbids (global on a per-project plugin) | refused | `plugin_scope_mismatch` |
| a `plugin.*`/`profile.*` call for an unknown plugin id | refused | `plugin_not_found` |
| a `profile.*` call while profiles is disabled | refused | `plugin_disabled` |
| `enabled.toml` missing or unreadable | nothing enabled; the daemon runs | — |
| a plugin's `handle`/`on_event` returns `Err` | the plugin is `degraded` in its status; the daemon continues | (the plugin's own code) |
| `profile.create` of an existing name | refused | `profile_exists` |
| `profile.apply` on a project whose workspace is gone | refused before any write | `profile_target_missing` |
| a fragment that is not valid JSON (settings/mcp) | `profile.check`/`apply` refuse naming the fragment | `profile_fragment_invalid` |
| `profile.pull` hits a non-fast-forward or conflict | refused, the profile named | `profile_sync_conflict` |
| a plugin panics | caught at the host boundary; the plugin is `degraded`, the daemon lives (`willied` forbids `unwrap`; the host wraps the call) | — |

## Testing

Host (`cargo test`):
- `willie-plugin-api`: the trait's default bodies; a `PluginCtx` gives the
  right store dir.
- `willied` `plugins`: enable/disable persists and round-trips
  `enabled.toml`; a scope mismatch and an unknown id are the right codes;
  `handle` routes `profile.x` to the profiles plugin; a plugin `Err`
  marks it degraded, not the daemon.
- profiles `apply`: the JSON merge keeps the target's key order and appends
  new keys; the Markdown merge touches only between the markers and leaves
  outside prose; a file fragment copies; the backup captures each changed
  file; an invalid JSON fragment is refused. Pure, host-run.
- profiles `profile.create`/`write_fragment`: scaffold and commit
  (the profiles plugin's own git wrapper, run inside the distribution for the git parts).

Distribution (`just test-linux`): `profile.create` + a real `git init`/commit;
`profile.apply` into a scratch workspace repo; `profile.push`/`pull`
against a bare local remote; the whole path uses real `git`.

Frontend (vitest): the Plugins screen lists and toggles; the profiles
panel creates a profile, edits a fragment, runs check and shows the
changes, and surfaces a sync conflict; `Ctrl+5` navigates.

## Rollout / compatibility

- `PROTOCOL_VERSION` unchanged: `plugin.*`, `profile.*`, `PluginStatus`,
  the `plugins` snapshot field and the new event kinds are additive; older
  clients ignore what they do not know.
- New state under `/var/lib/willie/`: `plugins/enabled.toml`,
  `plugins/profiles/…`, `profiles/<name>/` git dirs. All absent on an
  upgraded distribution, which reads as "no plugins enabled, no profiles".
- New codes in `docs/PROTOCOL.md`; §4.1–4.2 marked as built (the first
  cut: no SQLite, no MCP cost, minimal sync); §4.4's SQLite is explicitly
  still pending.
- Decisions: **0023 — the plugin host: the trait contract, file-based
  storage, SQLite deferred**; **0024 — profiles apply with a
  format-preserving merge and a differential backup**. The sync and the
  deferred MCP cost are named in 0024's "Not decided".
- Release notes per phase: plugins can be enabled; profiles can be
  created, edited, applied to a project, and synced between machines.

## Open questions

- Per-project enablement UI: the project row's menu (like "Sandbox…") or
  the profiles panel's Apply step. Favoured: the panel's Apply step for the
  first cut — one surface owns profiles — with the row action a follow-up
  if it proves wanted.
- Whether `profile.apply` writing the harness-state `settings.json` should
  be opt-in per apply (it affects every project's sessions, not just this
  one). Favoured: apply writes the project's `.claude/` always and the
  harness state only when the profile's `settings` fragment is marked
  `scope = "global"` in `profile.toml` — decided in the plan, defaulting to
  project-only to avoid a surprise global change.
