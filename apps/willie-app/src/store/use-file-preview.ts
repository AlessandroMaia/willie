import { useSyncExternalStore } from "react";

export interface FilePreviewState {
  path: string;
  expanded: boolean;
}

type Listener = () => void;

let state: FilePreviewState | null = null;
const listeners = new Set<Listener>();

function setState(next: FilePreviewState | null): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): FilePreviewState | null {
  return state;
}

function openFile(path: string): void {
  setState({ path, expanded: false });
}

function toggleExpanded(): void {
  if (state === null) return;
  setState({ ...state, expanded: !state.expanded });
}

function close(): void {
  setState(null);
}

export interface FilePreviewHandle {
  preview: FilePreviewState | null;
  openFile: (path: string) => void;
  toggleExpanded: () => void;
  close: () => void;
}

/**
 * The read-only file preview's open file and its expanded flag, a
 * module-level singleton so a tree row's click and the preview panel
 * itself agree on one state without a shared ancestor. A new click
 * always replaces the content — never merges with what was open.
 */
export function useFilePreview(): FilePreviewHandle {
  const preview = useSyncExternalStore(subscribe, getState);

  return { preview, openFile, toggleExpanded, close };
}
