# 0008 — Debian slim image, no systemd, three data zones

- **Date:** 2026-08-25
- **Status:** accepted

## Context

The distribution image must be small, compatible with the tools users
install (which expect glibc), replaceable on update without destroying
what the user installed, and free of services that fight WSL's own boot.

## Options

| Option            | For                                         | Against                                             |
| ----------------- | ------------------------------------------- | --------------------------------------------------- |
| Debian stable slim | glibc; apt; ~30–60 MB compressed; predictable | none significant                                   |
| Alpine            | tiny                                        | musl breaks common toolchains and native binaries   |
| Ubuntu            | familiar                                    | larger; ships user-namespace restrictions that break rootless sandboxes |

## Decision

Debian stable *slim* with only what Willie needs (`ca-certificates`,
`git`, `curl`, `bubblewrap`, `sudo`, a few utilities), the `willie` user
(uid 1000) and the Willie binaries under `/opt/willie`. systemd stays
off; the engine starts the daemon and the distribution boots on demand.
Three zones: **system** (packages, `/opt/willie`, `/etc/wsl*.conf` —
replaced on update), **Willie data** (`/var/lib/willie`) and **user data**
(`/home/willie`, including managed tools) — both preserved.

## Consequences

- Two update rhythms: Willie binaries are copied in place (frequent,
  cheap); the base image is re-imported rarely with the two data zones
  exported and restored.
- Managed tools never use `apt`, so the system zone stays pure.
- `appendWindowsPath` is disabled: Windows executables are not on the
  Linux `PATH`, which removes shell/tool collisions and slow lookups.
- The distribution's own Start-menu shortcut and auto-generated terminal
  profile are disabled; the engine manages the Windows Terminal profile.

## Not decided

Mounting a separate virtual disk for data (needs administrator) and a
second distribution for data. Both rejected for now in favour of the
export/restore path.
