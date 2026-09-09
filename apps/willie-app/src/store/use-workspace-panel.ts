import { useSyncExternalStore } from "react";

/** The panel's three surfaces onto the current system's workspace. */
export type PanelTab = "tree" | "file" | "shell";

export interface WorkspacePanelState {
  open: boolean;
  tab: PanelTab;
  /** The workspace's branch, set by the tree once its root listing
   * resolves — `null` until then, and on every system change. */
  branch: string | null;
  /** The workspace-relative path the File tab is showing, or `null`. */
  file: string | null;
}

type Listener = () => void;

const CLOSED: WorkspacePanelState = {
  open: false,
  tab: "tree",
  branch: null,
  file: null,
};

let state: WorkspacePanelState = CLOSED;
const listeners = new Set<Listener>();

function setState(next: WorkspacePanelState): void {
  state = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getState(): WorkspacePanelState {
  return state;
}

function toggle(): void {
  setState({ ...state, open: !state.open });
}

function close(): void {
  setState({ ...state, open: false });
}

function openAt(tab: PanelTab): void {
  setState({ ...state, open: true, tab });
}

function setBranch(branch: string | null): void {
  setState({ ...state, branch });
}

/** A file is only ever picked to be read, so this is also what selects
 * the tab that reads it. */
function openFile(file: string): void {
  setState({ ...state, open: true, tab: "file", file });
}

/**
 * What survives a system change and what does not: the branch and the
 * file describe one workspace, and carried across they would show the
 * previous system's file under the next system's name. Which surface
 * the user chose is theirs, and stays.
 */
function forSystem(): void {
  setState({ ...state, branch: null, file: null });
}

/** Back to closed on the tree, with no workspace read yet. Only a test
 * harness calls this. */
export function resetForTests(): void {
  setState(CLOSED);
}

export interface WorkspacePanelHandle extends WorkspacePanelState {
  toggle: () => void;
  close: () => void;
  openAt: (tab: PanelTab) => void;
  setBranch: (branch: string | null) => void;
  openFile: (file: string) => void;
  forSystem: () => void;
}

/**
 * The workspace panel's open flag, its active tab and what it is
 * showing — a module-level singleton because the header's toggle, the
 * panel itself and the tree rows inside it have no shared ancestor:
 * the toggle sits in the title bar, the panel in the screen's card.
 */
export function useWorkspacePanel(): WorkspacePanelHandle {
  const snapshot = useSyncExternalStore(subscribe, getState);

  return {
    ...snapshot,
    toggle,
    close,
    openAt,
    setBranch,
    openFile,
    forSystem,
  };
}
