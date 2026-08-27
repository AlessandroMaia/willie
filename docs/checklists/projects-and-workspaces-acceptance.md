# Projects and workspaces acceptance — register, sync, relocate, remove

Register a Windows checkout as a project, sync it both ways, relocate a
moved source, and remove it.

Before the walk:

- The `willie` distribution is registered and its daemon is healthy (walk
  the F0 acceptance first). If a `wsl` step fails with
  `Wsl/Service/.../HCS/0x80070569` ("logon type not granted"), the WSL2
  VM right was revoked — restore it (decision 0010) before continuing.
- Build and gate, then install:
  `$env:WILLIE_TEST_DISTRO = "willie"; just check` then `just app-build`;
  install `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`.
- Have a root folder holding one or two Windows git checkouts to register.
  **Each checkout must have at least one commit** — a folder freshly
  `git init`'d with no commit is refused.
- Set a git identity for the distribution user once, so the workspace
  commit steps below can commit (the image does not set one yet — see the
  design's follow-up):
  ```powershell
  wsl -d willie --user willie -- git config --global user.email "you@example.com"
  wsl -d willie --user willie -- git config --global user.name "Your Name"
  ```

Placeholders used below: `<root>` is that folder; `<slug>` is the
kebab-case name shown on a project's row; `<win>` is a checkout's Windows
path. Run the `wsl` commands from PowerShell as written — each is a single
command; do **not** chain them with `&&` inside one `sh -lc "…"` (PowerShell
and `wsl.exe` re-split that and `sh` fails with `"&&" unexpected`).

Every project command except **Rename** is **asynchronous**: the click
returns once the daemon queues the job, and the outcome — `ready`, or a
red failure code with its remediation — appears on the same row a moment
later. Expect a short, visible delay between an action and its result.

| # | Step | Expected |
| - | ---- | -------- |
| 1 | On the **Projects** tab, type `<root>`, click **Add root**, then **Discover**; tick one repository and click **Add** (or use **Add by path**) | the row shows `preparing` (spinner + clone log tail), then `ready` a moment later; `wsl -d willie --user willie -- ls ~/projects/<slug>` lists the clone |
| 2 | Inspect the workspace's remotes: `wsl -d willie --user willie -- git -C ~/projects/<slug> remote -v` | a `windows` remote for fetch and push, pointing at the checkout's `/mnt/...` path; plus any remote the checkout had other than `origin` (a plain `git init` checkout shows only `windows`) |
| 3 | Commit in the workspace (three separate commands — `git -C` avoids `cd`, no `&&`), then click **Send to Windows**: `wsl -d willie --user willie -- sh -c "echo hi > ~/projects/<slug>/a.txt"` then `wsl -d willie --user willie -- git -C ~/projects/<slug> add -A` then `wsl -d willie --user willie -- git -C ~/projects/<slug> commit -m agent` | the row settles to `ready`; `git -C <win> show --stat HEAD` shows the commit and `a.txt` now in the checkout's working tree |
| 4 | Dirty the Windows tree (`Set-Content <win>\dirty.txt hello`), commit again in the workspace, then click **Send to Windows** | the row shows a red `windows_tree_dirty` failure with its remediation; `<win>` still holds `dirty.txt`, untouched |
| 5 | Commit on the Windows side (`git -C <win> add -A; git -C <win> commit -m win`), click **Update from Windows**; then commit on both sides to diverge and click **Update from Windows** again | first click fast-forwards the workspace (row → `ready`); second click shows `workspace_diverged` on the row |
| 6 | Click the project name on its row and rename it | the name changes immediately, no spinner (Rename is synchronous); the slug and `~/projects/<slug>` are unchanged |
| 7 | Move `<win>` to a new path `<win2>` in Explorer, click **Relocate** and choose `<win2>`; separately, **Relocate** a different project to an unrelated repository | after the move the row shows a `source missing` badge; relocating to `<win2>` clears it and step 3 works again; relocating to an unrelated repository shows `source_unrelated` and changes nothing |
| 8 | Remove one project without delete-workspace; remove another with delete-workspace on a clean workspace; remove a third with delete-workspace after dirtying it (`wsl -d willie --user willie -- sh -c "echo x >> ~/projects/<slug>/a.txt"`) | keep: the row disappears, `wsl -d willie --user willie -- ls ~/projects/` still lists `<slug>`; clean + delete: the row disappears and `~/projects/<slug>` is gone; dirty + delete: the row shows a `workspace_dirty` failure with a one-click **Remove anyway** button — clicking it force-removes and the row disappears |
| 9 | Close and reopen Willie | the project list is restored from the daemon snapshot; no console window appeared at any point |
| 10 | Add a large repository and close Willie while its clone is still running; reopen Willie | the project shows `failed` with code `interrupted` and the remediation "remove the project and add it again" inline on the row; there is no retry button — recover by clicking **Remove**, then adding it again |

## Results

One line per step walked. Note the repositories used and any surprising
timing.

| Date | Step | Result | Notes |
| ---- | ---- | ------ | ----- |
|      |      |        |       |
