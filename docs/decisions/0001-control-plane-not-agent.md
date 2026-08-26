# 0001 — Willie is a control plane for agent CLIs, not an agent

- **Date:** 2026-08-25
- **Status:** accepted

## Context

Agent CLIs (Claude Code first) already do the reasoning and tool use well,
and the user pays for them through subscriptions. What is missing on a
Windows workstation is the layer around them: an environment they can run
in safely, per-project control over their configuration, visibility of
usage and cost, and a single place to start, watch and stop sessions.

## Options

| Option                          | For                                   | Against                                          |
| ------------------------------- | ------------------------------------- | ------------------------------------------------ |
| Control plane over existing CLIs | small core; reuses subscriptions; every CLI improvement is free | depends on CLI behaviour we do not control |
| Own agent loop                  | full control of tools and prompts     | re-implements the CLIs; loses subscription access; large surface |
| Context/memory platform only    | narrow                                | cannot manage sessions or sandboxes              |

## Decision

Willie prepares, launches, observes and governs agent CLIs. It never calls
an LLM API on its own. The unit of management is the project; capabilities
attach as plugins.

## Consequences

- The fixed core stays small: engine, projects, sessions, plugin contract,
  UI. Everything else is a plugin.
- Every supported CLI is described by a capability matrix so the rest of
  the system never branches on a CLI's name.
- If an own agent loop is ever wanted, it enters as one more harness in
  that matrix, not as a new core.

## Not decided

Which harness comes after Claude Code, and whether an own loop is ever
worth it. Both stay in the growth list, not the roadmap.
