# Willie distribution image

The reproducible recipe for the WSL root filesystem Willie registers at
install time (`willie-rootfs.tar.gz`).

## Contents

- `base.lock` — the Debian *slim* root filesystem the image starts from,
  pinned by commit and sha256. Written by `just distro-pin`, verified on
  every download by `just distro-fetch`.
- `provision.sh` — runs as root inside a throwaway builder distribution
  and turns the base into the image: the system packages Willie itself
  needs (`ca-certificates`, `curl`, `git`, `bubblewrap`, `sudo`,
  `procps`, `iproute2`, `less`), the `willie` user (uid 1000), and the
  Willie binaries under `/opt/willie/bin`.
- `wsl-distribution.conf` — first-launch configuration (default uid and
  name, no Start-menu shortcut, no auto-generated terminal profile).
- `wsl.conf` — per-distribution settings (no systemd, Windows PATH not
  appended, default user `willie`, `/run/willie` created on boot).
- `oobe.sh` — non-interactive first-run check.

## Building

`just distro-build` cross-compiles the Linux binaries, imports the base
as the `willie-build` distribution, provisions it, exports the result to
`target/distro/willie-rootfs.tar.gz` (plus `.sha256` and `.version`) and
unregisters the builder. `cargo xtask distro clean` removes a builder a
failed run left behind.

Nothing third-party that the user works with (agent CLIs, language
toolchains) is part of the image; those are managed tools installed into
the user's home.
