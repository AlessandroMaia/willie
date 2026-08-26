# F0 acceptance — distribution registered, daemon alive, health in the UI

Run on a machine **without** a `willie` distribution. Remove an existing
one with `wsl --unregister willie` first.

The installer is built with `just app-build` and lands in
`target/release/bundle/nsis/Willie_0.1.0_x64-setup.exe`.

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Run the NSIS installer without administrator rights | no UAC prompt; installs to `%LOCALAPPDATA%\Willie` — the bundler's per-user default, which is also the engine's state directory (§5.3 says `Programs\Willie`: deviation still to settle) |
| 2 | Launch Willie | window opens; WSL row green with the version; Distribution row yellow "not registered — image available"; Daemon yellow "stopped"; Doctor yellow "not run yet" |
| 3 | Click **Install distribution** | after a few seconds the Distribution row is green "registered, stopped"; `wsl --list --quiet` lists `willie`; `%LOCALAPPDATA%\Willie\distro\ext4.vhdx` exists |
| 4 | Click **Run doctor** | Daemon row green with `v0.1.0 · image 0.1.0+…`; Doctor row green; list shows `[ok]` for unprivileged user, state dir, run dir, bubblewrap, git, curl; `user namespaces`, `landlock LSM` and `network` show ok or skip with a reason |
| 5 | Click **Stop daemon** | Daemon row yellow "stopped"; `wsl --list --running` no longer lists `willie` after ~60 s |
| 6 | Close the window and reopen Willie | status restored from a fresh `engine_status`; no console window ever appeared |
| 7 | `wsl --terminate willie`, then **Run doctor** | daemon starts again transparently |
| 8 | Break it: renaming `wsl.exe` out of `PATH` is not possible — instead run `wsl --unregister willie` while Willie is open, then **Run doctor** | a red problem box with code `wsl_command_failed` (or `daemon_exited`) and a remediation; nothing crashes |
| 9 | Corporate machine only: **Run doctor** | `network` shows ok, or fail with the proxy/CA remediation (input for F2) |

## Results

One line per step walked. Record the image size, the install time and
the doctor output in the notes.

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
