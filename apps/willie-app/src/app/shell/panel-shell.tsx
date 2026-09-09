import { useEffect, useRef, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { Spinner } from "@/components/ui/spinner";
import { SessionTerminal } from "@/features/session/session-terminal";
import { isLive } from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { sessions as sessionsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useCurrentSystem } from "@/store/use-current-system";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSnapshot } from "@/store/use-snapshot";

/**
 * The workspace panel's Shell tab: one live shell per system.
 *
 * It resolves by kind and system rather than by who started one, so a
 * shell already running — from an earlier visit, or from another
 * client — is adopted instead of duplicated, and it starts one only
 * when the system has none. It never stops the session on unmount:
 * leaving the shell running between visits is the point of it, and it
 * ends with the daemon or from the Sessions panel, the same as any
 * other session.
 */
export function PanelShell() {
  const { system } = useCurrentSystem();
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot } = useSnapshot(daemonRunning);
  const [problem, setProblem] = useState<Problem | null>(null);

  const systemId = system?.id ?? null;
  const shell =
    snapshot?.sessions.find(
      (s) => s.project_id === systemId && s.kind === "shell" && isLive(s),
    ) ?? null;
  const shellId = shell?.id ?? null;

  /* One attempt per system, remembered across the re-render the
   * refusal itself causes: without it the effect would fire again the
   * moment it finished, retrying forever and wiping the problem it had
   * just set. */
  const attemptedForRef = useRef<string | null>(null);

  useEffect(() => {
    if (systemId === null || shellId !== null) return;
    /* The daemon is what a session is created on; without it there is
     * nothing to start, and the Tree tab already says why. */
    if (!daemonRunning) return;
    if (attemptedForRef.current === systemId) return;
    attemptedForRef.current = systemId;

    let cancelled = false;
    setProblem(null);

    sessionsApi.open(systemId, "shell").catch((error: unknown) => {
      if (!cancelled) setProblem(asProblem(error));
    });

    return () => {
      cancelled = true;
    };
  }, [systemId, shellId, daemonRunning]);

  /* A refusal is the whole answer — never an empty terminal beside it,
   * which reads as a shell that started and said nothing. */
  if (problem) return <ProblemAlert problem={problem} />;
  if (shellId === null) return <Spinner />;

  return <SessionTerminal key={shellId} id={shellId} title="zsh" />;
}
