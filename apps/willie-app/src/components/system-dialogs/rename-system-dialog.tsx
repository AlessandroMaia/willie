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

interface RenameSystemDialogProps {
  project: Project | null;
  name: string;
  problem: Problem | null;
  onNameChange: (value: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

export function RenameSystemDialog({
  project,
  name,
  problem,
  onNameChange,
  onConfirm,
  onCancel,
}: RenameSystemDialogProps) {
  return (
    <Dialog
      open={project !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Rename “{project?.name}”</DialogTitle>
          <DialogDescription>
            Choose a new name for this system.
          </DialogDescription>
        </DialogHeader>

        <Field>
          <FieldLabel htmlFor="rename-system-name">Name</FieldLabel>
          <Input
            id="rename-system-name"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
          />
        </Field>

        {problem && <ProblemAlert problem={problem} />}

        <DialogFooter>
          <DialogClose render={<Button variant="ghost" />}>Cancel</DialogClose>
          <Button disabled={name.trim() === ""} onClick={onConfirm}>
            Rename
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
