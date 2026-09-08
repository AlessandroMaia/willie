import { Link } from "@tanstack/react-router";
import { StatusDot } from "@/components/status-dot";
import { type Tone, toneForHealth } from "@/components/tone";
import { summarize } from "@/lib/domain/engine-status";
import { liveSessions } from "@/lib/domain/sessions";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSnapshot } from "@/store/use-snapshot";

/**
 * The always-visible line under the screen: engine health, the first
 * thing wrong or "Engine running", the daemon version, live sessions.
 * The dot and the headline link to the Dashboard, which holds the
 * details; the version and the count are plain text.
 */
export function StatusBar() {
  const { status, problem } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";

  /* Same gate as the Dashboard: `state_snapshot` boots the daemon on
   * demand, and the status bar must never be what starts Willie. */
  const snapshot = useSnapshot(daemonRunning).snapshot;

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

      {live !== null && (
        <span className="ml-auto">
          {live} live session{live === 1 ? "" : "s"}
        </span>
      )}
    </footer>
  );
}
