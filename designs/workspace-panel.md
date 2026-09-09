# The workspace panel: the tree, the file and the shell beside any screen

The workspace tree, the read-only file preview and one interactive shell
move out of the Session screen into a panel at the window's right edge,
opened from the header on any screen.

## Problem

The tree and the shell are things you reach for *while* doing something
else, and both are locked inside the Session screen. The tree opens from
that screen's tab strip and slides in from the left, over the terminal it
is meant to complement. The shell is a session tab, so running one costs
a tab beside the agent conversations and makes the Session screen own a
surface that has nothing to do with an agent. Neither is reachable from
Sandbox, Profiles or Usage: to look at a file while reading a denial, you
leave the screen you are reading.

The Session screen is the screen for agent work. It should not be the
only door to the workspace.

## Goals

- The tree, the file preview and one shell live in a panel at the right
  edge, opened by a header button on **any** of the four screens.
- The panel sits beside the screen and pushes it, never over it: the
  terminal stays usable while the tree is open.
- One shell per system, started the first time the tab is opened and
  reused after; changing system changes the shell.
- The Session screen keeps agent sessions and nothing else.
- No daemon or protocol change: the panel is a new arrangement of calls
  that already exist.

## Non-goals

- **Several shells per system.** One. The `+` that opened a second goes.
- **A resizable panel.** Fixed width in this cut; see Open questions.
- **Editing in the preview.** Still read-only; VS Code is one click away.
- **Moving the Sessions panel.** The finished-session list belongs to the
  Session screen and stays there.
- **Changing what a shell session *is*.** Same `SessionKind::Shell`, same
  sandbox, same event log. Only its surface moves.

## Design

### The panel — `apps/willie-app/src/app/shell/workspace-panel.tsx`

A column **inside** `SidebarInset`, beside the scroll area that hosts
the route, so the pane reads `| screen | panel |` within the one
rounded card the `inset` sidebar gives the screen. Opening the panel
narrows the screen instead of covering it; closed, it renders nothing
and the screen takes the width back.

Inside the card rather than beside it, for two reasons: the card's
rounding and its `overflow-hidden` then clip the panel's corners for
free, and the ground the card floats on stays a ground — two cards with
a strip of it between them would read as two windows. A hairline
`border-l` separates the panel from the screen, the way the tree drawer
already separates itself from the terminal.

It is a child of the *shell*, never of a route: that is what takes it
out of the Session screen and puts it on all four.

Its head is one row: the tab strip on the left, the workspace's current
branch on the right. Three tabs, from the registry's `tabs` (Base UI
variant, added with `pnpm dlx shadcn@latest add tabs` — it is not
vendored yet):

```
| 🗀 Tree | 📄 File | >_ Shell |                        ⑂ main |
```

- **Tree** — the listing that `tree-drawer.tsx` renders today, moved
  whole: `project_tree`, one row per entry, git status per row, folders
  expanding in place. Clicking a file opens it in File and activates
  that tab.
- **File** — the read-only preview `file-preview.tsx` renders today,
  moved whole: `project_read_file`, its truncation notice and its "Open
  in VS Code" for that one file. Empty until a file is picked, and it
  says so.
- **Shell** — one `SessionTerminal` bound to the system's shell session
  (below).

`tree-drawer.tsx` and `file-preview.tsx` lose their positioning and
their open/close state and become the panel's two tab bodies; their
listing, preview and git-status logic move unchanged.

### The toggle — `apps/willie-app/src/app/shell/header.tsx`

A button at the right end of the header, before the window controls,
mirroring the sidebar toggle at the left end: same `icon-sm` ghost
button, `aria-label="Workspace panel"`, `aria-pressed` reflecting the
panel's state, and `Ctrl+J` beside `Ctrl+B` in `app/hotkeys.tsx`. It is
disabled when there is no current system, since the panel has nothing to
list and nothing to run.

### State — `apps/willie-app/src/store/use-workspace-panel.ts`

One module-level singleton, the same idiom as `use-setup-drawer` and
`use-system-actions`, replacing `use-tree-drawer` and `use-file-preview`:

```ts
interface WorkspacePanelState {
  open: boolean;
  tab: "tree" | "file" | "shell";
  /** The workspace's branch as of the last root listing. */
  branch: string | null;
  /** The file the File tab is showing, or null. */
  file: { path: string } | null;
}
```

Open state and active tab are app-wide, like the sidebar's. `branch` and
`file` reset on a system change: they describe one workspace, and
carrying them across systems shows the previous system's file under the
next system's name.

The `expanded` flag `use-file-preview` carries goes: the preview
expanded to full width because it shared the pane with the tree, and in
a tab it always has the panel's whole width.

