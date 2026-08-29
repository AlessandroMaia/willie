# Sessions (Linux core) acceptance — daemon-driven, no app yet

Prove the sessions Linux core by hand: install the harness, create a
Claude Code session through the daemon, attach a real Windows Terminal
tab to it, stop it, and confirm the daemon re-adopts a live supervisor
after a restart. There is no app UI in this slice, so this walk drives
`willied` directly over its stdio JSON-RPC; the ergonomic, button-driven
walk lives in `sessions-acceptance.md` and belongs to the engine + app
slice (Plan B).

Before the walk:

- Walk the F0 acceptance first: the `willie` distribution is registered
  and healthy. If a `wsl` step fails with `Wsl/Service/.../HCS/0x80070569`
  ("logon type not granted"), the WSL2 VM right was revoked — restore it
  (decision 0010) before continuing.
- **Close the Willie desktop app.** While it runs it keeps its own daemon
  alive — that daemon holds `/opt/willie/bin/willied` (so a reinstall
  fails with "Text file busy") and would fight this walk's daemons over
  the shared `/var/lib/willie` state and `/run/willie` sockets. This walk
  runs the daemon by hand; nothing else may.
- **Install the current binaries into the distribution.** The installed
  daemon must be this slice's build (its `daemon.doctor` includes a
  `Claude Code` check — an older daemon does not). Either
  `$env:WILLIE_TEST_DISTRO = "willie"; just app-build` and install
  `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`, or, for a
  binaries-only refresh, from the repo root:
  `just build-linux` then, in one line,
  `wsl -d willie --user root -- sh -c "cp /mnt/c/github/pessoal/willie/target/x86_64-unknown-linux-musl/release/willied /mnt/c/github/pessoal/willie/target/x86_64-unknown-linux-musl/release/willie-sess /mnt/c/github/pessoal/willie/target/x86_64-unknown-linux-musl/release/willie /opt/willie/bin/ && chmod 0755 /opt/willie/bin/willied /opt/willie/bin/willie-sess /opt/willie/bin/willie"`
  (adjust the repo path if yours differs).
- Have one registered project with a `ready` workspace (walk
  `projects-and-workspaces-acceptance.md` rows 1–2, or reuse one).

Talking to the daemon by hand: paste this helper once into your
PowerShell session. It writes a request to a **no-BOM UTF-8 file** and
feeds it to a fresh `willied --stdio` — feeding JSON any other way fails,
because PowerShell prepends a UTF-8 BOM when it pipes to a native command
and `sh -c 'echo {\"…\"}'` strips the quotes. Each `willied --stdio`
reads its request, answers on stdout, and exits at end-of-input; the
**supervisor it spawns keeps running**, and a later helper call starts a
fresh daemon that rescans and re-adopts it — which is how
`session.list`/`session.stop` work in a later step.

