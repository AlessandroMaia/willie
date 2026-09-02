/* Mirrors of crates/willie-proto and the `Project` type from
 * crates/willie-core (see project.rs, job.rs, state.rs). The `state` and
 * `kind` tags below must match the Rust `#[serde(tag = "...")]` wire
 * shapes exactly. */

export type ProjectState =
  | { state: "preparing" }
  | { state: "ready" }
  | { state: "failed"; code: string; message: string; remediation: string };

export type PathMode = "ro" | "rw";

export interface ExtraPath {
  path: string;
  mode: PathMode;
}

/* An absent override means "whatever the harness decided". Rust omits
 * those keys rather than writing null, because the profile is also a
 * TOML file and TOML has no null. */
export interface SandboxProfile {
  project_rw?: boolean;
  agent_state?: boolean;
  tools_ro?: boolean;
  caches_rw?: boolean;
  git_identity?: boolean;
  extra_paths?: ExtraPath[];
  home_persistent?: boolean;
  ssh?: boolean;
  mnt_all?: boolean;
  windows_interop?: boolean;
}

export interface Project {
  id: string;
  name: string;
  slug: string;
  source: string;
  workspace: string;
  branch: string;
  state: ProjectState;
  source_present: boolean;
  created_at: string;
  sandbox: SandboxProfile;
}

/* Mirrors `willie_core::sandbox::Capability`: the dotted name and the
 * consequence sentence live in Rust (`display_name()`, `consequence()`)
 * and reach the app only through `sandbox.catalogue()`, never
 * duplicated here as prose. */
export type Capability =
  | "project_rw"
  | "agent_state"
  | "tools_ro"
  | "caches_rw"
  | "git_identity"
  | "extra_paths"
  | "home_persistent"
  | "ssh"
  | "mnt_all"
  | "windows_interop";

export interface CapabilityInfo {
  capability: Capability;
  display_name: string;
  consequence: string;
  implemented: boolean;
  /* Layer 1's answer, from `Harness::default_capabilities()`. What a
   * row shows when the project's profile says nothing about it; never
   * assumed here, because the trait's own default and Claude Code's
   * differ on `agent_state`. */
  default_enabled: boolean;
}

export type JobKind =
  | "add"
  | "remove"
  | "sync_to_windows"
  | "update_from_windows"
  | "relocate"
  | "install_harness";
export type JobState =
  | { state: "running" }
  | { state: "done" }
  | { state: "failed"; code: string; message: string; remediation: string };
export interface Job {
  id: string;
  kind: JobKind;
  /* Absent for tool jobs (Rust omits a None). */
  project_id?: string;
  state: JobState;
  started_at: string;
  /* Rust omits this field when `None` (serde skip_serializing_if). */
  finished_at?: string | null;
  log_tail: string;
}
export type SessionState =
  | { state: "creating" }
  | { state: "running" }
  | { state: "stopping" }
  | { state: "exited"; code: number | null; signal: number | null }
  | { state: "failed"; code: string; message: string; remediation: string };
export interface Session {
  id: string;
  project_id: string;
  harness: string;
  workspace: string;
  state: SessionState;
  created_at: string;
  started_at?: string | null;
  finished_at?: string | null;
  pid?: number | null;
  clients: number;
  resumed_from?: string | null;
}
export interface Snapshot {
  seq: number;
  projects: Project[];
  jobs: Job[];
  sessions: Session[];
}
export type EventKind =
  | { kind: "project_changed"; project: Project }
  | { kind: "project_removed"; id: string }
  | { kind: "job_changed"; job: Job }
  | { kind: "session_changed"; session: Session };
export type Event = { seq: number } & EventKind;
export interface Candidate {
  path: string;
  name: string;
}
