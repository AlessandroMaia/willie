import { EllipsisIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { RelocateProjectDialog } from "@/components/system-dialogs/relocate-project-dialog";
import { RemoveProjectDialog } from "@/components/system-dialogs/remove-project-dialog";
import { RenameSystemDialog } from "@/components/system-dialogs/rename-system-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { toast } from "@/components/ui/toast";
import type { Problem } from "@/lib/ipc";
import {
  dialogs,
  editorAvailable as editorAvailableApi,
  projects as projectsApi,
} from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useCurrentSystem } from "@/store/use-current-system";

/**
 * The "…" button beside the system selector: everything the old
 * per-project row menu offered, scoped to whichever system is current.
 * Rename/Relocate/Remove reuse the dialogs `features/projects` moved
 * into `components/system-dialogs` for exactly this; every other
 * action here is fire-and-forget, reported through a toast the same
 * way `ProjectsScreen`'s own successes are.
 */
export function SystemActionsMenu() {
  const { system } = useCurrentSystem();
  const [editorAvailable, setEditorAvailable] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [renameProblem, setRenameProblem] = useState<Problem | null>(null);
  const [relocating, setRelocating] = useState(false);
  const [relocatePath, setRelocatePath] = useState("");
  const [relocateProblem, setRelocateProblem] = useState<Problem | null>(null);
  const [removing, setRemoving] = useState(false);
  const [deleteWorkspace, setDeleteWorkspace] = useState(true);
  const [removeProblem, setRemoveProblem] = useState<Problem | null>(null);

  useEffect(() => {
    editorAvailableApi()
      .then(setEditorAvailable)
      .catch(() => setEditorAvailable(false));
  }, []);

  function openInExplorer(): void {
    if (!system) return;
    projectsApi.openInExplorer(system.workspace).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open Explorer",
        description: asProblem(error).message,
      }),
    );
  }

  function openInEditor(): void {
    if (!system) return;
    projectsApi.openInEditor(system.workspace).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open VS Code",
        description: asProblem(error).message,
      }),
    );
  }

  function syncFromWindows(): void {
    if (!system) return;
    projectsApi.updateFromWindows(system.id).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Update from Windows failed",
        description: asProblem(error).message,
      }),
    );
  }

  function openRenameDialog(): void {
    if (!system) return;
    setRenameValue(system.name);
    setRenameProblem(null);
    setRenaming(true);
  }

  async function confirmRename(): Promise<void> {
    if (!system) return;
    const name = renameValue.trim();
    if (name === "" || name === system.name) {
      setRenaming(false);
      return;
    }
    setRenameProblem(null);
    try {
      await projectsApi.rename(system.id, name);
      setRenaming(false);
    } catch (error) {
      setRenameProblem(asProblem(error));
    }
  }

  function openRelocateDialog(): void {
    if (!system) return;
    setRelocatePath(system.source);
    setRelocateProblem(null);
    setRelocating(true);
  }

  async function pickRelocateFolder(): Promise<void> {
    const picked = await dialogs.pickFolder();
    if (picked !== null) setRelocatePath(picked);
  }

  async function confirmRelocate(): Promise<void> {
    if (!system) return;
    const path = relocatePath.trim();
    if (path === "") return;
    setRelocateProblem(null);
    try {
      await projectsApi.relocate(system.id, path);
      setRelocating(false);
    } catch (error) {
      setRelocateProblem(asProblem(error));
    }
  }

  function openRemoveDialog(): void {
    if (!system) return;
    setDeleteWorkspace(true);
    setRemoveProblem(null);
    setRemoving(true);
  }

  async function confirmRemove(): Promise<void> {
    if (!system) return;
    setRemoveProblem(null);
    try {
      await projectsApi.remove(system.id, deleteWorkspace, false);
      setRemoving(false);
    } catch (error) {
      setRemoveProblem(asProblem(error));
    }
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="System actions"
              disabled={!system}
            />
          }
        >
          <EllipsisIcon />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem
            onClick={openInEditor}
            disabled={!editorAvailable}
            title={
              editorAvailable
                ? undefined
                : "VS Code was not found on this machine"
            }
          >
            Open in VS Code (WSL)
          </DropdownMenuItem>
          <DropdownMenuItem onClick={openInExplorer}>
            Open in Explorer
          </DropdownMenuItem>
          <DropdownMenuItem onClick={syncFromWindows}>
            Update from Windows
          </DropdownMenuItem>
          <DropdownMenuItem onClick={openRenameDialog}>Rename</DropdownMenuItem>
          <DropdownMenuItem onClick={openRelocateDialog}>
            Relocate
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onClick={openRemoveDialog}>
            Remove
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <RenameSystemDialog
        project={renaming ? system : null}
        name={renameValue}
        problem={renameProblem}
        onNameChange={setRenameValue}
        onConfirm={() => void confirmRename()}
        onCancel={() => setRenaming(false)}
      />

      <RelocateProjectDialog
        project={relocating ? system : null}
        path={relocatePath}
        problem={relocateProblem}
        onPathChange={setRelocatePath}
        onBrowse={() => void pickRelocateFolder()}
        onConfirm={() => void confirmRelocate()}
        onCancel={() => setRelocating(false)}
      />

      <RemoveProjectDialog
        project={removing ? system : null}
        deleteWorkspace={deleteWorkspace}
        problem={removeProblem}
        onToggleWorkspace={setDeleteWorkspace}
        onConfirm={() => void confirmRemove()}
        onCancel={() => setRemoving(false)}
      />
    </>
  );
}
