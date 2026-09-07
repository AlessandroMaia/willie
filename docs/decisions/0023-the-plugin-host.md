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
| A plugin `Err`/panic degrades only that plugin vs propagates | the daemon and other plugins survive | an ordinary business refusal also trips `degraded` until a plugin's methods distinguish the two |
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
`catch_unwind`; a returned `Err` or a caught panic marks the plugin
`degraded` (surfaced in its status and the snapshot) and, for a panic,
answers `plugin_panicked`, while the daemon keeps running. The host lives
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
- An ordinary `Err` from a plugin's `handle` currently also marks it
  `degraded` — acceptable for this slice, where the only `handle` is a
  placeholder; it wants refining once profiles' real methods can tell a
  business refusal from a fault.
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
