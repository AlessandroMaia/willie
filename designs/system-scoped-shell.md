# The system-scoped shell: one system in context, sessions in tabs, the sandbox as a screen

A redesign of the desktop shell around the thing a person does every day:
work inside an agent session of one project ("system"). The window loses
its native frame for a thin header; the sidebar stops being a menu of
screens and becomes the work context (the current system and its four
screens); the Session screen hosts several live sessions and a styled shell
side by side in tabs, with the workspace tree and a read-only file preview
beside it; the sandbox gets a screen that monitors first and edits in a
drawer; everything machine-wide moves behind the header's settings button.

## Problem

The shell gives six destinations the same weight and mixes the daily loop
(open, resume and watch a session) with setup and maintenance (engine,
tools, plugins), so the navigation reads larger than the use. The sandbox
is split: capabilities live in a dialog on the project row, denials live
per session on the Sessions screen, and nothing shows a project's posture
in one place. A project can have one live session at a time
(`session_already_live`), so a second task means stopping the first. There
is no way to look at the workspace's files or to run a shell next to the
agent without leaving the app. The projects list is a destination with
per-row actions instead of the context everything else reads from. The
user's summary: functional, but too complex, so it falls into disuse.

## Goals

- **A frameless window with a thin header.** Left: collapse the sidebar
  and open the app settings. Center: the title and the current system,
  draggable. Right: minimize, maximize, close, drawn by Willie.
- **The sidebar is the work context.** On top, a system selector (name,
  workspace path, branch, a live dot; a searchable list; "Add system…")
  and a "…" menu with the system's actions. Below, the system's screens:
  Session, Sandbox, Profiles, Usage (Ctrl+1–4). Nothing global lives here.
- **The global area is the header's settings button.** Engine, Tools,
  Plugins, Profile store, Systems and Settings open from a drawer and keep
  their existing content under `/setup/*` routes.
- **Several live sessions per system, one tab each.** A tab is named by
  the session's first prompt (best-effort) and can be renamed by double
  click; the name persists. `+` starts a new agent session or a new shell.
- **A styled shell in a tab.** `zsh` runs in the workspace under the same
  sandbox as an agent session, with a Willie prompt.
- **A Sessions panel** lists live and finished sessions; a finished one is
  resumed from there into a live tab.
- **The workspace tree as a drawer**, with git status per row, and a
  read-only **file preview beside it** (never over it) that expands into a
  full-width viewer. "Open in VS Code" stays one click away for the
  workspace (tree header, system menu) and for a file (preview header).
- **A Sandbox screen that monitors first**: the posture aggregated over
  the system's sessions, three counts, and the chronological denial
  history across sessions with a per-session filter. Capabilities are
  edited in a drawer. A denial never offers a one-click allow.
- **A thin footer** with engine health and, when a session tab is focused,
  that session's sandbox posture, context meter and tokens; clicking the
  sandbox glance opens the Sandbox screen filtered on that session.

## Non-goals

- **Redesigning the Profiles and Usage screens.** They mount the existing
  profiles and usage panels, scoped to the current system. Their content
  is the next design round.
- **Redesigning the setup screens' content.** Dashboard, Tools, Plugins
  and the project registry keep their content; only their home moves.
- **A styled bootstrap installer.** A later slice.
- **Real-time `usage.updated` delivery.** The panel polls; forwarding
  plugin emissions to the client is its own follow-up.
- **Editing files in the preview.** The preview is read-only by design;
  editing is VS Code's job, one click away.
- **A cap on live sessions per system.** None in this cut; see Open
  questions.
- **Multiple windows or a new theme.** The visual system is unchanged.

## Design

### Window chrome — `apps/willie-app/src-tauri/tauri.conf.json`, `capabilities/default.json`, `src/app/shell/header.tsx`

The main window sets `"decorations": false` and `"shadow": true` (Windows
11 keeps the rounded corners and the drop shadow of a decorated window).
The header is a 36 px row: two icon buttons on the left (collapse the
sidebar — also Ctrl+B — and the settings drawer), a centre `div` marked
`data-tauri-drag-region` showing `Willie · <system>`, and three window
controls on the right that call `getCurrentWindow().minimize()`,
`.toggleMaximize()` and `.close()` from `@tauri-apps/api/window`. The
capability adds `core:window:allow-minimize`,
`core:window:allow-toggle-maximize`, `core:window:allow-close`,
`core:window:allow-start-dragging` and `core:window:allow-is-maximized`.
The maximize glyph reflects `isMaximized()`. No native title bar remains.

