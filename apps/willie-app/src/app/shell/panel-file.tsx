import { Code2Icon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { FailureChip } from "@/components/failure-chip";
import { StatusBadge } from "@/components/status-badge";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import type { Problem } from "@/lib/ipc";
import {
  editorAvailable as editorAvailableApi,
  projects as projectsApi,
} from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import { useCurrentSystem } from "@/store/use-current-system";
import { useWorkspacePanel } from "@/store/use-workspace-panel";

/** Mirrors `crates/willied/src/workspace.rs`'s `READ_CAP` (512 KiB). */
const READ_CAP_KIB = 512;

/**
 * The workspace panel's File tab: a click on a file row in the Tree
 * opens it here through `projects.readFile` — never through anything
 * that could write it back, VS Code is the only path to a change.
 */
export function PanelFile() {
  const { system } = useCurrentSystem();
  const { file } = useWorkspacePanel();
  const [editorAvailable, setEditorAvailable] = useState(false);
  const [content, setContent] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);

  const systemId = system?.id ?? null;
  const workspace = system?.workspace ?? null;
  const hasFile = file !== null;

  /* Checked only once a file is actually open: the panel's own
   * workspace-level "Open in VS Code" already covers the
   * unconditional case for that action. */
  useEffect(() => {
    if (!hasFile) return;
    let cancelled = false;

    editorAvailableApi()
      .then((ok) => {
        if (!cancelled) setEditorAvailable(ok);
      })
      .catch(() => {
        if (!cancelled) setEditorAvailable(false);
      });

    return () => {
      cancelled = true;
    };
  }, [hasFile]);

  /* A ref guard on the pair, not the path alone: switching systems
   * updates `systemId` here a render before the panel's `forSystem()`
   * clears the file, which would otherwise re-read the OLD path
   * against the NEW system. */
  const openedForRef = useRef<{ systemId: string; file: string } | null>(null);

  useEffect(() => {
    if (file === null || systemId === null) {
      openedForRef.current = null;
      return;
    }

    const openedFor = openedForRef.current;
    if (
      openedFor &&
      openedFor.file === file &&
      openedFor.systemId !== systemId
    ) {
      return;
    }
    openedForRef.current = { systemId, file };

    let cancelled = false;
    setContent(null);
    setTruncated(false);
    setProblem(null);

    projectsApi
      .readFile(systemId, file)
      .then((result) => {
        if (cancelled) return;
        setContent(result.content);
        setTruncated(result.truncated);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setProblem(asProblem(error));
      });

    return () => {
      cancelled = true;
    };
  }, [file, systemId]);

  if (file === null) {
    return (
      <p className="px-3 py-6 text-center text-muted-foreground text-sm">
        Pick a file in the tree to read it here.
      </p>
    );
  }

  function openFileInEditor(): void {
    if (workspace === null || file === null) return;
    projectsApi.openInEditor(workspace, file).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open VS Code",
        description: asProblem(error).message,
      }),
    );
  }

  const lines = content === null ? [] : content.split("\n");

  return (
    <div data-slot="panel-file" className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-2 border-b px-2 py-1.5">
        <code className="min-w-0 flex-1 truncate font-mono text-xs">
          {file}
        </code>
        <StatusBadge tone="muted">read-only</StatusBadge>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Open file in VS Code"
          disabled={!editorAvailable}
          onClick={openFileInEditor}
        >
          <Code2Icon />
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {problem ? (
          <div className="p-3">
            <FailureChip
              code={problem.code}
              message={problem.message}
              remediation={problem.remediation}
            />
          </div>
        ) : (
          <>
            {truncated && (
              <p className="px-3 pt-2 text-muted-foreground text-xs">
                showing the first {READ_CAP_KIB} KiB
              </p>
            )}
            <pre className="flex px-3 py-2 text-sm">
              <span className="mr-3 select-none whitespace-pre text-right text-muted-foreground">
                {lines.map((_, index) => index + 1).join("\n")}
              </span>
              <code className="min-w-0 flex-1 whitespace-pre">{content}</code>
            </pre>
          </>
        )}
      </div>
    </div>
  );
}
