import { ProblemAlert } from "@/components/problem-alert";
import type { Problem } from "@/lib/ipc";
import type { Project } from "@/lib/proto";

interface RemoveProjectDialogProps {
  project: Project | null;
  deleteWorkspace: boolean;
  problem: Problem | null;
  onToggleWorkspace: (checked: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RemoveProjectDialog({
  project,
  deleteWorkspace,
  problem,
  onToggleWorkspace,
  onConfirm,
  onCancel,
}: RemoveProjectDialogProps) {
  if (!project) return null;
  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true">
        <h2>Remove “{project.name}”?</h2>
        <label>
          <input
            type="checkbox"
            checked={deleteWorkspace}
            onChange={(e) => onToggleWorkspace(e.target.checked)}
          />
          Delete the workspace clone too
        </label>
        {problem && <ProblemAlert problem={problem} />}
        <p className="muted">
          A workspace with uncommitted changes is refused; if that happens the
          project's row will offer a one-click "Remove anyway" once the daemon
          reports it.
        </p>
        <div className="actions">
          <button type="button" onClick={onConfirm}>
            Remove
          </button>
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