### The current system — `crates/willie-engine/src/config.rs`, `apps/willie-app/src/store/use-current-system.ts`

The selected system is a UI preference, so it lives in `engine.toml`:

```toml
[ui]
current_project = "proj_01J8W4QZ6M"   # absent → the first project
```

`EngineConfig` gains `#[serde(default)] pub ui: Ui { current_project:
Option<ProjectId> }`; two engine methods `ui_prefs() -> UiPrefs` and
`set_ui_prefs(UiPrefs)` back two Tauri commands. The store
`useCurrentSystem()` resolves the id against the snapshot's projects: the
preferred one if it still exists, else the first, else `null` (no systems
yet: the screens show an empty state pointing at "Add system…"). Every
system screen reads the current system from this store and never from a
route parameter, so switching systems keeps the screen.

### The sidebar — `apps/willie-app/src/app/shell/app-sidebar.tsx`, `system-selector.tsx`, `system-actions-menu.tsx`, `app/routes.ts`

The selector button shows the glyph (two letters), the name, `<workspace
path> · <branch>` and a live dot when the system has a live session; it
opens a menu with a search field, one row per system (live dot, name,
branch) and "Add system…" (opens the settings drawer on Systems). The "…"
button beside it opens the system's actions: Open in VS Code (WSL), Open
in Explorer, Sync from Windows, Rename, Relocate, Remove — the existing
commands and dialogs, moved off the project row.

