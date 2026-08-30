import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Candidate, Event, Session, Snapshot } from "./proto";

/* Mirrors of crates/willie-engine (EngineStatus) and willie-proto. */
export type CheckStatus = "ok" | "fail" | "skip";
export interface DoctorCheck {
  name: string;
  status: CheckStatus;
  detail: string;
  /* Rust omits this field when `None` (serde skip_serializing_if). */
  remediation?: string | null;
  required: boolean;
}
export interface DoctorReport {
  checks: DoctorCheck[];
}
export interface Problem {
  code: string;
  message: string;
  remediation: string;
}
export interface WslStatus {
  installed: boolean;
  version: string | null;
  meets_minimum: boolean;
  minimum: string;
}
export interface DistroStatus {
  registered: boolean;
  running: boolean;
  install_dir: string;
}
export type DaemonState =
  | { state: "stopped" }
  | { state: "running"; willie_version: string; image_version: string | null }
  | { state: "failed"; code: string; message: string };
export interface EngineStatus {
  engine_version: string;
  wsl: WslStatus;
  distro: DistroStatus | null;
  distro_error: Problem | null;
  daemon: DaemonState;
  doctor: DoctorReport | null;
  image_available: boolean;
}

const STATUS_EVENT = "engine://status";
const DAEMON_EVENT = "daemon://event";

export const engine = {
  status: () => invoke<EngineStatus>("engine_status"),
  installDistro: () => invoke<EngineStatus>("engine_install_distro"),
  startDaemon: () => invoke<EngineStatus>("engine_start_daemon"),
  stopDaemon: () => invoke<EngineStatus>("engine_stop_daemon"),
  doctor: () => invoke<DoctorReport>("engine_doctor"),
  onStatus: (cb: (status: EngineStatus) => void): Promise<UnlistenFn> =>
    listen<EngineStatus>(STATUS_EVENT, (event) => cb(event.payload)),
};

/* Project and job truth flows to the webview only through
 * `daemon://event` (see `onDaemonEvent`); every command here returns
 * just the raw RPC reply, never something the UI should render as
 * truth on its own. */
export const projects = {
  snapshot: () => invoke<Snapshot>("state_snapshot"),
  add: (windowsPath: string, name?: string) =>
    invoke("project_add", { windowsPath, name }),
  remove: (id: string, deleteWorkspace: boolean, force: boolean) =>
    invoke("project_remove", { id, deleteWorkspace, force }),
  syncToWindows: (id: string) => invoke("project_sync_to_windows", { id }),
  updateFromWindows: (id: string) =>
    invoke("project_update_from_windows", { id }),
  relocate: (id: string, windowsPath: string) =>
    invoke("project_relocate", { id, windowsPath }),
  rename: (id: string, name: string) => invoke("project_rename", { id, name }),
  cancelJob: (id: string) => invoke("job_cancel", { id }),
  roots: () => invoke<string[]>("projects_roots"),
  setRoots: (roots: string[]) => invoke("set_projects_roots", { roots }),
  discover: () => invoke<Candidate[]>("discover_projects"),
  openInExplorer: (path: string) => invoke("open_in_explorer", { path }),
};

export interface SessionOpened {
  session: Session;
  /* Present only when the session is live but no terminal tab opened;
   * its remediation is the copy-paste `willie attach` line. */
  terminal_problem?: Problem;
}

export const sessions = {
  open: (projectId: string) =>
    invoke<SessionOpened>("session_open", { projectId }),
  attach: (id: string, title: string) =>
    invoke("session_attach", { id, title }),
  stop: (id: string) => invoke("session_stop", { id }),
};

export const sessionTerminal = {
  open: (id: string) => invoke("session_terminal_open", { id }),
  input: (id: string, data: string) =>
    invoke("session_terminal_input", { id, data }),
  resize: (id: string, rows: number, cols: number) =>
    invoke("session_terminal_resize", { id, rows, cols }),
  close: (id: string) => invoke("session_terminal_close", { id }),
};

export const onSessionOutput = (
  cb: (out: { id: string; chunk: number[] }) => void,
): Promise<UnlistenFn> =>
  listen<{ id: string; chunk: number[] }>("session://output", (event) =>
    cb(event.payload),
  );

export const tools = {
  install: (harness: string) => invoke("tool_install", { harness }),
};

export const onDaemonEvent = (cb: (ev: Event) => void): Promise<UnlistenFn> =>
  listen<Event>(DAEMON_EVENT, (event) => cb(event.payload));

export function isProblem(value: unknown): value is Problem {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}
