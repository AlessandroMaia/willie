# Testing

`just check` is the only gate. It runs formatting checks, clippy with
warnings as errors, every test below, and the reference denylist scan.

## Layers

| Layer            | Where                                   | Runs on          |
| ---------------- | --------------------------------------- | ---------------- |
| Unit             | `#[cfg(test)] mod tests` per file       | any host         |
| Contract         | `willie-proto` round-trip tests         | any host         |
| Integration      | `crates/<crate>/tests/*.rs`, spawn bins | Windows or Linux |
| Linux (distro)   | inside the Willie distribution          | Linux only       |
| Frontend         | `apps/willie-app/src/**/*.test.ts(x)`   | any host         |
| Manual checklist | `docs/checklists/<slice>-acceptance.md` | human            |

Manual checklists live in `docs/checklists/`; a slice is not done until
its checklist has been walked and the results recorded in it.

## Rules

- A behaviour change ships with a test that fails before it and passes
  after. A bug fix ships with the regression test first.
- Test names are assertive sentences: `unknown_fields_are_ignored`, not
  `test_deser`.
- Tests are hermetic: paths, clock and network are injected. Shared
  fixtures live in one `#[cfg(test)]`-only module per crate.
- Integration tests use the real binary (`env!("CARGO_BIN_EXE_<name>")`),
  a private temp directory and a scrubbed environment.
- Tests that need the distribution or WSL detect the prerequisite at
  runtime and **skip with a printed reason** when it is missing; they
  never fail because the machine lacks it.
- Tests that disturb the registered distribution or boot the VM are
  opt-in through `WILLIE_TEST_DISTRO`; `just check` runs them only when
  it is set. Set it once (`$env:WILLIE_TEST_DISTRO = "willie"`) before
  claiming a slice done.
- No test is `#[ignore]`d without a comment naming the reason and the
  condition that removes it.
- Compatibility: when a serialised format changes, add a regression test
  with the **old** shape before changing the code; never delete one.

## Sandbox tests (Linux, inside the distro)

Prove negatives, not just positives: a capability that is off must be
observed as denied (the operation fails with the expected error), and the
capability sets must not overlap. Kill the daemon during a session and
assert the session is still alive and attachable.
