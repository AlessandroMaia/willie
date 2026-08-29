# Sessions acceptance — open, attach and stop from the app

Prove the sessions **engine + app** half by hand, entirely through
buttons: install the harness from the Dashboard, open a Claude Code
session from a project's row into a real Windows Terminal tab, watch it
in the Sessions screen, attach a second tab, stop it, exercise the
no-`wt.exe` fallback and the fail-closed refusals inline in the UI, and
confirm a restart restores the live list. This walk **supersedes**
`sessions-linux-core-acceptance.md` for the product path — that one
stays as the daemon-driven proof of the wire protocol, with no app
involved; this one is what a user actually clicks.

Before the walk:

- The `willie` distribution is registered and its daemon is healthy
  (walk the F0 acceptance first). If a `wsl` step fails with
  `Wsl/Service/.../HCS/0x80070569` ("logon type not granted"), the WSL2
  VM right was revoked — restore it (decision 0010) before continuing.
- Install the current build:
  `$env:WILLIE_TEST_DISTRO = "willie"; just app-build`; install
  `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`.
- Have one registered project with a `ready` workspace (walk
  `projects-and-workspaces-acceptance.md` rows 1–2, or reuse one); note
  its workspace folder name as `<slug>` and its Windows checkout path as
  `<win>`.
- **Remove the harness** so the Dashboard's Install row has something to
  do: `wsl -d willie --user willie -- rm -f /home/willie/.local/bin/claude`
  (the first path a session looks up on; if an earlier walk installed it
  somewhere else on `PATH`, remove that copy instead).
- For the fallback row (row 10 below), know how to hide `wt.exe`: rename
  `%LOCALAPPDATA%\Microsoft\WindowsApps\wt.exe` to `wt.exe.bak`, and do
  the same for any `wt.exe` earlier on `PATH`. Rename it back afterwards.

Placeholders: `<proj>` the project used throughout; `<slug>` its
workspace folder name; `<win>` its Windows checkout path.

| # | Action | Expected |
| - | ------ | -------- |
| 1 | Open the **Dashboard** | the doctor's `Claude Code` check shows `[fail]` "not installed" with a remediation, and an **Install** button next to it |
| 2 | Click **Install** | the button disables; while the job runs, its last log line is shown in place of the remediation |
| 3 | Wait for the install job to finish | the `Claude Code` check turns `[ok]` with a version string; the **Install** button is gone |
| 4 | On **Projects**, click **Open session** on `<proj>` | a new Windows Terminal tab opens, titled with the project's name, running the Claude Code TUI inside the distro; the project's row grows a badge reading "1 live" |
| 5 | Open the **Sessions** tab | the session is listed under **Live**: project name, state `running`, harness `claude-code`, "1 client attached", **Attach** and **Stop** buttons |
| 6 | Type in the Windows Terminal tab from row 4 | Claude Code echoes the keystrokes and responds like any normal terminal session |
| 7 | Drag an edge of that tab's window to resize it — **this is the human-eyes row Plan A's checklist left open (row 6 there), only exercisable now that the app can open a real tab** | the TUI reflows to the new width/height, the same way it would in a hand-run `willie attach` |
| 8 | On the Sessions row, click **Attach** | a second Windows Terminal tab opens onto the *same* session, showing the same screen; typing in either tab reaches the agent |
| 9 | On the Sessions row, click **Stop** | both tabs print "session stopped" and return; the Sessions screen moves the row from **Live** to **Recent** as `exited 0`; the Projects row's "1 live" badge disappears |
| 10 | With `wt.exe` hidden (see prereqs), click **Open session** on a `ready` project again | the session is created and listed under **Live** as `running` regardless; the Projects row shows a `terminal_launch_failed` notice with the message "terminal launch failed: …" and a remediation of the form `open a terminal and run: wsl -d willie --user willie -- /opt/willie/bin/willie attach <id>` |
| 11 | Copy that exact line into a PowerShell prompt and run it | it attaches to the very session from row 10, proving the paste-able fallback works; restore `wt.exe` afterwards |
| 12 | Start a long-running project job (e.g. **Send to Windows** on a large workspace), then immediately click **Open session** on that same project | the row shows an inline `project_busy` problem with its remediation, and no session is created |
| 13 | Remove the harness again (prereqs step), then click **Open session** on a `ready` project | the row shows an inline `harness_not_installed` problem with the remediation "click Install on the Dashboard", and no session is created |
| 14 | With a session still `running` (repeat row 4 first if needed), close the Willie window entirely, then reopen it | the Sessions screen shows the same session under **Live**, restored from the daemon's re-adoption scan, not re-created; no console window appeared at any point |

## Results

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
