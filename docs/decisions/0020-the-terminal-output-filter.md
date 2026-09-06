# 0020 — the terminal output crosses a byte filter that drops the sequences which act on the host or echo attacker text and passes everything that draws

- **Date:** 2026-09-06
- **Status:** accepted

## Context

The supervisor copies the harness's output to every attached terminal
verbatim, and that output is a control language, not just text. An escape
sequence can write the host's clipboard (OSC 52), change the terminal's
tab or window title (OSC 0/1/2), and ask the emulator a question it
answers *back into the input stream* — a reply the emulator types as if
the user had. An agent acting on injected instructions can emit any of
these, and both of Willie's output paths — the Windows Terminal tab and
the embedded terminal — render them. This is the one channel the kernel
sandbox cannot help with: it is not a syscall and not a mount, it is the
bytes the session exists to produce, and it leaves the boundary by design.

Not every sequence the emulator answers is dangerous, and the harness
depends on the safe ones. The harness queries the terminal for its
capabilities — primary device attributes, cursor position, background
colour, keyboard support — and needs those replies to render. Those
replies are a fixed, harness-independent form that carries no attacker
text; dropping them breaks the harness and buys nothing. The sequences
worth dropping are the ones that act on the host or whose reply echoes
attacker-controlled text back into the input: the clipboard, the title
report, and the two query-echo forms (DECRQSS, XTGETTCAP). And the
boundary the syscall filter set up already has a place to name a
refusal — `sandbox_denied` has carried a `class` since it was added
(decision 0018) — so a drop here can be recorded through the same tally.

## Options

What the filter does with the emulator's queries:

| Option | For | Against |
| --- | --- | --- |
| drop every sequence the emulator answers | one rule, nothing that answers can leak | breaks the harness: it needs the device-attributes, cursor-position and background-colour replies to render, and those replies carry no attacker text — rejected |
| drop only the acting and echo-controlled sequences, pass the fixed-form queries | the harness renders, and the only sequences dropped are the ones that act on the host or echo attacker text back as input | the pass set has to be enumerated and kept true to what the harness needs — chosen |
| a per-project switch to restore the acting sequences | a workflow that turned out to need one would not be blocked | a switch that restores the acting sequences is a capability that widens the one channel the boundary cannot otherwise defend, and nothing needs it — rejected |

## Decision

A byte automaton in `willie-sess` (`vt_filter.rs`) sits in `serve()`
between the PTY and the attach fan-out, before the ring buffer, so there
is one filter point for both output paths and a late attach replays bytes
that are already filtered. It owns state across reads — a sequence can
arrive split between them — and returns, per chunk, the bytes to forward
plus the names of what it dropped, which `serve()` feeds to the same
`Tally` the syscall denials use.

It drops OSC 52 (the clipboard write and read), OSC 0/1/2 (the title and
icon), the acting `CSI … t` window-manipulation subfunctions and every
`CSI > … t` title mode, and DCS `$q` (DECRQSS) and DCS `+q` (XTGETTCAP);
it records these as `clipboard`, `title`, `window` and `query_echo`. It
passes everything else byte-for-byte: OSC 4/7/8/9/10/11/12/133 and every
unlisted OSC, the read-only size and position report subfunctions of
`CSI … t` (11, 13, 14, 16, 18, 19), the device-attributes, cursor-position,
mode and version queries, the extended-keyboard query (`CSI ? u`), and
every other DCS — the fixed-form queries the harness needs and the
sequences that merely draw, a full-screen redraw included. A sequence
either passes whole or is dropped whole; the filter never edits content.

A C1 control is interpreted only through its two-byte UTF-8 form
(`C2 80`–`C2 9F`, with `C2 9C` as the string terminator): the emulator
decodes UTF-8 before it reads C1, so the UTF-8 form of the OSC introducer
opens an OSC as surely as `ESC ]`. A raw `0x80`–`0x9F` is left alone — in
UTF-8 it is a continuation byte, and treating it as a C1 control would
corrupt text. Memory is bounded: a CSI accumulates to a 256-byte cap and
an OSC or DCS only until its type is identified, a 16-byte cap, after
which a dropped sequence is streamed-and-dropped to its terminator and a
passed one is streamed through — a 100 KiB hyperlink or an inline image
passes without ever buffering, and an over-cap acting sequence drops
closed. A sequence truncated at the end of a read forwards nothing and
waits for the next chunk; it never emits a half-sequence. The drops are
recorded as `sandbox_denied { class: "terminal", name }` through the
phase-2 tally, coalesced by (class, name) like the syscall denials and
flushed before `exited`.

## Consequences

- The agent's output can no longer set the terminal's tab or window
  title — which belongs to Willie — or write the host clipboard; each
  such attempt is a named line in the session's log next to the syscall
  denials.
- Everything the terminal draws is unchanged, a full-screen TUI
  included: the pass set is byte-for-byte, and the fixed-form queries the
  harness needs still get their replies.
- The filter is not under `sandbox/`: it does not confine a process, it
  sanitises bytes that are supposed to leave, so it lives on the
  supervisor's own output path rather than among the mount and syscall
  mechanisms.
- No protocol field is added: `sandbox_denied` has carried `class` since
  decision 0018, so this is new `class`/`name` values, not a new event; a
  reader that folds by (class, name) folds the terminal denials with no
  change.
- A dropped sequence is dropped whole and a passed one passed whole; a
  test asserts the internal buffer stays under the identify cap while a
  100 KiB sequence streams through, and that a sequence split across
  reads is forwarded or dropped only once it is complete.

## Not decided

- Length-capping a passed OSC 8 hyperlink against a pathological line.
  Rejected for now: a hyperlink is drawing, not acting, the emulator
  already bounds what it renders, and a cap is a second policy to
  maintain.
- Dropping `CSI 14 t` (the text-area pixel-size report) as a
  fingerprinting vector. It stays in the passed set with the other size
  reports: its reply is a fixed form and some full-screen programs use it
  for image sizing, so it carries no attacker text.
- A per-project switch to restore the acting sequences (above): a
  capability that widens the one channel the boundary cannot otherwise
  defend, left out until something needs it.
