import { useEffect, useSyncExternalStore } from "react";
import {
  createEngineStatusStore,
  type EngineStatusState,
} from "@/lib/domain/engine-status";
import { engine } from "@/lib/ipc";

const store = createEngineStatusStore({
  status: () => engine.status(),
  onStatus: engine.onStatus,
});

export interface EngineStatusHandle extends EngineStatusState {
  refresh: () => Promise<void>;
}

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
