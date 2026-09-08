import { FolderGit2Icon, TerminalIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { SessionTabs } from "@/features/session/session-tabs";
import { SessionTerminal } from "@/features/session/session-terminal";
import { finishedOf, liveOf, sessionName } from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { sessions as sessionsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Session } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { useEngineStatus } from "@/store/use-engine-status";
import { useFocusedSession } from "@/store/use-focused-session";
import { useSetupDrawer } from "@/store/use-setup-drawer";
import { useSnapshot } from "@/store/use-snapshot";

/** Agent sessions first, then shells — both newest first within their
 * own group, matching the tab strip's left-to-right order. */
function orderedTabs(live: Session[]): Session[] {
  const agents = live.filter((s) => (s.kind ?? "agent") === "agent");
  const shells = live.filter((s) => s.kind === "shell");
  return [...agents, ...shells];
}

/**
 * The current system's Session screen: one tab per live session (agent
 * and shell side by side), every terminal kept mounted so switching
 * tabs never loses scrollback, and an empty state that offers to open
 * one or resume the latest finished session. `useFocusedSession` only
 * ever hears about the active tab from here — it never decides which
 * tab that is.
 */
export function SessionScreen() {
  const { system, loading } = useCurrentSystem();
  const { openAt } = useSetupDrawer();
  const { setFocused } = useFocusedSession();
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot: snap, problem } = useSnapshot(daemonRunning);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [actionProblem, setActionProblem] = useState<Problem | null>(null);
  const prevTabsRef = useRef<Session[]>([]);

  /* Narrowed to exactly what should invalidate these: the system's own
   * id and the sessions array reference from the snapshot store.
   * Memoised so an unrelated re-render (the footer's usage poll, for
   * one) never looks like a new tab list to the reconciliation effect
   * below. */
  const systemId = system?.id;
  const sessions = snap?.sessions;
  const tabs = useMemo(
    () => (systemId && sessions ? orderedTabs(liveOf(systemId, sessions)) : []),
    [systemId, sessions],
  );
  const finished = useMemo(
    () => (systemId && sessions ? finishedOf(systemId, sessions) : []),
    [systemId, sessions],
  );
  const latestFinished = finished[0] ?? null;

  /* A newly opened session becomes the active tab the moment it lands
   * in the snapshot; when the active tab's own session leaves the live
   * set, the nearest remaining tab (same position, clamped) takes
   * over; nothing else here ever moves the active tab on its own. */
  useEffect(() => {
    const prevTabs = prevTabsRef.current;
    const prevIds = new Set(prevTabs.map((s) => s.id));
    const currentIds = new Set(tabs.map((s) => s.id));
    const arrived = tabs.find((s) => !prevIds.has(s.id));

    if (arrived) {
      /* Also what fires on first mount with several live sessions
       * already open: the ref starts empty, so every tab counts as
       * "arrived" and the first one found — the newest, `tabs`'
       * own order — wins. */
      setActiveId(arrived.id);
    } else if (activeId !== null && !currentIds.has(activeId)) {
      const priorIndex = prevTabs.findIndex((s) => s.id === activeId);
      const nearestIndex = Math.min(Math.max(priorIndex, 0), tabs.length - 1);
      const nearest = tabs[nearestIndex] ?? null;
      setActiveId(nearest?.id ?? null);
    } else if (activeId === null) {
      const first = tabs[0];
      if (first) setActiveId(first.id);
    }

    prevTabsRef.current = tabs;
  }, [tabs, activeId]);

  useEffect(() => {
    setFocused(activeId);
  }, [activeId, setFocused]);

  useEffect(() => {
    return () => setFocused(null);
  }, [setFocused]);

  /* The house pattern for an action handler (see `projects-screen.tsx`'s
   * `run`): clear any problem from a previous action before this one
   * starts, so success or a fresh attempt both read as "no problem". */
  async function runAction(action: () => Promise<unknown>): Promise<void> {
    setActionProblem(null);
    try {
      await action();
    } catch (error) {
      setActionProblem(asProblem(error));
    }
  }

  function openSession(): void {
    if (!system) return;
    void runAction(() => sessionsApi.open(system.id));
  }

  function openZsh(): void {
    if (!system) return;
    void runAction(() => sessionsApi.open(system.id, "shell"));
  }

  function resumeLatest(): void {
    if (!system || !latestFinished) return;
    void runAction(() => sessionsApi.resume(system.id, latestFinished.id));
  }

  if (loading) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (system === null) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <FolderGit2Icon />
          </EmptyMedia>
          <EmptyTitle>No system yet</EmptyTitle>
          <EmptyDescription>Add one to start a session.</EmptyDescription>
        </EmptyHeader>
        <Button onClick={() => openAt("systems")}>Add a system</Button>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      {problem && <ProblemAlert problem={problem} />}
      {actionProblem && <ProblemAlert problem={actionProblem} />}

      {tabs.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <TerminalIcon />
            </EmptyMedia>
            <EmptyTitle>No live sessions</EmptyTitle>
            <EmptyDescription>Open one to get started.</EmptyDescription>
          </EmptyHeader>
          <div className="flex gap-2">
            <Button onClick={openSession}>New session</Button>
            {latestFinished && (
              <Button variant="outline" onClick={resumeLatest}>
                Resume {sessionName(latestFinished)}
              </Button>
            )}
          </div>
        </Empty>
      ) : (
        <>
          <SessionTabs
            sessions={tabs}
            activeId={activeId}
            onSelect={setActiveId}
            onNewSession={openSession}
            onNewZsh={openZsh}
          />
          {tabs.map((session) => (
            <div
              key={session.id}
              role="tabpanel"
              hidden={session.id !== activeId}
            >
              <SessionTerminal id={session.id} title={sessionName(session)} />
            </div>
          ))}
        </>
      )}
    </div>
  );
}
