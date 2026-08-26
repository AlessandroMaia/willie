# F0 acceptance — distribution registered, daemon alive, health in the UI

Run on a machine **without** a `willie` distribution. Remove an
existing one with `wsl --unregister willie` first — this destroys
everything inside it (F0 keeps no user data there). Then run
`wsl --list --quiet; $LASTEXITCODE` in PowerShell and record the output
and the exit code in Results (zero-distribution behaviour).

Before the walk, run `just check` once with
`$env:WILLIE_TEST_DISTRO = "willie"` so the recovery test runs.

The installer is built with `just app-build` and lands in
`target/release/bundle/nsis/Willie_0.1.0_x64-setup.exe`.

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Run the NSIS installer without administrator rights | no UAC prompt; installs to `%LOCALAPPDATA%\Willie`, the per-user default (§5.3) |
| 2 | Launch Willie | window opens; WSL row green with the version; Distribution row yellow "not registered — image available"; Daemon yellow "stopped"; Doctor yellow "not run yet" |
| 3 | Click **Install distribution** | after a few seconds the Distribution row is green "registered, stopped"; `wsl --list --quiet` lists `willie`; `%LOCALAPPDATA%\Willie\data\distro\ext4.vhdx` exists; the window stays responsive and the busy indicator is visible while it runs |
| 4 | Click **Run doctor** | Daemon row green with `v0.1.0 · image 0.1.0+...`; Doctor row green; `[ok]` for unprivileged user, state dir, run dir, bubblewrap, git, curl and user namespaces; `[skip]` for `landlock LSM`; `network (api.anthropic.com)` `[ok]`, or `[fail]` behind a filtering proxy — the Doctor light stays green because that check is not required (see row 9); the window stays responsive while the VM boots |
| 5 | Click **Stop daemon** | Daemon row yellow "stopped"; `wsl --list --running` no longer lists `willie` after ~60 s |
| 6 | Close the window and reopen Willie | WSL, Distribution and Daemon rows come from a fresh `engine_status`; the Doctor row reads "not run yet" again (the report is kept in memory only); no console window ever appeared |
| 7 | `wsl --terminate willie`, then **Run doctor** | one click: Daemon row green again with a fresh report — the engine notices the daemon is gone and restarts it |
| 8 | Break it: renaming `wsl.exe` out of `PATH` is not possible — instead run `wsl --unregister willie` while Willie is open, then **Run doctor** | a red problem box with code `distro_not_registered` and the remediation to click Install distribution; the Distribution row is yellow "not registered — image available" again; nothing crashes |
| 9 | Corporate machine only: **Run doctor** | `network (api.anthropic.com)` shows `[ok]`, or `[fail]` with the proxy/CA remediation (input for F2) |
| 10 | Uninstall Willie (Settings → Apps), then run the installer again | `%LOCALAPPDATA%\Willie\data\distro\ext4.vhdx` survives the uninstall and `wsl --list --quiet` still lists `willie`; after the reinstall the Distribution row is green "registered, stopped" without clicking Install |

## Results

One line per step walked. Record the image size, the install time and
the doctor output in the notes.

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
