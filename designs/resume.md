# Resume — continue a project's last conversation

## Problem

A session always starts the harness from scratch. When a user closes a
session and comes back, the agent has forgotten everything — there is no
way to pick up where the last conversation left off, even though the
harness (Claude Code) keeps its own transcript on disk and can resume it.
Willie's session model already anticipates this (the harness capability
matrix carries a `Resume` mode), but nothing uses it yet.

## Goals

- One click continues a project's **most recent** conversation: a new
  session launches the harness with its continue flag, in the project's
  workspace, so the agent resumes where it left off.
- Reuse the existing open/terminal path — a resumed session is an ordinary
  live session (attach, stop, embedded terminal all work).
- Record the lineage: the new session names the finished session it
  continued.
- Fail closed: refuse to resume when the harness cannot, or when a live
  session for the project already exists.

## Non-goals

- Resuming an **arbitrary older** session by id (`Resume::ById`). This
  slice does "continue the latest" (`--continue`) only; per-session resume
  by a captured harness id is a documented follow-up (the capability matrix
  already reserves `Resume::ById`). Setting the harness session id up front
  is not possible for an interactive session anyway (`--session-id` is
  documented as non-interactive only), so per-session resume would need
  transcript-id discovery — out of scope here.
- Visual/UX polish. The Resume affordance is functional and minimal; the
  look is the later UI/UX pass.
- Reviving a finished session in place. Resume creates a NEW session (the
  session event log stays append-only and terminal-once); the new session
  links back via `resumed_from`.

## Design

Resume is `session.create` with a `resume` flag. The daemon builds the
harness launch in "continue" mode instead of "fresh", records which
finished session it continued, and otherwise runs the normal create path;
the engine and UI treat it exactly like opening a session.

```
Projects row "Resume"  →  engine.session_resume(project_id)
                          →  daemon session.create { project_id, resume: true }
                             → harness launch = [claude, --continue]  (cwd = workspace)
                             → new Session { resumed_from: <latest finished> }
                          →  open the terminal (WT tab / embedded), as for open
```

### Harness — `crates/willie-harness/src/lib.rs`

A new launch intent, distinct from the capability enum:

```rust
pub enum LaunchMode { Fresh, Continue }
```

`Harness::launch(&self, binary, workspace, home, mode: LaunchMode) ->
Launch`. `ClaudeCode`: `Fresh` → `argv = [binary]` (today's behaviour);
`Continue` → `argv = [binary, "--continue"]`. The environment is
unchanged. The capability matrix (`HarnessCapabilities.resume`) is
untouched.

Simplification (recorded; YAGNI with one harness): the daemon treats any
harness whose `capabilities().resume != Resume::None` as able to continue
the latest, and uses `--continue`. Claude Code advertises `Resume::ById`
but supports both `--resume <id>` and `--continue`. A future multi-harness
world would split the capability into `resume_by_id` / `continue_latest`
bits; not now.

### Protocol + daemon

- `CreateParams` gains `#[serde(default)] resume: bool` (older callers
  default to `false` — a fresh session).
- `Session` gains `#[serde(default, skip_serializing_if = "Option::is_none")]
  resumed_from: Option<SessionId>`.
- `SessionOps::create`, when `resume` is true:
  1. Refuse if the harness cannot resume (`capabilities().resume ==
     Resume::None`) → `harness_cannot_resume`.
  2. Refuse if the project already has a live session → `session_already_live`
     (continuing a conversation that is live elsewhere would double-drive
     it and the shared transcript).
  3. Build the launch with `LaunchMode::Continue`.
  4. Set `resumed_from` to the project's most-recent finished (terminal)
     session, if any.
  Otherwise the create path is unchanged (the same `project_not_found` /
  `project_busy` / `project_not_ready` / `harness_not_installed` guards
  apply). `--continue` resolves against the workspace, which the supervisor
  already uses as the harness cwd (`pty::spawn(..., &spec.workspace, ...)`).

### Engine + app

- `Engine::session_resume(project_id) -> Result<SessionOpened, EngineError>`
  mirrors `session_open`: it calls the session-create RPC with `resume:
  true`, then opens the terminal (Windows Terminal tab; the embedded
  terminal path is unchanged). A failed terminal is still the non-fatal
  `terminal_problem`.
- Tauri command `session_resume(project_id)`; `engine.ts`
  `sessions.resume(projectId)`.
- Projects row: a **Resume** button beside Open session, enabled when the
  project is `ready`, has at least one finished session, and has no live
  session. It calls `sessions.resume(project.id)`; a thrown `Problem`
  renders inline like the row's other failures. Placement/styling minimal.

### Errors and edge cases

| Code | When | User-visible behaviour |
| --- | --- | --- |
| `harness_cannot_resume` | resume requested but the harness's `Resume` capability is `None` | inline row problem; the remediation says this harness cannot resume |
| `session_already_live` | resume requested while the project has a live session | inline row problem; the remediation says to use the running session or stop it first |
| (no prior conversation) | `--continue` finds nothing to continue | the harness prints its own message and exits; the session shows exited. The UI only offers Resume when a finished session exists, so this is rare |

The existing create guards (`project_not_found`, `project_not_ready`,
`project_busy`, `harness_not_installed`) apply to resume unchanged.

## Testing

- **`willie-harness`:** `launch(..., LaunchMode::Continue)` yields an argv
  ending in `--continue`; `LaunchMode::Fresh` is the bare binary. Pure.
- **`willied`:** `session.create { resume: true }` builds a `--continue`
  argv and sets `resumed_from` to the latest finished session; it refuses
  with `session_already_live` when a live session exists and
  `harness_cannot_resume` for a `Resume::None` harness (a test fake).
  Integration test in the distro alongside the existing session tests.
- **`willie-engine`:** `session_resume` wiring is covered by the
  `SessionOpened` mapping already tested; no new pure logic.
- **App:** thin (Tauri command, `engine.ts`, the Projects-row button);
  exercised in the acceptance walk.
- **Acceptance:** a new `docs/checklists/sessions-acceptance.md` row —
  after a session with some conversation ends, click **Resume**; a new
  session opens and the agent has the prior context; the row links it to
  the finished session; resume is refused (inline) while a session is live.

## Rollout / compatibility

- `CreateParams.resume` is additive with a `serde(default)` — older engines
  omit it and get a fresh session. `Session.resumed_from` is additive and
  optional. The daemon wire protocol version does not change.
- New engine/daemon error codes `harness_cannot_resume` and
  `session_already_live` are documented in `docs/PROTOCOL.md`.
- No new dependency.

## Open questions

- Whether Resume should also live on the Sessions screen's most-recent
  finished row (not only the Projects row). Favoured: Projects row only for
  this slice (it matches the project-scoped "continue the latest"
  semantics); revisit in the UI/UX pass.
