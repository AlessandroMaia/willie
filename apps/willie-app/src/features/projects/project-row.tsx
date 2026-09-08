import { EllipsisIcon } from "lucide-react";
import { ProblemAlert } from "@/components/problem-alert";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  Item,
  ItemActions,
  ItemDescription,
  ItemTitle,
} from "@/components/ui/item";
import { Spinner } from "@/components/ui/spinner";
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
  live: number;
  onEditingNameChange: (value: string) => void;
  onStartRename: () => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onRetry: (job: Job) => void;
  onCopyPath: () => void;
  onOpenInExplorer: () => void;
  onOpenRelocateDialog: () => void;
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
  live,
  onEditingNameChange,
  onStartRename,
  onSaveRename,
  onCancelRename,
  onRetry,
  onCopyPath,
  onOpenInExplorer,
  onOpenRelocateDialog,
  onSyncToWindows,
  onUpdateFromWindows,
  onCancelJob,
  onOpenRemoveDialog,
}: ProjectRowProps) {
  const actionable = !isBusy && project.state.state === "ready" && !jobRunning;

  return (
    <Item variant="outline" className="flex-col items-stretch gap-2">
      <div className="flex flex-wrap items-center gap-2">
        {isEditing ? (
          <div className="flex items-center gap-1.5">
            <Input
              value={editingName}
              onChange={(e) => onEditingNameChange(e.target.value)}
              aria-label="project name"
              className="h-7 w-56"
            />
            <Button size="sm" onClick={onSaveRename}>
              Save
            </Button>
            <Button size="sm" variant="ghost" onClick={onCancelRename}>
              Cancel
            </Button>
          </div>
        ) : (
          <ItemTitle className="font-semibold">{project.name}</ItemTitle>
        )}

        <ProjectStateChip project={project} job={job} onRetry={onRetry} />

        {live > 0 && (
          <StatusBadge
            tone="ok"
            title={`${live} live session${live === 1 ? "" : "s"}`}
          >
            {live} live
          </StatusBadge>
        )}

        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`More actions for ${project.name}`}
                className="ml-auto"
              />
            }
          >
            <EllipsisIcon />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={onStartRename}>Rename</DropdownMenuItem>
            <DropdownMenuItem onClick={onCopyPath}>
              Copy workspace path
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onOpenInExplorer}>
              Open in Explorer
            </DropdownMenuItem>
            {!project.source_present && (
              <DropdownMenuItem onClick={onOpenRelocateDialog}>
                Relocate source…
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              variant="destructive"
              onClick={onOpenRemoveDialog}
            >
              Remove…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <ItemDescription className="flex flex-col gap-0.5">
        <span>source: {project.source}</span>
        <span>
          workspace: <code className="font-mono">{path}</code>
        </span>
        <span>branch: {project.branch}</span>
        {!project.source_present && (
          <span className="flex items-center gap-2">
            <StatusBadge tone="warning">source missing</StatusBadge>
            <Button size="xs" variant="outline" onClick={onOpenRelocateDialog}>
              Relocate
            </Button>
          </span>
        )}
      </ItemDescription>

      {rowProblem && <ProblemAlert problem={rowProblem} />}

      <ItemActions className="flex flex-wrap gap-2">
        <ButtonGroup>
          <Button
            size="sm"
            variant="outline"
            disabled={!actionable}
            onClick={onSyncToWindows}
          >
            Send to Windows
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={!actionable}
            onClick={onUpdateFromWindows}
          >
            Update from Windows
          </Button>
        </ButtonGroup>
        {job && jobRunning && (
          <Button size="sm" variant="ghost" onClick={() => onCancelJob(job)}>
            Cancel job
          </Button>
        )}
        {isBusy && <Spinner className="text-muted-foreground" />}
      </ItemActions>
    </Item>
  );
}
