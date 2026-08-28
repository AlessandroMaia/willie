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
  and its daemon is healthy. If a `wsl` step fails with
  `Wsl/Service/.../HCS/0x80070569` ("logon type not granted"), the WSL2
  VM right was revoked — restore it (decision 0010) before continuing.
- Build and install the current binaries into the distribution:
  `$env:WILLIE_TEST_DISTRO = "willie"; just check` then `just app-build`
  and install `target\release\bundle\nsis\Willie_0.1.0_x64-setup.exe`
  (this refreshes `/opt/willie/bin/{willied,willie-sess,willie}`), OR,
  for a binaries-only refresh without the installer, `just build-linux`
  and copy the three musl binaries to `/opt/willie/bin/` as root.
- Have one registered project with a `ready` workspace (walk
  `projects-and-workspaces-acceptance.md` rows 1–2, or reuse one). Note
  its `proj_…` id: run the snapshot line in row 2 below and read `id`.

How to drive the daemon by hand: each step below pipes one or two
JSON-RPC lines into a fresh `willied --stdio` process. The daemon reads
one request per line, answers on stdout, and exits at end-of-input; the
session **supervisor it spawns keeps running** independently, which is
the whole point. A fresh daemon rescans the session directories on
start, so `session.list`/`session.stop` in a later step work against a
new process. `daemon.hello` is optional (the server dispatches any
method); the lines below omit it. Placeholders: `<proj>` is the project
id, `<sess>` a session id a reply gives you.

Run each `wsl` command from PowerShell exactly as written (a single
command). The heredocs use `sh -c` inside the distribution so the quoting
survives; keep the single quotes.

| # | Action | Expected |
| - | ------ | -------- |
| 1 | Ask the daemon's doctor whether the harness is present: `wsl -d willie --user willie -- /opt/willie/bin/willie doctor` — then the daemon's own view: `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"daemon.doctor\",\"params\":{}} \| /opt/willie/bin/willied --stdio'` | the RPC reply's `checks` array includes a `Claude Code` check — `ok` with a version if installed, else `fail` with a remediation naming the Dashboard/installer. (`willie doctor` alone does **not** list it — it bypasses the daemon; known Plan B follow-up.) |
| 2 | Snapshot the state to read your project id (and, later, sessions): `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"state.snapshot\",\"params\":{}} \| /opt/willie/bin/willied --stdio'` | the reply's `result.projects[]` lists your project with its `proj_…` `id` and `state:"ready"`; `result.sessions` is `[]` (none yet) |
| 3 | If row 1 showed Claude Code **not installed**, install it as a job: `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tool.install\",\"params\":{\"harness\":\"claude-code\"}} \| /opt/willie/bin/willied --stdio'` — wait, then re-run row 1's doctor line | the reply is a `JobRef { job_id }`; after the install finishes, row 1's doctor `Claude Code` check flips to `ok` with a version. (Re-running while it runs returns `harness_already_installed`/`tool_busy`.) |
| 4 | Create a session on your project: `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session.create\",\"params\":{\"project_id\":\"<proj>\"}} \| /opt/willie/bin/willied --stdio'` | the reply is `CreateResult { session }` with `state:"running"` and a `pid`; note its `id` as `<sess>`. On disk: `wsl -d willie --user willie -- ls /var/lib/willie/sessions/<sess>/` shows `spec.json` + `events.jsonl`; `wsl -d willie --user willie -- ls /run/willie/sessions/` lists `<sess>.sock` |
| 5 | Attach a real terminal — open a **new Windows Terminal tab** running: `wsl -d willie --user willie --exec /opt/willie/bin/willie attach <sess>` | the Claude Code TUI paints in the tab; a "detach with Ctrl-]" line appears; typing reaches the agent and it responds; 24-bit colour is clean |
| 6 | Resize the tab (drag its edge), then press `Ctrl-]` | the harness reflows to the new width; `Ctrl-]` detaches and returns you to the shell with "detached, the session keeps running"; the session is still alive (`ls /run/willie/sessions/` still lists `<sess>.sock`) |
| 7 | Re-attach: a new tab with the same `willie attach <sess>` command | the screen is restored (the ring replay, or a redraw if the harness is on the alternate screen); typing works again |
| 8 | List sessions from a fresh daemon: `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session.list\",\"params\":{}} \| /opt/willie/bin/willied --stdio'` | `result.sessions[]` contains `<sess>` as `running` with `clients` reflecting any attached tab — proving a freshly started daemon **re-adopted** the live supervisor by scanning the session dirs |
| 9 | Stop the session: `wsl -d willie --user willie -- sh -c 'echo {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session.stop\",\"params\":{\"id\":\"<sess>\"}} \| /opt/willie/bin/willied --stdio'` | any attached tab prints "session stopped" and returns; `events.jsonl` ends with `stop_requested` then `exited`; `<sess>.sock` is gone; a later `session.list` shows `<sess>` in a terminal state |
| 10 | Abrupt death: create another session (row 4), find the supervisor pid (`wsl -d willie --user willie -- cat /proc/<harness-pid>/status` parent, or the `willie-sess` process), `wsl -d willie --user willie -- kill -9 <supervisor-pid>`, then `session.list` (row 8) after a moment | the session becomes terminal (`failed { code:"supervisor_lost" }` or `exited`), **not** a phantom `running`; and `project.remove` on that project is then accepted (no lingering `sessions_running`) |
| 11 | Fail-closed spot checks (each via a `session.create`/`project.remove` RPC line like row 4): create on an unknown `proj_…`; create while a project has a job in flight; remove a project that has a live session | unknown project → `project_not_found`; busy project → `project_busy`; remove with a live session → `sessions_running`; and, on a distribution with no harness, create → `harness_not_installed` — every one refuses cleanly, nothing half-starts |
| 12 | Git identity: in the attached session (or a plain shell) make a commit in the workspace and inspect its author — `wsl -d willie --user willie -- git -C ~/projects/<slug> log --oneline -1 --format='%an <%ae>'` | the author is the distribution's **global** identity (the Windows git identity Willie wrote at the first `session.create`, else the source checkout's) — not a per-clone local identity (decision 0015; the old `copy_source_identity` stopgap is gone) |

## Results

One line per row walked. Note the harness version, the project used, and
anything surprising (timing, a reflow glitch, a code that differed).

| Date | Row | Result | Notes |
| ---- | --- | ------ | ----- |
|      |     |        |       |
