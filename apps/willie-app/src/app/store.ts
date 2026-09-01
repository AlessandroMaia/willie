import { useEffect, useSyncExternalStore } from "react";
import { createStore, type StoreState } from "@/lib/domain/store";
import { onDaemonEvent, projects } from "@/lib/ipc";

const store = createStore({
  snapshot: () => projects.snapshot(),
  onEvent: onDaemonEvent,
});

/**
 * Subscribes to the one store. `enabled` is the Dashboard's gate:
 * `state_snapshot` starts the daemon on demand — booting the WSL VM,
 * up to a 60s HELLO timeout — so a screen that must never be what
 * boots Willie passes false until it knows the daemon is running.
 */
export function useSnapshot(enabled = true): StoreState {
  const state = useSyncExternalStore(store.subscribe, store.getState);
  useEffect(() => {
    if (!enabled) return;
    return store.acquire();
  }, [enabled]);
  return state;
}
