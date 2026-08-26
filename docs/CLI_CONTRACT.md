# CLI contract

Applies to every Willie executable with a command line: `willie`,
`willied`, `willie-sess` and `cargo xtask`.

## The one rule: stdout is data, stderr is decoration

- **stdout** carries only output another program can consume: JSON when
  `--json` is given, otherwise plain lines with a stable shape.
- **stderr** carries everything meant for a human: progress, warnings,
  errors, hints, banners.
- Never print status decoration to stdout. Never print data to stderr.

## Exit codes

| Code | Meaning                                                        |
| ---- | -------------------------------------------------------------- |
| 0    | success                                                        |
| 1    | the operation failed; details on stderr                        |
| 2    | usage error, or the binary does not run on this platform       |
| 3    | precondition not met (`doctor`-style checks) — fixable by user |
| ≥64  | reserved for propagating a child process's exit code           |

## Errors

An error message is one sentence saying what failed, followed when
possible by one line saying what to do about it. Errors carry a stable
`snake_case` code; in `--json` mode the object is
`{"code": "...", "message": "...", "remediation": "..."}` (remediation
optional).

## Colour and terminal escapes

- Colour only when stderr is a TTY and `NO_COLOR` is unset; `--color
  always|never|auto` overrides.
- Raw ANSI sequences are written by one module per binary; nothing else
  emits escapes.
- Any text that originated outside the program (file contents, process
  output, network data) is sanitised before printing: control bytes are
  replaced so a hostile string cannot drive the user's terminal.

## Diagnostics (`doctor`)

One line per check, machine-readable prefix, then the human part:

```text
[ok ]  wsl.exe                     2.6.1
[FAIL] MSVC linker (link.exe)      elevated prompt: winget install …
[skip] Windows Terminal (wt.exe)   optional
```

Exit 3 when a required check fails, 0 otherwise.

## Adding a command — checklist

1. Data to stdout, everything else to stderr.
2. `--json` supported when there is data.
3. Exit codes from the table above; never invent new ones below 64.
4. Every failure path has a code and a remediation.
5. Help text (`--help`) is exact about defaults.
6. Unit tests for argument parsing; an integration test that spawns the
   binary for at least the happy path and one failure.
