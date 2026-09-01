# Frontend foundations — setup, layout and conventions

**Stage 1 of the frontend rework, and it changes no pixel.** It fixes how
a clone is prepared, where each kind of file belongs, and what the gate
refuses to let back in. The visual system and the store/router library
are separate stages; see "Non-goals".

## Problem

### A clean clone does not build without insider knowledge

The obvious command fails with an error that names no cure:

```
$ pnpm install                     # at the repository root
[ERR_PNPM_NO_PKG_MANIFEST] No package.json found in D:\Projects\willie
```

The only manifest is `apps/willie-app/package.json`. Four traps sit
behind that one:

- `just ensure` (`xtask/src/doctor.rs`) verifies ten tools and never
  verifies a single dependency, and `README.md:41-54` lists the recipes
  without `web-install` (`justfile:29-31`) — the one recipe a first
  clone needs.
- `apps/willie-app/package.json:6` pins `packageManager`, so corepack
  may ask permission before downloading that pnpm, while `justfile:7`
  runs PowerShell with `-NonInteractive`, where the question cannot be
  answered. It passes on a machine whose corepack cache is already
  warm, which is why it survives unnoticed.
- The required Node version is prose in `README.md:29` and validated
  nowhere (the README says 22; the development machine runs 24.18).
- `apps/willie-app/pnpm-workspace.yaml:8-12` pins a transitive
  dependency until it is "reachable" again, with no date and no test
  that says when the pin may go.
- `just check` — the only gate this repository has — cannot complete
  without a denylist file that is deliberately unversioned. Reproduced
  here: `check-refs` exits 1 with `cannot read denylist …`. A fresh
  clone fails the gate for a reason that has nothing to do with its
  code, and the failure names the file but not the fact that its
  absence is expected on a new machine.

### The UI has no structure to grow into

`docs/ARCHITECTURE.md:541-542` requires **one** store fed only by
`state.snapshot` + `state.events`, with the UI never computing truth.
There are three, and the duplication is mechanical:

| Duplicated                           | Copies | Where                                                                                                 |
| ------------------------------------ | ------ | ----------------------------------------------------------------------------------------------------- |
| snapshot + event + resnapshot effect | 3      | `Dashboard.tsx:84`, `Projects.tsx:188`, `Sessions.tsx:196`                                            |
| `Problem` rendered as JSX            | 8      | `Dashboard.tsx:210`, `Projects.tsx:545,738,827,867`, `Sessions.tsx:123,278`, `SessionTerminal.tsx:90` |
| `unknown` coerced into a `Problem`   | 4      | `Projects.tsx:44`, `Sessions.tsx:20`, `Dashboard.tsx:31`, `SessionTerminal.tsx:64`                    |

Each copy drifted: a failed snapshot is swallowed in `Dashboard.tsx:91`
and raised into a banner in the other two.

`Projects.tsx` is 892 lines with 21 `useState` and two hand-rolled
modals. `App.tsx` selects the screen with a three-way ternary over a
hardcoded `<nav>`, while `docs/ARCHITECTURE.md:530-538` still owes
Tools, Plugins, Settings and a tray, and §4.1 owes statically
registered plugin panels under `src/plugins/<id>/`.

`styles.css` carries four vocabularies for one concept — `.light-*`,
`.chip-*`, `.badge-*`, `.check-*` — repeating the same three colours in
each, although `lib/sessions.ts:26` already models that concept as the
`Tone` type.

Nothing in the gate prevents any of it from returning. The Rust side
states its dependency rules and they are reviewable; the frontend has
no equivalent rule and no check.

## Goals

- `pnpm install` at the repository root works, and `just setup`
  prepares a clean clone in one command — proven by preparing an actual
  fresh clone, not by reasoning about it.
- The snapshot subscription exists exactly once, still folding events
  with the pure `applyEvent` / `needsResnapshot` that already exist.
- Every kind of file has one named home, and a violation of the
  direction between homes fails `just check`.
- One filename convention, enforced by tooling rather than by review.
- Dependencies current, with every deliberate exception named.
- No visual change: the same screens render the same pixels when this
  stage lands.

## Non-goals

- The visual system (utility CSS, generated component primitives) — the
  next stage. Moving files and re-skinning in one step makes any
  regression impossible to attribute.
