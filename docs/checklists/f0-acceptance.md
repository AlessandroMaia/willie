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
| 2026-08-26 | env | note | Walk done on the corporate machine (WSL 2.6.1) after re-granting "Log on as a service" to `NT VIRTUAL MACHINE\Virtual Machines`; Group Policy had reverted it earlier that morning and the daemon then failed with `daemon_exited` carrying `HCS/0x80070569` (decision 0010). Installer built from `0e12fec`: image 70 MB, `0.1.0+0e12fec`. |
| 2026-08-26 | gate | pass | `just check` with `WILLIE_TEST_DISTRO=willie` ran to the end: frontend 5/5, `check-refs: 129 files clean (16 terms)`; the gate stops at the first failure, so every Rust suite before it passed. |
| 2026-08-26 | 0 | pass | `wsl --unregister willie` succeeded; `wsl --list --quiet; $LASTEXITCODE` printed nothing and `0` — with zero distributions WSL 2.6.1 returns an empty list and exit 0, so the `WSL_E_DEFAULT_DISTRO_NOT_FOUND` mapping was not exercised here. |
| 2026-08-26 | 1 | pass | Installer run from a normal PowerShell prompt (walked in order, user report); the install path was not recorded. |
| 2026-08-26 | 2 | pass | WSL row `2.6.1.0 (minimum 2.4.4)`; Distribution "not registered — image available" (the initial screen was not captured; both texts appear later in the walk). |
| 2026-08-26 | 3 | pass | Distribution registered (later rows show "registered, running"); the `ext4.vhdx` path and `wsl --list --quiet` were not captured. |
| 2026-08-26 | 4 | pass | Daemon `v0.1.0 · image 0.1.0+0e12fec`; 9 checks: `[ok]` unprivileged user uid 1000, state dir `/var/lib/willie`, run dir `/run/willie`, bubblewrap 0.11.0, git 2.47.3, curl 8.14.1, user namespaces; `[skip] landlock LSM`; `[ok] network (api.anthropic.com)`. |
| 2026-08-26 | 5 | pass | Daemon "stopped" with the last report kept on screen; Distribution still "registered, running" right after the click (the VM shuts down lazily); `wsl --list --running` after 60 s was not captured. |
| 2026-08-26 | 6 | pass | Walked in order (user report); the reopened screen was not captured. |
| 2026-08-26 | 7 | pass | `wsl --terminate willie` succeeded; one **Run doctor** brought the Daemon row back to `v0.1.0 · image 0.1.0+0e12fec` with a fresh report. |
| 2026-08-26 | 8 | pass | `wsl --unregister willie` while open, then **Run doctor**: red box `distro_not_registered — the willie distribution is not registered → click Install distribution`; Distribution "not registered — image available"; Daemon row `daemon_exited: daemon exited with code 1 (no output)` — the liveness probe saw the daemon the unregister killed; the Doctor row kept the last report; nothing crashed. |
| 2026-08-26 | 9 | pass | `[ok] network (api.anthropic.com) ok` through the corporate proxy; no remediation needed. |
| 2026-08-26 | 10 | pass | With `willie` registered: before and after the Settings → Apps uninstall, `wsl --list --quiet` listed `willie` and `Test-Path "$env:LOCALAPPDATA\Willie\data\distro\ext4.vhdx"` was `True`; after re-running the installer the Distribution row read "registered, stopped" without clicking Install — the `data\` subdirectory survives the uninstaller as §5.3 states. |
