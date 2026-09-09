import { useSyncExternalStore } from "react";

/** Which of the current system's confirmations is on screen. */
export type SystemDialog = "rename" | "relocate" | "remove" | null;

interface SystemActionsState {
  dialog: SystemDialog;
}

type Listener = () => void;

let state: SystemActionsState = { dialog: null };
const listeners = new Set<Listener>();

function setState(next: SystemActionsState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): SystemActionsState {
  return state;
}

function open(dialog: Exclude<SystemDialog, null>): void {
  setState({ dialog });
}

function close(): void {
  setState({ dialog: null });
}

/** Nothing open. Only a test harness calls this. */
export function resetForTests(): void {
  setState({ dialog: null });
}

export interface SystemActionsHandle extends SystemActionsState {
  open: (dialog: Exclude<SystemDialog, null>) => void;
  close: () => void;
}

/**
 * Which system confirmation is open, a module-level singleton because
 * the rows that open one and the dialogs themselves have no shared
 * ancestor: the rows live inside the system selector's popover, which
 * unmounts its content when it closes, and a dialog mounted in there
 * would go with it. The dialogs are mounted outside the popover and
 * read this instead.
 */
export function useSystemActions(): SystemActionsHandle {
  const snapshot = useSyncExternalStore(subscribe, getState);

  return { dialog: snapshot.dialog, open, close };
}
