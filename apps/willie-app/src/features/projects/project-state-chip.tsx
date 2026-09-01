import type { Job, JobKind, Project } from "@/lib/proto";

const RETRYABLE_KINDS: JobKind[] = ["sync_to_windows", "update_from_windows"];

interface ProjectStateChipProps {
  project: Project;
  job: Job | undefined;
  onRetry: (job: Job) => void;
}

export function ProjectStateChip({
  project,
  job,
  onRetry,
}: ProjectStateChipProps) {
  if (project.state.state === "failed") {
    return (
      <div className="chip chip-failed">
        <code>{project.state.code}</code>
        <span>{project.state.message}</span>
        {project.state.remediation && (
          <div className="muted">→ {project.state.remediation}</div>
        )}
      </div>
    );
  }
  if (job && job.state.state === "failed") {
    /* `workspace_dirty` only ever arrives this way: `project_remove`
     * resolves the instant the job is queued, so the dirty-workspace
     * refusal is never a promise rejection the confirm dialog can
     * catch — it is a `job_changed` event, exactly like any other job
     * outcome. The one-click "Remove anyway" re-submits with `force`,
     * the same safe-resubmit shape as a plain retry. */
    const forceRemove =
      job.kind === "remove" && job.state.code === "workspace_dirty";
    const canRetry = forceRemove || RETRYABLE_KINDS.includes(job.kind);
    return (
      <div className="chip chip-failed">
        <code>{job.state.code}</code>
        <span>{job.state.message}</span>
        {job.state.remediation && (
          <div className="muted">→ {job.state.remediation}</div>
        )}
        {canRetry && (
          <button type="button" onClick={() => onRetry(job)}>
            {forceRemove ? "Remove anyway" : "Retry"}
          </button>
        )}
      </div>
    );
  }
  if (project.state.state === "preparing" || job?.state.state === "running") {
    return (
      <div className="chip chip-busy">
        <span className="spinner" aria-hidden="true" />
        <span>{job?.log_tail || "working…"}</span>
      </div>
    );
  }
  return <span className="chip chip-ready">ready</span>;
}
