# 0023 — The plugin host: a compiled-in registry, file-based enablement, isolated failure

- **Date:** 2026-09-07
- **Status:** accepted

## Context

The `Plugin` trait had grown its real contract — `on_enable`,
`on_disable`, `handle`, `on_event` and a `PluginCtx` — but nothing in the
daemon hosted a plugin: no registry, no record of what was enabled, no
routing, and the snapshot carried no plugin state. The host had to do four
things: enumerate the compiled-in plugins, persist which are enabled (and,
for a per-project plugin, in which projects), route `<id>.<method>` calls
to them, and keep one plugin's failure from taking the daemon down. Two
constraints framed the choices: plugins are statically compiled in (not
dynamically loaded), and SQLite is deferred for the whole slice, so the
enablement record cannot lean on an index.

## Options

| Option | For | Against |
| --- | --- | --- |
| Enablement in SQLite vs a TOML file | queryable, transactional | pulls SQLite in early, against the slice's deferral; the record is tiny and read whole |
| Unreadable `enabled.toml` fails closed vs loads empty | a corrupt record is loud | a plugin being off is already the safe state; failing the daemon over it is worse than starting with nothing enabled |
| A fault (`Internal`) or panic degrades only that plugin vs propagates | the daemon and other plugins survive | a fault surfaces as `degraded` rather than a crash, but the caller must read the status to notice |
| Degrade on any `Err` vs only on a genuine fault | any-`Err` is simplest | a coded refusal (`profile_exists`) is a legitimate "no", not a malfunction; degrading on it mislabels a healthy plugin and flaps as refusals and successes alternate |
| Plugin id separate from its method namespace vs the id *is* the namespace | a plugin could host several namespaces | needs a namespace field the trait does not carry; one namespace per plugin is enough today |

## Decision

The host holds a `Vec<Box<dyn Plugin + Send>>` from a compiled-in
`registry()`. Enablement is a file, `<state_dir>/plugins/enabled.toml`: one
`[<id>]` table per enabled plugin, `global = <bool>` for a global plugin or
`projects = [<ids>]` for a per-project one. It loads empty on any failure
(missing or unreadable) and is rewritten on every enable/disable, logging
rather than failing when it cannot be written. `enable`/`disable` validate
the requested scope against the manifest (`plugin_scope_mismatch` on a
mismatch), run the lifecycle hook, then persist. `handle` splits the method
at the first `.` into `<id>.<rest>`, so the plugin id doubles as its method
namespace — the configuration-profiles plugin, whose methods are
`profile.*`, is therefore identified as `profile`. Every call into a plugin
— `handle`, the lifecycle hooks and `on_event` — runs inside
`catch_unwind`. A plugin is marked `degraded` (surfaced in its status and
the snapshot) only by a genuine fault: a caught panic (answered
`plugin_panicked`) or a returned `PluginError::Internal`. A `Coded` refusal
or a `BadRequest` passes through as its `OpError`, code and remediation
preserved, without degrading the plugin — a refusal is a legitimate answer,
not a malfunction. The daemon keeps running in every case. The host lives
behind a mutex shared between the server (which routes `plugin.*` and
`profile.*` and merges `list()` into the snapshot) and the session path
(which feeds it `SessionStarted`/`SessionExited` as `CoreEvent`s), so the
boxed plugins must be `Send`.

## Consequences

- A plugin cannot crash the daemon or another plugin: the boundary catches
  its panic, and its failures are visible as `degraded` rather than silent.
- Enablement survives a restart with no schema and no migration; a fresh
  distribution with no `enabled.toml` reads as "no plugins enabled".
- `plugin.*`, `profile.*`, the `plugins` snapshot field and the
  `plugin_changed` event are additive; `PROTOCOL_VERSION` is unchanged.
- `degraded` tracks genuine faults only — a panic or `PluginError::Internal`
  — so a plugin's ordinary coded refusal (`profile_exists`, and once Task 4
  lands the real methods, the rest) never mislabels a healthy plugin as
  broken and never flaps as refusals and successes alternate.
- Every hosted plugin must be `Send`, since the host is shared across
  threads. The concrete plugins are, and the registry boxes them as
  `dyn Plugin + Send`.

## Not decided

- **SQLite for a plugin's own index** (usage's session index, perhaps):
  introduced when a plugin needs a query, not here.
- **Forwarding a plugin's `emit` as a `plugin.emitted` notification.** The
  `PluginCtx` carries `emit`, but the host currently swallows emissions;
  wiring them to the outbound notification stream is a follow-up.
- **A method namespace distinct from the plugin id.** One namespace per
  plugin is enough today; a plugin that needs several would add a namespace
  field to the manifest rather than overload the id.
