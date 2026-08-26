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

## Installing

`just distro-install` registers the built image (70 MB compressed) as
the `willie` distribution under `%LOCALAPPDATA%\Willie\data\distro`,
unpacking to an `ext4.vhdx` of about 280 MB. Any distribution named
`willie` from an earlier install is terminated and unregistered first,
so the developer always runs the image now in `target/distro`.
`just distro-uninstall` removes it again, discarding that disk.

First run of a freshly installed image (`willie doctor`, exit 0; the
`curl` banner is elided here):

```text
[ok ]  unprivileged user            uid 1000
[ok ]  state dir                    /var/lib/willie
[ok ]  run dir                      /run/willie
[ok ]  bubblewrap                   bubblewrap 0.11.0
[ok ]  git                          git version 2.47.3
[ok ]  curl                         curl 8.14.1 (x86_64-pc-linux-gnu) …
[ok ]  user namespaces              ok
[skip] landlock LSM
[ok ]  network (api.anthropic.com)  ok
```

`landlock LSM` skips because the WSL kernel exposes no
`/sys/kernel/security/lsm`, so the check reports `kernel without
Landlock: sandboxing will be reduced` — not required, but the sandbox
slice has to account for it. `/run/willie` is created by the `wsl.conf`
boot command as `drwxr-x--- willie willie`.

The daemon answers on stdio, one JSON-RPC line in, one out, and exits 0
on EOF:

```text
> {"jsonrpc":"2.0","id":1,"method":"daemon.hello","params":{"client":"manual","willie_version":"0.1.0","protocol_version":1}}
< {"jsonrpc":"2.0","id":1,"result":{"distro_image_version":"0.1.0+9239cc0","protocol_version":1,"willie_version":"0.1.0"}}
```