- The store and routing library. TanStack is the stated preference for
  when one is needed; `app/` is shaped so a router drops in without
  touching `features/`.
- TypeScript 7. It is a rewritten compiler and deserves its own commit
  and its own gate run.
- Component tests. Logic stays pure and in `lib/`, where it is already
  cheap to test; a component-testing library earns its place when a
  component owns logic of its own.
- New screens (Tools, Plugins, Settings). This stage only gives them an
  address.
- Any new dependency, quality tooling included. Everything this stage
  enforces is already available in the linter the repository ships
  with.

## Design

### Repository root — `package.json`, `pnpm-workspace.yaml`

The workspace moves up. `pnpm-workspace.yaml` and `pnpm-lock.yaml`
become root files, and a root `package.json` declares `name`,
`private`, `packageManager` and `engines` while holding no
dependencies and no `scripts` of its own — the `justfile` drives every
recipe with `-C {{web}}` instead.

```
willie/
  package.json          private, packageManager, engines; no dependencies
  pnpm-workspace.yaml   packages: ["apps/*"], allowBuilds, overrides
  pnpm-lock.yaml
  apps/willie-app/package.json    unchanged
```

`pnpm install` at the root then resolves the whole repository, and the
root becomes the place for tooling shared by more than one package.

The `justfile` keeps `-C {{web}}` wherever a recipe means "the app",
and exports `COREPACK_ENABLE_DOWNLOAD_PROMPT=0` so a pinned
`packageManager` can never block a `-NonInteractive` shell.

### One command for a fresh clone — `justfile`, `xtask/src/doctor.rs`

```
just setup    hooks -> pnpm install -> cargo fetch --locked -> ensure
```

`ensure` runs **last**, as the verification that the machine is ready,
not first as a gate. Putting it first deadlocks: one of the new checks
below fails precisely because dependencies are not installed yet, which
is the very thing `setup` was about to do. A missing tool still stops
the run at the line that needs it, and the README says to run
`just ensure` for the install hints when a command is not found.

Recipes stay one native command per line, so the first failure stops
the sequence with its own exit code and never leaves a half-prepared
clone. `doctor-tools` gains two checks that end in a cure rather than a
diagnosis: Node below the floor, and dependencies not installed
(`node_modules` absent, remediation "run `just setup`").

The floor is `engines.node` in the root `package.json`, and nothing
else. A `.node-version` file was considered and rejected: it means a
*pin*, so a version manager reading it would downgrade a developer
already on a newer Node for no reason, while `engines` states the
minimum the repository actually requires. `doctor-tools` reads it and
compares major versions.

A third check covers the denylist. `check-refs` keeps failing closed
when the file is missing — a silent skip would turn the rule it
enforces into a suggestion — but the absence stops being a mystery:
`doctor-tools` reports it as a first-class prerequisite with the path
it expects and the `WILLIE_REFS_DENYLIST` override, and the README
says a new machine must obtain it before `just check` can pass.

### Source layout — `apps/willie-app/src/`

```
src/
  app/                    composition; the only layer that sees all others
    app.tsx               the shell: navigation chrome and active screen
    routes.ts             declarative screen registry
  store/                  its React binding only
    use-snapshot.ts       ~23 lines: useSyncExternalStore + acquire/release
  features/               one directory per domain, each exporting a screen
    projects/             projects-screen, project-row, roots-panel,
                          discover-panel, remove-project-dialog,
                          relocate-project-dialog
    sessions/             sessions-screen, session-row, session-terminal
    health/               dashboard-screen, health-lights, doctor-list
  components/
    ui/                   empty here; the next stage fills it
    problem-alert.tsx     the one renderer of Problem
    relative-time.tsx     moved out of the sessions screen
  lib/                    no React
    proto.ts              mirrors of the Rust wire types
    ipc.ts                the Tauri bridge: the only I/O in the frontend
    problem.ts            unknown -> Problem
    domain/               state, jobs, sessions, health, and the one
                          store: daemon-snapshot.ts — snapshot + events
                          + resnapshot, with no React of its own
      __tests__/          their suites, one file per module
  plugins/<id>/           plugin panels (docs/ARCHITECTURE.md §4.1)
  styles/                 styles.css, moved unchanged; the next stage
                          replaces its four tone vocabularies with tokens
```

