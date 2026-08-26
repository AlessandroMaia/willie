import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export const engine = {
  status: () => invoke<EngineStatus>("engine_status"),
  installDistro: () => invoke<EngineStatus>("engine_install_distro"),
  startDaemon: () => invoke<EngineStatus>("engine_start_daemon"),
  stopDaemon: () => invoke<EngineStatus>("engine_stop_daemon"),
  doctor: () => invoke<DoctorReport>("engine_doctor"),
  onStatus: (cb: (status: EngineStatus) => void): Promise<UnlistenFn> =>
    listen<EngineStatus>(STATUS_EVENT, (event) => cb(event.payload)),
};

export function isProblem(value: unknown): value is Problem {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value
  );
}
