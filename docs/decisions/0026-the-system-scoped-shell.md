# 0026 — The shell is scoped to one system at a time, not a menu of six equal destinations

- **Date:** 2026-09-08
- **Status:** accepted

## Context

The shell had grown a sidebar of six equal-weight destinations
(Dashboard, Projects, Sessions, Tools, Plugins, Settings) that mixed the
loop a person actually repeats all day — open, resume and watch a
session — with setup and maintenance work done rarely. The sandbox
picture was split in two: capabilities lived in a dialog on a project
row, denials lived per session on the Sessions screen, and nothing
showed a project's posture in one place. A project could hold at most
one live session (`session_already_live`), so a second task meant
stopping the first. There was no way to look at the workspace's files,
or to run a shell beside the agent, without leaving the app. None of
this was a single bug; together it read as more navigation than the
actual use, and the user's own summary was blunt: functional, but too
complex, so it falls into disuse. Several sub-decisions had to be made
together, because each changes what the others can assume: how the
sidebar itself is framed, where the rarely used screens go, whether a
project's live-session limit can be lifted safely, how far an
interactive shell can share the agent's own sandbox, how deep a
read-only file preview should go, and whether a denial should ever be
allowed with one click.

## Options

| Option | For | Against |
| --- | --- | --- |
| Reframe the sidebar as one system's work context (a selector plus that system's four screens) vs. keep six equal-weight top-level destinations and only reorganise their content | matches the thing done every day — pick a system, work in it — and removes the split between session facts and sandbox facts scattered across two of the six | a larger frontend change: routes, the shell's own layout and every screen's mount point move, not just their contents |
| Put Engine, Tools, Plugins, Profile store, Systems and Settings behind the header's settings button vs. keep them as sidebar entries beside the four system screens | keeps the sidebar exclusively about the system in front of the user; setup is rare, daily work is not | one more click to reach configuration that used to be one click away; a returning user must relearn where things live (mitigated by redirecting every old route) |
| Allow several live sessions per project vs. keep the one-live-session rule and force a stop before starting a second conversation | a second task no longer costs the first its progress; each session already runs in its own private sandbox home, so nothing about the sandbox model has to change to allow it | gives up the daemon's simplest guarantee — at most one live session to reason about per project — for a resume flow that must now name its target explicitly (`resume_from`) rather than assume "the" session |
| Run an interactive shell under the very same sandbox as an agent conversation vs. a separate, looser policy for it, or no shell at all | one policy to reason about and one code path (`prepare()`'s kind branch) instead of two; a shell is exactly as trusted as the agent conversation it sits beside in the same tab strip | it must borrow the agent harness's own `agent.state` bind to reach a login and its logs, coupling a shell session's sandbox to a decision made for an agent; its history, written into the same private tmpfs home every session gets, does not survive past the session |
| A read-only file preview beside the tree vs. an editable one, or none at all (send every look at a file to VS Code) | the tree becomes useful without leaving the app, and read-only keeps the daemon's new surface small — two read-only methods, no write path to reason about, no conflict with an editor open on the same file | a user who wants to fix a typo must still switch to VS Code; the preview is deliberately never the whole editing story |
| A denial row that only informs vs. one with a one-click "allow" action | allowing a capability is a security decision that deserves the whole catalogue in view (the capabilities drawer), not a reflex taken from a list of things that already went wrong | seeing exactly what broke and fixing it costs two screens' worth of navigation instead of one |
| The first-prompt title resolved lazily inside the daemon vs. behind the profiles/usage plugin surface | a session's display name never depends on a plugin being enabled — every session gets a title for free, the same guarantee the label (typed by the user) already has | one more piece of harness-log-reading logic lives in `willied` itself, alongside the (separately plugin-owned) usage reader that reads the same kind of log for a different purpose |

## Decision

The shell is rebuilt around one system in context rather than a flat
menu. The sidebar's top half is a system selector (name, workspace path
and branch, a live dot); its bottom half is that system's four screens
— Session, Sandbox, Profiles, Usage — the same four for every system,
none ever disabled. Everything that used to sit beside them in the
sidebar — Dashboard (renamed Engine), Tools, Plugins, the profile
store's own screen and the project registry (renamed Systems), plus
Settings — moves behind a drawer opened from the header's settings
button; every old route redirects so a saved location keeps working.
The one-live-session-per-project rule is lifted: a fresh
`session.create` no longer inspects the project's other sessions at
all, so `session_already_live` is retired (`resume_target_live` takes
over its one remaining job, refusing a resume that names a target
already open). This is safe in the sandbox: nothing about the model had
to change for it — each session already ran in its own private home, and
the harness writes one log file per conversation, so two conversations
in one workspace never collide on disk. It is *not* free for
attribution: the title reader and the usage plugin (decision 0025) both
match a session to a log by workspace and time window, which no longer
identifies one session when two are live at once in the same workspace. An interactive shell
(`zsh`) is introduced as a session kind, not a separate feature: it
runs through the same `prepare()`, the same resolved `CapabilitySet`
and the same supervisor as an agent conversation, borrowing only the
one bind that lookup actually needs — the agent harness's own
`agent.state`, since a shell is not itself an installable harness and
names none in the registry. Its prompt directory
(`/etc/willie/zsh`) is mounted read-only and tolerated absent, so an
older image without it still runs every other session; its history is
not persisted, because `$HOME` is the same private tmpfs every session
gets by default. The workspace tree gains a read-only file preview
beside it (never over it), reading through the one containment check
(`workspace::resolve_within`) that already had to exist for the tree
listing itself. The Sandbox screen aggregates a system's posture and
denial history for monitoring first; capabilities are still edited, but
in a drawer reached deliberately, with no one-click "allow" sitting
next to a denial. Finally, a session's display title — its first
prompt — is read lazily by the daemon itself
(`willied::session_title`), not by a plugin, so naming a session never
depends on one being enabled.

