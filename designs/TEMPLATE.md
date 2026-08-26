# <Feature name>

<!--
Copy this file to designs/<kebab-case-slug>.md. Keep the section order;
delete a section only when it truly does not apply and say so in one line.
Put the file path of the code a section describes in its heading.
-->

## Problem

What hurts today, for whom, and how we know (a measurement, a repeated
manual step, a failure mode).

## Goals

- Observable outcomes this design must deliver.

## Non-goals

- Things deliberately left out, with one line each on why.

## Design

### <Layer or component> — `path/to/file.rs`

How it works, the data it owns, the interfaces it exposes. Prefer a short
sequence diagram (ASCII) over prose for flows that cross a boundary.

### Errors and edge cases

Every failure mode and the user-visible behaviour, with its error code.

## Testing

Which tests prove each goal, and where they live (`#[cfg(test)]`, `tests/`,
inside the distro).

## Rollout / compatibility

Config or protocol changes, migrations, flags, what older builds see.

## Open questions

Decisions still pending, each with the option currently favoured.