### The shell session — `workspace-panel.tsx`, `lib/ipc.ts`

One live shell per system. When the Shell tab is first activated for a
system, the panel looks in the snapshot for a live session of that
system with `kind: "shell"`:

```
Shell tab activated
  ├─ live shell for this system in the snapshot?
  │     yes → attach its terminal (session_terminal_open)
  │     no  → session_open(projectId, "shell") → attach
  └─ system changed → detach, resolve again for the new system
```

The panel never stops a shell: leaving it running between visits is what
makes it worth having. It ends with the daemon, or from the Sessions
panel's stop, exactly as today.

### What the Session screen loses — `features/session/session-screen.tsx`, `session-tabs.tsx`

- The tree toggle leaves the tab strip; `useTreeDrawer` goes with it.
- `+` loses its dropdown and becomes a plain "New session" button:
  "New zsh" has no tab to open any more.
- The strip renders agent sessions only, so `sortForTabs`' shell branch
  goes and the strip's "a shell tab always reads `$ zsh`" rule with it.
- `SessionsPanel` is untouched: a finished shell already shows "no
  conversation" instead of Resume, which stays right — the panel starts
  a fresh shell rather than resuming one.

### The footer — `app/shell/status-bar.tsx`

`showGovernance` drops its `kind === "agent"` check. It existed because
a shell tab could be focused and a shell has no conversation to report;
with shells out of the tabs, the focused session is always an agent one.
The panel's shell does not drive the footer: the footer follows the
screen, and the panel is beside it.

### Errors and edge cases

| Situation | Behaviour |
| --- | --- |
| No current system | The header button is disabled; the panel cannot open. |
| Daemon stopped | The Tree tab shows the `ProblemAlert` for the failed `project_tree`, with its remediation. The Shell tab does not try to start a session. |
| `session_open` refused (`sandbox_backend_missing`, `sandbox_problem`) | The Shell tab shows the refusal and its remediation, never an empty terminal. |
| Workspace missing (`workspace_missing`) | The Tree tab shows the daemon's problem; the Shell tab is not offered. |
| Terminal attached but the session exits | The Shell tab shows the exit and offers to start another, the same wording a session tab uses today. |
| File unreadable or binary | Unchanged: the preview's existing truncation and error handling move with it. |
| System changed while the panel is open | The panel stays open on the same tab; `branch` and `file` clear; the shell re-resolves for the new system. |

## Testing

| Goal | Test | Where |
| --- | --- | --- |
| The button opens the panel on every screen | `the_panel_opens_from_the_header_on_every_screen` | `app/__tests__/workspace-panel.test.tsx` |
| The panel is a child of the shell, not of a route | `the_panel_is_mounted_by_the_shell_not_by_the_session_screen` | same |
| Clicking a file activates the File tab | `picking_a_file_in_the_tree_opens_the_file_tab` | same |
| One shell per system, reused | `the_shell_tab_reuses_the_systems_live_shell` | same |
| A refused shell shows the refusal | `a_refused_shell_shows_its_remediation_not_a_terminal` | same |
| A system change clears the file and the branch | `changing_system_clears_the_panel_to_the_new_workspace` | same |
| The Session screen no longer offers zsh or the tree | `the_tab_strip_has_no_tree_toggle_and_no_new_zsh` | `features/session/__tests__/session-tabs.test.tsx` |
| The window still never scrolls with the panel open | `measure.mjs`, panel open | `.claude/skills/ui-review` |

The existing `tree-drawer` and `file-preview` tests move to the panel's
suite with their assertions intact: what they prove about listing,
git status and truncation does not change with the surface.

## Rollout / compatibility

Frontend only. No `willie-proto` change, no daemon change, no config
key: `session.create` with `kind: "shell"` and `project.tree` are the
same calls the Session screen makes today.

`--tree-width` becomes `--panel-width` in `globals.css`. No persisted
state changes, since neither the tree drawer's nor the preview's state
was ever written to disk.

A user who had a shell session tab open when they upgrade finds it in
the Sessions panel's finished list once it ends; a live one keeps
running in the daemon and the panel adopts it, since the panel resolves
by `kind` and system, not by who started it.

## Open questions

- **Resizable width.** Fixed for now, at the `--panel-width` the tree
  uses today (280px). A drag handle is the obvious follow-up; leaving it
  out keeps this cut to the move.
- **Does opening the panel on the Shell tab start a shell?** Currently
  specified as yes — activating the tab starts one. The alternative is
  an explicit "Start shell" button, which costs a click every time but
  never starts a session you did not ask for.
- **`Ctrl+J`.** Proposed to mirror `Ctrl+B`. Nothing else claims it
  today.
