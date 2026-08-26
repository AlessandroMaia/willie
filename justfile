# Willie task runner. Run `just` to list recipes.
#
# Recipes are single native commands per line so exit codes propagate
# unchanged through PowerShell. Anything with real logic lives in the
# `xtask` crate (`cargo xtask …`), not in shell.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]

web := "apps/willie-app"

default:
    @just --list --unsorted

# ---------------------------------------------------------------- setup

# Verify the local toolchain and print install hints for what is missing.
[group('setup')]
ensure:
    cargo xtask doctor-tools

# Install the repository git hooks (pre-commit runs `just precommit`).
[group('setup')]
hooks:
    git config core.hooksPath .githooks

# Install frontend dependencies from the lockfile.
[group('setup')]
web-install:
    pnpm -C {{web}} install --frozen-lockfile

# -------------------------------------------------------------- quality

# Format Rust and frontend sources.
[group('quality')]
fmt:
    cargo fmt --all
    pnpm -C {{web}} format

# Fail if anything is not formatted.
[group('quality')]
fmt-check:
    cargo fmt --all --check
    pnpm -C {{web}} exec biome format .

# Clippy with warnings as errors, Biome, and TypeScript type-check.
[group('quality')]
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    pnpm -C {{web}} lint
    pnpm -C {{web}} type-check

# Rust unit/integration tests and frontend tests.
[group('quality')]
test:
    cargo test --workspace --locked
    pnpm -C {{web}} test

# Fail if versioned content mentions anything on the local reference denylist.
[group('quality')]
check-refs:
    cargo xtask check-refs

# The full local gate. Green here means the change is done.
[group('quality')]
check: fmt-check lint test check-refs

# Fast gate used by the pre-commit hook (no tests).
[group('quality')]
precommit: fmt-check check-refs lint

# ------------------------------------------------------------------ dev

# Run the desktop app with hot reload.
[group('dev')]
dev:
    pnpm -C {{web}} tauri dev

# Cross-compile the Linux binaries (daemon, session supervisor, CLI) to musl.
[group('dev')]
build-linux:
    cargo xtask build-linux