```powershell
function Invoke-WillieRpc {
  param([Parameter(Mandatory)][string]$Json)
  $dir = Join-Path $env:TEMP 'willie-rpc'; New-Item -ItemType Directory -Force $dir | Out-Null
  $f = Join-Path $dir 'req.jsonl'
  [IO.File]::WriteAllText($f, $Json + "`n", (New-Object System.Text.UTF8Encoding $false))
  $mnt = '/mnt/' + $f.Substring(0,1).ToLower() + ($f.Substring(2) -replace '\\','/')
  wsl -d willie --user willie -- sh -c "/opt/willie/bin/willied --stdio < '$mnt'"
}
```

Placeholders: `<proj>` a project id; `<sess>` a session id a reply gives
you; `<slug>` a project's workspace folder name.

| # | Action | Expected |
| - | ------ | -------- |
| 1 | `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"daemon.doctor","params":{}}'` | the reply's `result.checks` includes a `Claude Code` check — `ok` with a version if installed, else `fail` with a remediation. If there is **no** `Claude Code` check at all, the installed daemon is pre-sessions — redo the install step above. (`willie doctor` alone omits this line — it bypasses the daemon; a known Plan B follow-up.) |
| 2 | `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"state.snapshot","params":{}}'` | `result.projects[]` lists your project with its `proj_…` `id` and `state:"ready"` (note it as `<proj>`); `result.sessions` is `[]` |
| 3 | If row 1 showed Claude Code **not installed**: `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"tool.install","params":{"harness":"claude-code"}}'` — wait for it to finish, then repeat row 1 | the reply is a `JobRef { job_id }`; after the install, row 1's `Claude Code` check is `ok` with a version. Re-running mid-install returns `harness_already_installed`/`tool_busy` |
| 4 | Create a session: `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"session.create","params":{"project_id":"<proj>"}}'` | the reply is `CreateResult { session }` with `state:"running"` and a `pid`; note its `id` as `<sess>`. On disk: `wsl -d willie --user willie -- ls /var/lib/willie/sessions/<sess>/` shows `spec.json` + `events.jsonl`; `wsl -d willie --user willie -- ls /run/willie/sessions/` lists `<sess>.sock` |
| 5 | Attach a real terminal — open a **new Windows Terminal tab** running: `wsl -d willie --user willie --exec /opt/willie/bin/willie attach <sess>` | the Claude Code TUI paints in the tab; a "detach with Ctrl-]" line appears; typing reaches the agent and it responds; 24-bit colour is clean |
| 6 | Resize the tab (drag its edge), then press `Ctrl-]` | the harness reflows to the new width; `Ctrl-]` detaches, printing "detached, the session keeps running"; the session stays alive (`ls /run/willie/sessions/` still lists `<sess>.sock`) |
| 7 | Re-attach: a new tab with the same `willie attach <sess>` command | the screen is restored (the ring replay, or a redraw if the harness is on the alternate screen); typing works again |
| 8 | `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"session.list","params":{}}'` | `result.sessions[]` contains `<sess>` as `running` — proving a freshly started daemon **re-adopted** the live supervisor by scanning the session dirs |
| 9 | `Invoke-WillieRpc '{"jsonrpc":"2.0","id":1,"method":"session.stop","params":{"id":"<sess>"}}'` | any attached tab prints "session stopped" and returns; `events.jsonl` ends with `stop_requested` then `exited`; `<sess>.sock` is gone; a later row-8 list shows `<sess>` terminal |
| 10 | Abrupt death: create another session (row 4), find its supervisor (`wsl -d willie --user willie -- sh -c "ps -eo pid,args \| grep 'willie-sess run' \| grep -v grep"`), `wsl -d willie --user root -- kill -9 <supervisor-pid>`, wait, then list (row 8) | the session becomes terminal (`failed { code:"supervisor_lost" }`), **not** a phantom `running`; `project.remove` on that project is then accepted (no lingering `sessions_running`) |
| 11 | Fail-closed spot checks (each a `session.create`/`project.remove` helper call): create on an unknown `proj_…`; create while a project has a job in flight; remove a project that has a live session | unknown project → `project_not_found`; busy project → `project_busy`; remove with a live session → `sessions_running`; on a distro with no harness, create → `harness_not_installed` — every one refuses cleanly, nothing half-starts |
| 12 | Git identity: in the attached session (or a plain shell) make a commit in the workspace and inspect the author — `wsl -d willie --user willie -- git -C ~/projects/<slug> log --oneline -1 --format='%an <%ae>'` | the author is the distribution's **global** identity (the Windows git identity Willie wrote at the first `session.create`, else the source checkout's) — not a per-clone local identity (decision 0015; the old `copy_source_identity` stopgap is gone) |

When done, re-open the Willie app (it will start a fresh daemon that
re-adopts any session still running from this walk).

## Results

One line per row walked. Note the harness version, the project used, and
anything surprising (timing, a reflow glitch, a code that differed).

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
