import {
  Code2Icon,
  FileTextIcon,
  FolderTreeIcon,
  TerminalIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { PanelFile } from "@/app/shell/panel-file";
import { PanelShell } from "@/app/shell/panel-shell";
import { PanelTree } from "@/app/shell/panel-tree";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { toast } from "@/components/ui/toast";
import {
  editorAvailable as editorAvailableApi,
  projects as projectsApi,
} from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useCurrentSystem } from "@/store/use-current-system";
import type { PanelTab } from "@/store/use-workspace-panel";
import { useWorkspacePanel } from "@/store/use-workspace-panel";

/**
 * The workspace beside whatever screen is open: its tree, the file
 * being read and the system's shell, in three tabs at the right edge
 * of the screen's card.
 *
 * It is mounted by the shell, never by a route — that is what puts it
 * on all four screens instead of inside the Session screen — and it is
 * a column of the card rather than a layer over it, so the screen
 * narrows instead of being covered. Every tab stays mounted once
 * visited: the tree keeps its cache and the shell keeps its terminal
 * across a switch.
 */
export function WorkspacePanel() {
  const { system } = useCurrentSystem();
  const { open, tab, branch, openAt, forSystem } = useWorkspacePanel();
  const [editorAvailable, setEditorAvailable] = useState(false);
  /* Opening the panel is not asking for a shell: the Shell body only
   * mounts once its tab has been picked, and then stays mounted so
   * the terminal survives a switch back to the tree. */
  const [shellVisited, setShellVisited] = useState(false);

  const systemId = system?.id ?? null;
  const lastSystemIdRef = useRef(systemId);

  useEffect(() => {
    editorAvailableApi()
      .then(setEditorAvailable)
      .catch(() => setEditorAvailable(false));
  }, []);

  /* The branch and the open file belong to one workspace, so the
   * store drops them the moment the system changes. A ref guard, not a
   * dependency array: exactly once per distinct system. */
  useEffect(() => {
    if (lastSystemIdRef.current === systemId) return;
    lastSystemIdRef.current = systemId;
    forSystem();
  });

  useEffect(() => {
    if (tab === "shell") setShellVisited(true);
  }, [tab]);

  function openWorkspaceInEditor(): void {
    if (!system) return;
    projectsApi.openInEditor(system.workspace).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open VS Code",
        description: asProblem(error).message,
      }),
    );
  }

  if (!open || !system) return null;

  return (
    <aside
      aria-label="Workspace panel"
      className="flex w-(--panel-width) shrink-0 flex-col border-l"
    >
      <Tabs
        value={tab}
        onValueChange={(value) => openAt(value as PanelTab)}
        className="flex min-h-0 flex-1 flex-col gap-0"
      >
        <div className="flex items-center gap-2 border-b px-2 py-1.5">
          <TabsList variant="line">
            <TabsTrigger value="tree" aria-label="Tree">
              <FolderTreeIcon />
            </TabsTrigger>
            <TabsTrigger value="file" aria-label="File">
              <FileTextIcon />
            </TabsTrigger>
            <TabsTrigger value="shell" aria-label="Shell">
              <TerminalIcon />
            </TabsTrigger>
          </TabsList>

          {branch && (
            <span className="ml-auto min-w-0 truncate text-muted-foreground text-xs">
              {branch}
            </span>
          )}

          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Open in VS Code"
            disabled={!editorAvailable}
            onClick={openWorkspaceInEditor}
            className={branch ? undefined : "ml-auto"}
          >
            <Code2Icon />
          </Button>
        </div>

        <TabsContent
          value="tree"
          keepMounted
          className="min-h-0 flex-1 overflow-y-auto p-2"
        >
          <PanelTree />
        </TabsContent>

        <TabsContent value="file" keepMounted className="min-h-0 flex-1">
          <PanelFile />
        </TabsContent>

        <TabsContent value="shell" keepMounted className="min-h-0 flex-1">
          {shellVisited && <PanelShell />}
        </TabsContent>
      </Tabs>
    </aside>
  );
}
