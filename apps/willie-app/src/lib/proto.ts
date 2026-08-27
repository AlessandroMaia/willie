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
  | "relocate";
export type JobState =
  | { state: "running" }
  | { state: "done" }
  | { state: "failed"; code: string; message: string; remediation: string };
export interface Job {
  id: string;
  kind: JobKind;
  project_id: string;
  state: JobState;
  started_at: string;
  /* Rust omits this field when `None` (serde skip_serializing_if). */
  finished_at?: string | null;
  log_tail: string;
}
export interface Snapshot {
  seq: number;
  projects: Project[];
  jobs: Job[];
}
export type EventKind =
  | { kind: "project_changed"; project: Project }
  | { kind: "project_removed"; id: string }
  | { kind: "job_changed"; job: Job };
export type Event = { seq: number } & EventKind;
export interface Candidate {
  path: string;
  name: string;
}
