# Sandbox acceptance — part 1, the boundary a session runs inside

Prove by hand that a session opened from the app runs confined:
private home, project read-write, system and tools read-only, no
Windows drives beyond the project, no Windows executables, no reach
into Willie's state, an environment the sandbox builds rather than
inherits, and extra paths refused wherever they actually lead; and
that stopping, resuming and committing still work. Decision 0016
records what the kernel and the helper do.

Before the walk:

- The `willie` distribution is registered and healthy (F0 acceptance).
- **Put the current binaries in the distribution, and check that they
  are there.** Installing the app does not touch an already-registered
  distribution, and nothing warns when its binaries are behind, so a
  walk run without this step silently tests an older build and every row
  below fails for that one reason. Run `just distro-push`, which keeps
  projects and agent state, then confirm:

  ```powershell
  wsl -d willie --user willie -- sh -c 'cat /etc/willie/image-version; grep -c -a sandbox_applied /opt/willie/bin/willie-sess'
  ```

  The version must name the commit you are testing and the count must
  not be zero. If the app is already open, close and reopen it so the
  daemon restarts on the new binary.
- Install the current build if you are also walking the app's own
  screens: `$env:WILLIE_TEST_DISTRO = "willie"; just app-build`, then
  install `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`.
- One registered project with a `ready` workspace under
  `/home/willie/projects/<slug>`, Claude Code installed and logged in.
- Every command below is typed **inside the Claude Code session**,
  prefixed with `!` so Claude Code runs it as a shell command and shows
  the output — except where a step says it happens in a plain shell
  instead.

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Projects → the row's **Open session** | a terminal tab opens and Claude Code starts as before |
| 2 | `!cat /proc/1/comm` | `bwrap` — the session has its own pid namespace |
| 3 | `!ls /mnt` | `No such file or directory` |
| 4 | `!ls /init /run/WSL` | both `No such file or directory` |
| 5 | `!printenv \| grep -E "WSL_INTEROP\|WSL_DISTRO_NAME\|WSLENV\|WT_SESSION"` | no output — the sandbox clears the environment and sets back only its own allowlist, so none of the interop variables the launching shell had reach the session |
| 6 | `!cmd.exe /c echo hi` or `!/mnt/c/Windows/System32/cmd.exe` | not found / cannot execute — no Windows executable runs |
| 7 | `!ls -la ~` | only `.claude`, `.claude.json`, `.gitconfig`, `.local`, `.npm`, `.nuget`, `.cache`, `.willie`, `projects` (the policy's binds), plus `.dotnet` once the image ships that toolchain, and nothing else of the real home |
| 8 | `!touch ~/.local/bin/probe` | `Read-only file system` |
| 9 | `!echo probe > README.probe && rm README.probe` in the project | succeeds — the project is read-write |
| 10 | `!git -c core.editor=true commit --allow-empty -m probe && git log -1 --format=%an && git reset --hard HEAD~1` | the commit carries the user's name — `git.identity` works |
| 11 | `!ls /var/lib/willie /run/willie` | both `No such file or directory` |
| 12 | `!echo hi > /tmp/probe`; then in a plain `wsl -d willie` shell: `ls /tmp/probe` | absent outside — `/tmp` is private |
| 13 | Sessions → **Stop** on the running session | the row reaches `exited` within ~5 s; the tab shows the session closed |
| 14 | `wsl -d willie --user willie -- cat /var/lib/willie/sessions/<id>/events.jsonl` | reading top to bottom: a `sandbox_applied` line naming `["namespaces","mounts"]` appears *before* the `started` line — proof the mechanisms were recorded as part of the launch itself, not written after the fact; further down, the `exited` line names a signal or code |
| 15 | Projects → **Resume** | the conversation continues; rows 2–5 hold again |
| 16 | Sandbox… on the row → add `extra.paths` `/mnt/c/Users/<you>/Downloads` read-only → **Save** → open a session → `!ls /mnt/c/Users/<you>/Downloads`, then `!touch /mnt/c/Users/<you>/Downloads/probe` | the listing appears and the `touch` is refused as a read-only file system — the path is reachable exactly as granted and no more |
| 17 | Sandbox… → add `extra.paths` `/mnt/c` → **Save** | the save is refused and the dialog stays open, with the reason naming `mnt.all` — a row typed into the dialog changes nothing until it is saved |
| 18 | Sandbox… → add `extra.paths` `/run/WSL` → **Save** | the save is refused, with the reason naming `windows.interop` — the stronger of the two refusals: decision 0016 measured that this directory, not the absence of a Windows interpreter, is what actually keeps a Windows executable from running, so granting it would not just widen reach, it would undo the boundary entirely |
| 19 | In a plain `wsl -d willie --user willie` shell: `ln -s /etc /home/willie/projects/<slug>/looks-safe`; then Sandbox… on the row → add `extra.paths` `/home/willie/projects/<slug>/looks-safe` read-only, save → open a session on that project → once it fails (see Expected), remove that entry from the dialog again | the dialog accepts the save — the path named is not itself guarded; opening the session fails instead, with `sandbox_profile_invalid` naming the path it resolves to (`/etc`) — the guard is textual, so a symbolic link into a guarded location is only caught when the resolved path is checked again at launch, before anything is mounted; after the entry is removed, a session opens normally again |
| 20 | Sandbox… → turn `agent.state` off → open a session | Claude Code asks to log in — the credential is not there |

## Results

| # | Date | Result | Notes |
| - | ---- | ------ | ----- |
