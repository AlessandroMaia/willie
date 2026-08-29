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
  `Claude Code` check — an older daemon does not). The full path is
  `$env:WILLIE_TEST_DISTRO = "willie"; just app-build` and install
  `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`. For a
  binaries-only refresh, `just build-linux`, then copy the three release
  musl binaries in as root (`<repo>` is the checkout on the Windows
  drive, e.g. `/mnt/c/github/pessoal/willie`):

  ```sh
  R=<repo>/target/x86_64-unknown-linux-musl/release
  wsl -d willie --user root -- sh -c \
    "cp $R/willied $R/willie-sess $R/willie /opt/willie/bin/ && \
     chmod 0755 /opt/willie/bin/willied /opt/willie/bin/willie-sess /opt/willie/bin/willie"
  ```
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

Walked 2026-08-28 by the agent against **real Claude Code v2.1.251**, in
an isolated environment (new binaries in `/tmp/willie-new`, private
`WILLIE_STATE_DIR`/`WILLIE_RUN_DIR`/`WILLIE_HOME`/`WILLIE_PROJECTS_DIR`
under `/tmp/wt`) so it never touched `/opt` or the running desktop app.
The attach rows were exercised **headless** (`willie attach <sock>
--no-raw --size`), which proves output delivery, input and detach but not
a human's eyes on a GUI tab — the one row left for a person is the
visual TUI reflow when you drag a real Windows Terminal tab (row 6's
resize half). Re-walk from a real tab to confirm that.

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
| 2026-08-28 | 1 | pass | `daemon.doctor` lists a `Claude Code` check (the new daemon); reported `fail`/"not installed" before, `ok`/`2.1.251` after row 3 |
| 2026-08-28 | 2 | pass | `project.add` of a `/tmp` source repo: add job `running`→`done`, project `ready` |
| 2026-08-28 | 3 | pass | `tool.install` returned a `JobRef`, `install_harness` `running`→`done`; the real installer put Claude Code `2.1.251` in the isolated home |
| 2026-08-28 | 4 | pass | `session.create` returned a `running` session (real `claude` as the harness pid); `spec.json` + `events.jsonl` (`created`,`started`) + the socket appeared |
| 2026-08-28 | 5 | pass | headless attach received the genuine Claude Code v2.1.251 TUI over the framed socket (theme picker, logo, syntax-highlighted diff; ~5.9 KB) |
| 2026-08-28 | 6 | partial | `Ctrl-]` detached cleanly ("detached, the session keeps running") and the supervisor + `claude` stayed alive; **visual reflow on a GUI tab drag not tested headlessly — confirm from a real WT tab** |
| 2026-08-28 | 7 | pass | attaching again delivered the session's output again; the session survived detach/reattach |
| 2026-08-28 | 8 | pass | a fresh daemon re-adopted the live session (listed/stopped it); on start it also finalises dead ones |
| 2026-08-28 | 9 | pass | `session.stop`: `stop_requested{by:"daemon"}`→`exited{code:0}` (Claude Code exits 0 on the ladder's SIGINT); socket removed |
| 2026-08-28 | 10 | pass | a supervisor killed / lost without a terminal event is finalised `failed{code:"supervisor_lost"}` by the next daemon's scan (observed repeatedly) |
| 2026-08-28 | 11 | pass | `session.create` on an unknown project → `project_not_found`; `project_busy` and `sessions_running` are covered by the daemon integration tests |
| 2026-08-28 | 12 | pass | the distro-global `~/.gitconfig` identity was ensured at `session.create` from the source checkout (`t <t@t>`), per decision 0015 (no per-clone local identity) |
