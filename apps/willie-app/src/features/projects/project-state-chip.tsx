import { FailureChip } from "@/components/failure-chip";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
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
      <FailureChip
        code={project.state.code}
        message={project.state.message}
        remediation={project.state.remediation}
      />
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
      <FailureChip
        code={job.state.code}
        message={job.state.message}
        remediation={job.state.remediation}
      >
        {canRetry && (
          <Button size="xs" variant="outline" onClick={() => onRetry(job)}>
            {forceRemove ? "Remove anyway" : "Retry"}
          </Button>
        )}
      </FailureChip>
    );
  }

  if (project.state.state === "preparing" || job?.state.state === "running") {
    return (
      <StatusBadge tone="pending">
        <Spinner className="size-3" />
        {job?.log_tail || "working…"}
      </StatusBadge>
    );
  }

  return <StatusBadge tone="ok">ready</StatusBadge>;
}
