# Sandbox part 2, phase 5 — the applied and denied state in the interface

**F3b, phase 5 of five, the last.** Everything the earlier phases record in
the session's `sandbox` field — what applied, what the kernel could not
offer, what was denied and how often — exists only in the event log and the
session record. This phase shows it on the Sessions screen, so a person
sees the boundary a session ran under and what it refused without reading
`events.jsonl` by hand.

Read `designs/sandbox.md` — "Explaining it". No new decision: this is
presentation.

## Problem

### The report is data with no reader

Phases 1–4 fold `sandbox_applied` and `sandbox_denied` into
`Session.sandbox` (`applied`, `unavailable`, `degraded`, `denied`), and the
daemon already emits `session_changed` on every event, so a denial reaches
the client in the same second. But the Sessions screen shows a session's
project, harness, client count and state, and nothing of its sandbox. "What
did this session run under" and "what did it try that was refused" have no
answer in the interface.

## Goals

- Each session row shows its sandbox posture: the mechanisms applied,
  anything the kernel could not offer, anything that degraded, and the
  denials with their counts.
- A finished session keeps its denial history — the useful part of a
  session that already ended.
- The layering holds: `lib` returns facts, the feature chooses the tone; no
  colour literal, no tone in `lib`.
- Pure facts and the composition are unit-tested; no new distro test (the
  data path is proven by phases 1–4).

## Non-goals

- **A dedicated Sandbox screen.** There is no new truth to compute; the
  session row is where someone comparing sessions already looks.
- **The full argument vector in the row.** `designs/sandbox.md`'s open
  question favoured the applied state and denials, not the vector, which is
  long and belongs beside the capability editor. This phase answers that
  question that way.
- **Editing anything.** The row shows the resolved, applied state;
  capabilities are edited in the Sandbox dialog (part 1).
- **Wiring `sandbox explain`'s filter summary into the app.** That is the
  named `explain` follow-up, not this phase.

## Design

### The wire mirror — `lib/proto.ts`

```ts
export interface Denied {
  class: "syscall" | "terminal";
  name: string;
  count: number;
  first_at: string;
  last_at: string;
}
export interface SandboxState {
  applied: string[];
  unavailable: string[];
  degraded: string[];
  denied: Denied[];
}
```

`Session` gains `sandbox: SandboxState`. Every field defaults empty, because
a session recorded before part 2 has no such data — the TS type takes it as
optional and normalises to empty, so an old session renders as "unknown
posture", never as a crash.

### The facts — `lib/domain/sessions.ts` (no React)

```ts
export type Posture = "full" | "reduced" | "unknown";

/** full: everything reported applied. reduced: something is unavailable or
 * degraded. unknown: nothing reported yet (creating, or a pre-part-2 log). */
export function sandboxPosture(session: Session): Posture

/** Denials sorted by count, with the total. */
export function denials(session: Session): { items: Denied[]; total: number }
```

Both pure, both unit-tested. `sandboxPosture` returns `unknown` when
`applied` is empty (nothing folded), `reduced` when `unavailable` or
`degraded` is non-empty, `full` otherwise.

### The presentation — `features/sessions/sandbox-line.tsx`

One composition, used by both `LiveRow` and `RecentRow`. A single line:

- a `muted` chip per applied mechanism, with a human label
  (`rlimits` → "limits", `seccomp` → "syscall filter", `landlock` → "path
  rules"; an unknown name shown as-is);
- a `warning` chip per unavailable mechanism ("path rules unavailable");
- an `error` chip per degraded mechanism;
- a `warning` badge "N denials" that expands the list —
  `unshare · syscall · ×3 · 2 min ago`, `clipboard · terminal · ×1`;
- posture `unknown` shows a `muted` "no sandbox report", never a guess.

Colour only through `Tone` (`components/tone.ts`). The human labels live in
the feature, not in `lib`. If `components/ui/` lacks a collapsible or
popover for the expandable list, it is added through the registry as
`AGENTS.md` requires, never hand-written.

### Errors and edge cases

| Condition | Behaviour |
| --- | --- |
| a session with no sandbox data (pre-part-2, or still `creating`) | "no sandbox report", posture unknown |
| a mechanism name the UI does not label | shown verbatim |
| many denials | listed by count, the total on the badge; the list scrolls in its own container |

No new error code; a missing report is a posture, not a failure.

## Testing

Frontend (vitest), on any host:

- `sandboxPosture` for full / reduced (unavailable, degraded) / unknown
  (empty, creating);
- `denials` ordering by count and the total;
- the composition renders each case: applied chips, an unavailable warning,
  a degraded error, the denial badge with the list expanded, and the
  "no sandbox report" line;
- Biome and `tsc` clean in the gate.

No distro test: the data path (event → fold → `session_changed`) is proven
by phases 1–4.

## Rollout / compatibility

TypeScript-only. `Session.sandbox` is additive and defaulted, so a session
without it renders as unknown posture. No protocol, image or Rust change.
The release note says the Sessions screen now shows what each session's
sandbox applied and what it denied.

`designs/sandbox.md`'s open question — "Does `sandbox explain` belong in the
session row as well?" — is answered here: the applied state and the
denials, yes; the full argument vector, no.

## Open questions

- Should the denial list show the syscall's own remediation ("this session
  tried to trace a process; nothing you run should")? Favoured: not yet —
  the name and count are the fact; a glossary of every denied syscall is a
  larger surface than this phase warrants.
