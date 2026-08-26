# 0004 — The package is pure: no bundled third-party tools

- **Date:** 2026-08-25
- **Status:** accepted

## Context

An installer could ship the agent CLI and language toolchains inside the
distribution image so that everything works offline on first run. That
couples Willie's release cycle to theirs, inflates the image to gigabytes
and makes Willie responsible for software it does not own.

## Options

| Option                         | For                             | Against                                        |
| ------------------------------ | ------------------------------- | ---------------------------------------------- |
| Pure package, managed tools    | small image; independent updates; clear ownership | first use needs network to install tools |
| Everything embedded            | offline; fully predictable      | 1–2 GB image; every update is heavy            |
| Minimal stub, download all     | tiniest installer               | fragile first run; supply chain exposed        |

## Decision

The installer contains only Willie: the desktop app, the engine, the Linux
binaries and a minimal base image. Agent CLIs and toolchains are *managed
tools*: detected in the distribution, shown with their versions, and
installed or updated from their official sources only on the user's
explicit action.

## Consequences

- The image is tens of megabytes and changes rarely.
- Managed tools install into the user's home, never through the image's
  package manager, so the system zone stays replaceable.
- Willie must be useful before any tool is installed (`doctor`, projects,
  settings) and must explain clearly what is missing.

## Not decided

Whether Willie may pre-fetch tool installers for offline use. Rejected
for now: it blurs the line the decision draws.
