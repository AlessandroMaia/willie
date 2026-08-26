# 0002 — Windows is the only platform; WSL2 is the mechanism

- **Date:** 2026-08-25
- **Status:** accepted

## Context

The user works exclusively on Windows, on both a personal and a corporate
machine. Windows lacks what agent tooling needs most: an OS sandbox,
POSIX process semantics for hooks, a fast filesystem for source trees,
and predictable shells. WSL2 provides all of it, and since WSL 2.4.4 a
distribution is a plain tar file that an application can register
without administrator rights.

## Options

| Option                         | For                              | Against                                           |
| ------------------------------ | -------------------------------- | ------------------------------------------------- |
| Windows-only, own WSL2 distro  | one platform to test; full Linux capabilities; no admin at install | requires WSL enabled beforehand |
| Cross-platform desktop app     | broader reach                    | no user for it; triples platform surface          |
| Windows-native only            | simplest install                 | no sandbox; hook/PATH/shell problems remain       |

## Decision

Willie is a Windows product. It registers and manages its own WSL2
distribution (a *system* distribution the user does not administer) and
uses it for everything the Windows side cannot do well.

## Consequences

- The UI is Tauri for Windows only; no platform abstractions elsewhere.
- Enabling WSL itself is a prerequisite checked by `doctor`, not something
  Willie does (it needs an administrator).
- The Windows side keeps only Windows concerns: UI, tray, proxy and
  certificate export, Windows Terminal, provisioning through `wsl.exe`.

## Not decided

Support for the `mirrored` networking mode or any other global WSL
setting; Willie never writes the user's `.wslconfig`.
