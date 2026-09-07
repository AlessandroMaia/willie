# Managed tools — the Tools screen, the manifest, and update (F4, first cut)

The distribution runs agent CLIs and their toolchains, all installed into
the home, never through `apt` (§2.2). Today the daemon can install the
harness as a job, and the Dashboard shows a doctor line with an Install
button; there is no screen that lists what is installed, its version, or a
way to update it, and no durable record of what Willie installed. This
slice adds the Tools screen, the `tools` manifest, and `tool.update`, with
Claude Code as the first — and today only — managed tool. The framework is
general enough for a second tool (Node, .NET) to slot in without touching
the wire, but no such tool is built here.

## Problem

- **No management surface.** The only view of the harness is a doctor line
  on the Dashboard. The sidebar's "Tools" entry is a disabled placeholder
  (`available: false`). There is nowhere to see the installed version or
  update it; an out-of-date harness can only be fixed by running the
  installer by hand inside the distribution.
- **No durable record.** ARCHITECTURE §2.2 says the daemon keeps a
  manifest (`tools`) "for detection, display and reinstall after
  migration". Nothing writes one, so the image-migration flow (§2.4) has
  nothing to reinstall from, and the daemon re-detects from scratch every
  start.

## Goals

- A Tools screen lists each managed tool with its status (installed and
  its version, or not installed) and offers Install for a missing tool and
  Update for an installed one.
- The daemon keeps a `tools` manifest recording what it installed
  (version, when, the installer), written on install/update success, read
  on start.
- `tool.list` reports each catalogue tool with live detection; `tool.update`
  re-runs the official installer for an installed tool and re-detects.
- The extension point for a non-harness tool (Node, .NET) is named, so
  adding one later needs no protocol or manifest change.
- The Dashboard's bootstrap Install stays; the two share one job.

## Non-goals

- **Managing Node or .NET.** Named as the next tools; not built here. The
  catalogue is the harness registry today.
- **Uninstall.** Removing the harness would orphan its login state
  (`agent.state`); that needs its own decision about the dependent state.
  Deferred with the second cut.
- **Driving image migration from the manifest.** The base-image update
  flow (§2.4) is itself "not implemented in the app"; the manifest is
  written and ready for it, not consumed by it yet.
- **A `ManagedTool` trait.** One managed tool (the harness) does not earn a
  trait. The wire types and the manifest are tool-general (keyed by id); a
  `ManagedTool` trait is the documented extension point, introduced when
  the second tool is.
- **Version-pinning or a chooser.** The installer installs latest; there is
  no "install version X". A pinning story arrives with a tool that needs
  it (a Node version manager), not now.

## Design

### The catalogue and the manifest — `crates/willied/src/tools.rs`, a new `manifest` module

The **catalogue** of manageable tools is the harness registry
(`willie_harness::registry()`) today: one entry, Claude Code. `tool.list`
maps it.

The **manifest** is `/var/lib/willie/tools.toml`, one table per installed
tool, keyed by tool id:

```toml
[claude-code]
version = "2.1.246"
installed_at = "1757000000"
installer = "curl -fsSL https://claude.ai/install.sh | bash"
```

A small module beside `store.rs`, same shape:

```rust
pub struct ToolRecord { pub version: String, pub installed_at: String, pub installer: String }
pub fn load(state_dir: &Path) -> BTreeMap<String, ToolRecord> // empty on any read/parse failure
pub fn record(state_dir: &Path, id: &str, rec: &ToolRecord)   // save_or_log style: logs, never fails the caller
```

`load` tolerates a missing file and an unknown key (a tool a newer Willie
recorded): `#[serde(default)]`, unknown fields ignored, so an id the
registry no longer has is simply not shown. The manifest is written on
install/update success only; detection, not the manifest, is the truth the
screen shows, because the user can update a tool outside Willie.

### `tool.*` — `crates/willie-proto/src/tool.rs`

