# 0024 — Profiles apply with a format-preserving merge, a differential backup, and project-only settings by default

- **Date:** 2026-09-07
- **Status:** accepted

## Context

Task 5 gave the profiles plugin pure merge functions (`merge_json`,
`merge_markdown`, `plan_changes`) with no filesystem access and nothing
callable over the wire. `profile.check`/`profile.apply` had to wire them
into two real methods: read a project's current `.claude/settings.json`,
`CLAUDE.md` and rule/hook files, plan what applying would change, and —
for `apply` — write it, having first preserved what was there. Three
questions needed an answer that was not already settled: how a project id
becomes a filesystem path without the plugin reaching into daemon state;
what a "differential backup" is shaped like on disk; and whether a
`settings` fragment applies only to the project or also to the harness
state every session reads (the design's own open question).

## Options

| Option | For | Against |
| --- | --- | --- |
| The plugin resolves `project_id` itself (a daemon handle in `PluginCtx`) vs the daemon resolves it and injects the paths | one fewer indirection | a plugin reaching into daemon state (project storage) is exactly the coupling the plugin host was built to avoid; every other plugin call would need the same handle |
| The daemon injects `_workspace`/`_harness_settings` into the request params vs a distinct typed field on `PluginRequest` | no change to the plugin API, generic across every `profile.*` method that happens to carry a `project_id` | the underscore prefix is a convention, not enforced by the type system |
| Backup folder named by a formatted calendar timestamp vs decimal nanoseconds since the Unix epoch | reads as a date a person could navigate to | a calendar formatter is nontrivial in `std` alone and no date-time crate is already a workspace dependency (AGENTS.md prefers `std`); nanoseconds are trivially monotonic and collision-free between two applies |
| A `settings` fragment applies to the harness state by default (opt-out) vs only the project by default (opt-in via `settings_scope = "global"`) | one profile keeps every project's Claude Code settings consistent without an extra step | the harness state is shared by every project's sessions; applying one profile to one project silently changing every other project's sessions is exactly the surprise the design's own open question warned against |
| The harness-scoped change is a distinct `Change` (its own `path`, `kind`, `after`) vs folded into the project's single settings change | each destination's merge runs against its own existing content and key order — they are genuinely different files that can diverge | the response carries two entries with the same semantic field (`.claude/settings.json`) unless the harness one's `path` is instead an absolute path, which is the distinguishing signal `apply_to_project`'s writer already needs |

## Decision

The daemon's `profile.*` route (`willied::handlers::profile_handle`, ahead
of `plugin_handle`) resolves a `project_id` present in the request's
params against `State.projects`, refusing an unknown one with
`project_not_found` **before the plugin ever runs** — so a disabled
profiles plugin never gets the chance to answer `plugin_disabled` first —
then injects the project's `workspace` and the harness state's
`settings.json` path as `_workspace`/`_harness_settings`. A request with no
`project_id` (`profile.list`, `profile.create`, …) passes through
untouched. The plugin's `CheckParams`/`ApplyParams` read only those two
injected fields; it never opens `State` or knows a `Project` exists.

`profile.check` reads every active fragment's file from the profile
directory and the corresponding target's current content from the
workspace, and runs `plan_changes` — the same function for both methods,
so a check is a truthful preview. `profile.apply` additionally copies each
changing target's *prior* content into
`<workspace>/.willie-bak/<nanoseconds-since-epoch>/` at its relative path
before writing anything (a `Create` has nothing to copy), then writes; the
backup path is always returned, even when every change was a `Create` and
nothing was copied. A `settings` fragment marked `settings_scope =
"global"` in `profile.toml`'s `[fragments]` table (default: `"project"`)
plans and applies one additional `Change` — the same `settings`/`mcp`
fragment content, merged independently against the harness state's own
`settings.json` — with its `path` set to the harness path itself (an
absolute path, versus the project's workspace-relative ones), which is how
the writer tells the two destinations apart.

## Consequences

- The profiles plugin stays daemon-ignorant: it never resolves a project
  id, never touches `State`, and could be tested (and was) with nothing
  but a scratch directory standing in for a workspace.
- An unknown `project_id` on `profile.check`/`profile.apply` is
  `project_not_found`, consistent with every other `project_id`-taking
  method, not a plugin-specific code.
- Applying a profile never changes another project's or another session's
  configuration unless a person explicitly marks a fragment
  `settings_scope = "global"` — the safe default is the narrowest one.
- A backup directory's name is not a human-readable date; it is unique and
  monotonic enough to survive two applies run back to back, which is what
  the differential backup actually needs.
- `.claude/settings.json`'s target file can end up written twice by one
  `profile.apply` (the project's own copy, and — when global-scoped — the
  harness state's), each keeping its own prior key order; they are not
  required to converge to the same bytes.

## Not decided

- **Token cost of an MCP server.** No accounting is implemented: the
  `mcp` fragment merges its JSON into `mcpServers` verbatim, and nothing
  estimates what an added server costs against a session's context
  budget.
- **A conflict-resolution UI for `profile_sync_conflict`.** The panel
  surfaces the code's own remediation text through a failure chip;
  nothing in the app opens a terminal or helps reconcile the divergent
  history itself.
- **A conflict between two applies racing on the same workspace.** Nothing
  locks a workspace across a `check`/`apply` pair or two concurrent
  applies; the daemon's one-request-at-a-time dispatch makes this unlikely
  in practice, but nothing enforces it structurally.

Two items originally listed here as not decided have since landed in
later tasks of the same plugin, without a new architectural choice
beyond what the design already called for: the minimal git sync
(`profile.set_remote`/`push`/`pull`, `git pull --ff-only`) and the
per-project enablement UI with the Apply flow's confirmation step (the
Plugins screen and profiles panel). See `docs/PROTOCOL.md`'s `profile.*`
section and `releases/v0.1.0.md`.
