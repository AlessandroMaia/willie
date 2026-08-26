# Willie distribution image

This directory will hold the reproducible recipe for the WSL root
filesystem that Willie registers at install time (`willie-rootfs.tar.gz`).

Planned contents (see `docs/ARCHITECTURE.md` §2):

- `build.sh` — builds the rootfs from Debian stable *slim* with the system
  packages Willie itself needs (`ca-certificates`, `git`, `curl`,
  `bubblewrap`, `sudo`, …), the `willie` user (uid 1000) and the Willie
  binaries under `/opt/willie/bin`.
- `wsl-distribution.conf` — first-launch configuration (default uid and
  name, no Start-menu shortcut, no auto-generated terminal profile).
- `wsl.conf` — per-distribution settings (no systemd, Windows PATH not
  appended, default user `willie`).
- `oobe.sh` — non-interactive first-run check.

Nothing third-party that the user works with (agent CLIs, language
toolchains) is part of the image; those are managed tools installed into
the user's home.
