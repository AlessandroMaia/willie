# Embedded terminal — a session inside the Willie window

## Problem

Opening a session today launches it in a **separate Windows Terminal tab**
(`wt.exe … wsl.exe --exec willie attach <id>`), or a console when `wt.exe`
is absent. That works, but the terminal lives outside Willie: the user
juggles windows, and Willie — a control plane for agents — cannot show the
agent it just launched. Two concrete pains:

- There is no way to watch or drive a session from inside the app; the
  Sessions screen only lists state, it cannot render the running agent.
- The current `wt.exe` path also flashes a stray Windows Terminal window
  on every open, because `terminal::locate_wt` *executes* `wt.exe
  --version` to detect it and `wt.exe` is a GUI app — the probe opens a
  window the user must close by hand.

## Goals

- Open a live session **inside the Willie window** and interact with it:
  type, see the full-screen TUI, and have it **reflow when the pane is
  resized**.
- Reuse the existing session socket and its framed protocol — the embedded
  terminal is "one more client of the socket", not a second mechanism.
- Keep the Windows-Terminal-tab path working unchanged, as a coexisting
  alternative and fallback.
- Remove the stray `wt.exe` window from the tab path.

## Non-goals

- Visual/UX polish (layout, theming, chrome). This slice is functional
  only; a later UI/UX pass owns the look.
- More than one embedded terminal rendered at once. One active session at a
  time; switching detaches one and attaches the next. Concurrent panes are
  a follow-up (the single bridge is the unit that multiplies later).
- Session resume, scrollback search, copy/paste affordances beyond what
  `xterm.js` gives for free — later slices.
- Replacing the Windows-Terminal-tab path. It stays; the embedded terminal
  is additive.

## Design

The embedded terminal is a fourth **client of the session socket**, hosted
in the app. It reuses `willie attach` — the socket/protocol owner — running
it in a new `--host` mode whose stdio the app drives, instead of a real
terminal. The app is a dumb byte bridge between that child process and an
`xterm.js` terminal in the webview.

```
webview (xterm.js)  ⇄  Tauri commands/events  ⇄  willie-engine (bridge)
     │  input/resize  (command)                         │  spawns child
     │  output        (event, chunked)                  ▼
     │                                   wsl.exe … willie attach <id> --host
     │                                                   │  (owns the socket)
     └───────────────────────────────────────►  session supervisor (PTY)
```

Open flow: the UI asks to open session `X` in the app → the engine spawns
`wsl.exe -d willie --user willie --exec /opt/willie/bin/willie attach X
--host` → `attach` connects to `/run/willie/sessions/X.sock`, sends
`hello` with `Role::Terminal`, and the supervisor replays its ring → the
session's bytes come out on the child's stdout → the engine reads them and
emits them to the webview → `xterm.js` renders. A keystroke goes
`xterm.onData` → command → an `input` host-frame on the child's stdin →
`attach` re-frames it as a wire `input` → the PTY. A resize goes fit-addon
→ command → a `resize` host-frame → wire `resize` → `SIGWINCH` → the TUI
reflows.

### Host dialect — `crates/willie-proto/src/hostterm.rs` (new)

A tiny, pure, transport-agnostic codec for the two messages the app sends
`willie attach --host` on its stdin. It lives in `willie-proto` because
both sides already depend on that crate (the engine encodes, the CLI
decodes) and it does no I/O, so nothing in the crate layering changes.

- `encode_input(bytes: &[u8]) -> Vec<u8>` — tag `b'i'`, `u32` big-endian
  length, then the bytes.
- `encode_resize(rows: u16, cols: u16) -> Vec<u8>` — tag `b'r'`, `u16`
  rows, `u16` cols.
- A decoder (`HostFrame` enum `Input(Vec<u8>)` / `Resize { rows, cols }`
  plus a reader that pulls one frame from a byte stream) for the CLI side.

Output needs no host framing: the session's bytes travel raw on the
child's stdout (a terminal stream has no message boundaries `xterm.js`
cares about).

### Attach host mode — `crates/willie-cli/src/attach.rs`, `main.rs`

`willie attach` gains a `--host` flag. The socket half is unchanged —
connect, `hello(Role::Terminal)`, the reader/writer threads, `closed`
handling all stay. Only the *local transport* is swapped behind a small
seam:

- Today (raw mode): local input comes from the tty in raw mode, output
  goes to the tty, resize comes from `SIGWINCH`/`ioctl`.
- Host mode: local input is `hostterm` frames read from stdin (each
  `Input` → a wire `input` frame to the supervisor; each `Resize` → a wire
  `resize` frame); output is the raw payload of each supervisor `output`
  frame written to stdout. EOF on stdin is a clean detach: send `detach`
  and exit, leaving the session running.

The refactor extracts the local-side I/O into two implementations of one
seam so the socket loop is written once.

### Engine bridge — `crates/willie-engine/src/embed.rs` (new)

Owns the single active embedded terminal. `#[cfg(windows)]` for the spawn,
with a non-Windows stub so the workspace checks on any host.

