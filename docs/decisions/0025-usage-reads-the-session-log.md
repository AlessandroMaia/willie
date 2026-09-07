# 0025 — Usage reads the harness's own session log; the provider endpoint, the tray and push delivery wait

- **Date:** 2026-09-07
- **Status:** accepted

## Context

The usage design named three sources for `usage.snapshot` — a provider's
OAuth-backed usage endpoint (credits, 5h/7d limit windows), the harness's
own session transcripts (tokens, context fill), and matching a Willie
session to one of those transcripts by workspace and time — plus a tray
icon, threshold notifications, and real-time `usage.updated` delivery to
clients. Shipping all of it at once was never the plan; this task had to
decide what a first, robust cut looks like. Three questions needed an
answer that was not already settled: which source(s) a first cut builds
(the plugin host had no HTTP client seam yet, deliberately, per decision
0023); how the plugin learns a session's workspace and log location
without becoming daemon-aware or harness-specific; and how a client stays
current without a scheduler inside the daemon or real-time push out of
it, since forwarding a plugin's own emission was already left open by
0023.

## Options

| Option | For | Against |
| --- | --- | --- |
| Build source 1 (the OAuth usage endpoint) first vs. sources 2+3 (the session log, matched by workspace and time) first | source 1 is the number a user actually asks "how much do I have left" about | it is the fragile source by design — a network call, the provider's own client identity, a staleness flag, backoff on rate-limiting — exactly the failure surface a first cut should not have to answer for; sources 2+3 are local files Willie already reads elsewhere |
| A scheduler inside the daemon keeping a warm snapshot vs. recomputing only inside a `usage.snapshot` call, with the panel driving refresh through a light client poll | a warm snapshot answers instantly, no client polling | another actor to manage, a cadence to tune, and work spent projecting a snapshot nobody may be viewing; the plugin host has no scheduler primitive today (0023) and this slice does not need one to be correct |
| The plugin resolves each session's workspace and harness itself (a daemon-state handle in `PluginCtx`) vs. the daemon injects `_sessions`/`_home` into the request, the same seam `profile.check` uses for `_workspace` | fewer moving parts, one lookup instead of two | reaching into daemon state from inside a plugin is exactly the coupling the plugin host was built to avoid (0023) and profiles already proved the injection seam works (0024); usage should not invent a second way to get the same kind of fact |
| A usage-specific `Harness` method returning both an OAuth path and a log glob in one call vs. reusing the two already-general methods a harness needs anyway (`session_logs_dir`, `escape_workspace`) | one call instead of two, purpose-built | it bundles a source (OAuth) that is not built yet into a trait method that would ship unused; `session_logs_dir`/`escape_workspace` already answer the JSONL half for any consumer, not only usage, so no new obligation lands on a harness until source 1 is real |
| `providers` omitted from `UsageSnapshot` until source 1 exists vs. present and always empty this cut | no field until there is something to put in it | adding a field later changes the shape every existing client already deserialises; an always-present empty array costs nothing today and lets source 1 land as a pure addition |
| A session whose log cannot be read (missing harness match, unreadable directory, a corrupt or torn last line) refuses `usage.snapshot` vs. that session degrades to zero tokens, no context percentage | a refusal is an unambiguous signal something is wrong | usage is an advisory side panel, not a metering system; one session's bad log must never hide every other session's numbers, and a log torn mid-write by the very process still appending to it is the ordinary case, not an exceptional one |

## Decision

This cut ships sources 2 and 3 only, folded into one lookup rather than
two passes. `usage.snapshot`'s params are entirely daemon-filled:
`willied::handlers::usage_handle`/`enrich_usage_targets` strips any
caller-supplied `_sessions`/`_home` and injects the daemon's own —
`_sessions`, every session it knows as `{ id, project_id, workspace,
window }` (a still-live session's window left open), and `_home`, the
distro home directory — the same daemon-fills-targets seam
`profile.check` uses for `_workspace` (0024), so the usage plugin never
opens `State` and never learns a session or project exists on its own.
For each session, the plugin (`crates/willie-plugins/usage`) resolves the
first registry harness whose `Harness::session_logs_dir(home)` exists,
turns the session's `workspace` into that harness's log-directory name
via `Harness::escape_workspace`, lists its `*.jsonl` files, and picks the
one whose modified time falls inside the session's window — this is
sources 2 and 3 together, since the daemon's own session index already
carries the workspace and time window a separate `events.jsonl` match
would otherwise recover independently. A bounded tail (64 KiB) of the
picked file is read; the pure `read` module sums the newest usage-bearing
line's four token fields and derives a context percentage from a small
model-prefix table, `None` rather than a guessed denominator when the
model is unrecognised. Every failure mode along this path — no matching
harness, no log directory, an empty listing, no file overlapping the
window, an unparseable line — resolves to the same zero-token, no-context
reading, never a `PluginError` and never a panic; `usage.snapshot` itself
cannot fail. `UsageSnapshot` carries `providers: Vec<ProviderUsage>`,
always empty this cut, so source 1 is additive to the shape whenever it
lands. Nothing inside the daemon schedules a recompute: `usage.snapshot`
computes fresh on every call, and the Usage panel keeps itself current
with its own light client-side poll (`POLL_INTERVAL_MS`, 4 seconds)
rather than a push — `on_event`'s `usage.updated` emission fires on every
`SessionStarted`/`SessionExited`, but the plugin host still swallows every
plugin emission rather than forwarding it (0023's own open item), so no
client can react to it yet. Source 1, the tray, threshold notifications,
and forwarding `usage.updated` are out of this cut by the same reasoning.

## Consequences

- The usage plugin makes no network call and reaches no daemon state:
  it is testable end to end with a scratch directory standing in for a
  session's log tree, the same story 0024 already established for
  profiles.
- Every gap in the data — an unmatched harness, a missing log, a torn
  last line, a session with nothing logged yet — surfaces as "no usage
  data" for that one session, never a failed call and never a `degraded`
  plugin; a partial picture beats none for an advisory panel.
- Source 1 can be added later without changing `UsageSnapshot`'s shape
  (an already-present `providers`) or touching `session_logs_dir`/
  `escape_workspace` (a new `Harness` method carries it instead).
- No daemon actor holds a usage cadence: nothing computes a snapshot
  nobody is viewing, but also nothing is warm before the first
  `usage.snapshot` call after the panel opens, and the panel's picture
  can lag its true state by up to one poll interval.
- A harness with more than one installed on the same `home` is not
  disambiguated: `usage.snapshot` matches a session to the *first*
  registry harness whose log directory exists, not necessarily the one
  that session actually ran under — acceptable while only one harness
  ships, a documented gap once a second one does.

## Not decided

- **Source 1** — a provider's OAuth-backed usage endpoint (credits, 5h/7d
  limit windows): a fragile source needing its own cache, staleness flag
  and backoff, left for a slice that also grows the plugin host an HTTP
  client seam.
- **The tray icon and Windows notifications on a configurable
  threshold** — both need source 1's numbers before there is anything to
  show or alert on.
- **Real-time `usage.updated` delivery.** The plugin already emits it;
  the host still does not forward any plugin emission as a
  `plugin.emitted` notification (0023's own "not decided", unchanged by
  this task) — this cut is poll-only by necessity, not by preference.
- **A coalescing refresh timer** batching several session-start/exit
  emissions into one recompute — moot while nothing forwards the
  emission that would trigger it.
