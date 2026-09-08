import { deniedCount } from "@/lib/domain/sandbox";
import type { Denied, SandboxState, Session } from "@/lib/proto";

/**
 * A non-terminal session — the UI's "live vs history" split. This is the
 * complement of the domain's `is_terminal` (exited/failed), so it includes
 * `creating`. It is intentionally broader than the core's socket-level
 * `SessionState::is_live` (running/stopping only): the UI groups a session
 * being set up with the running ones, not with the history.
 */
export function isLive(s: Session): boolean {
  return (
    s.state.state === "creating" ||
    s.state.state === "running" ||
    s.state.state === "stopping"
  );
}

/** How many of a project's sessions are live right now. */
export function liveCount(sessions: Session[], projectId: string): number {
  return sessions.filter((s) => s.project_id === projectId && isLive(s)).length;
}

/** Live sessions, newest `created_at` first. */
export function liveSessions(sessions: Session[]): Session[] {
  return sessions
    .filter(isLive)
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
}

/** How many finished sessions the history shows. */
export const TERMINAL_HISTORY_LIMIT = 20;

/**
 * The most recent finished (terminal) sessions, newest first, capped at
 * `limit`. Orders by `finished_at` when present, else `created_at`.
 */
export function recentTerminal(sessions: Session[], limit: number): Session[] {
  const at = (s: Session) => s.finished_at ?? s.created_at;
  return sessions
    .filter((s) => !isLive(s))
    .sort((a, b) => at(b).localeCompare(at(a)))
    .slice(0, limit);
}

export type Posture = "full" | "reduced" | "unknown";

const EMPTY_SANDBOX: SandboxState = {
  applied: [],
  unavailable: [],
  degraded: [],
  denied: [],
};

/**
 * A session's sandbox report as a fully-populated value. A session
 * recorded before part 2 carries no `sandbox` field on the wire; it
 * normalises to the all-empty state rather than `undefined`, so every
 * caller reads arrays, never a crash.
 */
export function sandboxOf(session: Session): SandboxState {
  const sb = session.sandbox;

  if (!sb) return EMPTY_SANDBOX;

  return {
    applied: sb.applied ?? [],
    unavailable: sb.unavailable ?? [],
    degraded: sb.degraded ?? [],
    denied: sb.denied ?? [],
  };
}

/**
 * The boundary a session ran under, at a glance. `full`: everything
 * reported applied and nothing missing or degraded. `reduced`: a
 * mechanism the kernel could not offer, or one that degraded. `unknown`:
 * nothing reported yet — a session still `creating`, or one recorded
 * before part 2.
 */
export function sandboxPosture(session: Session): Posture {
  const sb = sandboxOf(session);

  if (sb.applied.length === 0) return "unknown";

  if (sb.unavailable.length > 0 || sb.degraded.length > 0) return "reduced";

  return "full";
}

/** The session's denials, highest count first, with the total across all. */
export function denials(session: Session): { items: Denied[]; total: number } {
  const sb = sandboxOf(session);
  const items = [...sb.denied].sort((a, b) => b.count - a.count);

  return { items, total: deniedCount(sb) };
}
