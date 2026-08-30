/* Mirrors of crates/willie-proto and the `Project` type from
 * crates/willie-core (see project.rs, job.rs, state.rs). The `state` and
 * `kind` tags below must match the Rust `#[serde(tag = "...")]` wire
 * shapes exactly. */

export type ProjectState =
  | { state: "preparing" }
  | { state: "ready" }
  | { state: "failed"; code: string; message: string; remediation: string };

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
