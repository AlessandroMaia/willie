import { ProblemAlert } from "@/components/problem-alert";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Field, FieldLabel } from "@/components/ui/field";
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

/* Confirm is a plain Button, not AlertDialogAction: the action closes
 * the dialog on click, and a synchronous rejection (project busy,
 * project gone) must stay visible inside it. */
export function RemoveProjectDialog({
  project,
  deleteWorkspace,
  problem,
  onToggleWorkspace,
  onConfirm,
  onCancel,
}: RemoveProjectDialogProps) {
  return (
    <AlertDialog
      open={project !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove “{project?.name}”?</AlertDialogTitle>
          <AlertDialogDescription>
            A workspace with uncommitted changes is refused; if that happens the
            project's row will offer a one-click "Remove anyway" once the daemon
            reports it.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <Field orientation="horizontal">
          <Checkbox
            id="remove-delete-workspace"
            checked={deleteWorkspace}
            onCheckedChange={(checked) => onToggleWorkspace(checked === true)}
          />
          <FieldLabel htmlFor="remove-delete-workspace">
            Delete the workspace clone too
          </FieldLabel>
        </Field>

        {problem && <ProblemAlert problem={problem} />}

        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <Button variant="destructive" onClick={onConfirm}>
            Remove
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
