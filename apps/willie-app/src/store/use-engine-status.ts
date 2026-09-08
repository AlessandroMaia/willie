import { useEffect, useSyncExternalStore } from "react";
import {
  createEngineStatusStore,
  type EngineStatusState,
} from "@/lib/domain/engine-status";
import { engine } from "@/lib/ipc";

/* Both sides of the bridge are reached through a closure, never read
 * at module load: importing this store must not touch the bridge, only
 * acquiring it does. */
const store = createEngineStatusStore({
  status: () => engine.status(),
  onStatus: (cb) => engine.onStatus(cb),
});

export interface EngineStatusHandle extends EngineStatusState {
  refresh: () => Promise<void>;
}

/** Drops the last status and every holder. Only a test harness calls
 * this. */
export const resetForTests = store.reset;

/**
 * Subscribes to the engine's view of itself. Unlike `useSnapshot`,
 * reading it never boots the daemon, so there is no gate: the status
 * bar holds it for as long as the window is open.
 */
export function useEngineStatus(): EngineStatusHandle {
  const state = useSyncExternalStore(store.subscribe, store.getState);

  useEffect(() => store.acquire(), []);

  return { ...state, refresh: store.refresh };
}
