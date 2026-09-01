import { applyEvent, needsResnapshot } from "@/lib/domain/state";
import type { Problem } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { Event, Snapshot } from "@/lib/proto";

/** What the store needs from the outside. Injected so the store is
 * testable without the bridge; `store/use-snapshot.ts` wires the real
 * one. */
export interface StoreSource {
  snapshot: () => Promise<Snapshot>;
  onEvent: (cb: (ev: Event) => void) => Promise<() => void>;
}

export interface StoreState {
  /** `idle` until something acquires it, `loading` while the first
   * snapshot is in flight, `ready` once one has arrived, `failed`
   * when the last fetch or subscription rejected. */
  status: "idle" | "loading" | "ready" | "failed";
  /** The last snapshot that arrived. Kept across a failed refetch,
   * so a screen goes on showing rows it already had; null until the
   * first one arrives, and cleared when the last holder leaves. */
  snapshot: Snapshot | null;
  /** Why the last fetch or the live subscription failed. A snapshot
   * that later resolves updates `snapshot` but never clears this on
   * its own while the subscription itself is still dead — see
   * `subscriptionProblem` below. */
  problem: Problem | null;
}

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
  let state: StoreState = { status: "idle", snapshot: null, problem: null };
  let holders = 0;
  /* Bumped on every acquire and release so a snapshot or an event that
   * resolves after a teardown cannot revive a released store. */
  let generation = 0;
  let unlisten: (() => void) | undefined;
  /* Set when `onEvent` has rejected for the current generation. Tracked
   * apart from `state.problem` so a snapshot that resolves afterward —
   * the fetch and the subscription are independent promises racing
   * each other — cannot silently report "ready" while the daemon
   * subscription is actually dead. Cleared on every new generation. */
  let subscriptionProblem: Problem | null = null;
  const listeners = new Set<() => void>();

  function set(next: StoreState) {
    state = next;
    for (const listener of listeners) listener();
  }

  /* A rejected fetch never throws away what is already on screen: it
   * keeps the last snapshot, so a screen that already has rows keeps
   * showing them under its own banner instead of going blank. */
  function fail(error: unknown) {
    set({
      status: "failed",
      snapshot: state.snapshot,
      problem: asProblem(error),
    });
  }

  function load(gen: number) {
    source.snapshot().then(
      (snapshot) => {
        if (gen !== generation) return;
        /* A dead subscription outlives a snapshot that happens to
         * resolve later: the store stays "failed" with the snapshot
         * refreshed underneath, rather than flipping to "ready" and
         * erasing the only sign that daemon events stopped arriving. */
        if (subscriptionProblem) {
          set({ status: "failed", snapshot, problem: subscriptionProblem });
          return;
        }
        set({ status: "ready", snapshot, problem: null });
      },
      (error: unknown) => {
        if (gen === generation) fail(error);
      },
    );
  }

  function handle(ev: Event, gen: number) {
    if (gen !== generation) return;
    if (
      state.status !== "ready" ||
      state.snapshot === null ||
      needsResnapshot(state.snapshot, ev)
    ) {
      load(gen);
      return;
    }
    set({
      status: "ready",
      snapshot: applyEvent(state.snapshot, ev),
      problem: null,
    });
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
        subscriptionProblem = null;
        set({ ...state, status: "loading" });
        load(gen);
        void source
          .onEvent((ev) => handle(ev, gen))
          .then((fn) => {
            if (gen === generation) unlisten = fn;
            else fn();
          })
          .catch((error: unknown) => {
            if (gen !== generation) return;
            subscriptionProblem = asProblem(error);
            fail(error);
          });
      }
      let released = false;
      return () => {
        if (released) return;
        released = true;
        holders -= 1;
        if (holders === 0) {
          generation += 1;
          subscriptionProblem = null;
          unlisten?.();
          unlisten = undefined;
          set({ status: "idle", snapshot: null, problem: null });
        }
      };
    },
  };
}
