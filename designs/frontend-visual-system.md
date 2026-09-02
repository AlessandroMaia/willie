# Frontend visual system — tokens, primitives and the shell

**Stage 2 of the frontend rework.** Stage 1
(`designs/frontend-foundations.md`) gave every file a home and fenced
the layers without changing a pixel. This stage changes the pixels: a
token-based visual system on generated component primitives, a
sidebar-and-status-bar shell on a typed router with keyboard shortcuts,
and the three existing screens rebuilt on those primitives. Behaviour
stays: the same actions call the same commands under the same gates.

## Problem

### Four vocabularies, twelve literals, no system

`apps/willie-app/src/styles/styles.css` is 318 lines of hand-written
CSS. One concept — the tone of a state — is spelt four ways
(`.light-*`, `.chip-*`, `.badge-*`, `.check-*`), each repeating the same
three colour literals; the three appear twelve times in the file and
nowhere else in the tree knows them. Buttons, inputs and checkboxes are
the browser's defaults. The two modals are hand-rolled `div`s with no
focus trap and no `Escape`. The embedded terminal is constructed with no
theme, so it paints a black surface in both colour schemes.

### Presentation computed where truth lives

`lib/` is the layer that holds facts and no React, yet
`lib/domain/sessions.ts` exports `Tone` (`ok | muted | error | pending`)
and `stateChip()` returns one, and `lib/domain/health.ts` exports
`Light` (`red | yellow | green`). Two colour vocabularies, computed in
the domain, and different from each other. A component library brings
its own variant vocabulary; keeping a third one in the domain means
every state is translated twice.

### A shell that cannot grow

`app/app.tsx` is three tab buttons over a `useState`.
`docs/ARCHITECTURE.md` §5.1 owes Tools, Plugins, Settings and a tray;
nothing shows engine health without opening the Dashboard; the keyboard
does nothing. The `engine://status` subscription lives inside
`DashboardScreen`, so no other component can show health at all.

### Logic with no test address

Stage 1 kept every test in `lib/` because components owned no logic.
The shell now gets logic — which screen a route renders, what a
shortcut does, when `.dark` is applied, what the status bar says — and
has no place to prove it.

### An unwritten rhythm

The code runs statements together with no blank line between a hook,
the next hook and the `return`. The formatter preserves blank lines but
never adds them, and no lint rule exists for it, so the rhythm depends
on whoever typed the file.

## Goals

- One vocabulary: every colour, radius and font comes from tokens in one
  file, a `tone` variant replaces the four class families, and no
  colour literal exists outside `styles/globals.css`.
- A shell with sidebar navigation (six entries, three available now), a
  status bar that always shows engine health and live sessions, light
  and dark following Windows, and keyboard shortcuts for navigation.
- Primitives generated from a registry and never hand-edited; Willie's
  own components are compositions over them.
- `lib/` returns facts, never presentation.
- The shell's behaviour is tested, with the Tauri bridge the only thing
  faked.
- The three screens keep their behaviour and their data; only the
  rendering changes.

## Non-goals

- A command palette. It needs an action registry; shortcuts and `Kbd`
  hints give it an address without building it.
- TanStack Store, Table and Virtual. The hand-written snapshot store
  meets its contract and is tested; the lists are small until session
  history moves to SQLite.
- A custom title bar (window controls, drag regions). Its own design.
- A theme override in Settings. The theme effect is shaped for it, but
  there is no Settings screen yet.
- New screens. Tools, Plugins and Settings appear disabled; each is a
  slice of its own.
- Visual regression tooling. The manual walk is the check while the UI
  is still moving; a screenshot harness earns its place once it settles.
- The tray.

## Design

### Toolchain — `apps/willie-app/package.json`, `vite.config.ts`, `components.json`

Tailwind 4 through its Vite plugin. The component registry is
initialised with the preset `bcivVKXS`, which encodes: style `base-nova`
(primitives from Base UI), base colour zinc, CSS variables for theming,
Inter Variable, lucide icons, translucent menus with a subtle accent,
radius `0.625rem`, dark mode by class. `components.json` maps the
registry's aliases onto the layers stage 1 created:

