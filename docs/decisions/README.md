# Decision records

One file per decision, numbered in the order taken:
`NNNN-<kebab-slug>.md`. A record is never edited to change its meaning;
a later record supersedes it and both link to each other.

## Format

```markdown
# NNNN — <Decision as a sentence>

- **Date:** YYYY-MM-DD
- **Status:** accepted | superseded by NNNN

## Context
Why a decision was needed; the constraints that mattered.

## Options
| Option | For | Against |

## Decision
What was chosen, in one paragraph.

## Consequences
What becomes easier, what becomes harder, what must now be true.

## Not decided
Things deliberately left open or explicitly rejected for now, and why.
```

## Index

| #    | Decision                                                      |
| ---- | ------------------------------------------------------------- |
| 0001 | Willie is a control plane for agent CLIs, not an agent        |
| 0002 | Windows is the only platform; WSL2 is the mechanism           |
| 0003 | Agents run inside the Willie distribution                     |
| 0004 | The package is pure: no bundled third-party tools             |
| 0005 | The Linux daemon owns the truth; stdio is the first transport |
| 0006 | One detached supervisor per session                           |
| 0007 | Sandbox capabilities are named, positive and monotonic        |
| 0008 | Debian slim image, no systemd, three data zones               |
| 0009 | Local-only quality gates on a pinned toolchain                |
