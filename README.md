# Willie

A personal control plane for AI coding agents on Windows.

Willie provisions its own WSL2 distribution, launches agent CLIs (Claude
Code first) inside it under an OS sandbox, and gives per-project control
over sessions, configuration and usage from a desktop app. Willie does not
talk to LLM APIs itself; it prepares, launches, observes and governs.

## Status

Pre-alpha. The first slice (F0) is built — the distribution image
builds and registers, its daemon answers, and the app shows health and
`doctor` — and awaits its acceptance walk
(`docs/checklists/f0-acceptance.md`). Nothing is released yet.

## Prerequisites (development)

- Windows 11 with WSL 2.4.4 or newer already enabled (`wsl --version`).
- Rust via `rustup` (the exact toolchain is pinned in `rust-toolchain.toml`
  and installed automatically on first `cargo` invocation).
- Visual Studio Build Tools with the "Desktop development with C++"
  workload (MSVC linker and Windows SDK). Requires an elevated prompt:

  ```powershell
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```

- Node.js 22 and pnpm.
- `just` (task runner) and `cargo-zigbuild` + `zig` (Linux cross-compile).
- On a managed machine, the WSL 2 VM needs the "Log on as a service"
  right for `NT VIRTUAL MACHINE\Virtual Machines` (`S-1-5-83-0`). The HCS
  error `0x80070569` on `wsl --import` or on the first daemon start means
  it is missing; only an administrator can grant it. See
  `docs/decisions/0010-wsl-stdio-transport-findings.md`.

Run `just ensure` to check everything and get install hints.

## Development

```text
just                   list recipes
just ensure            verify the toolchain
just hooks             install the pre-commit hook
just check             the local quality gate (format, clippy, tests, denylist)
just dev               run the desktop app
just build-linux       cross-compile the Linux binaries to musl
just distro-pin        pin the base root filesystem (distro/base.lock)
just distro-fetch      download and verify the pinned base root filesystem
just distro-build      build the distribution image from distro/
just distro-install    register the image as the `willie` distribution
just distro-uninstall  unregister it, discarding its disk
just app-build         build the Windows installer (NSIS, per-user)
```

## Documents

- `AGENTS.md` — rules for agents and contributors.
- `docs/ARCHITECTURE.md` — the system as designed.
- `docs/decisions/` — decision records.
- `designs/` — feature designs.
- `releases/` — release notes.