`components/ui/` and `plugins/` are created empty, as addresses. Every
other file in the tree exists today and only moves.

Two rules make the tree hold.

**Dependency direction:** `app -> features -> components ->
components/ui`; `app` and `features` may both read `store/`, which reads
only `lib/`; every layer may read `lib/`; `lib/` reads nothing above it;
`features/a` never imports `features/b` — what two features share moves
down into `components/`, `store/` or `lib/`.

`store/` is a layer rather than a file inside `app/` because the screens
consume it. A shared thing the screens must read cannot live in the one
directory they are forbidden to import, and the alternative — an
exception carved into the rule for a single file — is how a dependency
rule starts rotting: the next file gets its own exception, and the one
after that. If `features` may read it, it is not `app/`.

The store itself splits along the same "One home per kind" rule below:
the state machine that folds snapshot + events + resnapshot has no
React in it, so it lives in `lib/domain/daemon-snapshot.ts` like every
other pure module. `store/use-snapshot.ts` is the thin React binding
over it — a `useSyncExternalStore` call and the `acquire`/`release`
effect, nothing else — and it is the only file `store/` holds, and the
one file outside `lib/` that imports `daemon-snapshot.ts` directly.
`app` and `features` never import it themselves; they go through the
`useSnapshot` hook.

**One home per kind:** a screen lives in its feature; a component two
features share lives in `components/`; anything without React lives in
`lib/`. `SessionTerminal.tsx` leaves `screens/` under that rule — it
was never a screen.

### Enforcing the direction — `apps/willie-app/biome.json`

Cross-layer imports become alias-based (`@/lib/…`, `@/features/…`)
through `paths` in `tsconfig.json` and a matching `resolve.alias` in
`vite.config.ts`, so a specifier is stable wherever the importing file
sits and a glob can reason about it. The layering is then
`noRestrictedImports` (`style`, available since 1.6.0) inside per-path
`overrides`:

| Files under                | May not import                                                          |
| --------------------------- | ------------------------------------------------------------------------ |
| `src/lib/**`               | `@/app/*`, `@/store/*`, `@/features/*`, `@/components/*`, `@/plugins/*` |
| `src/store/**`             | `@/app/*`, `@/features/*`, `@/components/*`, `@/plugins/*`              |
| `src/components/**`        | `@/app/*`, `@/store/*`, `@/features/*`, `@/plugins/*`                   |
| `src/features/projects/**` | `@/app/*`, and every `@/features/*` but its own                         |
| `src/features/sessions/**` | idem                                                                     |
| `src/features/health/**`   | idem                                                                     |

Each group also lists the relative form (`**/features/**`) so a stray
`../../features/x` is caught alongside the aliased one. A violation is
a lint error, so it fails `just lint` and therefore `just check`.

### Naming — `apps/willie-app/biome.json`

Filenames are kebab-case, enforced by `useFilenamingConvention`
(`style`, available since 1.5.0) with `filenameCases: ["kebab-case"]`.
Exported components keep PascalCase: `project-row.tsx` exports
`ProjectRow`.

Two reasons beyond consistency. On a case-insensitive Windows
filesystem a rename that changes only capitalisation does not reach
git, and Willie synchronises its own workspaces across that boundary
into ext4 on every sync, so the class of bug is not hypothetical here.
And the rest of the repository already names files this way
(`designs/projects-and-workspaces.md`, `docs/decisions/NNNN-<slug>.md`).

The rename is mechanical and lands in its own commit, separate from any
move between directories, so both diffs stay reviewable.

### Rules turned on — `apps/willie-app/biome.json`

| Rule                        | Category          | What it catches here                            |
| --------------------------- | ----------------- | ----------------------------------------------- |
| `useFilenamingConvention`   | `style`           | the convention above                            |
| `noRestrictedImports`       | `style`           | the direction above                             |
| `noFloatingPromises`        | `nursery` (types) | fire-and-forget calls in the effects            |
| `useExhaustiveDependencies` | `correctness`     | already `error`; confirm it reaches the effects |