| Alias        | Path                 | Why there                                       |
| ------------ | -------------------- | ----------------------------------------------- |
| `ui`         | `@/components/ui`    | the address stage 1 left empty                  |
| `components` | `@/components`       | compositions two features share                 |
| `hooks`      | `@/components/hooks` | the registry's hooks use React; `lib/` has none |
| `utils`      | `@/lib/utils`        | `cn()` is pure                                  |
| `lib`        | `@/lib`              |                                                 |

Dependencies, each justified in its commit:

| Package                                              | Role                                         |
| ---------------------------------------------------- | -------------------------------------------- |
| `tailwindcss`, `@tailwindcss/vite`                   | utility classes, compiled by Vite            |
| `@base-ui/react`                                     | unstyled, accessible primitives              |
| `shadcn`                                             | the registry's runtime `tailwind.css`        |
| `class-variance-authority`, `clsx`, `tailwind-merge` | variants and class merging                   |
| `lucide-react`                                       | icons                                        |
| `tw-animate-css`                                     | the primitives' enter and exit animations    |
| `@fontsource-variable/inter`                         | the UI font, bundled, works offline          |
| `@tanstack/react-router`                             | typed routes                                 |
| `@tanstack/react-hotkeys`                            | shortcuts; alpha, pinned to an exact version |
| `jsdom`, `@testing-library/react`, `@testing-library/user-event` (dev) | shell tests |

A registry over a component *library* dependency: generated files are
owned source, readable and fenced by the same lint as the rest, with no
version coupling between the library's React and the app's. The price
is a regeneration discipline, stated under "Primitives".

### Tokens — `apps/willie-app/src/styles/globals.css`

`styles.css` is deleted at the end of the stage; `globals.css` replaces
it and holds only what the preset generates plus three adjustments:

1. **Warm neutrals.** The zinc scale keeps its lightness steps and
   gains a slight warm hue with chroma kept very low, so the ground
   reads as paper in light mode and as warm charcoal in dark mode
   rather than as blue-grey. Values are tuned by eye against both modes
   in the first commit and recorded in a comment above the block.
2. **Semantic tokens** the registry does not ship: `success`, `warning`
   and `info`, each with a `-foreground`, defined for both modes and
   exposed to Tailwind through `@theme inline` as `--color-success` and
   friends. `destructive` already exists. These are the only colours a
   state may take.
3. **`color-scheme`** `light` on `:root` and `dark` under `.dark`, so
   scrollbars and native controls follow the theme.

Fonts: `--font-sans` is Inter Variable; `--font-mono` is
`"Cascadia Mono", Consolas, ui-monospace, monospace` — the system
stack, present on Windows 11, nothing downloaded. Paths, ids, versions
and the terminal use it.

Rule: no colour literal outside this file. A state is coloured through
a `tone` variant and nothing else.

### Domain without presentation — `lib/domain/health.ts`, `lib/domain/sessions.ts`

`Light` becomes `Health = "ok" | "degraded" | "failed"`, a fact about
the engine; `healthFor(part, status)` and `overallHealth(status)` return
it. `Tone` and `stateChip()` leave `lib/` entirely: the mapping from a
`SessionState` to a label and a tone is presentation, so it lives
beside the component that renders it, as `badgeFor(state)` in
`features/sessions/session-state-badge.tsx`, with its test in
`features/sessions/__tests__/`. The projects chip already lives in its
feature and only changes what it renders.

The rule this leaves behind: `lib/` returns states, health levels and
counts; `features/` and `components/` decide how they look.

### Engine status store — `lib/domain/engine-status.ts`, `store/use-engine-status.ts`

