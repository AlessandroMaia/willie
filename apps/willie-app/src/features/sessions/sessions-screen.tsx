import { useState } from "react";
import { useSnapshot } from "@/app/store";
import { SessionTerminal } from "@/features/sessions/session-terminal";
import {
  liveSessions,
  recentTerminal,
  stateChip,
  TERMINAL_HISTORY_LIMIT,
  type Tone,
} from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { sessions as sessionsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Session, SessionState, Snapshot } from "@/lib/proto";

/* Every row needs the owning project's display name, but a session can
 * outlive the project it belonged to (removed while the session was
 * still winding down), so the join always has a fallback label instead
 * of assuming the id still resolves. */
function projectNameFor(snap: Snapshot, projectId: string): string {
  const project = snap.projects.find((p) => p.id === projectId);
  return project ? project.name : `removed project (${projectId})`;
}

const TONE_CLASS: Record<Tone, string> = {
  ok: "ready",
  pending: "busy",
  error: "failed",
  muted: "muted",
};

interface SessionChipProps {
  state: SessionState;
}

/* Mirrors `Projects.tsx`'s `StateChip`: a failed state carries its own
 * code, message and remediation, so it renders as the same stacked
 * chip-failed shape; every other state is a single-line pill. */
function SessionChip({ state }: SessionChipProps) {
  const { label, tone } = stateChip(state);
  const cls = `chip chip-${TONE_CLASS[tone]}`;
  if (state.state === "failed") {
    return (
      <div className={cls}>
        <code>{label}</code>
        <span>{state.message}</span>
        {state.remediation && (
          <div className="muted">→ {state.remediation}</div>
        )}
      </div>
    );
  }
  return (
    <span className={cls}>
      {tone === "pending" && <span className="spinner" aria-hidden="true" />}
      {label}
    </span>
  );
}

/* A tiny, self-contained relative-time label. `created_at`/`started_at`
 * are whole-second epoch strings (the daemon's format, same as jobs), so
 * parsing as seconds — not a calendar date — is what actually matches
 * the wire. */
