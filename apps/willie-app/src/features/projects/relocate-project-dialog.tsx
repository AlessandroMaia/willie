import { ProblemAlert } from "@/components/problem-alert";
import type { Problem } from "@/lib/ipc";
import type { Project } from "@/lib/proto";

interface RelocateProjectDialogProps {
  project: Project | null;
  path: string;
  problem: Problem | null;
  onPathChange: (value: string) => void;
  onBrowse: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RelocateProjectDialog({
  project,
  path,
  problem,
  onPathChange,
  onBrowse,
  onConfirm,
  onCancel,
}: RelocateProjectDialogProps) {
  if (!project) return null;
  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true">
        <h2>Relocate “{project.name}”</h2>
        <div className="actions">
          <input
            value={path}
            onChange={(e) => onPathChange(e.target.value)}
            placeholder="C:\github\..."
            aria-label="new source path"
          />
          <button type="button" onClick={onBrowse}>
            Browse…
          </button>
        </div>
        {problem && <ProblemAlert problem={problem} />}
        <div className="actions">
          <button
            type="button"
            disabled={path.trim() === ""}
            onClick={onConfirm}
          >
            Relocate
          </button>
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
