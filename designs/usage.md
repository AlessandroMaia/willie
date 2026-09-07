# Usage — context and tokens, the robust core (F5, first cut)

A person running an agent session wants to see how full the context window
is before a surprise compaction, and how many tokens a project has cost.
Both are in the harness's own session logs, on disk, no network. This
slice reads them and shows them in a Usage panel. The fragile OAuth usage
endpoint (limit windows, credits) and the tray are deferred: the first cut
is the source that always works.

Per the user's decision: the robust core only — the harness session JSONL
(tokens, context %) matched to Willie sessions, no fragile endpoint, no
tray, no notifications.

## Problem

- **No context visibility.** A session's context fills as it runs; when it
  is nearly full the harness compacts, losing detail. Nothing in Willie
  shows how full it is, so a compaction is always a surprise.
- **No cost visibility.** Token spend per session and per project lives in
  the harness's JSONL logs; Willie never surfaces it.

The high-value, always-available signals (context %, tokens) come from a
local file. The limit-window percentages come from an undocumented
endpoint that is fragile (§4.3, source 1); this slice does not depend on
it.

## Goals

- A Usage panel shows, per live or recent session, its context-window
  percentage and token totals, and a per-project token summary.
- The data comes from the harness session JSONL, read best-effort —
  bounded tails, skipping sidechains and corrupt records, **never a
  visible failure** (§4.3).
- A Willie session is matched to its harness log by the workspace and the
  session's time window, harness-agnostically (the harness owns where its
  logs live and how it names a project's directory).
- `usage.snapshot` is additive JSON with an empty `providers` array, so the
  deferred source 1 slots in without a wire change.
- Usage is a plugin on the host the profiles slice builds; this slice adds
  no HTTP client or scheduler.

## Non-goals

- **The OAuth usage endpoint (source 1).** The limit windows (5h/7d, pct,
  resets_at) and credits. Fragile, undocumented; a later slice, behind the
  `providers` field this cut leaves empty.
- **The tray and Windows notifications.** The tray percentage is source 1's
  number; thresholds → `usage.alert` → notification are its follow-on. Net
  Windows-facilities work, deferred with source 1.
- **A daemon scheduler / periodic poll.** Usage recomputes on demand and on
  session events; the panel polls lightly or reacts to `usage.updated`.
  The scheduler stays a named host seam (usage source 1 will want it).
- **SQLite.** Deferred with the host; usage reads files and holds its
  projection in memory.
- **The HTTP client in `PluginCtx`.** Source 1's; a named seam, not built.

## Design

This slice depends on the plugin host (the profiles slice). It registers
the `usage` plugin, adds two harness methods so the plugin stays
harness-agnostic, reads the JSONL, projects it, and shows it in a panel.

### The harness sources — `crates/willie-harness/src/lib.rs`

Two methods on `Harness`, so the plugin never hard-codes Claude Code's
layout:

```rust
/// Where the harness writes its per-session logs, as a host path under
/// the agent-state directory. `None` for a harness that keeps none.
fn session_logs_dir(&self, home: &Path) -> Option<PathBuf>;

/// How the harness names a workspace's log subdirectory. Claude Code
/// replaces the path separators, so `/home/willie/projects/x` becomes
/// `-home-willie-projects-x`.
fn escape_workspace(&self, workspace: &str) -> String;
```

`ClaudeCode`: `session_logs_dir` =
`~/.willie/agent-state/claude/dot-claude/projects` (the agent-state tree
`~/.claude` links into); `escape_workspace` = the separator replacement.
Both tested against the known layout. A harness with no logs (a future
one) returns `None`, and usage shows nothing for its sessions rather than
guessing.

### The usage plugin — `crates/willie-plugins/usage/src/lib.rs`, `read.rs`

Registered on the host, `Scope::Global`. It holds an in-memory projection,
recomputed on demand and on session events.

**Reading (`read.rs`, pure over injected file contents where it can be):**

- For a Willie session, resolve its harness log directory:
  `session_logs_dir(home)/escape_workspace(session.workspace)`. Within it,
  pick the `*.jsonl` whose time window (file mtime, or the first record's
  timestamp) overlaps the session's `created…exited` from its
  `events.jsonl`.
- Read a **bounded tail** of that JSONL (the last N KiB), parse line by
  line, skip a line that is not valid JSON or is a sidechain
  (`isSidechain`/`parentUuid` markers), and take the newest assistant
  record carrying token counts.
- `tokens = input + cache_creation + cache_read + output` (cumulative as
  the harness reports them on the latest record, best-effort);
  `context_pct = (input + cache_creation + cache_read) / context_window`,
  clamped to 0–100, `None` when the window is unknown.
- Per project: sum the token totals of its sessions' logs.

**The plugin (`lib.rs`):**

- `handle("usage.snapshot", _)` → the projection as `UsageSnapshot`
  (recomputes first).
