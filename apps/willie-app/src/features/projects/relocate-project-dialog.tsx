import { ProblemAlert } from "@/components/problem-alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
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
  return (
    <Dialog
      open={project !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Relocate “{project?.name}”</DialogTitle>
          <DialogDescription>
            Point the project at the folder its source moved to.
          </DialogDescription>
        </DialogHeader>

        <Field>
          <FieldLabel htmlFor="relocate-path">New source path</FieldLabel>
          <div className="flex gap-2">
            <Input
              id="relocate-path"
              value={path}
              onChange={(e) => onPathChange(e.target.value)}
              placeholder="C:\github\..."
            />
            <Button variant="outline" onClick={onBrowse}>
              Browse…
            </Button>
          </div>
        </Field>

        {problem && <ProblemAlert problem={problem} />}

        <DialogFooter>
          <DialogClose render={<Button variant="ghost" />}>Cancel</DialogClose>
          <Button disabled={path.trim() === ""} onClick={onConfirm}>
            Relocate
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
