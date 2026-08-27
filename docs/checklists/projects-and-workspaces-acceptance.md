# Projects and workspaces acceptance — register, sync, relocate, remove

Run on a machine with the `willie` distribution already registered and
its daemon healthy (F0 acceptance walked first). You need a folder that
holds one or more Windows git checkouts to register — a couple of small
throwaway repositories under a scratch folder are enough.

Before the walk, run `just check` once with
`$env:WILLIE_TEST_DISTRO = "willie"` so the daemon's project-operation
tests run.

The installer is built with `just app-build` and lands in
`target/release/bundle/nsis/Willie_0.1.0_x64-setup.exe`.

Every mutating project command (`add`, `sync_to_windows`,
`update_from_windows`, `relocate`, `remove`) is **asynchronous**: the
click returns as soon as the daemon has queued the job, and the outcome
— `ready`, or a failure code with its remediation — lands on the same
row a moment later, pushed by the daemon's event stream. Only `rename`
is synchronous. Expect a short, visible delay between an action and its
result throughout this walk.

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Open Projects, add a root (a folder holding a couple of repos), click **Discover** → then add one repository | Discover lists the repositories under the root; adding one shows its row `preparing` with a spinner and the clone's log tail, then — a moment later, via the live event — `ready`; `wsl -d willie --user willie -- ls ~/projects/<slug>` shows the clone |
| 2 | In the workspace, inspect its remotes from a shell | `git -C ~willie/projects/<slug> remote` lists `windows` alongside the checkout's original remotes |
| 3 | Make a commit in the workspace (from a shell), click **Send to Windows** | a moment later the row settles back to `ready`; the Windows checkout's working tree shows the change (clean-tree case) |
| 4 | Dirty the Windows tree, click **Send to Windows** | a moment later the row shows a red `windows_tree_dirty` failure with its remediation; the Windows tree is untouched |
| 5 | Commit on the Windows side, click **Update from Windows**; then diverge both sides and **Update from Windows** again | the first click fast-forwards the workspace (row settles to `ready`); the second shows `workspace_diverged` on the row |
| 6 | Rename a project inline | the name changes immediately (no spinner — `rename` is synchronous); the slug and the workspace path do not change |
| 7 | Move the source folder in Explorer; then **Relocate** to the new path; then **Relocate** a different project to an unrelated repository | after the move, the row shows a `source missing` badge; relocating to the new path clears the badge and sync works again; relocating to an unrelated repository shows `source_unrelated` on the row instead of clearing anything |
| 8 | Remove a project keeping the workspace; separately, remove a project with delete-workspace checked, once with a clean workspace and once with a dirty one | keep-workspace: the row disappears, the ext4 directory remains; clean + delete: the row disappears and the ext4 directory is gone; dirty + delete: the row shows a `workspace_dirty` failure with a one-click **Remove anyway** button — clicking it force-removes and the row disappears |
| 9 | Close and reopen Willie | the project list is restored from `state.snapshot`; no console window appeared at any point |
| 10 | Start a long clone (a large repository) and close Willie mid-clone, then reopen it | on reopen the project shows `failed` with code `interrupted` and the remediation "remove the project and add it again" shown inline on the row — there is no retry button; the recovery is to **Remove** it and **Add** it again |

## Results

One line per step walked. Record the roots used, the repositories
registered and any surprising timing in the notes.

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
