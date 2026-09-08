/**
 * The setup drawer's own sections — the same ids `SETUP_ENTRIES`
 * (`app/routes.ts`) lists. Kept here, not in `app/`, so
 * `store/use-setup-drawer.ts` can narrow its state to this type
 * without importing upward out of `lib/` (see `docs/ARCHITECTURE.md`'s
 * frontend layering).
 */
export type SetupEntry =
  | "engine"
  | "tools"
  | "plugins"
  | "profile-store"
  | "systems"
  | "settings";
