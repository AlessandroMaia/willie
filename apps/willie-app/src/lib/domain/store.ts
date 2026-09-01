import { applyEvent, needsResnapshot } from "@/lib/domain/state";
import type { Problem } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Event, Snapshot } from "@/lib/proto";

/** What the store needs from the outside. Injected so the store is
 * testable without the bridge; `app/store.ts` wires the real one. */
export interface StoreSource {
  snapshot: () => Promise<Snapshot>;
  onEvent: (cb: (ev: Event) => void) => Promise<() => void>;
}

export type StoreState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; snapshot: Snapshot }
  | { status: "failed"; problem: Problem };

export interface Store {
  getState: () => StoreState;
  subscribe: (listener: () => void) => () => void;
  /** Refcounted: the daemon subscription lives while at least one
   * holder is active, and the last release tears it down. Returns the
   * release, which is safe to call twice. */
  acquire: () => () => void;
}

/**
 * The one place project, job and session truth is held. Events are
 * folded with the same pure `applyEvent` the screens used to call
 * themselves; a sequence gap or a daemon restart falls back to a fresh
 * snapshot, because the incremental history is no longer trustworthy.
 */
export function createStore(source: StoreSource): Store {
  let state: StoreState = { status: "idle" };
  let holders = 0;
  /* Bumped on every acquire and release so a snapshot or an event that
   * resolves after a teardown cannot revive a released store. */
  let generation = 0;
  let unlisten: (() => void) | undefined;
  const listeners = new Set<() => void>();

  function set(next: StoreState) {
    state = next;
    for (const listener of listeners) listener();
  }

  function load(gen: number) {
    source.snapshot().then(
      (snapshot) => {
        if (gen === generation) set({ status: "ready", snapshot });
      },
      (error: unknown) => {
        if (gen === generation) {
          set({ status: "failed", problem: asProblem(error) });
        }
      },
    );
  }

  function handle(ev: Event, gen: number) {
    if (gen !== generation) return;
    if (state.status !== "ready" || needsResnapshot(state.snapshot, ev)) {
      load(gen);
      return;
    }
    set({ status: "ready", snapshot: applyEvent(state.snapshot, ev) });
  }

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    acquire() {
      holders += 1;
      if (holders === 1) {
        generation += 1;
        const gen = generation;
        set({ status: "loading" });
        load(gen);
        void source
          .onEvent((ev) => handle(ev, gen))
          .then((fn) => {
            if (gen === generation) unlisten = fn;
            else fn();
          });
      }
      let released = false;
      return () => {
        if (released) return;
        released = true;
        holders -= 1;
        if (holders === 0) {
          generation += 1;
          unlisten?.();
          unlisten = undefined;
          set({ status: "idle" });
        }
      };
    },
  };
}
