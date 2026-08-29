import type { Session, SessionState } from "./proto";

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

export type Tone = "ok" | "muted" | "error" | "pending";

/** The chip label and tone for a session's state. */
export function stateChip(state: SessionState): { label: string; tone: Tone } {
  switch (state.state) {
    case "creating":
      return { label: "creating", tone: "pending" };
    case "running":
      return { label: "running", tone: "ok" };
    case "stopping":
      return { label: "stopping", tone: "pending" };
    case "exited": {
      if (state.code == null && state.signal != null) {
        return { label: `exited (signal ${state.signal})`, tone: "error" };
      }
      const code = state.code ?? 0;
      return { label: `exited ${code}`, tone: code === 0 ? "muted" : "error" };
    }
    case "failed":
      return { label: `failed: ${state.code}`, tone: "error" };
  }
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
