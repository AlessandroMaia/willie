# Agent guidance

Guidance for AI coding agents (Claude Code and others) working in this
repository. Humans should read it too: it is the shortest accurate
description of how Willie is built and what must never break.

## What Willie is

Willie is a personal **control plane for AI coding agents on Windows**. It
provisions its own WSL2 distribution, launches agent CLIs (Claude Code
first) inside it under an OS sandbox, and gives per-project control over
sessions, configuration and usage from a Tauri desktop UI. Willie never
calls an LLM API itself: it prepares, launches, observes and governs. The
agents do the work. Full picture: `docs/ARCHITECTURE.md`.

## Repository map

```
crates/
  willie-core        domain types (ids, config, capability sets) — zero I/O
  willie-linux       Linux-side helpers shared by willied, willie-sess
                     and willie-cli (well-known paths, doctor checks)
  willie-proto       JSON-RPC message types; transport-agnostic
  willie-harness     `Harness` trait + capability matrix; `ClaudeCode` impl
  willie-plugin-api  `Plugin` trait and manifest
  willie-plugins/
    profiles         per-project configuration profiles (apply/check)
    usage            usage windows, cost and context tracking
  willie-engine      Windows side: wsl.exe wrapper, provisioning, proxy/CA,
                     Windows Terminal profile, daemon supervision
  willied            Linux daemon: source of truth (projects, sessions,
                     plugins, SQLite index); runs unprivileged in the distro
  willie-sess        Linux per-session supervisor: PTY, sandbox, socket,
                     append-only event log; outlives the daemon
  willie-cli         Linux stateless CLI: `attach`, `doctor`, …
xtask/               dev tasks in Rust (`cargo xtask …`); `just` delegates here
apps/willie-app/     Tauri 2 + React/TypeScript desktop app (hosts the engine)
distro/              reproducible recipe for the Willie WSL root filesystem
docs/                living contracts (UPPERCASE) and `decisions/` (ADRs)
designs/             feature designs (see `designs/TEMPLATE.md`)
releases/            curated release notes, one file per version
```

## Golden rules

1. **State lives in the distro (ext4).** Never under `/mnt/*`; on the
   Windows side only `engine.toml` (machine profile, UI preferences).
2. **The daemon never runs as root.** Privileged steps are one-shot
   `wsl.exe --user root` calls issued by the engine.
3. **Fail closed.** Invalid configuration, unreadable policy or a missing
   sandbox backend refuses to start the session with an actionable error.
   Never degrade silently.
4. **Sandbox capabilities are monotonic.** Repository-level configuration
   can only tighten what the project profile allows.
5. **stdout is data, stderr is decoration.** See `docs/CLI_CONTRACT.md`.
6. **No `unwrap`/`expect` outside tests** in `willied`, `willie-sess` and
   `willie-cli`. A panic there takes user sessions down with it.
7. **Nothing third-party is bundled.** Agent CLIs and toolchains are
   *managed tools*: detected in the distro and installed from their
   official sources only on the user's explicit action.
8. **Versioned content never cites external projects, authors or
   tickets.** Decisions are explained on their technical merits.
   `just check-refs` enforces a denylist that is kept outside the
   repository.
9. **`docs/blueprint/` and `docs/superpowers/` are unversioned working
   notes.** Do not read `docs/blueprint/` unless the user explicitly asks
   in the current conversation; a hook blocks it otherwise.

## Coding conventions

- Rust 2024 on the exact toolchain in `rust-toolchain.toml`. `Cargo.lock`
  is authoritative: always pass `--locked`.
- Small crates with one purpose. `willie-core` and `willie-proto` do no
  I/O. `willie-engine` (Windows) and `willied` (Linux) never depend on
  each other — only on `willie-proto`. Plugins depend on
  `willie-plugin-api`, `willie-core` and `willie-harness`, never on the
  daemon.
- Platform code is gated with `#[cfg(windows)]` /
  `#[cfg(target_os = "linux")]`. Linux binaries keep a stub `main` for
  other targets so `cargo check --workspace` works everywhere; their real
  build is `just build-linux` (musl, cross-compiled).
- Minimal dependencies. Adding a crate needs a one-line justification in
  the commit message; prefer `std`.
- Errors are typed (`thiserror`), carry a stable `snake_case` code and a
  remediation hint. Never `String` errors across crate boundaries.
- Tests live in `#[cfg(test)] mod tests` at the bottom of the file they
  cover. Integration tests under `tests/` spawn the real binaries. Test
  names are assertive sentences (`unknown_fields_are_ignored`).
- Shared test fixtures go in a `#[cfg(test)]`-only module, never
  duplicated across test modules.
- Formatting is `just fmt` (80 columns, LF). Frontend: Biome and strict
  TypeScript.
