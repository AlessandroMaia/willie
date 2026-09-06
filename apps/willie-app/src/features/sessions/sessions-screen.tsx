import { TerminalIcon, XIcon } from "lucide-react";
import { useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { relativeTime } from "@/components/relative-time";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Item,
  ItemActions,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
import { SandboxLine } from "@/features/sessions/sandbox-line";
import { SessionStateBadge } from "@/features/sessions/session-state-badge";
import { SessionTerminal } from "@/features/sessions/session-terminal";
import {
  liveSessions,
  recentTerminal,
  TERMINAL_HISTORY_LIMIT,
} from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { sessions as sessionsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Session, Snapshot } from "@/lib/proto";
import { useSnapshot } from "@/store/use-snapshot";

/* Every row needs the owning project's display name, but a session can
 * outlive the project it belonged to (removed while the session was
 * still winding down), so the join always has a fallback label instead
 * of assuming the id still resolves. */
function projectNameFor(snap: Snapshot, projectId: string): string {
  const project = snap.projects.find((p) => p.id === projectId);
  return project ? project.name : `removed project (${projectId})`;
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
    <Item variant="outline" className="flex-col items-stretch gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <ItemTitle className="font-semibold">{projectName}</ItemTitle>
        <SessionStateBadge state={session.state} />
      </div>
      <ItemDescription className="flex flex-col gap-0.5">
        <span>harness: {session.harness}</span>
        <span>
          {session.clients} client{session.clients === 1 ? "" : "s"} attached
        </span>
        <span>
          started {relativeTime(session.started_at ?? session.created_at)}
        </span>
      </ItemDescription>
      <SandboxLine session={session} />
      {problem && <ProblemAlert problem={problem} />}
      <ItemActions className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" disabled={busy} onClick={onAttach}>
          Attach
        </Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={onStop}>
          Stop
        </Button>
        <Button size="sm" disabled={busy} onClick={onOpenInApp}>
          Open in app
        </Button>
        {busy && <Spinner className="text-muted-foreground" />}
      </ItemActions>
    </Item>
  );
}

interface RecentRowProps {
  session: Session;
  projectName: string;
}

function RecentRow({ session, projectName }: RecentRowProps) {
  return (
    <Item
      variant="muted"
      size="sm"
      className="flex-col items-stretch gap-1 opacity-80"
    >
      <div className="flex flex-wrap items-center gap-2">
        <ItemTitle>{projectName}</ItemTitle>
        <SessionStateBadge state={session.state} />
      </div>
      <ItemDescription className="flex gap-3">
        <span>harness: {session.harness}</span>
        <span>
          finished {relativeTime(session.finished_at ?? session.created_at)}
        </span>
      </ItemDescription>
      <SandboxLine session={session} />
    </Item>
  );
}

export function SessionsScreen() {
  const store = useSnapshot();
  const snap = store.snapshot;
  const problem = store.problem;
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
    <div className="mx-auto flex max-w-4xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Sessions</h1>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      {snap === null ? (
        !problem && (
          <div className="flex items-center gap-2 text-muted-foreground text-sm">
            <Spinner /> Loading sessions…
          </div>
        )
      ) : snap.sessions.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <TerminalIcon />
            </EmptyMedia>
            <EmptyTitle>No sessions yet</EmptyTitle>
            <EmptyDescription>Open one from a project.</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <>
          <section className="flex flex-col gap-2">
            <h2 className="font-medium text-muted-foreground text-sm">Live</h2>
            {live.length === 0 ? (
              <p className="text-muted-foreground text-sm">No live sessions.</p>
            ) : (
              <ItemGroup className="gap-3">
                {live.map((session) => (
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
                ))}
              </ItemGroup>
            )}
          </section>

          <section className="flex flex-col gap-2">
            <h2 className="font-medium text-muted-foreground text-sm">
              Recent
            </h2>
            {recent.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                No recent sessions.
              </p>
            ) : (
              <ItemGroup className="gap-2">
                {recent.map((session) => (
                  <RecentRow
                    key={session.id}
                    session={session}
                    projectName={projectNameFor(snap, session.project_id)}
                  />
                ))}
              </ItemGroup>
            )}
          </section>

          {openId && (
            <section className="flex flex-col gap-2 rounded-lg border bg-card p-3">
              <div className="flex items-center justify-between gap-3">
                <span className="font-semibold text-sm">
                  {titleFor(openId)}
                </span>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label="Close terminal"
                  onClick={() => setOpenId(null)}
                >
                  <XIcon />
                </Button>
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
    </div>
  );
}
