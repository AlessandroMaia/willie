import { useEffect, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemTitle,
} from "@/components/ui/item";
import { contextTone } from "@/lib/domain/usage";
import type { Problem } from "@/lib/ipc";
import { usage } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { UsageSnapshot } from "@/lib/proto";

/** How often the panel re-fetches while open. Real-time `usage.updated`
 * delivery is deferred, so this poll is the primary refresh — short
 * enough to feel live without hammering the daemon. */
const POLL_INTERVAL_MS = 4000;

const EMPTY_SNAPSHOT: UsageSnapshot = {
  providers: [],
  sessions: [],
  projects: [],
  fetched_at: "",
};

/**
 * The usage plugin's own screen: a context meter and token total per
 * session, and a per-project token summary, refreshed on a light poll
 * while the panel is open. Reaches the daemon only through
 * `usage.snapshot` (the `usage` bridge in `lib/ipc.ts`) — this module
 * never imports a feature.
 */
export function UsagePanel() {
  const [snapshot, setSnapshot] = useState<UsageSnapshot>(EMPTY_SNAPSHOT);
  const [problem, setProblem] = useState<Problem | null>(null);

  useEffect(() => {
    let cancelled = false;

    function load() {
      usage
        .snapshot()
        .then((next) => {
          if (cancelled) return;
          setSnapshot(next);
          setProblem(null);
        })
        .catch((error: unknown) => {
          if (!cancelled) setProblem(asProblem(error));
        });
    }

    load();
    const interval = setInterval(load, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return (
    <div className="flex flex-col gap-6 border-t pt-4">
      <header>
        <h2 className="font-semibold text-base">Session usage</h2>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      <ItemGroup className="gap-1">
        {snapshot.sessions.map((session) => {
          const pct = session.context_pct ?? null;
          const tone = contextTone(pct);
          return (
            <Item key={session.id} variant="outline">
              <ItemContent>
                <ItemTitle>{session.id}</ItemTitle>
                <ItemDescription>{session.tokens} tokens</ItemDescription>
              </ItemContent>
              <StatusBadge tone={tone}>
                {pct === null ? "no usage yet" : `${pct}% context`}
              </StatusBadge>
            </Item>
          );
        })}
      </ItemGroup>

      <section className="flex flex-col gap-2">
        <h3 className="font-medium text-sm">Per-project totals</h3>
        <ItemGroup className="gap-1">
          {snapshot.projects.map((project) => (
            <Item key={project.id} variant="outline">
              <ItemContent>
                <ItemTitle>{project.id}</ItemTitle>
                <ItemDescription>{project.tokens} tokens</ItemDescription>
              </ItemContent>
            </Item>
          ))}
        </ItemGroup>
      </section>
    </div>
  );
}
