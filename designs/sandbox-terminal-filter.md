# Sandbox part 2, phase 4 — the terminal filter

**F3b, phase 4 of five.** The one channel that crosses the boundary by
design — the harness's output, on its way to whoever is attached — gets the
one defence that is not a kernel mechanism. A byte-stream parser drops the
sequences that act on the host or echo attacker-controlled text back into
the input stream, and passes everything that draws. What it drops it also
records, in the same `sandbox_denied` event the syscall filter uses.

Read `designs/sandbox.md` — "The terminal filter". This phase's decision is
0020.

## Problem

### The output is a control language, and nothing filters it

The supervisor copies the harness's output to every attached terminal
verbatim. That output is not just text: escape sequences can write the
host's clipboard (OSC 52), ask the emulator a question it answers *back
into the input stream* (device queries), and change the window title. An
agent acting on injected instructions can emit any of these. Willie has two
output paths — the Windows Terminal tab and the embedded xterm.js terminal
— and neither filters anything. The sandbox's kernel mechanisms cannot
help, because this channel is the session's whole purpose.

### Not every query is dangerous, and the harness depends on the safe ones

The naive fix — drop every sequence the emulator answers — breaks the
harness. Claude Code queries the terminal for its capabilities (primary
device attributes, background colour, kitty keyboard support) and needs
those replies to render. The queries worth dropping are the ones whose
reply carries *attacker-controlled text*: OSC 52's clipboard read, the
title report, DECRQSS, XTGETTCAP. The queries whose reply is a fixed,
harness-independent form (DA, DSR, background colour) must pass.

## Goals

- A parser between the PTY and the attach fan-out that drops the acting and
  text-echoing sequences and passes everything that draws, including a
  full-screen TUI's redraws untouched.
- Bounded memory: a sequence split across reads, and a 100 KiB hyperlink or
  inline image, must not buffer without limit.
- What it drops is recorded as `sandbox_denied { class: "terminal", name }`,
  coalesced like the syscall denials.
- Pure and portable: the parser is a byte automaton with no I/O, tested on
  any host.

## Non-goals

- **`TIOCSTI` (input injection).** The kernel refuses it and the syscall
  filter denies it (phase 2). This filter is about *output*.
- **A per-project switch to restore the acting sequences.** The design
  favours not yet: nothing needs them, and a switch that restores them is a
  capability that widens the one channel the boundary cannot otherwise
  defend.
- **Dropping the fixed-form queries** (DA, DSR, DECRQM, OSC 10/11, kitty,
  size reports). The harness depends on them and their replies carry no
  attacker text.
- **Rewriting or sanitising the passed sequences.** A sequence either
  passes whole or is dropped whole; the filter does not edit content.

## Design

### Where it sits — `willie-sess/src/vt_filter.rs`, `server.rs`

Between `pty::read` and `broadcast` in `serve()`, before the ring buffer,
so a late terminal's replay is already filtered and there is one filter
point for both output paths. It is not under `sandbox/`: it does not
confine a process, it sanitises bytes that are supposed to leave. The
alternate-screen tracker keeps working because it reads the already-filtered
stream and `CSI ? 1049 h/l` passes.

The parser owns state across calls (a sequence can arrive split between
reads) and returns, per input chunk, the bytes to forward plus the names of
what it dropped, which `serve()` feeds to the shared `Tally`.

### What it drops and what it passes

| Dropped | Passed |
| --- | --- |
| OSC 52 (clipboard write and read) | OSC 4, 10, 11, 12 including the `?` queries |
| OSC 0, 1, 2 (title and icon), and the unnumbered OSC (empty selector defaults to 0 on a supported output path) | OSC 7, 8, 9, 133, and every unlisted numbered OSC |
| `CSI … t` except the size/position reports 11, 13, 14, 16, 18, 19; and every `CSI > … t` (title modes) | DA1/2/3, DSR, DECRQM, XTVERSION, `CSI ? u` (kitty) |
| DCS `$q` (DECRQSS) and DCS `+q` (XTGETTCAP) | every other DCS, tmux passthrough included |

The names recorded: `clipboard` (OSC 52), `title` (OSC 0/1/2, `CSI > t`),
`window` (the acting `CSI t`), `query_echo` (DECRQSS, XTGETTCAP).

### C1 controls only through UTF-8 — the honesty of the filter

xterm.js interprets C1 controls *after* UTF-8 decoding, so `C2 9D` (U+009D,
the OSC introducer, as UTF-8) is as good an OSC opener as `ESC ]`. The
filter treats the two-byte UTF-8 forms `C2 80`–`C2 9F` as C1 introducers
and `C2 9C` as ST. Raw bytes `0x80`–`0x9F` are **not** interpreted: in
UTF-8 they are continuation bytes, and treating them as C1 would corrupt
text. This is the difference between a filter that can be bypassed and one
that cannot.