- `open(id: SessionId) -> Result<(), EngineError>` — compose the child
  argv (a pure, host-testable function, like `terminal::attach_argv`),
  spawn `wsl.exe … willie attach <id> --host` with piped stdin/stdout and
  `CREATE_NO_WINDOW`, start a reader thread that forwards stdout chunks to
  the app, and record the child + its stdin as the active bridge. Opening a
  new session first closes the previous bridge.
- `input(id, &[u8])` / `resize(id, rows, cols)` — write the matching
  `hostterm` frame to the active child's stdin (ignored if `id` is not the
  active bridge — a stale call after a switch).
- `close(id)` — close the child's stdin (clean detach), then reap; kill as
  a fallback.

The engine's snapshot/event machinery is untouched: the session's own
lifecycle still flows over `daemon://event`; this bridge is only the byte
pipe for rendering.

### Terminal tab fix — `crates/willie-engine/src/terminal.rs`

`locate_wt` stops executing `wt.exe`. It resolves `wt.exe` by file
lookup only — on `PATH` and under
`%LOCALAPPDATA%\Microsoft\WindowsApps\wt.exe` — so detecting Windows
Terminal never opens a window. The launch path (`open_tab`) is otherwise
unchanged.

### App — `apps/willie-app/src-tauri/src/lib.rs`, `apps/willie-app/src/`

- Tauri commands: `session_terminal_open(id)`, `session_input(id, data)`,
  `session_resize(id, rows, cols)`, `session_terminal_close(id)` — thin
  wrappers over the engine bridge, following the existing
  `daemon_command`/`with_engine` pattern. Output is delivered as a Tauri
  event `session://output` carrying `{ id, chunk }` (bytes base64-encoded,
  since Tauri events are JSON).
- Frontend: a terminal component built on `xterm.js` + the fit addon (new
  frontend dependencies `@xterm/xterm`, `@xterm/addon-fit`). On open it
  calls `session_terminal_open`, subscribes to `session://output` (filtered
  by id) and writes chunks to the terminal; `xterm.onData` → `session_input`;
  the fit addon plus a `ResizeObserver` → `session_resize`; unmount/switch →
  `session_terminal_close`. A live-session row gains a functional "Open in
  app" action that reveals this terminal for that session. Placement and
  styling are intentionally minimal — the UI/UX pass owns them.

### Errors and edge cases

| Code | When | User-visible behaviour |
| --- | --- | --- |
| `embedded_terminal_failed` | the engine cannot spawn `wsl.exe`/`willie attach --host` | an inline problem with its remediation; the Windows-Terminal-tab path stays available as the alternative |
| (child exits) | the session already ended, or exits while attached | the terminal shows it closed; the row reflects the terminal state from `daemon://event` as usual |
| (switch) | opening session Y while X is shown | X's bridge is closed (clean detach, X keeps running); Y attaches and the ring replays its screen |

`terminal_launch_failed` (the Windows-Terminal-tab path) is unchanged and
independent of the embedded path.

## Testing

- **`willie-proto`:** `hostterm` round-trips — `encode_input`/`encode_resize`
  decode back to the same `HostFrame`, and a truncated buffer is an error,
  not a panic. Pure, `#[cfg(test)]`.
- **`willie-cli`:** an integration test drives `willie attach --host`
  against a fake supervisor socket (reusing the existing supervisor test
  fixtures): host `input`/`resize` frames on stdin produce the right wire
  frames on the socket, and supervisor `output` frames reach stdout raw;
  EOF on stdin sends `detach`.
- **`willie-engine`:** the child-argv builder is pure and host-tested (like
  `terminal::attach_argv`); the `hostterm` encoding used by the bridge is
  covered by the `willie-proto` tests. The spawn itself is `#[cfg(windows)]`.
- **Acceptance:** a new row in `docs/checklists/sessions-acceptance.md` —
  open a session in the app, type and get a response, **resize the pane and
  see the TUI reflow**, switch to another session and back, close it and
  confirm the session keeps running.

## Rollout / compatibility

- The daemon protocol is unchanged; `hostterm` is a private app↔`attach`
  dialect, not part of the daemon RPC or the session wire protocol.
- `willie attach` keeps its current behaviour by default; `--host` is
  additive and used only by the app.
- New engine problem code `embedded_terminal_failed` is documented in
  `docs/PROTOCOL.md` alongside `terminal_launch_failed`.
- New frontend dependencies (`@xterm/xterm`, `@xterm/addon-fit`) — the
  standard terminal emulator; writing one is out of scope.
- The `wt.exe` detection fix changes no configuration and no behaviour
  other than removing the stray window.

## Open questions

- **Byte channel for output.** Start with Tauri events (base64 chunks); if
  a redraw-heavy TUI shows lag, move output to a local `127.0.0.1` socket
  the webview reads directly. Favoured: start with events, measure, upgrade
  only if needed.
- **Where "Open in app" lives.** A functional action on the live-session
  row for now; the UI/UX pass decides the final placement (a panel, a
  dedicated view, or concurrent panes). Favoured: minimal row action now.