```rust
pub mod method {
    pub const INSTALL: &str = "tool.install"; // unchanged
    pub const LIST: &str = "tool.list";       // new
    pub const UPDATE: &str = "tool.update";   // new
}

pub struct ToolStatus {
    pub id: String,            // "claude-code"
    pub name: String,          // "Claude Code" — display name the daemon owns
    pub installed: bool,
    pub version: Option<String>,   // detected, present iff installed
    pub recorded_version: Option<String>, // from the manifest, for "installed outside Willie" display
}
pub struct ToolList { pub tools: Vec<ToolStatus> }
// InstallParams { harness } stays; UpdateParams { tool } mirrors it.
```

`tool.list` → `ToolList`; `tool.update` → the same `JobRef` as install.
`ToolStatus.name` is the daemon's, because the display name is domain data
the frontend must not restate (the same rule the capability catalogue
follows).

### The daemon — `crates/willied/src/tools.rs`, `handlers.rs`, `server.rs`

- `list(home) -> Vec<ToolStatus>`: for each catalogue tool, `detect` it
  (live `--version`), merge the manifest's `recorded_version`. Pure over
  its inputs where it can be; the detection is the I/O.
- `install` (exists): unchanged refusal rules; on success, `manifest::record`
  the detected version. The success path already re-detects.
- `update(runner, home, id) -> JobId`: refuses `tool_not_installed` when
  the tool is absent (Update is only offered for an installed tool, but the
  daemon fails closed), else submits a global job with `JobKind::UpdateHarness`
  running the same installer command, re-detecting and recording on success.
- `handlers::tool_list` and `handlers::tool_update`; `server.rs` routes
  `tool::LIST` and `tool::UPDATE` beside `tool::INSTALL`.

`JobKind` gains `UpdateHarness` (`#[serde(rename_all = "snake_case")]` →
`update_harness`). The install and update jobs are both global (no
project); the runner already serialises tool jobs (`tool_busy`).

### The engine and the Tauri host — `crates/willie-engine/src/engine.rs`, `apps/willie-app/src-tauri/src/lib.rs`

- `Engine::tool_list() -> Result<ToolList, EngineError>` (`daemon_call(tool::LIST, {})`)
  and `Engine::tool_update(id) -> Result<JobRef, EngineError>`.
- Tauri commands `tool_list` and `tool_update`, registered beside
  `tool_install`, same `daemon_command` wrapper.

### The Tools screen — `apps/willie-app/src/app/routes.ts`, `router.tsx`, `features/tools/*`

- `routes.ts`: the `tools` entry becomes an `AvailableEntry` with
  `path: "/tools"` and `shortcut: "Mod+4"`. The `ScreenPath` union gains
  `"/tools"`. The sidebar already renders an available entry as a link with
  its shortcut; the disabled placeholder is gone.
- `router.tsx`: register `/tools` → `ToolsScreen`; the hotkey map picks it
  up from `ROUTES.filter(available)`.
