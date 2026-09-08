import { Code2Icon, Maximize2Icon, Minimize2Icon, XIcon } from "lucide-react";
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
import { cn } from "@/lib/utils";
import { useCurrentSystem } from "@/store/use-current-system";
import { useFilePreview } from "@/store/use-file-preview";
import { useTreeDrawer } from "@/store/use-tree-drawer";

/** Mirrors `crates/willied/src/workspace.rs`'s `READ_CAP` (512 KiB). */
const READ_CAP_KIB = 512;

/**
 * The read-only file preview beside the tree drawer: a click on a
 * file row opens it here through `projects.readFile` — never through
 * anything that could write it back, VS Code is the only path to a
 * change. Mounted next to the drawer in the shell, sharing its
 * containing block, so its left edge sits flush against the tree's
 * right edge while the drawer is open (`| tree | file |`), at the
 * centre's own left edge while it is closed, and fills everything
 * right of the tree once expanded.
 */
export function FilePreview() {
  const { system } = useCurrentSystem();
  const { preview, toggleExpanded, close } = useFilePreview();
  const { open: drawerOpen } = useTreeDrawer();
  const [editorAvailable, setEditorAvailable] = useState(false);
  const [content, setContent] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);

  const systemId = system?.id ?? null;
  const workspace = system?.workspace ?? null;
  const path = preview?.path ?? null;
  const hasPreview = preview !== null;

  /* Checked only once a preview is actually open, not on every mount
   * of this always-present component — the workspace-level "Open in
   * VS Code" button on the drawer already covers the unconditional
   * case for that action. */
  useEffect(() => {
    if (!hasPreview) return;
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
  }, [hasPreview]);

  /* Depends only on the path and the system id, never on `expanded` —
   * toggling the width must never re-read the file. A ref instead of
   * relying on the drawer's own reset alone: switching systems updates
   * `systemId` here a render before the drawer's `closePreview()`
   * lands, which would otherwise re-fire this effect with the OLD
   * path against the NEW system. When the path is unchanged but the
   * system it was opened under is not the current one, skip the read
   * — the drawer closes this preview right after anyway. */
  const openedForRef = useRef<{ systemId: string; path: string } | null>(null);

  useEffect(() => {
    if (path === null || systemId === null) {
      openedForRef.current = null;
      return;
    }
    const openedFor = openedForRef.current;
    if (
      openedFor &&
      openedFor.path === path &&
      openedFor.systemId !== systemId
    ) {
      return;
    }
    openedForRef.current = { systemId, path };

    let cancelled = false;
    setContent(null);
    setTruncated(false);
    setProblem(null);
    projectsApi
      .readFile(systemId, path)
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
  }, [path, systemId]);

  if (!preview) return null;

  /* Narrowed once here: a nested function declaration does not keep
   * the `if (!preview)` guard's narrowing across its own closure. */
  const previewPath = preview.path;

  function openFileInEditor(): void {
    if (workspace === null) return;
    projectsApi.openInEditor(workspace, previewPath).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open VS Code",
        description: asProblem(error).message,
      }),
    );
  }

  const lines = content === null ? [] : content.split("\n");

  return (
    <div
      data-slot="file-preview"
      style={{ left: drawerOpen ? "var(--tree-width)" : "0" }}
      className={cn(
        "absolute inset-y-0 z-10 flex flex-col border-l bg-card shadow-lg transition-[left] duration-200 ease-linear",
        preview.expanded ? "right-0" : "w-[36rem] max-w-full",
      )}
    >
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <code className="min-w-0 flex-1 truncate font-mono text-sm">
          {preview.path}
        </code>
        <StatusBadge tone="muted">read-only</StatusBadge>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={preview.expanded ? "Collapse preview" : "Expand preview"}
          onClick={toggleExpanded}
        >
          {preview.expanded ? <Minimize2Icon /> : <Maximize2Icon />}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Open in VS Code"
          disabled={!editorAvailable}
          onClick={openFileInEditor}
        >
          <Code2Icon />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Close preview"
          onClick={close}
        >
          <XIcon />
        </Button>
      </div>

      <div className="flex-1 overflow-auto">
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
