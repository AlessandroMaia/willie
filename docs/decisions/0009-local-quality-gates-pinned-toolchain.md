# 0009 — Local-only quality gates on a pinned toolchain

- **Date:** 2026-08-25
- **Status:** accepted

## Context

Willie is developed by one person, largely with AI coding agents, on two
Windows machines. Quality must be enforced where the work happens, with
one command an agent can run, and results must be identical on both
machines.

## Options

| Option                          | For                                        | Against                                     |
| ------------------------------- | ------------------------------------------ | ------------------------------------------- |
| Local gate (`just check`) + hook | works offline; no remote accounts; one command | relies on discipline to run it          |
| Hosted CI pipelines             | enforced on push                           | remote setup and secrets; no value for solo |

## Decision

- The Rust toolchain is pinned to an exact version in
  `rust-toolchain.toml`; `Cargo.lock` is authoritative (`--locked`).
- `just check` = format check, clippy with warnings as errors, all tests,
  and `check-refs`. A pre-commit hook runs the fast subset.
- Development tasks with any logic live in the `xtask` crate; `just`
  recipes are single native commands so exit codes propagate.
- No remote CI, release automation or dependency bots.

## Consequences

- Every machine reproduces the same result from the same commit.
- Linux crates are cross-compiled to musl from Windows; `cargo check` on
  Windows only sees their platform stubs, so `just build-linux` is part of
  finishing any change to them.
- Release notes are curated by hand in `releases/`.

## Not decided

Reintroducing hosted checks if a second contributor ever appears.