- `features/tools/tools-screen.tsx`: on mount, `toolsApi.list()` into state;
  re-fetch when a tool job leaves `running` (the snapshot's job list drives
  it, like the Dashboard's install tracking). One row per `ToolStatus`:
  - installed: a `StatusBadge tone="ok"` "installed v{version}" and an
    **Update** button; if `recorded_version` differs from `version`, a
    `muted` note "updated outside Willie";
  - not installed: a `muted` "not installed" and an **Install** button;
  - while a tool job runs: a spinner and the job's `log_tail`, the Dashboard's
    pattern.
- `lib/domain/jobs.ts`: `latestInstallJob` generalises to `latestToolJob`
  (matches `install_harness` or `update_harness`); the Dashboard keeps its
  install-only view by filtering kind, or reuses the general one and checks
  the kind it cares about.
- `lib/ipc.ts`: `tools.list()` → `invoke<ToolList>("tool_list")` and
  `tools.update(id)` → `invoke("tool_update", { tool: id })`, beside the
  existing `tools.install`.

### The Dashboard — `apps/willie-app/src/features/health/dashboard-screen.tsx`

Unchanged in behaviour: the inline Install on the Claude Code doctor line
stays, because installing the harness right after the distribution is the
bootstrap flow and must not require finding another screen. It and the
Tools screen share the `install_harness` job, so a running install shows in
both. No second code path.

### Errors and edge cases

| Condition | Behaviour | Code |
| --- | --- | --- |
| `tool.update` on a tool that is not installed | refused | `tool_not_installed` |
| `tool.install`/`tool.update` with an unknown id | refused | `invalid_params` |
| a tool job while another tool job runs | refused | `tool_busy` (exists) |
| the installer fails | the job fails with its stderr | `install_failed` (exists) |
| the manifest cannot be written after a successful install | logged to stderr, the job still succeeds | — |
| the manifest file is missing or unparseable at load | treated as empty; every tool shows by live detection | — |
| a manifest entry for an id the registry no longer has | ignored (not shown) | — |

`tool_not_installed` remediation: "install it first, then update".

## Testing

Host (`cargo test`, in `just check`):

- `manifest`: round-trips a `ToolRecord` through TOML; an unknown key
  loads as empty rather than failing; a missing file is empty.
- `tools::list`: with a planted fake `claude --version` (the existing
  `plant_claude` fixture), reports `installed: true` and the version; with
  none, `installed: false`, `version: None`.
- `tools::update`: refuses `tool_not_installed` when absent; runs the
  (fake) installer and records the manifest when present (extend the
  existing install test's harness).
- `server`/`handlers`: `tool.list` and `tool.update` are dispatched and
  answer the right shapes / codes.
- `JobKind::UpdateHarness` serialises as `update_harness`.

Frontend (vitest):

- the Tools screen renders an installed tool with its version and an
  Update button, and a missing tool with an Install button;
- a running tool job shows progress;
- `latestToolJob` picks the newest of install/update;
- the shell shows Tools as an available entry with `Ctrl+4`, and the route
  renders (extend `shell.test.tsx`, which today asserts Tools is disabled —
  that assertion moves to Plugins/Settings).

No new distribution test: the installer path is already covered by
`tools.rs`'s job test; `tool.update` reuses it with a fake installer.

## Rollout / compatibility

- `PROTOCOL_VERSION` unchanged: `tool.list`/`tool.update` and `ToolStatus`
  are additive; `JobKind::UpdateHarness` is a new variant (older clients
  ignore an unknown kind — the frontend's job domain already treats only
  the kinds it knows).
- The manifest is new state under `/var/lib/willie/`; its absence is the
  same as an empty one, so a distribution upgraded in place needs no
  migration — the first install writes it.
- `docs/PROTOCOL.md` gains the two methods and `ToolStatus`;
  `docs/ARCHITECTURE.md` marks the Tools screen delivered in §5.5 and
  describes the manifest's format and location; decision **0022 — the
  managed-tool model: the harness registry is the catalogue, the manifest
  records installs, `ManagedTool` is the named extension point**.
- Release note: a Tools screen shows the installed agent CLI and its
  version and lets you update it; the daemon now records what it installed.
- `shell.test.tsx`'s "planned screens disabled" assertion drops Tools and
  keeps Plugins and Settings.

## Open questions

- Should the Tools row show "an update is available" rather than only
  offering Update blindly? Favoured: no — the installer has no "is there a
  newer version" query short of running it, and Claude Code auto-updates
  itself in normal use; Update here is the manual escape hatch, not a
  version watcher. Revisit if a tool without self-update (a Node manager)
  becomes managed.
- Where does a second tool's descriptor live — a `ManagedTool` trait in
  `willie-harness`, or a new `willie-tools` crate? Decided when the second
  tool lands; the manifest and wire do not care.
