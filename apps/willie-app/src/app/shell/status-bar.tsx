import { Link, useLocation } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { StatusDot } from "@/components/status-dot";
import { TONE_DOT, type Tone, toneForHealth } from "@/components/tone";
import { summarize } from "@/lib/domain/engine-status";
import { deniedCount, postureLine } from "@/lib/domain/sandbox";
import { liveSessions, sandboxOf, sandboxPosture } from "@/lib/domain/sessions";
import { contextTone } from "@/lib/domain/usage";
import { usage } from "@/lib/ipc";
import type { Session, SessionUsage, UsageSnapshot } from "@/lib/proto";
import { cn } from "@/lib/utils";
import { useEngineStatus } from "@/store/use-engine-status";
import { useFocusedSession } from "@/store/use-focused-session";
import { useSnapshot } from "@/store/use-snapshot";

/** How often the governance segment re-polls usage while a session is
 * focused and visible — the same cadence as the Usage panel's own
 * poll (`plugins/usage/usage-panel.tsx`). */
const POLL_INTERVAL_MS = 4000;

interface GovernanceSegmentProps {
  session: Session;
  usageRow: SessionUsage | undefined;
  /** Task 15's system aggregate (shown on `/sandbox`) overrides the
   * focused session's own posture line with the system-wide one. */
  posture?: string;
}

/** `sandbox: <applied mechanisms> · <n> denied`, then a context meter
 * and the token count when usage has a row for this session — never a
 * fake 0% meter when it does not. A session that has not reported any
 * mechanism yet (`sandboxPosture` is "unknown" — still `creating`, or
 * recorded before sandbox reporting existed) shows the same "no sandbox
 * report" line `features/sessions/sandbox-line.tsx` already uses,
 * with neither a denied count nor a meter — never built by
 * concatenating through an empty `postureLine`. Always a link to the
 * Sandbox screen filtered to this session. */
function GovernanceSegment({
  session,
  usageRow,
  posture,
}: GovernanceSegmentProps) {
  const noReport = !posture && sandboxPosture(session) === "unknown";
  const sandbox = sandboxOf(session);
  const line =
    posture ??
    (noReport
      ? "no sandbox report"
      : `${postureLine(sandbox)} · ${deniedCount(sandbox)} denied`);
  const showUsage = !noReport;
  const pct = usageRow?.context_pct ?? null;
  const tone: Tone = contextTone(pct);
  const clampedPct = pct === null ? null : Math.min(100, Math.max(0, pct));

  return (
    <Link
      to="/sandbox"
      search={{ session: session.id }}
      className="flex min-w-0 items-center gap-2 hover:text-foreground"
    >
      <span className="truncate">sandbox: {line}</span>

      {showUsage && usageRow && clampedPct !== null && (
        <span
          role="img"
          aria-label={`${pct}% context`}
          className="h-1.5 w-10 shrink-0 overflow-hidden rounded-full bg-muted"
        >
          <span
            className={cn("block h-full", TONE_DOT[tone])}
            style={{ width: `${clampedPct}%` }}
          />
        </span>
      )}

      {showUsage && usageRow && (
        <span className="shrink-0 font-mono">{usageRow.tokens} tokens</span>
      )}
    </Link>
  );
}

/**
 * The always-visible line under the screen: engine health, the first
 * thing wrong or "Engine running", the daemon version, live sessions,
 * and — while a session is focused on the Session screen — its
 * sandbox posture and context. The dot and the headline link to the
 * Engine setup screen, which holds the details; the version and the
 * count are plain text.
 */
export function StatusBar() {
  const { status, problem } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";

  /* Same gate as the Dashboard: `state_snapshot` boots the daemon on
   * demand, and the status bar must never be what starts Willie. */
  const snapshot = useSnapshot(daemonRunning).snapshot;
  const { session } = useFocusedSession();
  const pathname = useLocation({ select: (location) => location.pathname });

  const showGovernance =
    pathname === "/session" &&
    session !== null &&
    (session.kind ?? "agent") === "agent";

  const [usageSnapshot, setUsageSnapshot] = useState<UsageSnapshot | null>(
    null,
  );

  /* Polls only while the segment is actually visible: a shell session
   * or any other screen tears the interval down rather than leaving it
   * running for a segment nobody sees. */
  useEffect(() => {
    if (!showGovernance) {
      setUsageSnapshot(null);
      return;
    }

    let cancelled = false;

    function load() {
      usage
        .snapshot()
        .then((next) => {
          if (!cancelled) setUsageSnapshot(next);
        })
        .catch(() => {
          /* The segment keeps showing the posture without a meter;
           * usage is a nicety here, not something worth a banner. */
        });
    }

    load();
    const interval = setInterval(load, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [showGovernance]);

  const summary = summarize(status);
  const tone: Tone = problem ? "error" : toneForHealth(summary.health);
  const live =
    daemonRunning && snapshot ? liveSessions(snapshot.sessions).length : null;

  return (
    <footer
      data-slot="status-bar"
      className="flex h-7 shrink-0 items-center gap-3 border-t bg-sidebar px-3 text-muted-foreground text-xs"
    >
      <Link
        to="/setup/engine"
        className="flex min-w-0 items-center gap-2 hover:text-foreground"
      >
        <StatusDot tone={tone} label={`engine ${summary.health}`} />
        <span className="truncate">
          {problem ? problem.message : summary.headline}
        </span>
      </Link>

      {summary.version && (
        <span className="font-mono">willied {summary.version}</span>
      )}

      {showGovernance && session && (
        <GovernanceSegment
          session={session}
          usageRow={usageSnapshot?.sessions.find((s) => s.id === session.id)}
        />
      )}

      {live !== null && (
        <span className="ml-auto">
          {live} live session{live === 1 ? "" : "s"}
        </span>
      )}
    </footer>
  );
}
