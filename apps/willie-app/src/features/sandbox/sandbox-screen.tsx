import { useSearch } from "@tanstack/react-router";
import { ShieldCheckIcon } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { DenialHistory } from "@/features/sandbox/denial-history";
import { SandboxDrawer } from "@/features/sandbox/sandbox-drawer";
import { counts, denialRows, posture } from "@/lib/domain/sandbox";
import { finishedOf, liveOf, sessionName } from "@/lib/domain/sessions";
import type { Problem } from "@/lib/ipc";
import { projects as projectsApi, sandbox as sandboxApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { CapabilityInfo, SandboxProfile, Session } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSetupDrawer } from "@/store/use-setup-drawer";
import { useSnapshot } from "@/store/use-snapshot";

/** The human label for a sandbox mechanism. An unknown name shows as-is,
 * so a mechanism added in a later phase is never hidden. Presentation,
 * so it lives here rather than in `lib/`. */
function mechanismLabel(name: string): string {
  switch (name) {
    case "rlimits":
      return "limits";
    case "seccomp":
      return "syscall filter";
    case "landlock":
      return "path rules";
    default:
      return name;
  }
}

/**
 * The current system's Sandbox screen: its mechanisms unioned across
 * every one of its sessions, the three headline counts, a
 * chronological denial history filterable per session, and the
 * capability editor moved into a drawer. There is deliberately no
 * one-click "allow" anywhere on a denial row — a denial is a fact the
 * sandbox recorded, and the only way to change what is allowed is the
 * drawer's own edit-and-save.
 */