- `on_event(SessionStarted | SessionExited)` → recompute, then
  `ctx.emit("usage.updated", …)`.
- Everything best-effort: any read error for one session leaves that
  session without usage data and never fails the call.

The daemon fills each live/recent session's `workspace` and its
`events.jsonl`-derived time window into the request (the same
daemon-fills-context seam the profiles apply uses), so the plugin does not
read the daemon's session store directly.

### The protocol — `crates/willie-proto/src/usage.rs` (new)

```rust
pub mod method { pub const SNAPSHOT: &str = "usage.snapshot"; }

pub struct UsageSnapshot {
    pub providers: Vec<ProviderUsage>,        // empty in this cut (source 1 deferred)
    pub sessions: Vec<SessionUsage>,
    pub projects: Vec<ProjectUsage>,
    pub fetched_at: String,
}
pub struct SessionUsage { pub id: SessionId, pub tokens: u64, pub context_pct: Option<u8> }
pub struct ProjectUsage { pub id: ProjectId, pub tokens: u64 }
pub struct ProviderUsage { /* source 1's shape, defined but unused here */ }
```

`usage.snapshot` is routed through `plugin.handle` (like `profile.*`); the
notification `usage.updated` (no id) rides the existing daemon→client
stream. `providers` is present and empty so source 1 is purely additive.

### The Usage panel — `apps/willie-app/src/plugins/usage/`

A plugin UI module (§4.1), mounted by the Plugins screen when usage is
enabled:

- one row per live/recent session: a context meter (the bar that warns
  before a compaction) with its percentage, and the session's token total;
  a session with no usage data reads "no usage yet", never an error;
- a per-project token summary;
- refresh: subscribe to `usage.updated` and poll `usage.snapshot` on a
  light interval while the panel is open (the context grows mid-session).

Colour through `Tone` — the context meter warms as it fills (a `warning`
past a threshold, `error` near full), through the tone map, never a
literal. The bridge is `usageApi.snapshot()`; the meter thresholds are the
panel's, documented beside it.

### Errors and edge cases

| Condition | Behaviour | Code |
| --- | --- | --- |
| a session with no harness log (never logged in, fresh session) | shown with no usage data | — |
| a corrupt or sidechain JSONL record | skipped | — |
| the context window size is unknown | `context_pct: None`; the meter shows tokens only | — |
| a harness that keeps no logs (`session_logs_dir` None) | its sessions show no usage | — |
| `usage.snapshot` while usage is disabled | refused | `plugin_disabled` |
| the whole read fails | the session is simply absent from `sessions`; never an error reply | — |

No new error code beyond the host's `plugin_disabled`: usage never fails
visibly (§4.3).

## Testing

Host (`cargo test`):
- `willie-harness`: `ClaudeCode::escape_workspace` replaces separators;
  `session_logs_dir` is the agent-state projects path.
- usage `read`: given a fixture JSONL (a few records, one sidechain, one
  corrupt line), the projection is the newest valid assistant record's
  tokens and the right context %; a missing file yields no data, not an
  error; the tail bound is respected; the time-window pick chooses the
  overlapping file among two.
- usage `lib`: `handle("usage.snapshot")` returns the projection;
  `on_event(SessionExited)` recomputes and emits.

Distribution (`just test-linux`): the plugin reading a planted JSONL under
a scratch agent-state tree matched to a scratch session's `events.jsonl`.

Frontend (vitest): the panel renders a session's context meter and tokens,
a "no usage yet" session, and the per-project summary; it reacts to a
`usage.updated` event; the meter tone warms past its thresholds.

## Rollout / compatibility

- `PROTOCOL_VERSION` unchanged: `usage.*`, `UsageSnapshot` and the
  `usage.updated` notification are additive; `providers` is empty now and
  fills when source 1 lands.
- Depends on the plugin host (profiles slice) being built first — usage is
  a plugin on it. No new host capability: it uses `handle`, `on_event`,
  `emit` and file access, all built for profiles.
- New harness methods are additive (defaulted where a trait default fits).
- `docs/PROTOCOL.md` gains `usage.*`; `docs/ARCHITECTURE.md` marks §4.3
  partially delivered (sources 2 and 3, no source 1, no tray); decision
  **0025 — usage's robust core: the session JSONL, the fragile endpoint and
  the tray deferred**. Release note: a Usage panel shows each session's
  context fill and token cost.

## Open questions

- The context-window size per model: hard-coded table, or read from the
  harness record if it carries it? Favoured: read it from the record when
  present, else a small model→window table in the plugin, else
  `context_pct: None` — never a wrong denominator.
- Recompute cadence: on every session event may be chatty on a busy
  machine. Favoured: recompute on session start/exit and on a
  panel-driven pull; a coalescing timer is a source-1-era refinement, when
  the scheduler seam is built.
