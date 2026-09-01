import { ProblemAlert } from "@/components/problem-alert";
import { ProjectStateChip } from "@/features/projects/project-state-chip";
import type { Problem } from "@/lib/ipc";
import type { Job, Project } from "@/lib/proto";

interface ProjectRowProps {
  project: Project;
  job: Job | undefined;
  isEditing: boolean;
  editingName: string;
  isBusy: boolean;
  jobRunning: boolean;
  path: string;
  rowProblem: Problem | null;
  openNotice: Problem | null;
  live: number;
  canResume: boolean;
  onEditingNameChange: (value: string) => void;
  onStartRename: () => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onRetry: (job: Job) => void;
  onCopyPath: () => void;
  onOpenInExplorer: () => void;
  onOpenRelocateDialog: () => void;
  onOpenSession: () => void;
  onResumeSession: () => void;
  onSyncToWindows: () => void;
  onUpdateFromWindows: () => void;
  onCancelJob: (job: Job) => void;
  onOpenRemoveDialog: () => void;
}

export function ProjectRow({
  project,
  job,
  isEditing,
  editingName,
  isBusy,
  jobRunning,
  path,
  rowProblem,
  openNotice,
  live,
  canResume,
  onEditingNameChange,
  onStartRename,
  onSaveRename,
  onCancelRename,
  onRetry,
  onCopyPath,
  onOpenInExplorer,
  onOpenRelocateDialog,
  onOpenSession,
  onResumeSession,
  onSyncToWindows,
  onUpdateFromWindows,
  onCancelJob,
  onOpenRemoveDialog,
}: ProjectRowProps) {
  return (
    <div className="project-row">
      <div className="project-row-main">
        {isEditing ? (
          <span className="rename">
            <input
              value={editingName}
              onChange={(e) => onEditingNameChange(e.target.value)}
              aria-label="project name"
            />
            <button type="button" onClick={onSaveRename}>
              Save
            </button>
            <button type="button" onClick={onCancelRename}>
              Cancel
            </button>
          </span>
        ) : (
          <span className="project-name">
            <strong>{project.name}</strong>
            <button type="button" onClick={onStartRename}>
              Rename
            </button>
          </span>
        )}
        <ProjectStateChip project={project} job={job} onRetry={onRetry} />
        {live > 0 && (
          <span
            className="badge badge-live"
            title={`${live} live session${live === 1 ? "" : "s"}`}
          >
            {live} live
          </span>
        )}
      </div>

      <div className="project-row-detail muted">
        <div>source: {project.source}</div>
        <div>
          workspace: <code>{path}</code>
          <button type="button" onClick={onCopyPath}>
            Copy
          </button>
          <button type="button" onClick={onOpenInExplorer}>
            Open in Explorer
          </button>
        </div>
        <div>branch: {project.branch}</div>
        {!project.source_present && (
          <div className="badge badge-warning">
            source missing
            <button type="button" onClick={onOpenRelocateDialog}>
              Relocate
            </button>
          </div>
        )}
      </div>

      {rowProblem && <ProblemAlert problem={rowProblem} />}

      {openNotice && <ProblemAlert problem={openNotice} tone="notice" />}

      <div className="actions">
        <button
          type="button"
          disabled={isBusy || project.state.state !== "ready" || jobRunning}
          onClick={onOpenSession}
        >
          Open session
        </button>
        <button
          type="button"
          disabled={
            isBusy ||
            project.state.state !== "ready" ||
            jobRunning ||
            !canResume
          }
          onClick={onResumeSession}
        >
          Resume
        </button>
        <button
          type="button"
          disabled={isBusy || project.state.state !== "ready" || jobRunning}
          onClick={onSyncToWindows}
        >
          Send to Windows
        </button>
        <button
          type="button"
          disabled={isBusy || project.state.state !== "ready" || jobRunning}
          onClick={onUpdateFromWindows}
        >
          Update from Windows
        </button>
        {job && jobRunning && (
          <button type="button" onClick={() => onCancelJob(job)}>
            Cancel
          </button>
        )}
        <button type="button" onClick={onOpenRemoveDialog}>
          Remove
        </button>
        {isBusy && <span className="muted">working…</span>}
      </div>
    </div>
  );
}
