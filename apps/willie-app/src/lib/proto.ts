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
  /* Mirrors `willie_core::project::SandboxProblem`: set only when the
   * daemon could not read this project's `[sandbox]` table. `sandbox`
   * then holds the default profile instead, and the daemon refuses
   * `session.create`/`sandbox.explain` while this stays set. Same
   * shape as `Problem` (`lib/ipc.ts`), declared structurally here
   * rather than imported, because `ipc.ts` already imports `Project`
   * from this file. */
  sandbox_problem?: { code: string; message: string; remediation: string };
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
  | "install_harness"
  | "update_harness";
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
export interface Denied {
  class: "syscall" | "terminal";
  name: string;
  count: number;
  first_at: string;
  last_at: string;
}
export interface SandboxState {
  applied: string[];
  unavailable: string[];
  degraded: string[];
  denied: Denied[];
}
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
  sandbox?: SandboxState;
}
/* Mirrors `willie_proto::plugin::{Scope, Enablement, PluginStatus}`.
 * `Enablement` is externally tagged: the variant name is itself the key,
 * `{ global: bool }` for a `Global`-scoped plugin or `{ per_project:
 * [projectId] }` for a `PerProject` one. */
export type Scope = "global" | "per_project";
export type Enablement = { global: boolean } | { per_project: string[] };
export interface PluginStatus {
  id: string;
  name: string;
  scope: Scope;
  enabled: Enablement;
  degraded: boolean;
}

/* Mirrors `willie_plugins::profiles::model::ProfileSummary` and
 * `apply::{Change, ChangeKind}`: plain field names, `ChangeKind` in
 * `snake_case`. */
export interface ProfileSummary {
  name: string;
  fragments_active: string[];
}
export type ChangeKind = "create" | "merge" | "overwrite";
export interface Change {
  path: string;
  kind: ChangeKind;
  after: string;
}

export interface Snapshot {
  seq: number;
  projects: Project[];
  jobs: Job[];
  sessions: Session[];
  /* Optional: `#[serde(default)]` on the Rust side, and every existing
   * test snapshot in this app predates the field. */
  plugins?: PluginStatus[];
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

/* Mirrors `willie_proto::tool::ToolStatus`. `version` is the live
 * detection, absent exactly when `installed` is false. `recorded_version`
 * is what the daemon's manifest last recorded installing — absent when
 * Willie has never itself installed or updated the tool, even if it is
 * present (detected some other way). Rust omits both keys rather than
 * writing null, same convention as `Job.finished_at`. */
export interface ToolStatus {
  id: string;
  name: string;
  installed: boolean;
  version?: string;
  recorded_version?: string;
}

export interface ToolList {
  tools: ToolStatus[];
}

/* Mirrors `willie_proto::usage::{ProviderUsage, SessionUsage,
 * ProjectUsage, UsageSnapshot}`. `context_pct` is omitted (Rust's
 * `skip_serializing_if`) whenever the harness has not reported one yet —
 * always the case this cut, so the panel treats a missing value as "no
 * usage yet" rather than a 0% meter. */
export interface ProviderUsage {
  id: string;
  windows: string[];
}
export interface SessionUsage {
  id: string;
  tokens: number;
  context_pct?: number | null;
}
export interface ProjectUsage {
  id: string;
  tokens: number;
}
export interface UsageSnapshot {
  providers: ProviderUsage[];
  sessions: SessionUsage[];
  projects: ProjectUsage[];
  fetched_at: string;
}