The second store, in the mould of the first. The pure half,
`createEngineStatusStore({ status, onStatus })`, holds
`{ status: EngineStatus | null, problem: Problem | null }`, refcounts
`acquire()`/`release()`, keeps the last good status when a refresh or
the subscription fails, and exposes `refresh()` for the Dashboard's
actions. It also exports `summarize(status)`: the overall `Health`, a
one-line headline (the first part that is not `ok`, in the order WSL,
distribution, daemon, doctor, or "Engine running"), and the daemon
version when running. The binding, `useEngineStatus()`, is the
`useSyncExternalStore` call plus the acquire/release effect.

```
engine://status ──▶ engine-status store ──▶ StatusBar (dot, headline)
engine_status   ──▶        │              ──▶ DashboardScreen (details,
                           └── refresh() ◀──    actions)
```

Unlike the snapshot store this one never boots anything: `engine_status`
reads the engine's own view. The Dashboard's local `status` state and
its subscription effect are deleted.

### Primitives — `apps/willie-app/src/components/ui/`, `components/`

The initial set, generated with the registry's `add` and limited to what
the screens use: `button`, `button-group`, `input`, `field`, `label`,
`checkbox`, `badge`, `tooltip`, `dialog`, `alert-dialog`, `alert`,
`separator`, `scroll-area`, `sidebar` (which brings `sheet` and
`skeleton`), `item`, `empty`, `spinner`, `kbd`, `dropdown-menu`,
`toast`. Adding one later is one `add`.

**Ownership rule.** `components/ui/` is generated only: never edited by
hand, so `add --overwrite` stays safe. Everything Willie-specific is a
composition in `components/`:

- `tone.ts` — the design system's vocabulary,
  `Tone = "ok" | "warning" | "error" | "pending" | "muted"`, and the
  class map from a tone to the semantic tokens.
- `status-dot.tsx` — a dot with a `tone`; replaces `.light-*`.
- `status-badge.tsx` — `Badge` with a `tone`; replaces `.chip-*`,
  `.badge-*` and `.check-*`.
- `problem-alert.tsx` — rebuilt over `Alert`, same props.
- `relative-time.tsx` — unchanged.

Regeneration runs from inside `apps/willie-app`: the registry CLI
resolves the project from its working directory, so `pnpm -C` does not
reach it.

`components/hooks/` holds the hooks the registry generates
(`use-mobile`). Both directories sit under the `components/**` fence
from stage 1 and need no new rule.

### Shell — `app/router.tsx`, `app/routes.ts`, `app/shell/`, `app/theme.ts`, `app/hotkeys.ts`

**Routes in code.** `createRootRoute` renders the shell layout with an
`Outlet`; one `createRoute` per screen — `/dashboard`, `/projects`,
`/sessions` — renders the feature's screen; the index redirects to
`/dashboard` and so does `notFoundComponent`, so a stale hash never
shows a blank. Hash history: it survives a dev-server reload and needs
no real URL under Tauri. The `Register` interface types every `Link`
and `navigate`. `app/routes.ts` stays the registry the sidebar reads,
now a row per entry: `id`, `path`, `label`, `icon`, `shortcut`,
`available`. Tools, Plugins and Settings are rows with
`available: false`: rendered disabled with a tooltip, no route, no
shortcut.

**Layout** (`app/shell/shell.tsx`, `sidebar.tsx`, `status-bar.tsx`).
`SidebarProvider` → `Sidebar collapsible="icon"` with the six entries
and their `Kbd` hints (in the tooltip when collapsed) → `SidebarInset`
holding a `ScrollArea` for the screen and the status bar pinned to the
bottom. The root element gets `isolation: isolate`, which Base UI needs
for its portals to stack correctly.

**Status bar.** Reads `useEngineStatus()` and
`useSnapshot(daemon === "running")` — the same gate the Dashboard uses,
so the bar never boots the daemon. Shows `StatusDot` with the overall
health (`ok`, `degraded`, `failed` map to the tones `ok`, `warning`,
`error`), the headline from `summarize()`, the daemon version, and the
count of live sessions when the daemon is running. The health part of
the bar — the dot and the headline — is a link to `/dashboard`, which
remains the detailed page with the doctor list and the actions; the
version and the count are plain text. A store `problem` renders as
tone `error` with its message.

