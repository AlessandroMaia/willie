# 0011 — ext4 workspace for projects; DrvFs is a warned fallback (measured)

- **Date:** 2026-08-26
- **Status:** accepted

## Context

Spike S5 asked how much slower an agent works when the repository sits
on the Windows drive (`/mnt/c`, DrvFs over 9p) than on the
distribution's own ext4 disk. The answer decides whether F1 merely warns
about `/mnt/c` or ships an ext4 workspace from the start.

Method: the same commit of this repository (129 tracked files) on both
sides — the working copy under `C:\github\…` and a `git clone` of it
under `/home/willie/s5/willie` — measured inside the `willie`
distribution (WSL 2.6.1, kernel 6.6.87, Debian 13, git 2.47.3,
ripgrep 14.1.1) with `hyperfine -N --warmup 2 --runs 10`, plus a first
run after a pause as the closest thing to a cold cache. Windows-native
`git status` (git 2.52.0.windows.1, `Measure-Command`, 10 warm runs) is
the third reference point. DrvFs mount options at the time:
`9p, cache=5, msize=65536, metadata`.

## Findings

| Command (warm, mean) | DrvFs `/mnt/c` | ext4 | Windows native | Ratio DrvFs/ext4 |
| --- | --- | --- | --- | --- |
| `git status --porcelain` | **511 ms** ± 45 | **4.9 ms** ± 0.4 | 70.7 ms | ~100× |
| `rg -c fn <repo>` | 118 ms ± 11 | 6.3 ms ± 0.4 | — | ~19× |
| `git ls-files` | 23.7 ms ± 1.0 | 3.0 ms ± 0.4 | — | ~8× |
| `find` traversal (143 / 129 files) | 163 ms | 6 ms | — | ~27× |
| First `git status` after a pause | 3 140 ms | 104 ms | — | ~30× |
| First `rg` after a pause | 119 ms | 13 ms | — | ~9× |

Every per-file operation pays a 9p round trip on DrvFs, so the cost
grows with the number of files the tool touches: `git status` on this
tiny tree already costs half a second; a Node or monorepo checkout with
tens of thousands of files would put each `git status` — which agents
run before and after every edit — at several seconds, and `rg` over the
tree in the same range. Windows-native git on the same files is 7× faster
than git inside WSL on DrvFs and 14× slower than git on ext4.

Claude Code start-up (`claude --version`, 2.1.246, native binary in the
user's home) is the same on both sides — 18.4 ms ± 1.8 with the DrvFs
checkout as cwd, 17.8 ms ± 3.0 with the ext4 one — so the location of
the repository costs nothing until the agent starts touching files;
then the table above applies to every `git status`, search and read.

## Decision

F1 gives every project an **ext4 workspace inside the distribution** as
the default place where the agent runs, and treats a repository that
stays on `/mnt/c` as an explicit, warned choice — `doctor` and the
Projects screen show the warning with these numbers behind it. How the
workspace is populated (clone, move, or two-way sync with the Windows
checkout) and how Windows tools reach it (`\\wsl.localhost\willie\…`)
are design questions for F1, not for this record.

## Consequences

- F1's scope grows by the workspace mechanism; it does not shrink to a
  warning. The alternative — shipping `/mnt/c` first and adding ext4
  later — would make the first agent sessions unusably slow on any
  realistic repository and teach users the wrong workflow.
- Golden rule 1 ("state lives in the distro") extends naturally to
  project workspaces; backups and migration of `ext4.vhdx` become a
  user-visible concern earlier (data-preserving upgrade, §2.4).
- The measurement recipe is reproducible from the commands above; rerun
  it when WSL changes its DrvFs transport (the 9p `cache` mode or a
  virtio-fs successor could move these numbers a lot).
