# Open a project's workspace in VS Code (Remote-WSL)

A per-project action that opens the ext4 workspace in VS Code's Remote-WSL,
so a person develops the code inside the distribution instead of editing
the Windows checkout and syncing. It mirrors the existing "Open in
Explorer" action: a Tauri command that launches a Windows program against
a path, surfaced on the project row.

## Problem

A project's real working tree is the ext4 clone at
`/home/willie/projects/<slug>`; the Windows `source` is only a sync
endpoint (decision 0011 measured DrvFs ~100× slower for `git status`).
Today the only way to edit the workspace is to open the Windows checkout,
edit there, and sync — which reintroduces the slow path and a second
source of truth. There is no one-click way to open the fast, canonical
copy in an editor.

## Goals

- A project row offers "Open in VS Code" that opens
  `/home/willie/projects/<slug>` in a VS Code Remote-WSL window on the
  `willie` distribution.
- The action is present only when VS Code is found; otherwise it is
  disabled with a reason, the way the sidebar disables what it cannot do.
- It works whatever the project's state or sandbox: editing code is not an
  agent session and does not depend on either.
- The launch mechanics are a pure, testable function; only the lookup
  touches the filesystem.

## Non-goals

- **Any sandbox interaction.** Remote-WSL runs VS Code's server in the
  distro user's own home, outside every session's private namespace. This
  is human development in the workspace, not an agent session; the sandbox
  wraps sessions, not editing. Nothing here reads or changes a policy.
- **Opening the Windows `source`.** The whole point is to develop in the
  distribution; the Windows checkout stays a sync endpoint.
- **Editors other than VS Code.** VS Code only, by the user's decision.
  Cursor and VSCodium share the Remote-WSL mechanism and could be added
  later behind the same lookup, but no code names them now.
- **A settings toggle or a configurable path.** The lookup covers the two
  standard install locations and the `PATH`; a machine that puts `code`
  elsewhere still works if it is on the `PATH`.

## Design

### The launch, as data — `apps/willie-app/src-tauri/src/lib.rs`

A Tauri command beside `open_in_explorer`, same shape (synchronous,
returns `Result<(), Problem>`):

```rust
#[tauri::command(async)]
fn open_in_editor(workspace: String) -> Result<(), Problem>
```

It resolves the VS Code launcher, then spawns it with the Remote-WSL
arguments. VS Code's Windows CLI is `code.cmd`, a batch file, so
`Command::new("code")` cannot run it directly; the command runs the
resolved `.cmd` through the shell (`cmd /c <code> --remote wsl+willie
<workspace>`), or `Command::new(<resolved code.cmd>)` when the resolver
returns a concrete path (a `.cmd` is executable by the OS when named in
full). The argument vector is a pure function so it is tested without a
VS Code install:

```rust
/// The Remote-WSL arguments that open `workspace` on the willie distro.
fn editor_argv(workspace: &str) -> Vec<String> {
    vec!["--remote".into(), format!("wsl+{}", wsl::DISTRO_NAME), workspace.into()]
}
```

`wsl::DISTRO_NAME` is `"willie"`, so this is `--remote wsl+willie
/home/willie/projects/<slug>`, VS Code's documented form for opening a
folder in a named WSL distribution.

### The lookup — `apps/willie-app/src-tauri/src/lib.rs`

```rust
/// The `code.cmd` launcher, if VS Code is installed: the PATH first, then
/// the per-user and per-machine install locations. `None` when absent.
fn locate_code() -> Option<PathBuf>
```

In order:

1. `code.cmd` resolvable on the `PATH` (the installer adds VS Code's `bin`
   to it; probe `where code` or walk `PATH` for `code.cmd`).
2. `%LOCALAPPDATA%\Programs\Microsoft VS Code\bin\code.cmd` — the default
   per-user install.
3. `%ProgramFiles%\Microsoft VS Code\bin\code.cmd` — the per-machine
   install.

This mirrors `terminal::locate_wt`, which does the same for `wt.exe`
(`%LOCALAPPDATA%\Microsoft\WindowsApps` first, then the `PATH`).

### The availability gate — `apps/willie-app/src-tauri/src/lib.rs`

```rust
#[tauri::command]
fn editor_available() -> bool { locate_code().is_some() }
```

Called once when the Projects screen mounts, like the daemon-health gate,
its boolean stored in screen state. The row reads it to enable or disable
the action. A `false` result never blocks anything else on the screen.

### The bridge and the row — `apps/willie-app/src/lib/ipc.ts`, `features/projects/*`

- `ipc.ts`: `projectsApi.openInEditor(workspace: string)` →
  `invoke("open_in_editor", { workspace })`, and a top-level
  `editorAvailable(): Promise<boolean>` → `invoke("editor_available")`.
- `projects-screen.tsx`: an `editorAvailable` state, set in a mount
  `useEffect` (the file already has several); an `openInEditor(project)`
  handler mirroring `openInExplorer`, passing `project.workspace` and
  routing a rejection through `setRowProblem`.
- `project-row.tsx`: a `DropdownMenuItem` "Open in VS Code" next to "Open
  in Explorer", calling `onOpenInEditor`. When `editorAvailable` is false
  the item is `disabled` with a `title` of "VS Code was not found on this
  machine". It stays enabled regardless of `project.state` or a running
  job — editing the workspace is independent of both.

The row gains `editorAvailable: boolean` and `onOpenInEditor: () => void`
props, wired from the screen like the other per-row callbacks.

### Errors and edge cases

| Condition | Behaviour | Code |
| --- | --- | --- |
| VS Code not found at mount | the menu item is disabled with a tooltip | — |
| VS Code vanished between mount and click | the command returns a problem on the row | `editor_not_found` |
| `code` found but the spawn fails | the OS error on the row | `editor_launch_failed` |
| a workspace whose distro folder was deleted by hand | VS Code opens and shows its own "folder not found"; Willie does not pre-check the ext4 path from Windows | — |

`editor_not_found` remediation: "install VS Code and its `code` command,
or reopen Willie so it detects a new install". `editor_launch_failed`
remediation: "try opening the workspace from VS Code directly". Both are
`Problem { code, message, remediation }`, exactly like `explorer_failed`.

## Testing

Host (Rust, `cargo test`):

- `editor_argv` produces `["--remote", "wsl+willie", "<workspace>"]`.
- `locate_code` finds a `code.cmd` planted in a fake per-user directory
  pointed at by an overridden `LOCALAPPDATA`, and returns `None` when no
  candidate exists. (Follow `locate_wt`'s test if it has one; otherwise
  gate the env-dependent case so it stays deterministic.)

Frontend (vitest):

- the "Open in VS Code" item renders and calls `onOpenInEditor` with the
  project's workspace;
- it is `disabled` when `editorAvailable` is false and enabled when true,
  including for a non-ready or busy project;
- a rejected `openInEditor` sets the row problem.

No distribution test: there is no new Linux binary, and whether VS Code's
server actually attaches is VS Code's contract, not Willie's.

## Rollout / compatibility

Frontend + Tauri host only; no protocol, daemon, image or Rust-daemon
change. Additive: a machine without VS Code sees a disabled item and
nothing else changes. `docs/ARCHITECTURE.md` gains one line noting the
action beside "Open in Explorer"; a release note records it. No decision
record: opening an editor against a path is a convenience, not a
structural choice, and the existing "Open in Explorer" set the precedent
without one.

## Open questions

None. If a second editor is ever wanted, `locate_code` becomes a small
table of (launcher name, install locations) and the menu offers each that
resolves; the argument vector is already editor-agnostic (`--remote
wsl+<distro>` is the same for every VS Code fork).
