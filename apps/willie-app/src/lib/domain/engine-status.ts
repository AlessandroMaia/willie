import {
  type Health,
  healthFor,
  overallHealth,
  PARTS,
  type Part,
} from "@/lib/domain/health";
import type { EngineStatus, Problem } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";

/** What the store needs from the outside; `store/use-engine-status.ts`
 * wires the bridge, tests wire a fake. */
export interface EngineStatusSource {
  status: () => Promise<EngineStatus>;
  onStatus: (cb: (status: EngineStatus) => void) => Promise<() => void>;
}

export interface EngineStatusState {
  /** The last status that arrived, kept across a failed refresh. */
  status: EngineStatus | null;
  /** Why the last refresh or the subscription failed; cleared by the
   * next status that arrives, pushed or fetched. */
  problem: Problem | null;
}

export interface EngineStatusStore {
  getState: () => EngineStatusState;
  subscribe: (listener: () => void) => () => void;
  /** Refcounted: the engine subscription lives while at least one
   * holder is active. Returns the release, safe to call twice. */
  acquire: () => () => void;
  /** Re-reads the status now and resolves after the state has been
   * updated, so a caller can await it right after an action. */
  refresh: () => Promise<void>;
}

/**
 * The engine's own view of itself: WSL, distribution, daemon, doctor.
 * Reading it never starts anything, which is what lets the status bar
 * hold it for the whole life of the window.
 */
export function createEngineStatusStore(
  source: EngineStatusSource,
): EngineStatusStore {
  let state: EngineStatusState = { status: null, problem: null };
  let holders = 0;
  /* Bumped on every first acquire and last release so a status that
   * resolves after a teardown cannot revive a released store. */
  let generation = 0;
  let unlisten: (() => void) | undefined;
  const listeners = new Set<() => void>();

  function set(next: EngineStatusState) {
    state = next;
    for (const listener of listeners) listener();
  }

  async function refresh(gen: number): Promise<void> {
    try {
      const status = await source.status();
      if (gen !== generation) return;
      set({ status, problem: null });
    } catch (error) {
      if (gen !== generation) return;
      set({ status: state.status, problem: asProblem(error) });
    }
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
        void refresh(gen);
        void source
          .onStatus((status) => {
            if (gen === generation) set({ status, problem: null });
          })
          .then((fn) => {
            if (gen === generation) unlisten = fn;
            else fn();
          })
          .catch((error: unknown) => {
            if (gen !== generation) return;
            set({ status: state.status, problem: asProblem(error) });
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
        }
      };
    },
    refresh: () => refresh(generation),
  };
}

export interface StatusSummary {
  health: Health;
  /** One line for a status bar: the first part that is not ok, or
   * "Engine running". */
  headline: string;
  /** The daemon's version while it runs; null otherwise. */
  version: string | null;
}

const HEADLINES: Record<Part, (s: EngineStatus) => string> = {
  wsl: (s) => (s.wsl.installed ? "WSL too old" : "WSL not installed"),
  distro: (s) =>
    s.distro_error ? s.distro_error.message : "Distribution not registered",
  daemon: (s) =>
    s.daemon.state === "failed"
      ? `Daemon failed: ${s.daemon.message}`
      : "Daemon stopped",
  doctor: (s) => (s.doctor ? "Doctor reports a failure" : "Doctor not run yet"),
};

export function summarize(status: EngineStatus | null): StatusSummary {
  if (status === null) {
    return {
      health: "degraded",
      headline: "Engine status unknown",
      version: null,
    };
  }

  const first = PARTS.find((part) => healthFor(part, status) !== "ok");
  const headline = first ? HEADLINES[first](status) : "Engine running";
  const version =
    status.daemon.state === "running" ? status.daemon.willie_version : null;

  return { health: overallHealth(status), headline, version };
}