**Theme** (`app/theme.ts`). `followSystemTheme()` reads
`matchMedia("(prefers-color-scheme: dark)")`, toggles `.dark` on
`<html>`, listens for `change`, and returns the cleanup; `app.tsx` runs
it in an effect. A future override from Settings is one more argument
to this function and touches no component. The terminal, the one
component that needs computed colours, watches the `class` attribute of
`<html>` with a `MutationObserver` and re-reads the tokens; it does not
import `app/`.

**Shortcuts** (`app/hotkeys.ts`). `HotkeysProvider` wraps the tree.
`useShellHotkeys()` registers `Mod+1`…`Mod+N` for the available rows in
registry order and `Mod+B` for the sidebar, with `preventDefault`. The
terminal's custom key handler lets `Mod+digit` through so the screen
shortcuts work while it has focus; `Mod+B` stays with the session,
because the agent it hosts and ordinary terminal programs bind it. A
feature that needs a shortcut calls `useHotkey` itself; the package
import crosses no fence. The shell is the only consumer in this stage.

### Screens — `features/health/`, `features/projects/`, `features/sessions/`

Same commands, same gates, new rendering:

- **Dashboard.** Health rows as `Item` with `StatusDot`; the doctor
  report as an `Item` list, each check with its tone and remediation;
  actions in a `ButtonGroup`; the install job with `Spinner`.
- **Projects.** Roots and discovery as `Item` lists with `Field`,
  `Input` and `Checkbox`; each project an `Item` with `StatusBadge`,
  the primary actions as buttons and the destructive ones behind a
  `DropdownMenu`; *remove* becomes an `AlertDialog`, *relocate* a
  `Dialog` with a `Field` — both trap focus and close on `Escape`.
  Transient confirmations (synced, roots saved) become a `toast`;
  errors with a remediation stay inline in `ProblemAlert`.
- **Sessions.** Rows as `Item` with `SessionStateBadge`; the terminal
  in a panel whose xterm instance receives `theme` (background,
  foreground, selection, cursor) read from the tokens and `fontFamily`
  from `--font-mono`.
- Every empty list renders `Empty` with the action that fills it.

Density is the preset's default — `text-sm`, `h-8` controls — which is
the compact register this tool wants.

### Conventions — `AGENTS.md`, `apps/willie-app/biome.json`, `package.json`

- `useSortedClasses` (nursery) at `error`, recognising `cn`, `cva` and
  `clsx`, so a class string has one canonical order.
- `pnpm format` becomes `biome check --write .`, so `just fmt` applies
  class sorting and import order along with formatting.
- Written in `AGENTS.md`, under coding conventions, for the frontend:
  one blank line between logical blocks inside a function (setup, each
  hook, handlers, the `return`) and between top-level declarations;
  imports in the linter's order (packages, then `@/`, then relative)
  with the editor's Biome extension organising on save; no colour
  literal outside `globals.css`; `components/ui/` only by generation.

### Errors and edge cases

- **Engine status fails.** The store keeps the last status and sets
  `problem`; the bar shows tone `error` with the message, the Dashboard
  shows `ProblemAlert`. Nothing goes blank.
- **Daemon stopped.** Headline "Daemon stopped", tone `warning`, no
  session count; the bar must not be what starts it.
- **Stale hash** (`#/tools`, an old bookmark): `notFoundComponent`
  redirects to `/dashboard`.
- **Shortcut while the terminal has focus.** xterm's key handler
  returns `false` only for `Mod+digit`, letting the shell switch
  screens; every other chord, `Mod+B` included, stays with the terminal,
  so collapsing the sidebar needs the pointer or focus outside the
  terminal.
- **`matchMedia` absent** (jsdom): the theme effect treats it as light;
  tests install a stub with a controllable `change`.
