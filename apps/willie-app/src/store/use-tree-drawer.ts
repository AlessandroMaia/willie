import { useSyncExternalStore } from "react";

interface TreeDrawerState {
  open: boolean;
  /** The workspace's current branch, set by the drawer once its root
   * load resolves — `null` until then, and reset on every system
   * change so the selector never shows a stale one. */
  branch: string | null;
}

type Listener = () => void;

let state: TreeDrawerState = { open: false, branch: null };
const listeners = new Set<Listener>();

function setState(next: TreeDrawerState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): TreeDrawerState {
  return state;
}

function toggle(): void {
  setState({ ...state, open: !state.open });
}

function close(): void {
  setState({ ...state, open: false });
}

function setBranch(branch: string | null): void {
  setState({ ...state, branch });
}

export interface TreeDrawerHandle extends TreeDrawerState {
  toggle: () => void;
  close: () => void;
  setBranch: (branch: string | null) => void;
}

/**
 * The workspace tree drawer's open flag and the root's current
 * branch, a module-level singleton so the tab strip's toggle, the
 * drawer itself and the system selector agree on one state without a
 * shared ancestor.
 */
export function useTreeDrawer(): TreeDrawerHandle {
  const snapshot = useSyncExternalStore(subscribe, getState);

  return { ...snapshot, toggle, close, setBranch };
}