No new dependency is added for any of this: every rule above already
ships with the linter in the tree. The gap that leaves is a file that
ends the move referenced by nothing — `noUnusedLocals` and
`noUnusedImports` catch unused symbols, not orphaned modules. The
mitigation is the shape of the work rather than a tool: the move is
mechanical, and a module no longer imported by anything shows up in
the commit as a path nothing points at.

### Dependency updates — `package.json`

| Package                | From   | To      | Why now                              |
| ---------------------- | ------ | ------- | ------------------------------------ |
| `@biomejs/biome`       | 2.5.10 | 2.5.11  | patch; the `biome.json:2` schema follows |
| `@types/react-dom`     | 19.2.4 | 19.2.5  | patch                                |
| `@vitejs/plugin-react` | 4.7.0  | 6.1.1   | one block with the two below         |
| `vite`                 | 7.3.6  | 8.2.2   | idem                                 |
| `vitest`               | 3.2.7  | 4.1.11  | idem                                 |
| `typescript`           | 5.8.3  | —       | deferred, see Non-goals              |

The build toolchain moves as one block because the three majors are
released against each other. TypeScript stays behind: it interacts with
`verbatimModuleSyntax` and `isolatedModules`
(`apps/willie-app/tsconfig.json:11,15`), and bundling it with a
repository-wide rename would make an unexplained failure
unattributable.

The `baseline-browser-mapping` override is retested against the
registry: removed if the current release installs, dated in its comment
if it does not.

### Errors and edge cases

- **The lockfile moves.** It is regenerated at the root and judged by
  `just check`; a resolution that changes a version beyond the table
  above is treated as a finding, not as noise.
- **A relative import escapes the alias.** Both forms are listed in
  every group, so `../../features/x` fails exactly like `@/features/x`.
- **A case-only rename on Windows.** Each is done as a two-step
  `git mv`, verified by `git status` showing a rename.
- **A missing tool during `just setup`.** `ensure` prints its hints and
  the recipe stops there; nothing downstream runs against a machine
  that cannot build.
- **A stale `apps/willie-app/node_modules`** after the workspace moves.
  It is named in the rollout note below, since a developer with an
  existing tree keeps it until they delete it.

## Testing

- `xtask` unit tests cover the two new `doctor-tools` checks, beside
  the existing test that every check carries a probe and a hint
  (`xtask/src/doctor.rs:192`).
- The `lib/` tests (`state`, `jobs`, `sessions`, `health`) must pass
  **unchanged** through the move: they are the evidence that relocating
  files preserved behaviour.
- The layering rules are proven once, by hand, at implementation time —
  introduce a forbidden import, watch `just lint` fail, revert it. A
  permanently broken fixture cannot live in the tree, so the proof is
  recorded here rather than committed.
- **Clean-clone test**: clone into a fresh directory, run `just setup`
  then `just check`. Both green before the README claims either works.
- `just check` on the working tree.

Manual verification: `just dev`, then confirm the three screens behave
exactly as before — the project list with its state chips, opening and
resuming a session, and the embedded terminal.

## Rollout / compatibility

Developer-facing only. Nothing in the protocol, the daemon or
`engine.toml` changes, and `releases/` gets no line because no
user-visible behaviour changes. A developer with an existing tree runs
one `pnpm install` at the new root after pulling; the old
`apps/willie-app/node_modules` is stale and can be deleted.

One task, one commit:

1. workspace to the root, `just setup`, README
2. the `doctor-tools` prerequisite checks
3. the dependency block
4. the `@/` path alias
5. the kebab-case rename
6. the directory move (no behaviour change)
7. the layering and nursery rules
8. the single store, replacing the three subscriptions
9. `problem-alert`, `relative-time`, and splitting `projects-screen`

## Open questions

- Should `just setup` also fetch the pinned base root filesystem?
  Favoured: no — it is a large download that only the distro work
  needs.
- Should Biome move to the root and cover the whole repository?
  Favoured: keep it in the app while there is one JS package; revisit
  when a second appears.
- When does TypeScript 7 land? Favoured: immediately after this stage,
  on its own.
- Is a dead-code scan worth a dependency later? Favoured: no, unless
  orphaned modules actually survive a move — decide on evidence, not
  in advance.
