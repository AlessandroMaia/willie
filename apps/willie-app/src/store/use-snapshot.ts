import { useEffect, useSyncExternalStore } from "react";
import { createStore, type StoreState } from "@/lib/domain/daemon-snapshot";
import { onDaemonEvent, projects } from "@/lib/ipc";

/* Both sides of the bridge are reached through a closure, never read
 * at module load: importing this store must not touch the bridge, only
 * acquiring it does. */
const store = createStore({
  snapshot: () => projects.snapshot(),
  onEvent: (cb) => onDaemonEvent(cb),
});

/** Drops the held snapshot and every holder. Only a test harness calls
 * this. */
export const resetForTests = store.reset;

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
