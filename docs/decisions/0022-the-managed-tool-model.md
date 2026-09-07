# 0022 — The managed-tool model: the harness registry is the catalogue, the manifest records installs

- **Date:** 2026-09-06
- **Status:** accepted

## Context
F4 adds a Tools screen and the ability to update a managed tool. Claude
Code is the only managed tool today; ARCHITECTURE names .NET and a Node
version manager as future ones. A manifest was promised (§2.2) for
display and reinstall after image migration.

## Options
| Option | For | Against |
| --- | --- | --- |
| Harness registry as the catalogue, no new trait (chosen) | one tool today needs no abstraction; wire and manifest are id-keyed and already general | a second, non-harness tool needs a small trait then |
| A ManagedTool trait now | uniform from the start | a trait for one impl; the second tool's real shape is unknown |
| Detection only, no manifest | less state | nothing to reinstall after migration; re-detect every start |

## Decision
The catalogue of manageable tools is the harness registry. `tool.list`
maps it with live detection; the display name lives in one daemon-side map
until a tool type owns it. The manifest (`/var/lib/willie/tools.toml`)
records what the daemon installed, for display and future reinstall, and
is not the truth the screen shows — live detection is, because a user can
update a tool outside Willie. `tool.update` re-runs the installer for an
installed tool. Uninstall is deferred: removing the harness orphans its
login state, which needs its own decision.

## Consequences
A second managed tool (Node, .NET) adds a `ManagedTool` trait — `detect`,
`install`, `update`, `name` — with the harness as its first impl; the wire
(`ToolStatus`, keyed by id) and the manifest do not change. The Dashboard's
bootstrap Install and the Tools screen share one job.

## Not decided
- Uninstall, and what becomes of dependent state.
- "An update is available" without running the installer — no query exists
  for it, and the harness self-updates in normal use.
- Version pinning — arrives with a tool that needs it.