function relativeTime(epochSeconds: string): string {
  const started = Number(epochSeconds) * 1000;
  if (!Number.isFinite(started)) return "unknown";
  const diffSeconds = Math.floor((Date.now() - started) / 1000);
  if (diffSeconds < 60) return "just now";
  const minutes = Math.floor(diffSeconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `${days} d ago`;
}

interface LiveRowProps {
  session: Session;
  projectName: string;
  busy: boolean;
  problem: Problem | null;
  onAttach: () => void;
  onStop: () => void;
  onOpenInApp: () => void;
}

function LiveRow({
  session,
  projectName,
  busy,
  problem,
  onAttach,
  onStop,
  onOpenInApp,
}: LiveRowProps) {
  return (
    <div className="session-row">
      <div className="session-row-main">
        <strong>{projectName}</strong>
        <SessionChip state={session.state} />
      </div>
      <div className="session-row-detail muted">
        <div>harness: {session.harness}</div>
        <div>
          {session.clients} client{session.clients === 1 ? "" : "s"} attached
        </div>
        <div>
          started {relativeTime(session.started_at ?? session.created_at)}
        </div>
      </div>
      {problem && (
        <div className="problem" role="alert">
          <strong>{problem.code}</strong> — {problem.message}
          {problem.remediation && (
            <div className="muted">→ {problem.remediation}</div>
          )}
        </div>
      )}
      <div className="actions">
        <button type="button" disabled={busy} onClick={onAttach}>
          Attach
        </button>
        <button type="button" disabled={busy} onClick={onStop}>
          Stop
        </button>
        <button type="button" disabled={busy} onClick={onOpenInApp}>
          Open in app
        </button>
        {busy && <span className="muted">working…</span>}
      </div>
    </div>
  );
}

interface RecentRowProps {
  session: Session;
  projectName: string;
}

function RecentRow({ session, projectName }: RecentRowProps) {
  return (
    <div className="session-row session-row-recent">
      <div className="session-row-main">
        <strong>{projectName}</strong>
        <SessionChip state={session.state} />
      </div>
      <div className="session-row-detail muted">
        <div>harness: {session.harness}</div>
        <div>
          finished {relativeTime(session.finished_at ?? session.created_at)}
        </div>
      </div>
    </div>
  );
}

export function SessionsScreen() {
  const store = useSnapshot();
  const snap = store.status === "ready" ? store.snapshot : null;
  const problem = store.status === "failed" ? store.problem : null;
  const [busyId, setBusyId] = useState<string | null>(null);
  const [rowProblems, setRowProblems] = useState<Map<string, Problem>>(
    new Map(),
  );
  const [openId, setOpenId] = useState<string | null>(null);

  function setRowProblem(sessionId: string, problem: Problem | null) {
    setRowProblems((prev) => {
      const next = new Map(prev);
      if (problem) next.set(sessionId, problem);
      else next.delete(sessionId);
      return next;
    });
  }

  async function attach(session: Session, projectName: string) {
    setBusyId(session.id);
    setRowProblem(session.id, null);
    try {
      await sessionsApi.attach(session.id, projectName);
    } catch (error) {
      setRowProblem(session.id, asProblem(error));
    } finally {
      setBusyId(null);
    }
  }

  async function stop(session: Session) {
    setBusyId(session.id);
    setRowProblem(session.id, null);
    try {
      await sessionsApi.stop(session.id);
    } catch (error) {
      setRowProblem(session.id, asProblem(error));
    } finally {
      setBusyId(null);
    }
  }

  const live = snap ? liveSessions(snap.sessions) : [];
  const recent = snap
    ? recentTerminal(snap.sessions, TERMINAL_HISTORY_LIMIT)
    : [];

  /* Same join the rows use, resolved by id for the open terminal panel's
   * heading. Falls back to the id itself, matching `projectNameFor`'s own
   * fallback shape for a session whose project is gone. */
  function titleFor(sessionId: string): string {
    const session = snap?.sessions.find((s) => s.id === sessionId);
    return session && snap
      ? projectNameFor(snap, session.project_id)
      : sessionId;
  }

  /* The embedded panel only makes sense for a session still in the live
   * set; if it exited or was stopped out from under the open panel,
   * clear `openId` on this render rather than waiting for a follow-up
   * effect — the panel disappears in the same frame the row does. */
  if (openId && !live.some((s) => s.id === openId)) {
    setOpenId(null);
  }

  return (
    <main className="sessions">
      <header>
        <h1>Sessions</h1>
      </header>

      {problem && (
        <section className="problem" role="alert">
          <strong>{problem.code}</strong> — {problem.message}
          {problem.remediation && (
            <div className="muted">→ {problem.remediation}</div>
          )}
        </section>
      )}

      {snap === null ? (
        !problem && <p className="muted">Loading sessions…</p>
      ) : snap.sessions.length === 0 ? (
        <p className="muted">No sessions yet — open one from a project.</p>
      ) : (
        <>
          <section className="session-list">
            <h2>Live</h2>
            {live.length === 0 ? (
              <p className="muted">No live sessions.</p>
            ) : (
              live.map((session) => (
                <LiveRow
                  key={session.id}
                  session={session}
                  projectName={projectNameFor(snap, session.project_id)}
                  busy={busyId === session.id}
                  problem={rowProblems.get(session.id) ?? null}
                  onAttach={() =>
                    attach(session, projectNameFor(snap, session.project_id))
                  }
                  onStop={() => stop(session)}
                  onOpenInApp={() => setOpenId(session.id)}
                />
              ))
            )}
          </section>

          <section className="session-list">
            <h2>Recent</h2>
            {recent.length === 0 ? (
              <p className="muted">No recent sessions.</p>
            ) : (
              recent.map((session) => (
                <RecentRow
                  key={session.id}
                  session={session}
                  projectName={projectNameFor(snap, session.project_id)}
                />
              ))
            )}
          </section>

          {openId && (
            <section className="session-terminal">
              <div className="session-terminal-head">
                <strong>{titleFor(openId)}</strong>
                <button type="button" onClick={() => setOpenId(null)}>
                  Close
                </button>
              </div>
              <SessionTerminal
                key={openId}
                id={openId}
                title={titleFor(openId)}
              />
            </section>
          )}
        </>
      )}
    </main>
  );
}