- **Regenerating a primitive** overwrites the file. Nothing is lost
  because nothing was hand-edited; compositions use public props only.
- **Fonts under a future CSP.** They ship as bundled assets; if the
  Tauri CSP is ever turned on, `font-src 'self'` is the line to add.
- **Alpha shortcuts API.** Pinned exactly; a bump is a deliberate commit
  that runs the shell tests.

## Testing

Pure, in `lib/domain/__tests__/` and `features/sessions/__tests__/`:

- `engine-status.test.ts`: acquire/release refcount; a failed refresh
  keeps the last status and sets the problem; `refresh()` replaces the
  status; `summarize()` picks the first non-ok part in order and the
  daemon version.
- `health.test.ts`: the `Health` levels per part and overall.
- `session-state-badge.test.ts`: `badgeFor()` per session state (moved
  from `sessions.test.ts` with the function).

Component, in `app/__tests__/shell.test.tsx`, in jsdom with
`@/lib/ipc` replaced by fakes — the only module faked, because it is
the only I/O:

- the initial route renders the Dashboard;
- clicking a sidebar entry and pressing `Mod+2` both navigate;
- unavailable entries are disabled and have no shortcut;
- `.dark` follows the stubbed `matchMedia` and its `change` event;
- the status bar shows the dot, headline and version for a fixture
  `EngineStatus`, and the session count only when the daemon runs.

Vitest runs two projects: `*.test.ts` in node (the `lib/` suites,
unchanged — the evidence the move preserved behaviour) and `*.test.tsx`
in jsdom with `globals` on, which Testing Library needs for its
automatic cleanup.

Proven once by hand and recorded here: an unsorted class string fails
`just lint`; a hand edit under `components/ui/` is caught by
`add --overwrite` on the next regeneration, not by the gate.

Manual verification after `just dev`: the sidebar shows six entries,
three disabled with a tooltip; `Ctrl+1`, `Ctrl+2`, `Ctrl+3` switch
screens and `Ctrl+B` collapses the sidebar; switching Windows between
light and dark re-themes the app and the terminal without a restart;
the status bar shows the engine state and, with the daemon up, the live
session count; every action of the three screens does what it did
before (add, discover, sync, relocate, remove a project; open, resume,
stop a session; open the embedded terminal).

## Rollout / compatibility

Nothing in the protocol, the daemon or `engine.toml` changes. A
developer runs `pnpm install` at the root after pulling. User-visible:
one line in the unreleased release note (sidebar navigation, status
bar, keyboard shortcuts, light and dark following Windows).

One task, one commit; `styles.css` dies last so every intermediate
commit renders:

1. toolchain: dependencies, the Vite plugin, the registry init with the
   preset, `globals.css` with the three adjustments, `useSortedClasses`,
   `format` as `check --write`
2. test harness: jsdom, Testing Library, the two Vitest projects
3. the engine status store; the Dashboard reads it (no visual change)
4. domain without presentation: `Health`, `badgeFor`, tests moved
5. primitives: the initial set, `tone.ts`, `StatusDot`, `StatusBadge`,
   `ProblemAlert` over `Alert`
6. the shell: router, theme, shortcuts, sidebar and status bar, with the
   shell tests; screens still in their old skin inside it
7. Dashboard on the primitives
8. Projects on the primitives, dialogs and toast
9. Sessions on the primitives, the terminal theme
10. delete `styles.css`; `AGENTS.md` conventions, `docs/ARCHITECTURE.md`
    §5.1, the release note

## Open questions

- The exact warm hue. Favoured: tuned by eye in commit 1, a single hue
  for the whole neutral scale, chroma low enough that a screenshot
  reads as neutral; recorded in the file.
- Should the status bar carry usage once the usage plugin lands?
  Favoured: yes, on its right side; the address is left free now.
- Should a thin wrapper isolate the alpha shortcuts API? Favoured: not
  until a second consumer exists; the exact pin is the protection.
- When does the custom title bar come? Favoured: after the screens have
  settled on this system, as its own design.
