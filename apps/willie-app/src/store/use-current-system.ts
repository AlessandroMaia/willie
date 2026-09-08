import { useEffect, useSyncExternalStore } from "react";
import { resolveCurrent } from "@/lib/domain/systems";
import { ui } from "@/lib/ipc";
import type { Project } from "@/lib/proto";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSnapshot } from "@/store/use-snapshot";

interface PreferenceState {
  preferred: string | null;
  /** True until the one `ui.prefs()` read settles, resolved or
   * rejected. Gates `system` below `resolveCurrent`, not inside it —
   * `resolveCurrent` stays pure and knows nothing about loading. */
  loading: boolean;
}

type Listener = () => void;

let state: PreferenceState = { preferred: null, loading: true };
const listeners = new Set<Listener>();
/* Guards the one `ui.prefs()` read per app life; `state.loading` is
 * the reactive half of that guard, this is the imperative half so a
 * second `ensureLoaded()` call (a second mount) never starts a second
 * read while the first is still in flight or already done. */
let started = false;

function setState(next: PreferenceState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): PreferenceState {
  return state;
}

function ensureLoaded(): void {
  if (started) return;
  started = true;
  ui.prefs()
    .then((prefs) => {
      setState({ preferred: prefs.current_project ?? null, loading: false });
    })
    .catch((error: unknown) => {
      /* A UI preference must never block the shell: a read that fails
       * degrades to "no preference" (resolves to the first system),
       * never to a blank one — but it is not swallowed silently. */
      console.warn("failed to read the current-system preference", error);
      setState({ preferred: null, loading: false });
    });
}

/** Back to "nothing read yet", the one `ui.prefs()` read included.
 * Only a test harness calls this — the app reads the preference once
 * per launch and never unreads it. */
export function resetForTests(): void {
  started = false;
  setState({ preferred: null, loading: true });
}

export interface CurrentSystemHandle {
  system: Project | null;
  setSystem: (id: string) => void;
  /** True until the persisted preference has been read once; `system`
   * stays `null` for as long as this is true, so a screen never
   * paints the first project and then flickers to the saved one. */
  loading: boolean;
}

/**
 * The one system every screen is scoped to: the persisted
 * `current_project` (Task 7's `ui.prefs`) resolved against the
 * daemon's live project list — gated on the daemon actually running,
 * the same invariant `useSnapshot` asks every other caller to hold, so
 * this is never what boots Willie — and persisted back through
 * `ui.setPrefs` whenever the user picks a different one.
 */
export function useCurrentSystem(): CurrentSystemHandle {
  const { preferred, loading } = useSyncExternalStore(subscribe, getState);
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot } = useSnapshot(daemonRunning);

  useEffect(() => {
    ensureLoaded();
  }, []);

  function setSystem(id: string): void {
    setState({ ...state, preferred: id });
    void ui.setPrefs({ current_project: id });
  }

  return {
    system: loading
      ? null
      : resolveCurrent(preferred, snapshot?.projects ?? []),
    setSystem,
    loading,
  };
}
