import { useEffect, useState } from "react";
import { RelocateProjectDialog } from "@/components/system-dialogs/relocate-project-dialog";
import { RemoveProjectDialog } from "@/components/system-dialogs/remove-project-dialog";
import { RenameSystemDialog } from "@/components/system-dialogs/rename-system-dialog";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import type { Problem } from "@/lib/ipc";
import {
  dialogs,
  editorAvailable as editorAvailableApi,
  projects as projectsApi,
} from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useCurrentSystem } from "@/store/use-current-system";
import { useSystemActions } from "@/store/use-system-actions";

/** The rows share the selector's own row shape, so switching system
 * and acting on it read as one list. */
const ROW = "justify-start gap-2 px-2 font-normal";

/**
 * Everything the old per-project row menu offered, scoped to whichever
 * system is current, as rows inside the system selector's menu. The
 * three fire-and-forget commands report through a toast the same way
 * `ProjectsScreen`'s own successes do; the three that need a
 * confirmation raise it through `useSystemActions`, because this list
 * unmounts with the popover the moment one is clicked.
 */
export function SystemActionRows() {
  const { system } = useCurrentSystem();
  const { open } = useSystemActions();
  const [editorAvailable, setEditorAvailable] = useState(false);

  useEffect(() => {
    editorAvailableApi()
      .then(setEditorAvailable)
      .catch(() => setEditorAvailable(false));
  }, []);

  function report(title: string, error: unknown): void {
    toast.add({
      type: "error",
      title,
      description: asProblem(error).message,
    });
  }

  function openInExplorer(): void {
    if (!system) return;
    projectsApi
      .openInExplorer(system.workspace)
      .catch((error: unknown) => report("Could not open Explorer", error));
  }

  function openInEditor(): void {
    if (!system) return;
    projectsApi
      .openInEditor(system.workspace)
      .catch((error: unknown) => report("Could not open VS Code", error));
  }

  function syncFromWindows(): void {
    if (!system) return;
    projectsApi
      .updateFromWindows(system.id)
      .catch((error: unknown) => report("Update from Windows failed", error));
  }

  return (
    <>
      <Button
        variant="ghost"
        className={ROW}
        onClick={openInEditor}
        disabled={!system || !editorAvailable}
        title={
          editorAvailable ? undefined : "VS Code was not found on this machine"
        }
      >
        Open in VS Code (WSL)
      </Button>

      <Button
        variant="ghost"
        className={ROW}
        onClick={openInExplorer}
        disabled={!system}
      >
        Open in Explorer
      </Button>

      <Button
        variant="ghost"
        className={ROW}
        onClick={syncFromWindows}
        disabled={!system}
      >
        Update from Windows
      </Button>

      <Button
        variant="ghost"
        className={ROW}
        onClick={() => open("rename")}
        disabled={!system}
      >
        Rename
      </Button>

      <Button
        variant="ghost"
        className={ROW}
        onClick={() => open("relocate")}
        disabled={!system}
      >
        Relocate
      </Button>

      <Button
        variant="ghost"
        className={`${ROW} text-destructive hover:text-destructive`}
        onClick={() => open("remove")}
        disabled={!system}
      >
        Remove
      </Button>
    </>
  );
}

/**
 * The three confirmations the rows above raise. Mounted outside the
 * selector's popover, which unmounts its own content on close: a
 * dialog opened from a row has to outlive the row that opened it.
 */
export function SystemActionDialogs() {
  const { system } = useCurrentSystem();
  const { dialog, close } = useSystemActions();
  const [renameValue, setRenameValue] = useState("");
  const [renameProblem, setRenameProblem] = useState<Problem | null>(null);
  const [relocatePath, setRelocatePath] = useState("");
  const [relocateProblem, setRelocateProblem] = useState<Problem | null>(null);
  const [deleteWorkspace, setDeleteWorkspace] = useState(true);
  const [removeProblem, setRemoveProblem] = useState<Problem | null>(null);

  const systemId = system?.id;

  /* Every dialog here acts on whichever system is current when its
   * Confirm is pressed, and Remove deletes the workspace by default.
   * The current system can re-resolve underneath an open dialog — a
   * removal by another client makes `resolveCurrent` fall back to the
   * first project — so a system change closes all three rather than
   * silently retargeting a confirmation the user already read. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: reacts to a system change on purpose — the body has nothing left to read once the dialogs are closed
  useEffect(() => {
    close();
  }, [systemId]);

  /* Each dialog opens on the value the system carries right now, not on
   * whatever the last one left behind. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: seeds the field for the dialog that just opened
  useEffect(() => {
    if (!system) return;
    if (dialog === "rename") {
      setRenameValue(system.name);
      setRenameProblem(null);
    }
    if (dialog === "relocate") {
      setRelocatePath(system.source);
      setRelocateProblem(null);
    }
    if (dialog === "remove") {
      setDeleteWorkspace(true);
      setRemoveProblem(null);
    }
  }, [dialog]);

  async function confirmRename(): Promise<void> {
    if (!system) return;
    const name = renameValue.trim();
    if (name === "" || name === system.name) {
      close();
      return;
    }
    setRenameProblem(null);
    try {
      await projectsApi.rename(system.id, name);
      close();
    } catch (error) {
      setRenameProblem(asProblem(error));
    }
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
      close();
    } catch (error) {
      setRelocateProblem(asProblem(error));
    }
  }

  async function confirmRemove(): Promise<void> {
    if (!system) return;
    setRemoveProblem(null);
    try {
      await projectsApi.remove(system.id, deleteWorkspace, false);
      close();
    } catch (error) {
      setRemoveProblem(asProblem(error));
    }
  }

  return (
    <>
      <RenameSystemDialog
        key={`rename-${systemId}`}
        project={dialog === "rename" ? system : null}
        name={renameValue}
        problem={renameProblem}
        onNameChange={setRenameValue}
        onConfirm={() => void confirmRename()}
        onCancel={close}
      />

      <RelocateProjectDialog
        key={`relocate-${systemId}`}
        project={dialog === "relocate" ? system : null}
        path={relocatePath}
        problem={relocateProblem}
        onPathChange={setRelocatePath}
        onBrowse={() => void pickRelocateFolder()}
        onConfirm={() => void confirmRelocate()}
        onCancel={close}
      />

      <RemoveProjectDialog
        key={`remove-${systemId}`}
        project={dialog === "remove" ? system : null}
        deleteWorkspace={deleteWorkspace}
        problem={removeProblem}
        onToggleWorkspace={setDeleteWorkspace}
        onConfirm={() => void confirmRemove()}
        onCancel={close}
      />
    </>
  );
}
