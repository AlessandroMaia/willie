import { GaugeIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import { UsagePanel } from "@/plugins/usage/usage-panel";
import { useCurrentSystem } from "@/store/use-current-system";
import { useSetupDrawer } from "@/store/use-setup-drawer";

/**
 * The current system's own Usage screen: mounts the usage panel
 * scoped to it. `loading` is checked before `system`, so the
 * preference read never flashes the empty state for a system that is
 * simply still resolving.
 */
export function UsageScreen() {
  const { system, loading } = useCurrentSystem();
  const { openAt } = useSetupDrawer();

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
            <GaugeIcon />
          </EmptyMedia>
          <EmptyTitle>No system yet</EmptyTitle>
          <EmptyDescription>Add one to see its usage.</EmptyDescription>
        </EmptyHeader>
        <Button onClick={() => openAt("systems")}>Add a system</Button>
      </Empty>
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      <header>
        <h1 className="font-semibold text-lg">Usage</h1>
      </header>
      <UsagePanel projectId={system.id} />
    </div>
  );
}
