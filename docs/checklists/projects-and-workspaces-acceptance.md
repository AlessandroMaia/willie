# Projects and workspaces acceptance — register, sync, relocate, remove

Register a Windows checkout as a project, sync it both ways, relocate a
moved source, and remove it. Each row is a single action and the exact
command or observation that confirms it.

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
  `git init`'d with no commit is refused with `source_no_commits`.
- The workspace has no git identity of its own: `add` no longer copies
  the source checkout's `HEAD` author into it (decision 0015). Row 3a's
  commit below uses the distribution's global git identity instead,
  which Willie writes the first time any Claude Code session is
  created — from the Windows git identity, else the source checkout's,
  else it refuses. Before row 3a, either open one session first, or set
  the identity by hand once per distribution:
  `wsl -d willie --user willie -- git config --global user.name "<you>"`
  then the same with `user.email "<you>@example.com"`.

Placeholders below: `<root>` is that folder; `<slug>` is the kebab-case
name shown on a project's row; `<win>` is a checkout's Windows path;
`<win2>` is a second path you move a checkout to. Run every `wsl` command
from PowerShell as written — each is a single command. Do **not** chain
several into one `sh -lc "… && … && …"`: PowerShell and `wsl.exe`
re-split that and `sh` fails with `"&&" unexpected`.

Every project command except **Rename** is **asynchronous**: the click
returns once the daemon queues the job, and the outcome — `ready`, or a
red failure code with its remediation — appears on the same row a moment
later. Expect a short, visible delay between an action and its result.

| # | Action | Expected |
| - | ------ | -------- |
| 1 | On the **Projects** tab, type `<root>`, click **Add root**, then **Discover**; tick one repository and click **Add** (or use **Add by path** to pick a folder directly) | the row shows `preparing` (spinner + clone log tail), then `ready` a moment later; `wsl -d willie --user willie -- ls ~/projects/<slug>` lists the cloned files |
| 2 | Inspect the workspace's remotes: `wsl -d willie --user willie -- git -C ~/projects/<slug> remote -v` | one `windows` remote for fetch and push, pointing at the checkout's `/mnt/...` path, plus any remote the checkout had other than `origin` (a plain `git init` checkout shows only `windows`) |
| 3a | Create a file and commit it in the workspace — three separate commands (`git -C` avoids `cd`): `wsl -d willie --user willie -- sh -c "echo hi > ~/projects/<slug>/a.txt"`, then `wsl -d willie --user willie -- git -C ~/projects/<slug> add -A`, then `wsl -d willie --user willie -- git -C ~/projects/<slug> commit -m agent` | all three succeed; the workspace now has one new commit (`git -C ~/projects/<slug> log --oneline -1` shows `agent`) |
| 3b | Click **Send to Windows** | the row settles to `ready`; `git -C <win> show --stat HEAD` shows the `agent` commit and `a.txt` now in the Windows checkout's working tree |
| 4 | Leave an uncommitted change in the Windows checkout (`Set-Content <win>\dirty.txt hello`), make another workspace commit, then click **Send to Windows** | the row shows a red `windows_tree_dirty` failure with its remediation; `<win>` still holds `dirty.txt`, untouched |
| 5a | Commit on the **Windows** side (`git -C <win> add -A; git -C <win> commit -m win`), then click **Update from Windows** | the workspace fast-forwards; the row → `ready`; `wsl -d willie --user willie -- git -C ~/projects/<slug> log --oneline -1` shows the `win` commit |
| 5b | Now diverge both sides — one commit in `<win>` **and** one in the workspace, on the same branch — then click **Update from Windows** again | the row shows `workspace_diverged` (Willie refuses to fast-forward because both sides moved) |
| 6 | Click the project name on its row and rename it | the name changes immediately, with no spinner (Rename is synchronous); the slug and `~/projects/<slug>` are unchanged |
| 7a | Move `<win>` to a new path `<win2>` in Explorer | the row shows a `source missing` badge (the daemon can no longer find the checkout) |
| 7b | Click **Relocate** and choose `<win2>`; then, on a *different* project, click **Relocate** and choose an unrelated repository | relocating to `<win2>` clears the badge and step 3 works again; relocating to an unrelated repository shows `source_unrelated` and changes nothing |
| 8a | Remove a project **without** delete-workspace | the row disappears; `wsl -d willie --user willie -- ls ~/projects/` still lists `<slug>` (the ext4 clone is kept) |
| 8b | Click **Add** for the same checkout again | the page-level banner shows `workspace_exists`, its message naming `/home/willie/projects/<slug>` and its remediation saying to delete that directory (`rm -rf` inside the distribution) and add the checkout again |
| 8c | Delete the kept clone (`wsl -d willie --user willie -- rm -rf ~/projects/<slug>`), then click **Add** for the same checkout once more | the row shows `preparing`, then `ready` |
| 8d | Remove another project **with** delete-workspace, its workspace clean | the row disappears and `~/projects/<slug>` is gone |
| 8e | Remove a third **with** delete-workspace after leaving an uncommitted change (`wsl -d willie --user willie -- sh -c "echo x >> ~/projects/<slug>/a.txt"`) | the row shows a `workspace_dirty` failure with a one-click **Remove anyway** button; clicking it force-removes and the row disappears |
| 9 | Close and reopen Willie | the project list is restored from the daemon snapshot; no console window appeared at any point |
| 10 | Add a large repository and close Willie while its clone is still running; reopen Willie | the project shows `failed` with code `interrupted` and the remediation "remove the project and add it again" inline on the row; there is no retry button — recover by clicking **Remove**, then adding it again |

## Results

One line per row walked. Note the repositories used and any surprising
timing.

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