- Frontend rhythm: one blank line between logical blocks inside a
  function (setup, each hook, handlers, the `return`) and between
  top-level declarations. Imports in Biome's order — packages, then
  `@/`, then relative — and the editor's Biome extension organises them
  on save (`.vscode/settings.json`). `pnpm -C apps/willie-app format`
  applies both order and class sorting.
- Frontend colours live only in `apps/willie-app/src/styles/globals.css`.
  A state is coloured through a `Tone` (`components/tone.ts`), never
  through a literal or an ad-hoc class.
- `apps/willie-app/src/components/ui/` is generated, never hand-edited:
  from inside `apps/willie-app` (the registry CLI resolves the project
  from its working directory, not from `-C`),
  `pnpm dlx shadcn@latest add <name>`, then
  `pnpm exec biome check --write --unsafe src/components/ui src/components/hooks`.
  Willie's own components are compositions in `components/`.
- Frontend layering: `app → features → components → components/ui`;
  `app` and `features` may read `store`; everything may read `lib`;
  `lib` has no React and returns facts, never presentation; a feature
  never imports another feature. Biome enforces it.
- Configuration files are TOML; JSON only where a foreign tool requires
  it. Writing a foreign config file preserves its existing format,
  comments and key order.

## Workflow

| Command                 | Purpose                                                 |
| ----------------------- | ------------------------------------------------------- |
| `just ensure`           | verify toolchain; prints install hints                  |
| `just hooks`            | install the pre-commit hook                             |
| `just fmt`              | format everything                                       |
| `just lint`             | clippy `-D warnings` (host + musl target), Biome, `tsc` |
| `just test`             | Rust + frontend tests                                   |
| `just check-refs`       | denylist scan of versioned content                      |
| `just check`            | **the gate**: fmt-check + lint + test + check-refs      |
| `just dev`              | run the desktop app                                     |
| `just build-linux`      | cross-compile daemon, supervisor and CLI to musl        |
| `just test-linux`       | run the Linux crates' tests inside the distribution     |
| `just distro-pin`       | pin the base root filesystem (`distro/base.lock`)       |
| `just distro-fetch`     | download and verify the pinned base root filesystem     |
| `just distro-build`     | build the distribution image (runs `build-linux` first) |
| `just distro-install`   | register the image as the `willie` distribution         |
| `just distro-uninstall` | unregister it, discarding its disk                      |
| `just app-build`        | build the Windows installer (runs `distro-build` first) |

There is no remote CI. `just check` on the developer machine is the only
gate, so run it before claiming anything is done.

## Definition of done

- `just check` is green.
- New behaviour has a test that fails before the change and passes after.
- Documentation changed in the same commit: `docs/ARCHITECTURE.md` or the
  relevant contract, a decision record when a choice was made, a design
  when a feature was shaped.
- User-visible changes get a line in the unreleased `releases/vX.Y.Z.md`.
- **Tell the user exactly how to verify the change manually** — concrete
  steps and expected results, not only test commands.

## Where documents go

| Kind                                   | Location                          |
| -------------------------------------- | --------------------------------- |
| Rules for agents and contributors      | `AGENTS.md` (this file)           |
| User-facing overview and install       | `README.md`                       |
| Living contracts (protocol, CLI, …)    | `docs/UPPERCASE.md`               |
| Architecture as built                  | `docs/ARCHITECTURE.md`            |
| Decision records                       | `docs/decisions/NNNN-<slug>.md`   |
| Feature designs                        | `designs/<slug>.md`               |
| Release notes                          | `releases/vX.Y.Z.md`              |
| Unversioned working notes              | `docs/blueprint/`, `docs/superpowers/` |

Every document is written in English.

## Commits

Conventional Commits: `type(scope): imperative summary` (`feat`, `fix`,
`test`, `docs`, `chore`, `refactor`), body explaining the scenario when the
subject is not enough. **No trailers of any kind** — no authorship,
co-authorship or tool attribution lines. One task, one commit; the
pre-commit hook must pass (never `--no-verify`).

## Comments

Describe the scenario and the invariant, not the change history. One to
three lines; anything longer belongs in `docs/` or `designs/`. Never
reference tickets, PRs or external projects.

## Deprecation

Old configuration keys and CLI flags keep working: `#[serde(alias)]`,
`#[serde(default)]`, unknown fields ignored. Name the version that removes
them both in code and in the release note.

## Naming

- Identifiers are prefixed ULIDs: `proj_…`, `sess_…`, `tool_…`, `prof_…`.
- Environment variables use the `WILLIE_` prefix; industry-standard names
  (`HTTP_PROXY`, `SSL_CERT_FILE`, `NO_COLOR`) are kept as they are.
- Error codes are `snake_case` and documented with when *not* to use them.
- Sandbox capabilities have positive names (`network`, `agent.state`),
  never negated ones.
