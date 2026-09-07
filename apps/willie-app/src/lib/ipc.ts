import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Candidate,
  CapabilityInfo,
  Change,
  Event,
  PluginStatus,
  ProfileSummary,
  Project,
  SandboxProfile,
  Session,
  Snapshot,
  ToolList,
} from "./proto";

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
  logonFixScript: () => invoke<string>("engine_logon_fix_script"),
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
  setSandbox: (id: string, profile: SandboxProfile) =>
    invoke<Project>("project_set_sandbox", { id, profile }),
  cancelJob: (id: string) => invoke("job_cancel", { id }),
  roots: () => invoke<string[]>("projects_roots"),
  setRoots: (roots: string[]) => invoke("set_projects_roots", { roots }),
  discover: () => invoke<Candidate[]>("discover_projects"),
  openInExplorer: (path: string) => invoke("open_in_explorer", { path }),
  openInEditor: (workspace: string) => invoke("open_in_editor", { workspace }),
};

/* Static per-machine fact (is VS Code installed?), checked once when the
 * Projects screen mounts — a screen-level gate like the daemon-health
 * check, not a per-project RPC, so it lives beside `projects` rather
 * than inside it. */
export const editorAvailable = (): Promise<boolean> =>
  invoke<boolean>("editor_available");

export interface SessionOpened {
  session: Session;
  /* Present only when the session is live but no terminal tab opened;
   * its remediation is the copy-paste `willie attach` line. */
  terminal_problem?: Problem;
}

export const sessions = {
  open: (projectId: string) =>
    invoke<SessionOpened>("session_open", { projectId }),
  resume: (projectId: string) =>
    invoke<SessionOpened>("session_resume", { projectId }),
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
  update: (tool: string) => invoke("tool_update", { tool }),
  list: () => invoke<ToolList>("tool_list"),
};

/* A plugin's own state (enabled/disabled, degraded) is also carried on
 * every `Snapshot` (`snapshot.plugins`), but that copy only ever
 * refreshes on a full resnapshot or a `PluginChanged` event — and the
 * daemon does not emit that event yet. `enable`/`disable` return the
 * plugin's fresh status directly, so the Plugins screen refetches with
 * `list()` after either rather than trusting the snapshot to catch up. */
export const plugins = {
  list: () => invoke<PluginStatus[]>("plugin_list"),
  enable: (id: string, projectId?: string) =>
    invoke<PluginStatus>("plugin_enable", { id, projectId }),
  disable: (id: string, projectId?: string) =>
    invoke<PluginStatus>("plugin_disable", { id, projectId }),
};

/* The profiles plugin's own methods (`profile.*`), reached through the
 * engine's one guarded pass-through (`plugin_call`): willie-engine never
 * grows a typed method per plugin, so every call here names its own
 * `profile.*` method and shapes its own params/result. */
function pluginCall<R>(method: string, params: unknown): Promise<R> {
  return invoke<R>("plugin_call", { method, params });
}

export const profiles = {
  list: () => pluginCall<ProfileSummary[]>("profile.list", {}),
  create: (name: string) =>
    pluginCall<ProfileSummary>("profile.create", { name }),
  readFragment: (name: string, fragment: string) =>
    pluginCall<{ content: string }>("profile.read_fragment", {
      name,
      fragment,
    }),
  writeFragment: (name: string, fragment: string, content: string) =>
    pluginCall<{ content: string }>("profile.write_fragment", {
      name,
      fragment,
      content,
    }),
  check: (name: string, projectId: string) =>
    pluginCall<{ changes: Change[] }>("profile.check", {
      name,
      project_id: projectId,
    }),
  apply: (name: string, projectId: string) =>
    pluginCall<{ changes: Change[]; backup_path: string }>("profile.apply", {
      name,
      project_id: projectId,
    }),
  setRemote: (name: string, url: string) =>
    pluginCall<Record<string, never>>("profile.set_remote", { name, url }),
  push: (name: string) =>
    pluginCall<Record<string, never>>("profile.push", { name }),
  pull: (name: string) =>
    pluginCall<Record<string, never>>("profile.pull", { name }),
};

/* Static domain data (`willie_core::sandbox::Capability`), not a
 * daemon RPC: no project id, no engine lock, safe to fetch once. */
export const sandbox = {
  catalogue: () => invoke<CapabilityInfo[]>("sandbox_catalogue"),
};

/* The folder picker is I/O like every `invoke`, so it lives here and
 * tests fake one module. */
export const dialogs = {
  pickFolder: async (): Promise<string | null> => {
    const picked = await open({ directory: true });

    return typeof picked === "string" ? picked : null;
  },
};

export const onDaemonEvent = (cb: (ev: Event) => void): Promise<UnlistenFn> =>
  listen<Event>(DAEMON_EVENT, (event) => cb(event.payload));