`ROUTES` becomes the four system screens with Ctrl+1–4:
`/session`, `/sandbox`, `/profiles`, `/usage`. The global entries move to
`SETUP_ENTRIES`: `/setup/engine` (the health screen, today's Dashboard),
`/setup/tools`, `/setup/plugins`, `/setup/profile-store` (the profile
store's create/edit/sync surface), `/setup/systems` (the project registry:
add, discover, roots), `/setup/settings` (theme). The old paths
(`/dashboard`, `/projects`, `/sessions`, `/tools`, `/plugins`) redirect
to their new homes so a saved location keeps working.

The sidebar fills the shell's body row, never the window. It is
`position: fixed` inside that row's containing block, so the height it
ships with (`h-svh`) would leave it a header plus a status bar too
tall — covering the status bar's left edge and giving the window a
scrollbar. The window itself never scrolls: the header and the status
bar are fixed rows and each screen scrolls inside the centre pane.

### The settings drawer — `apps/willie-app/src/app/shell/setup-drawer.tsx`

A Sheet from the right listing the six setup entries with a one-line
description each; choosing one navigates to its route and closes the
drawer. The Plugins screen loses the profiles and usage panels it mounts
today (those become the system's Profiles and Usage screens); the project
registry screen loses its per-row session and sandbox actions (the Session
and Sandbox screens own them) and keeps add, discover, roots, rename,
relocate and remove.

### Several live sessions — `crates/willied/src/sessions.rs`, `crates/willie-proto/src/session.rs`

The one-live-session-per-project rule goes. A fresh `session.create` no
longer looks at other sessions of the project. A resume names its target:

```rust
pub struct CreateParams {
    pub project_id: ProjectId,
    #[serde(default)] pub resume: bool,
    /// The finished session to continue. Absent with `resume`, the
    /// project's most recent terminal session (today's behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from: Option<SessionId>,
    #[serde(default)] pub kind: SessionKind,        // see below
    // … existing fields (git_identity)
}
```

`resume_decision` keeps its fail-closed shape but its `live` input is
replaced by the target's state: a `resume_from` that does not exist is
`resume_target_not_found`; one that is not terminal is
`resume_target_live` (it is already a tab). `session_already_live` is no
longer produced.

Why this is safe for the harness: each session runs in its own private
sandbox home; the shared `agent.state` bind holds the login and the
harness's own per-session logs, which the harness writes one file per
conversation, so two conversations in one workspace do not collide on
disk. Attributing a log to one session (its title, its usage) is by
workspace and time window until per-session log identity lands; two
concurrent sessions in one workspace may share those figures.

### Names — `crates/willie-core/src/session.rs`, `crates/willied/src/sessions.rs`, `session_title.rs`

`Session` gains two optional strings, both `#[serde(default,
skip_serializing_if = "Option::is_none")]`:

- `title`: the session's first prompt, best-effort, set once by the
  daemon after the session starts running. `session_title::first_prompt`
  finds the harness log for the session (`Harness::session_logs_dir(home)
  / escape_workspace(workspace)`, the newest `*.jsonl` whose modified time
  is at or after the session's start), reads its first record whose
  `type` is `user` (or whose `message.role` is `user`), and returns its
  text trimmed to 80 characters. Anything missing or malformed is `None`;
  the UI then shows the short id. The read is bounded (first 64 KiB) and
  never fails the session.
- `label`: what the user typed. `session.rename { id, label }` appends a
  `Renamed { label }` `SessionEventKind` to the session's append-only
  event log (so a restart folds it back), updates the in-memory session,
  and emits `session_changed`. An empty label clears it. An unknown id is
  `session_not_found`.

The UI shows `label ?? title ?? short id` everywhere a session is named.

### Shell sessions — `crates/willie-core/src/session.rs`, `crates/willied/src/sessions.rs`, `distro/provision.sh`, `distro/zsh/.zshrc`

```rust
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind { #[default] Agent, Shell }
```

`SessionSpec` and `Session` carry `#[serde(default)] pub kind:
SessionKind`, so a spec written before this design re-adopts as `Agent`.
A `Shell` session is created like an agent session — same project checks,
same resolved `CapabilitySet`, same supervisor, same sandbox plan — but its
launch is built by the daemon, not the harness:

```rust
/// `argv`/`env` for an interactive zsh in `workspace` under the session's
/// private home. The env is the same allowlist the harness gets; ZDOTDIR
/// points at Willie's own zsh configuration so the prompt is ours.
fn shell_launch(workspace: &Path, home: &Path) -> Launch
// argv: ["/usr/bin/zsh", "-l"]; env: PATH, HOME, USER, TERM, COLORTERM,
// LANG, TZ (if set), ZDOTDIR=/etc/willie/zsh, WILLIE_WORKSPACE=<workspace>
```

`harness` on a shell session is `"zsh"`; a shell session cannot be
resumed (`harness_cannot_resume`) and is excluded from usage (the
daemon's `_sessions` enrichment sends `Agent` sessions only). If
`/usr/bin/zsh` is absent the create is refused fail-closed with
`shell_unavailable` ("rebuild and reinstall the distribution").

The image installs `zsh` (`distro/provision.sh` package list) and
`distro/zsh/.zshrc` to `/etc/willie/zsh/.zshrc` (mode 0644): history in
the private home, `cd "$WILLIE_WORKSPACE"`, and a two-line prompt —
`<user>@willie <cwd> <branch> ❯` — built from `git rev-parse
--abbrev-ref HEAD` with plain 256-colour escapes. The login shell of the
`willie` user stays `bash`; the session spawns zsh explicitly.

### The workspace tree and the file preview — `crates/willied/src/workspace.rs`, `handlers.rs`, `apps/willie-app/src/app/shell/tree-drawer.tsx`, `features/session/file-preview.tsx`

Two read-only daemon methods, both resolved against the project's
workspace and refused when the path escapes it:

```rust
project.tree      { id, path? }  →  { entries: [ { name, kind: "dir"|"file", git?: "M"|"A"|"D"|"?"|"R" } ] }
project.read_file { id, path }   →  { content: String, truncated: bool }
```

`workspace::resolve_within(workspace, rel) -> Result<PathBuf, WsError>`
is the one containment check: `rel` must be relative, must not contain a
`..` component, and the canonicalised result must start with the
canonicalised workspace (a symlink that points outside is refused);
otherwise `path_outside_workspace`. `project.tree` lists one directory
level (sorted: directories first, then names), skipping `.git`; its git
column comes from one `git status --porcelain=v1 --untracked-files=all`
run in the workspace per call, parsed by a pure function into a map of
path → flag, so a directory shows a flag when anything under it changed.
`project.read_file` reads at most 512 KiB (`truncated: true` past that),
refuses a file whose first 8 KiB contain a NUL byte with `file_not_text`,
and returns UTF-8 with invalid sequences replaced. The check
canonicalises and then opens by path rather than by file descriptor, so
a writer inside the workspace racing a symlink swap between those two
steps is an accepted residual (the fix, if ever wanted, is opening path
component by component, or comparing the opened file's device/inode
afterwards); a target that is not a regular file — a directory, a
FIFO — is refused the same way, as `file_not_text`.

The tree drawer slides from under the sidebar over the left edge of the
centre (toggled by the button at the left of the tab strip, Esc closes),
loads folders lazily on expand, shows the git flag per row, and carries
"Open in VS Code" for the workspace in its header. Clicking a file opens
the preview: a read-only, line-numbered panel that starts at the right
edge of the tree when the tree is open (`| tree | file |`) and at the
centre's left edge when it is closed; its header shows the path, a
"read-only" badge, an expand toggle (the file fills everything right of
the tree), "Open in VS Code" for that file, and close. Esc closes the
preview first, then the tree.

`open_in_editor` gains an optional file: the pure `editor_argv(workspace,
file: Option<&str>)` appends the file after the workspace, which opens the
folder window with the file active. The file arrives workspace-relative
and is joined onto the workspace before it goes on the command line —
VS Code resolves a relative argument against the launching process's own
directory, which is on the Windows side and names nothing in the distro.

### The Session screen — `apps/willie-app/src/features/session/session-screen.tsx`, `sessions-panel.tsx`

The tab strip holds, in order: the tree toggle; one tab per live agent
session of the current system (a green dot and the name); the shell tabs
(`$ zsh`); `+`; and, at the right end, the Sessions panel button. Each
tab hosts a `SessionTerminal` for its session id; tabs stay mounted while
hidden so switching does not lose scrollback. `+` is a dropdown: "New
session" (`session.create { project_id }`) and "New zsh" (`kind:
"shell"`). Double-clicking a session tab turns its name into an inline
field; Enter calls `session.rename`, Esc cancels, blur commits.

The Sessions panel is a Sheet: live sessions with "Open" (focus the tab),
finished ones with the name and when they finished. "Resume"
(`session.create { project_id, resume: true, resume_from }`) adds a live
tab and focuses it, and is offered on the newest finished *agent*
session only: `resume_from` records and validates the lineage, but the
harness is launched with a bare continue and always reopens the
workspace's most recent conversation, so a Resume anywhere else would
name one session and open another. The other rows say so. With no live session the centre shows an empty
state: "New session" and, when a finished one exists, "Resume
<its name>".

### The footer — `apps/willie-app/src/app/shell/status-bar.tsx`, `store/use-focused-session.ts`

The status bar keeps engine health, the first problem, the daemon version
and the live-session count, and gains a governance segment shown while a
session tab is focused: `sandbox: <applied…> · <n> denied`, a context
meter and the token count, read from the focused session's `sandbox` and
from `usage.snapshot`'s row for it. The segment is a link to
`/sandbox?session=<id>`. On a shell tab the segment hides; on the Sandbox
screen it shows the system's aggregate.

### The Sandbox screen — `apps/willie-app/src/features/sandbox/sandbox-screen.tsx`, `sandbox-drawer.tsx`, `lib/domain/sandbox.ts`

Monitoring first. The header names the system and holds "Edit
capabilities". Below it, the posture as chips: every mechanism in the
union of the system's sessions' `sandbox.applied` in the ok tone, every
one in the union of `unavailable` or `degraded` in the warning tone. The
chips carry the mechanism name only: `SandboxState` puts mechanism names
on the wire, not the `sandbox_degraded` event's message, so there is no
per-mechanism reason to show on hover. Three counts: syscalls denied,
terminal sequences
denied, sessions covered. Then the history: every `Denied` of every
session of the system flattened into rows — class, name with a one-line
explanation from a small table keyed by class and name (unknown names get
the class's generic line), the session's name, `last_at`, count — sorted
by `last_at` descending, with chips filtering by session ("all sessions"
plus one per session that has denials). `?session=<id>` preselects a
chip. A system with no denials says so in place of the table.
`lib/domain/sandbox.ts` holds the pure aggregation (`posture(sessions)`,
`denialRows(sessions)`, `counts(rows)`).

"Edit capabilities" opens the existing Sandbox dialog's content in a Sheet
(`sandbox-drawer.tsx`; the dialog file is removed): the catalogue with a
switch per capability, the extra paths with their mode, Cancel and Save
calling `project.set_sandbox` as today, and the line that repository
configuration can only tighten the profile. There is deliberately no
"allow" action on a denial row: a denial is information, allowing goes
through the drawer with the whole catalogue in view.

### Errors and edge cases

| Situation | Behaviour | Code |
| --- | --- | --- |
| `resume_from` names an unknown session | refused | `resume_target_not_found` |
| `resume_from` names a live session | refused ("it is already open") | `resume_target_live` |
| resume of a shell session | refused | `harness_cannot_resume` |
| shell session with no `/usr/bin/zsh` in the image | refused fail-closed, nothing spawned | `shell_unavailable` |
| `session.rename` on an unknown id | refused | `session_not_found` |
| tree/read path with `..`, absolute, or resolving outside the workspace | refused | `path_outside_workspace` |
| `project.read_file` on a binary | refused | `file_not_text` |
| `project.read_file` past 512 KiB | content cut, `truncated: true` | — |
| tree of a directory that vanished mid-listing | empty `entries` | — |
| first-prompt title cannot be read | `title` absent; UI shows the short id | — |
| no system registered | selector says "No systems yet"; screens show an empty state with "Add system…" | — |
| the preferred `current_project` was removed | the first project is selected; the preference is rewritten | — |
| VS Code absent | the three "Open in VS Code" affordances are disabled with the existing reason | — |
| `session_already_live` | no longer produced; documented as retired | — |

## Testing

- **Rust, host** (`#[cfg(test)]`): `SessionKind` defaults on old specs;
  `Renamed` folds into `label`; `resume_decision` with `resume_from`
  (missing, live, terminal, absent → latest); `shell_launch` argv/env;
  `resolve_within` (`..`, absolute, symlink escape, a clean relative path);
  the porcelain parser (M/A/D/?/R, a path under a directory marks the
  directory); `read_file` cap and NUL refusal; `first_prompt` over fixture
  strings (user record found, malformed lines skipped, none → `None`);
  `editor_argv` with and without a file; `EngineConfig` `[ui]` round-trip.
- **Rust, inside the distribution** (`WILLIE_TEST_DISTRO=willie just
  check`): a shell session spawns under the sandbox and its terminal shows
  the Willie prompt; two agent sessions live at once in one project;
  `project.tree`/`project.read_file` against a real workspace with a
  symlink pointing outside.
- **Frontend** (Vitest, mocking only `@/lib/ipc`): the header's controls
  call the window API; the selector switches and persists the system;
  Ctrl+1–4 reach the four screens; N live sessions render N tabs and each
  hosts a terminal for its id; `+` offers session and zsh; the Sessions
  panel resumes with `resume_from`; double-click rename calls
  `session_rename`; the tree loads lazily and the preview opens beside it,
  expands, and closes on Esc; the Sandbox screen aggregates posture,
  sorts and filters denials, shows the empty state, and opens the
  capabilities drawer; the footer segment follows the focused tab and
  links to the filtered Sandbox screen; old routes redirect.

## Rollout / compatibility

- `tauri.conf.json`: `decorations: false`, `shadow: true`; the capability
  file gains the window permissions. Nothing else on the Windows side.
- `engine.toml`: a new optional `[ui]` table; absent means "first
  project". Older builds ignore it.
- Protocol (additive): `Session.kind`, `.label`, `.title`;
  `CreateParams.kind`, `.resume_from`; new methods `session.rename`,
  `project.tree`, `project.read_file`; new `SessionEventKind::Renamed`.
  `SessionSpec.kind` defaults to `Agent`, so existing session directories
  re-adopt unchanged. `session_already_live` is retired: documented as no
  longer produced, its row kept so a client that matched it knows why it
  stopped appearing.
- The image needs a rebuild for `zsh` and the prompt file (`just
  distro-build` / `distro-push`); without it, "New zsh" is refused with
  `shell_unavailable` and the rest of the shell works.
- Routes: `/dashboard`, `/projects`, `/sessions`, `/tools`, `/plugins`
  redirect to the new homes.

## Open questions

- **A cap on live sessions per system.** None now; a machine is bounded
  by its own memory. Favoured: leave it uncapped, revisit if the Sessions
  panel shows people opening many by accident.
- **Where the first-prompt reader lives.** Favoured: a small reader in
  the daemon (`session_title.rs`) using the harness methods, so a title
  never depends on a plugin being enabled.
- **Git flags on directories.** Favoured: a directory shows the flag of
  any changed path beneath it, computed from the same porcelain run.
