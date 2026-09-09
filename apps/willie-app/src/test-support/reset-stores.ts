import { resetForTests as resetCurrentSystem } from "@/store/use-current-system";
import { resetForTests as resetEngineStatus } from "@/store/use-engine-status";
import { resetForTests as resetFilePreview } from "@/store/use-file-preview";
import { resetForTests as resetFocusedSession } from "@/store/use-focused-session";
import { resetForTests as resetSetupDrawer } from "@/store/use-setup-drawer";
import { resetForTests as resetSnapshot } from "@/store/use-snapshot";
import { resetForTests as resetSystemActions } from "@/store/use-system-actions";
import { resetForTests as resetTreeDrawer } from "@/store/use-tree-drawer";

/**
 * Every module-level store back to the state a freshly launched app
 * starts from. A rendering test calls this in its `beforeEach`: the
 * stores are singletons that outlive a render, so without it one case's
 * system, snapshot, focused tab or open drawer would still be there for
 * the next case's first synchronous render. It replaces resetting the
 * whole module graph, which re-imported the entire feature tree per
 * test and made the suite's hook timeouts a matter of machine load.
 */
export function resetStores(): void {
  resetCurrentSystem();
  resetEngineStatus();
  resetFilePreview();
  resetFocusedSession();
  resetSetupDrawer();
  resetSnapshot();
  resetSystemActions();
  resetTreeDrawer();
}