### Bounded memory

- **CSI** accumulates until the final byte, capped at 256 bytes (clear of
  the longest real sequence — a truecolor SGR combining a 24-bit
  foreground, background and underline colour is on the order of fifty
  bytes). Past the cap the buffer freezes and the CSI is **dropped
  closed** at its final byte: nothing of it is forwarded, so leading-zero
  or padding bytes cannot re-form an acting `… t` op past the cap. The
  first parameter is read numerically, so `018 t` is the 18 size report.
- **OSC / DCS** accumulate only until the type is identified — the OSC
  number (parsed numerically, as the emulator parses it, so leading zeros
  collapse and never grow the buffer), or the DCS's params and
  intermediate before the `q` final — capped at 16 bytes. Once identified
  the sequence is either streamed-and-dropped until its terminator (BEL or
  ST) or streamed-through: a 100 KiB OSC 8 hyperlink or an inline image
  passes without buffering. An OSC number or a DCS identify window that
  runs past the cap is **dropped closed**, not passed: an id or query that
  long is only ever adversarial padding.
- A sequence truncated at the end of the stream emits nothing and waits for
  the next chunk; it never forwards a half-sequence.

### Recording the drops — reuses phase 2's `Tally`

The dropped sequence's name goes into the same `Tally` in `Shared`, emitting
`sandbox_denied { class: "terminal", name, count }`, coalesced: first drop
of each name immediate, repeats folded to at most every 5 s and at
`finish`. `apply_event` folds it into `SandboxState.denied` by (class,
name), so a discarded clipboard write shows on the session's row (phase 5)
next to the syscall denials.

### Errors and edge cases

| Condition | Behaviour |
| --- | --- |
| a sequence split across reads | state survives; forwarded or dropped whole when complete |
| a 100 KiB OSC 8 or inline image | streamed through, internal buffer stays under the identify cap |
| an acting sequence padded past the cap | dropped closed once the cap is hit — nothing of it is forwarded |
| a malformed / never-terminated non-acting sequence | withheld at end of stream; a filter never forwards a half-sequence |
| a C1 control as raw `0x9D` | passed as a UTF-8 continuation byte, never treated as an introducer |
| a C1 control as `C2 9D` | treated as the OSC introducer, filtered like `ESC ]` |

A filter does not fail; there is no error code.

## Testing

Host, no kernel:

- a synthetic full-screen stream (SGR colour, cursor motion, `CSI ? 1049
  h`, `CSI ? 25 l`, DECSTBM, multibyte UTF-8) passes byte-for-byte;
- OSC 52 with BEL and with ST, whole and split into three chunks, is
  dropped;
- title (OSC 2) dropped; `CSI 21 t` dropped; `CSI 18 t` passes;
- DA (`CSI c`), DSR (`CSI 6 n`), `OSC 11 ; ?` pass;
- DECRQSS (`DCS $ q`) dropped; `DCS tmux ; …` passes;
- OSC 52 encoded as C1 in UTF-8 (`C2 9D … C2 9C`) dropped; a lone `0x9D`
  passes;
- a 100 KiB OSC 8 passes with the internal buffer asserted under the cap;
- a sequence truncated at end-of-stream emits nothing;
- the `Tally` records `clipboard`, `title`, `query_echo` with counts;
- in `server.rs`, `serve()` filters before the ring (a dropped sequence is
  absent from a late client's replay).

Distro, through `just test-linux`:

- a fake harness emits an OSC 52 and an OSC 2; a test terminal client
  receives neither, and `sandbox_denied` with `class: "terminal"` is in the
  log.

## Rollout / compatibility

`sandbox_denied` already carries `class` (added in phase 2), so this phase
adds no protocol field — only new `name`/`class` values. No image or
config change. A user-visible consequence for the release note: the agent's
output can no longer change the tab or window title (which belongs to
Willie) or write the host clipboard; everything the terminal draws is
unchanged.

This is design rollout item 9 (the terminal filter).

## Open questions

- Should the passed-through OSC 8 hyperlink be length-capped as a courtesy
  against a pathological line? Favoured: no — it is drawing, not acting, and
  a cap is a second policy to maintain; the emulator already bounds what it
  renders.
- Should `CSI 14 t` (text-area pixel size) be dropped as a fingerprinting
  vector? Favoured: no — its reply is a fixed form and some TUIs use it for
  sixel sizing; it is in the passed set with the other size reports.