## Consequences

- The sidebar answers "what system am I in, and what can I do with
  it", not "which of six screens do I want" — switching systems is now
  the primary navigation gesture, and the settings drawer is reached
  rarely, by design.
- A project's sandbox story is told in one place (the Sandbox screen)
  instead of split between a dialog and a per-session list; the
  footer's governance segment gives the same facts a glance's worth of
  attention while a session is focused.
- Several live sessions cost nothing new in the sandbox model, but the
  protocol's resume contract is now more explicit: a caller names
  *which* finished session to continue (`resume_from`), and a client
  that only ever expected "the" one live session per project has one
  fewer invariant to lean on.
- Two mechanisms that leaned on that invariant are now approximate:
  a session's first-prompt title and its usage row are both matched by
  workspace and time window, so two sessions live at once in one
  workspace may share a title and show identical token and context
  figures. `resume_from` has the same shape of gap — it records and
  validates the lineage, but the harness is launched with a bare
  continue and reopens the workspace's most recent conversation, so the
  UI offers Resume on the newest finished agent session only. All three
  want the same thing: a per-session log identity (the harness's own
  conversation id, claimed by the session that owns it), which is the
  next slice, not this one.
- A shell session is exactly as sandboxed as an agent conversation,
  which also means it is exactly as limited: no persisted history, no
  reach beyond the workspace and the shared managed tools any agent
  session already has.
- The file preview and the workspace tree add a small, deliberately
  narrow surface to the daemon (two read-only methods behind one
  containment check) rather than growing into a second editor; VS Code
  stays the one place a file is actually edited.
- A returning user who had `/dashboard`, `/sessions`, `/tools` or
  `/plugins` bookmarked, or muscle-memoried, lands on the new location
  automatically; nothing breaks, but the six-screen mental model is
  gone.

## Not decided

- **A cap on live sessions per system.** None in this cut; a machine is
  bounded by its own memory. Revisit if the Sessions panel shows people
  opening many by accident.
- **Editable previews.** The preview stays read-only by design; VS Code
  remains the only editing path, one click away from the tree and from
  the preview itself.
- **Scoping the Profiles/Usage daemon-side gate to one system.** The
  daemon still refuses every `profile.*` call with `plugin_disabled`
  until *any* project has enabled the plugin, not specifically the one
  the Profiles screen is showing; the screen's "enable for this system"
  framing is a frontend accommodation today, and the daemon-side
  semantics change is a named follow-up.
- **A shell's persisted history.** Left as a product follow-up, not an
  oversight: giving it a persisted home would mean either a shared
  history across every project (wrong) or a new per-project,
  per-kind state directory the sandbox has no capability for yet.
