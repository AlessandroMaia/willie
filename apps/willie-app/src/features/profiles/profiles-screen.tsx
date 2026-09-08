import { LayersIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
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
import { Spinner } from "@/components/ui/spinner";
import { isPluginDisabled } from "@/lib/domain/plugins";
import type { Problem } from "@/lib/ipc";
import { plugins, profiles } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { ApplyPanel } from "@/plugins/profiles/apply-panel";
import { useCurrentSystem } from "@/store/use-current-system";
import { useSetupDrawer } from "@/store/use-setup-drawer";

type Gate = "checking" | "disabled" | "ready";

/**
 * The current system's own Profiles screen: whether the profiles
 * plugin is enabled for this project gates everything else — on a
 * fresh install `profile.*` refuses every call until then, and that
 * refusal is a first-class state here, an Enable button, never an
 * error chip.
 */
export function ProfilesScreen() {
  const { system, loading } = useCurrentSystem();
  const { openAt } = useSetupDrawer();
  const [gate, setGate] = useState<Gate>("checking");
  const [enabling, setEnabling] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);

  /* `profile.list` carries no project id — the daemon's own gate is
   * machine-wide ("has any system enabled this plugin"), not yet
   * scoped to the one this screen is showing. That daemon-side
   * semantics fix is the next plan, not this task; this screen reads
   * whatever the daemon answers today. */
  const checkEnabled = useCallback(() => {
    setGate("checking");
    setProblem(null);
    profiles
      .list()
      .then(() => setGate("ready"))
      .catch((error: unknown) => {
        if (isPluginDisabled(error)) {
          setGate("disabled");
          return;
        }
        setProblem(asProblem(error));
        setGate("ready");
      });
  }, []);

  useEffect(() => {
    if (system) checkEnabled();
  }, [system, checkEnabled]);

  async function enable() {
    if (!system) return;
    setEnabling(true);
    setProblem(null);
    try {
      await plugins.enable("profile", system.id);
      checkEnabled();
    } catch (error) {
      setProblem(asProblem(error));
    } finally {
      setEnabling(false);
    }
  }

  if (loading) {
    return (
      <div className="mx-auto flex max-w-3xl flex-col gap-4">
        <Skeleton className="h-6 w-32" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (system === null) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <LayersIcon />
          </EmptyMedia>
          <EmptyTitle>No system yet</EmptyTitle>
          <EmptyDescription>Add one to manage its profiles.</EmptyDescription>
        </EmptyHeader>
        <Button onClick={() => openAt("systems")}>Add a system</Button>
      </Empty>
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Profiles</h1>
      </header>

      {problem && <ProblemAlert problem={problem} />}

      {gate === "checking" && <Skeleton className="h-40 w-full" />}

      {gate === "disabled" && (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <LayersIcon />
            </EmptyMedia>
            <EmptyTitle>Profiles are off for this system</EmptyTitle>
            <EmptyDescription>
              Enable the profiles plugin to apply a profile here.
            </EmptyDescription>
          </EmptyHeader>
          <Button disabled={enabling} onClick={() => void enable()}>
            {enabling && <Spinner />} Enable
          </Button>
        </Empty>
      )}

      {gate === "ready" && <ApplyPanel projectId={system.id} />}
    </div>
  );
}
