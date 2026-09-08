import { useSyncExternalStore } from "react";
import type { SetupEntry } from "@/lib/domain/setup-entries";

interface SetupDrawerState {
  open: boolean;
  entry: SetupEntry | null;
}

type Listener = () => void;

let state: SetupDrawerState = { open: false, entry: null };
const listeners = new Set<Listener>();

function setState(next: SetupDrawerState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): SetupDrawerState {
  return state;
}

function openAt(entry?: SetupEntry): void {
  setState({ open: true, entry: entry ?? null });
}

function close(): void {
  setState({ open: false, entry: null });
}

/** Back to closed, on no entry. Only a test harness calls this. */
export function resetForTests(): void {
  close();
}

export interface SetupDrawerHandle extends SetupDrawerState {
  openAt: (entry?: SetupEntry) => void;
  close: () => void;
}

/**
 * The setup drawer's own open flag and which entry it should land on,
 * a module-level singleton so the header's button and the drawer
 * itself agree on one state without a shared ancestor.
 */
export function useSetupDrawer(): SetupDrawerHandle {
  const snapshot = useSyncExternalStore(subscribe, getState);

  return { ...snapshot, openAt, close };
}