export function SandboxScreen() {
  const { system, loading } = useCurrentSystem();
  const { openAt } = useSetupDrawer();
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot } = useSnapshot(daemonRunning);
  const search = useSearch({ from: "/sandbox" });

  const [drawerOpen, setDrawerOpen] = useState(false);
  const [catalogue, setCatalogue] = useState<CapabilityInfo[]>([]);
  const [catalogueProblem, setCatalogueProblem] = useState<Problem | null>(
    null,
  );
  const [sandboxProblem, setSandboxProblem] = useState<Problem | null>(null);
  const [filter, setFilter] = useState<string>(search.session ?? "all");

  /* Fetched once: static domain data, no project id, safe for the
   * lifetime of the screen. A failure here is not silent — without the
   * catalogue the drawer has nothing to render. */
  useEffect(() => {
    sandboxApi
      .catalogue()
      .then(setCatalogue)
      .catch((error: unknown) => setCatalogueProblem(asProblem(error)));
  }, []);

  /* A capability profile is edited for exactly one system: if the
   * current system changes while the drawer is open (the daemon is
   * shared — another client can rename, remove, or the selector
   * itself can switch) the drawer closes and its unsaved edits are
   * discarded, rather than risk a Save recapturing the new system's
   * id and writing one system's edits onto another. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: reacts to a system change on purpose — the body itself has nothing left to read once the drawer is closed
  useEffect(() => {
    setDrawerOpen(false);
  }, [system?.id]);

  const systemId = system?.id;
  const sessions = snapshot?.sessions;

  /* Denial history spans the system's whole life, not only its live
   * sessions — a session that already finished keeps its place in the
   * chronology. */
  const systemSessions = useMemo<Session[]>(
    () =>
      systemId && sessions
        ? [...liveOf(systemId, sessions), ...finishedOf(systemId, sessions)]
        : [],
    [systemId, sessions],
  );

  const mechanisms = useMemo(() => posture(systemSessions), [systemSessions]);
  const rows = useMemo(() => denialRows(systemSessions), [systemSessions]);
  const totals = useMemo(() => counts(rows), [rows]);

  const sessionChips = useMemo(() => {
    const chips: { id: string; label: string }[] = [];
    const seen = new Set<string>();
    for (const row of rows) {
      if (seen.has(row.sessionId)) continue;
      seen.add(row.sessionId);
      const named = systemSessions.find((s) => s.id === row.sessionId);
      chips.push({
        id: row.sessionId,
        label: named ? sessionName(named) : row.sessionId,
      });
    }
    return chips;
  }, [rows, systemSessions]);

  /* An id with no denials — the query named a session that never
   * triggered one, or one the system no longer has — falls back to
   * "all" rather than showing a filtered view with no way out. Derived
   * on every render instead of written back into `filter`, so a later
   * snapshot update never overrides a choice the user made by hand. */
  const activeFilter =
    filter !== "all" && sessionChips.some((chip) => chip.id === filter)
      ? filter
      : "all";

  const filteredRows =
    activeFilter === "all"
      ? rows
      : rows.filter((row) => row.sessionId === activeFilter);

  function openDrawer(): void {
    setSandboxProblem(null);
    setDrawerOpen(true);
  }

  /* Same shape as the former `projects-screen.tsx`'s `saveSandbox`:
   * `project.set_sandbox` validates by resolving before it persists, so
   * the one rejection this ever sees is a policy the daemon refuses
   * outright — kept visible inside the still-open drawer. */
  async function saveSandbox(profile: SandboxProfile): Promise<void> {
    if (!system) return;
    setSandboxProblem(null);
    try {
      await projectsApi.setSandbox(system.id, profile);
      setDrawerOpen(false);
    } catch (error) {
      setSandboxProblem(asProblem(error));
    }
  }

  if (loading) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (system === null) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <ShieldCheckIcon />
          </EmptyMedia>
          <EmptyTitle>No system yet</EmptyTitle>
          <EmptyDescription>
            Add one to see what its sessions may reach.
          </EmptyDescription>
        </EmptyHeader>
        <Button onClick={() => openAt("systems")}>Add a system</Button>
      </Empty>
    );
  }

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-6">
      <header className="flex items-center justify-between gap-2">
        <h1 className="font-semibold text-lg">{system.name}</h1>
        <Button variant="outline" onClick={openDrawer}>
          Edit capabilities
        </Button>
      </header>

      {catalogueProblem && <ProblemAlert problem={catalogueProblem} />}

      <div className="flex flex-wrap gap-2">
        {mechanisms.applied.map((m) => (
          <StatusBadge key={`applied-${m}`} tone="ok">
            {mechanismLabel(m)}
          </StatusBadge>
        ))}
        {/* A mechanism the kernel could not offer and one that
         * degraded read the same here, in the warning tone: the wire
         * carries mechanism names only, no per-mechanism reason, and a
         * chip may not hover a line it does not have. */}
        {mechanisms.unavailable.map((m) => (
          <StatusBadge key={`unavailable-${m}`} tone="warning">
            {mechanismLabel(m)}
          </StatusBadge>
        ))}
        {mechanisms.degraded.map((m) => (
          <StatusBadge key={`degraded-${m}`} tone="warning">
            {mechanismLabel(m)}
          </StatusBadge>
        ))}
      </div>

      <div className="grid grid-cols-3 gap-3">
        <CountTile label="Syscall denials" value={totals.syscalls} />
        <CountTile label="Terminal denials" value={totals.terminal} />
        <CountTile label="Sessions affected" value={totals.sessions} />
      </div>

      {rows.length === 0 ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ShieldCheckIcon />
            </EmptyMedia>
            <EmptyTitle>No denials</EmptyTitle>
            <EmptyDescription>
              Every mechanism has run clean so far.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant={activeFilter === "all" ? "secondary" : "ghost"}
              aria-pressed={activeFilter === "all"}
              onClick={() => setFilter("all")}
            >
              all
            </Button>
            {sessionChips.map((chip) => (
              <Button
                key={chip.id}
                type="button"
                size="sm"
                variant={activeFilter === chip.id ? "secondary" : "ghost"}
                aria-pressed={activeFilter === chip.id}
                onClick={() => setFilter(chip.id)}
              >
                {chip.label}
              </Button>
            ))}
          </div>

          <DenialHistory rows={filteredRows} sessions={systemSessions} />
        </>
      )}

      <SandboxDrawer
        /* Remounts every time it opens, and again if the system itself
         * changes while it is open, so the drawer's local edits always
         * start from the CURRENT system's `sandbox` — never from
         * whatever an earlier open, or another system, left behind. */
        key={drawerOpen ? `open-${system.id}` : "closed"}
        project={drawerOpen ? system : null}
        catalogue={catalogue}
        problem={sandboxProblem}
        onSave={saveSandbox}
        onCancel={() => setDrawerOpen(false)}
      />
    </div>
  );
}

interface CountTileProps {
  label: string;
  value: number;
}

function CountTile({ label, value }: CountTileProps) {
  return (
    <div className="flex flex-col gap-1 rounded-lg border p-3">
      <span className="font-semibold text-2xl">{value}</span>
      <span className="text-muted-foreground text-sm">{label}</span>
    </div>
  );
}
