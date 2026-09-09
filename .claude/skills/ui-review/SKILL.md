---
name: ui-review
description: Use when reviewing, fixing or reporting on Willie's desktop UI — captures every screen in both themes from the running frontend, measures the live DOM instead of guessing from a screenshot, and builds the review page the user comments on.
---

# UI review

Willie's frontend is a Tauri app, but its screens are ordinary web
pages. This harness runs them in a headless Chromium with the Tauri
bridge answered by fixtures, so every screen can be opened, driven,
measured and captured without WSL, the distribution or the daemon.

**Never report a UI defect you have not measured.** A screenshot shows
a symptom; `probe.mjs` and `measure.mjs` show the geometry that caused
it. Two of the first findings this harness produced were fixture bugs,
not app bugs.

## Before anything

The frontend must be serving:

```
just ui-dev      # Vite alone, no Tauri window — what the harness needs
just dev         # the full app, if you also want the real window
```

Every script fails closed with that instruction when nothing answers on
`http://localhost:1420`. Point `WILLIE_UI_URL` elsewhere to override,
and `WILLIE_UI_BROWSER` at a browser binary when neither Chrome nor
Edge is where it is expected.

## Capture

```
just ui-shots
```

Writes `target/ui-shots/{dark,light}/<screen>.png` — the ten routes
plus three states no URL reaches (setup drawer, system selector,
collapsed sidebar). `target/` is ignored, so screenshots are never
versioned.

Two rules the capture already honours, and any new screen must too:

- **Reload between screens.** Route to route is a hash change, not a
  reload; without a fresh document the previous screen's React state
  leaks into the next shot (a collapsed sidebar, an open drawer).
- **Both themes.** The app follows `prefers-color-scheme`, which CDP
  emulates. A screen reviewed in one theme is half reviewed.

## Measure

```
node .claude/skills/ui-review/measure.mjs
node .claude/skills/ui-review/probe.mjs /session '[data-slot="status-bar"]'
node .claude/skills/ui-review/probe.mjs /session '[data-slot="sidebar-container"]' --collapse
```

`measure.mjs` audits one invariant across every route and exits
non-zero when it breaks: **the window itself never scrolls.** The
header and the status bar are fixed rows; each screen scrolls inside
the centre pane. Content taller than the pane is clipped by the scroll
area and is not a defect — only `document.scrollHeight` exceeding
`clientHeight` is.

`probe.mjs` prints the rect, text and markup of whatever a selector
matches, which is how a vague "that looks off" becomes "the button is
18px inside a 48px rail while its glyph is 24px".

## The fixtures

`bridge.js` answers every `invoke` the app makes. When a screen renders
empty or wrong, suspect it before the app:

- Timestamps are **whole-second epochs as strings**, the daemon's wire
  format. An ISO date renders as `unknown`.
- Shapes mirror `apps/willie-app/src/lib/proto.ts` and the engine's
  `EngineStatus`. When a type changes there, change it here.
- Unknown commands resolve to `null` rather than throwing, so a new
  screen renders before its fixture exists — check the console errors
  every script prints.

## The review page

```
node .claude/skills/ui-review/gallery.mjs [findings.mjs]
```

Reads `target/ui-shots/findings.mjs` (this round's content, rewritten
every pass — not tooling, which is why it lives under `target/`),
inlines every screenshot and writes `target/ui-shots/review.html`.
Publish that file as an Artifact so the user can comment on individual
screens; republish the same path to keep the link.

The findings file exports three things:

```js
export const BRANCH = "fix/ui-polish";

export const FINDINGS = [
  {
    id: "f-rail",              // stable: the user quotes it back
    state: "fixed",            // fixed | open | question
    where: "Ctrl+B",           // shown in the left column
    anchor: "sidebar-collapsed", // the screen id it links to
    title: "One sentence naming the defect",
    body: "What it is and why, with <code>markup</code> allowed.",
    evidence: {                // optional, but prefer having one
      head: ["", "before", "after"],
      rows: [["document scrollHeight", "756", "720"]],
    },
  },
];

export const SCREENS = [
  {
    id: "session",             // matches the screenshot filename
    route: "#/session",
    title: "Session — Ctrl+1",
    notes: [["open", "One line, one observation."]],
  },
];
```

## Working rhythm

1. `just ui-dev`, then `just ui-shots`.
2. Look at the screenshots. For anything suspicious, `probe.mjs` it
   before writing it down.
3. Fix on a branch, with a test that fails before the change. Layout
   invariants are testable in jsdom as class contracts — see
   `the_sidebar_is_as_tall_as_the_body_row_not_the_window` in
   `apps/willie-app/src/app/__tests__/shell.test.tsx`.
4. Re-capture, rebuild the page, republish to the same Artifact URL.
5. Commit only when the user asks.
