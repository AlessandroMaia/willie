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
| 15 | With `<proj>` having a live session (repeat row 4 if needed), on its **Sessions** row click **Open in app** | an embedded terminal panel opens inside the Willie window, painting the Claude Code TUI; no Windows Terminal tab opens |
| 16 | Type in the embedded terminal | Claude Code echoes the keystrokes and responds like any normal terminal session |
| 17 | Resize the **Willie window** — the human-eyes row for the embedded path | the embedded TUI reflows to the new width/height |
| 18 | Open a second live session (**Open session** again, on `<proj>` or another `ready` project), then click **Open in app** on its row | the first session's embedded terminal detaches (it keeps running, still listed under **Live**); the second session's terminal attaches, its screen restored from the ring replay |
| 19 | On the embedded terminal panel, click **Close** | the panel goes away; the session stays listed under **Live**, unaffected |
| 20 | Regression: with `wt.exe` present (not hidden), click **Open session** (not **Open in app**) on a `ready` project | exactly one Windows Terminal tab opens; no stray `wt.exe` window appears at any point |
| 21 | With `<proj>` having a finished session that held some conversation (stop the session from row 9 first if needed) and no live session, click **Resume** on its Projects row | a new Windows Terminal tab opens, continuing the conversation — Claude Code recalls the prior context — instead of starting fresh; the Projects row's live badge goes to "1 live" |
| 22 | With that resumed session still live, open the Session screen's **Sessions** panel | the live session is listed under **Live** with **Open**, and a resume is offered on the newest finished agent session only — a project may hold several live sessions now, so nothing is disabled for having one; `session_already_live` is retired and no longer produced |

## Results

Walked 2026-08-29 by the user, through the app UI, against real Claude Code
2.1.251, and approved. Concrete evidence was captured for rows 1–5 (below);
rows 6–14 were walked and approved by the user and are recorded on that
attestation (no separate per-row transcript was kept for them).

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
| 2026-08-29 | 1 | pass | Dashboard `Claude Code` check showed `[fail]` "not installed" with a remediation and an **Install** button. |
| 2026-08-29 | 2 | pass | **Install** clicked; the install job ran. |
| 2026-08-29 | 3 | pass | The check turned `[ok]` **Claude Code 2.1.251**; the Install button was gone. |
| 2026-08-29 | 4 | pass | **Open session** on `projteste` created a live session (a tab attached — see row 5's "1 client attached"); the Projects row reflected the live session. |
| 2026-08-29 | 5 | pass | **Sessions → Live** listed `projteste`, state `running`, harness `claude-code`, "1 client attached", started ~3 min ago, with **Attach**/**Stop**; **Recent** was empty ("No recent sessions."). |
| 2026-08-29 | 6 | pass | Walked and approved by the user (typing reaches the agent). |
| 2026-08-29 | 7 | pass | The human-eyes TUI-reflow-on-resize row (Plan A left it open) — walked and approved by the user. |
| 2026-08-29 | 8 | pass | Attach opens a second tab onto the same session — walked and approved by the user. |
| 2026-08-29 | 9 | pass | Stop ends the session; row moves Live → Recent as `exited 0`; badge clears — walked and approved by the user. |
| 2026-08-29 | 10 | pass | `wt.exe`-hidden fallback: session still created, `terminal_launch_failed` notice with the paste-able attach line — walked and approved by the user. |
| 2026-08-29 | 11 | pass | The pasted attach line attaches the session — walked and approved by the user. |
| 2026-08-29 | 12 | pass | `project_busy` refusal inline, no session created — walked and approved by the user. |
| 2026-08-29 | 13 | pass | `harness_not_installed` refusal inline ("click Install on the Dashboard") — walked and approved by the user. |
| 2026-08-29 | 14 | pass | Close/reopen restores the live session via re-adoption; no console window appeared — walked and approved by the user. |
