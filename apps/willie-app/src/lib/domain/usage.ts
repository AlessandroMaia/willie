/** Where a session's context fill turns worrying, then urgent. Named
 * consts rather than magic numbers so a future tuning change has one
 * place to land. */
export const CONTEXT_WARNING_PCT = 75;
export const CONTEXT_ERROR_PCT = 90;

/** The subset of the app-wide `Tone` this fact can return. Declared
 * locally rather than importing `components/tone`: `lib/` depends on
 * nothing above it. Every member here is also a `Tone` member, so a
 * caller passes the result straight into a `StatusBadge`'s `tone` prop. */
export type ContextTone = "ok" | "warning" | "error" | "muted";

/** The tone a session's context percentage takes. `null` means no
 * context window was resolved for this session's model — rendered
 * `muted`, never a 0% meter, since 0 and "unknown" mean different
 * things to the user. */
export function contextTone(pct: number | null): ContextTone {
  if (pct === null) return "muted";
  if (pct >= CONTEXT_ERROR_PCT) return "error";
  if (pct >= CONTEXT_WARNING_PCT) return "warning";
  return "ok";
}
