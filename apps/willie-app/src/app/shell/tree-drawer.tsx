import { ChevronDownIcon, ChevronRightIcon, Code2Icon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { FailureChip } from "@/components/failure-chip";
import { ProblemAlert } from "@/components/problem-alert";
import { TONE_TEXT } from "@/components/tone";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "@/components/ui/toast";
import { joinPath, sortEntries } from "@/lib/domain/tree";
import type { Problem } from "@/lib/ipc";
import {
  editorAvailable as editorAvailableApi,
  projects as projectsApi,
} from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { TreeEntry } from "@/lib/proto";
import { cn } from "@/lib/utils";
import { useCurrentSystem } from "@/store/use-current-system";
import { useFilePreview } from "@/store/use-file-preview";
import { useTreeDrawer } from "@/store/use-tree-drawer";

interface TreeRowsProps {
  entries: TreeEntry[];
  parentPath: string;
  depth: number;
  expanded: Set<string>;
  cache: Map<string, TreeEntry[]>;
  loading: Set<string>;
  /** A failed expand for one folder, keyed by its own path — never
   * the shared root `problem`, so one bad subfolder never wipes the
   * rest of an otherwise healthy tree. */
  nodeProblems: Map<string, Problem>;
  onToggleFolder: (path: string) => void;
  onOpenFile: (path: string) => void;
}

/** One row per entry, dirs first and case-insensitive within a kind
 * (`sortEntries`); a directory's children render right below it once
 * its cache entry lands, indented one step further. */
function TreeRows({
  entries,
  parentPath,
  depth,
  expanded,
  cache,
  loading,
  nodeProblems,
  onToggleFolder,
  onOpenFile,
}: TreeRowsProps) {
  return (
    <ul className="flex flex-col gap-0.5">
      {sortEntries(entries).map((entry) => {
        const path = joinPath(parentPath, entry.name);
        const isDir = entry.kind === "dir";
        const isExpanded = expanded.has(path);
        const children = cache.get(path);
        const nodeProblem = nodeProblems.get(path);

        return (
          <li key={path}>
            <button
              type="button"
              style={{ paddingLeft: `${depth * 0.875}rem` }}
              className="flex w-full items-center gap-1.5 rounded px-1.5 py-1 text-left text-sm hover:bg-muted/50"
              onClick={() => (isDir ? onToggleFolder(path) : onOpenFile(path))}
            >
              {isDir ? (
                isExpanded ? (
                  <ChevronDownIcon className="size-3.5 shrink-0 text-muted-foreground" />
                ) : (
                  <ChevronRightIcon className="size-3.5 shrink-0 text-muted-foreground" />
                )
              ) : (
                <span className="size-3.5 shrink-0" />
              )}
              <span className="min-w-0 flex-1 truncate">{entry.name}</span>
              {entry.git && (
                <span className={`font-mono text-xs ${TONE_TEXT.warning}`}>
                  {entry.git}
                </span>
              )}
            </button>

            {isDir && isExpanded && (
              <div>
                {loading.has(path) ? (
                  <Spinner className="ml-4" />
                ) : nodeProblem ? (
                  <div className="pl-4">
                    <FailureChip
                      code={nodeProblem.code}
                      message={nodeProblem.message}
                      remediation={nodeProblem.remediation}
                    />
                  </div>
                ) : (
                  children && (
                    <TreeRows
                      entries={children}
                      parentPath={path}
                      depth={depth + 1}
                      expanded={expanded}
                      cache={cache}
                      loading={loading}
                      nodeProblems={nodeProblems}
                      onToggleFolder={onToggleFolder}
                      onOpenFile={onOpenFile}
                    />
                  )
                )}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * The workspace tree as a lateral drawer: always mounted inside the
 * `relative` `SidebarInset` (its containing block), `left: 0` there so
 * it hugs the sidebar's true current edge regardless of expanded or
 * icon-collapsed width — never a sidebar-width token of its own — and
 * slid fully into view or fully parked behind the sidebar with a
 * `translate-x` transition. Its root loads the moment it opens and
 * every folder's children fetch lazily on first expand, then stay
 * cached for as long as this component stays mounted — collapsing a
 * folder never re-fetches it, and a failed subfolder gets its own
 * inline `FailureChip` rather than blanking the rest of an otherwise
 * healthy tree. The single Escape listener here owns the whole shell's
 * close order: the file preview first, this drawer only once nothing
 * else is covering it — never two competing listeners racing the same
 * keydown.
 */
export function TreeDrawer() {
  const { system } = useCurrentSystem();
  const { open, branch, setBranch, close: closeDrawer } = useTreeDrawer();
  const { preview, openFile, close: closePreview } = useFilePreview();
  const [root, setRoot] = useState<TreeEntry[] | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [cache, setCache] = useState<Map<string, TreeEntry[]>>(new Map());
  const [loading, setLoading] = useState<Set<string>>(new Set());
  const [nodeProblems, setNodeProblems] = useState<Map<string, Problem>>(
    new Map(),
  );
  const [editorAvailable, setEditorAvailable] = useState(false);

  const systemId = system?.id ?? null;
  const workspace = system?.workspace ?? null;
  const lastSystemIdRef = useRef(systemId);

  useEffect(() => {
    editorAvailableApi()
      .then(setEditorAvailable)
      .catch(() => setEditorAvailable(false));
  }, []);

  /* A new system invalidates every cached path, the branch shown in
   * the header and the selector, and any file the preview had open
   * against the system just left — never a stale tree or a stale read
   * left over from it. A ref guard instead of a dependency array: this
   * must fire exactly once per distinct `systemId`, whether the drawer
   * is open or not, not merely whenever some other dependency happens
   * to change. */
  useEffect(() => {
    if (lastSystemIdRef.current === systemId) return;
    lastSystemIdRef.current = systemId;
    setRoot(null);
    setProblem(null);
    setExpanded(new Set());
    setCache(new Map());
    setLoading(new Set());
    setNodeProblems(new Map());
    setBranch(null);
    closePreview();
  });

  useEffect(() => {
    if (!open || systemId === null || root !== null) return;
    let cancelled = false;
    projectsApi
      .tree(systemId)
      .then((result) => {
        if (cancelled) return;
        setRoot(result.entries);
        setBranch(result.branch ?? null);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setProblem(asProblem(error));
      });
    return () => {
      cancelled = true;
    };
  }, [open, systemId, root, setBranch]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key !== "Escape") return;
      if (preview) {
        closePreview();
      } else if (open) {
        closeDrawer();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [preview, open, closePreview, closeDrawer]);

  function toggleFolder(path: string): void {
    const nextExpanded = new Set(expanded);
    if (nextExpanded.has(path)) {
      nextExpanded.delete(path);
      setExpanded(nextExpanded);
      return;
    }
    nextExpanded.add(path);
    setExpanded(nextExpanded);

    if (cache.has(path) || systemId === null) return;
    setLoading((prev) => new Set(prev).add(path));
    setNodeProblems((prev) => {
      if (!prev.has(path)) return prev;
      const next = new Map(prev);
      next.delete(path);
      return next;
    });
    projectsApi
      .tree(systemId, path)
      .then((result) => {
        setCache((prev) => new Map(prev).set(path, result.entries));
      })
      .catch((error: unknown) => {
        setNodeProblems((prev) => new Map(prev).set(path, asProblem(error)));
      })
      .finally(() => {
        setLoading((prev) => {
          const next = new Set(prev);
          next.delete(path);
          return next;
        });
      });
  }

  function openWorkspaceInEditor(): void {
    if (workspace === null) return;
    projectsApi.openInEditor(workspace).catch((error: unknown) =>
      toast.add({
        type: "error",
        title: "Could not open VS Code",
        description: asProblem(error).message,
      }),
    );
  }

  return (
    <div
      data-slot="tree-drawer"
      aria-hidden={!open}
      /* Parked off-screen is not gone: without `inert` every row and
       * the editor button stay in the tab order while the drawer is
       * closed, and `aria-hidden` over focusable content is itself the
       * violation. */
      inert={!open}
      className={cn(
        "absolute inset-y-0 left-0 z-10 flex w-(--tree-width) flex-col border-r bg-sidebar shadow-lg transition-transform duration-200 ease-linear",
        open ? "translate-x-0" : "-translate-x-full",
      )}
    >
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="flex-1 truncate font-medium text-sm">
          Tree
          {branch && (
            <span className="ml-1.5 font-normal text-muted-foreground text-xs">
              {branch}
            </span>
          )}
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Open in VS Code"
          disabled={!editorAvailable}
          onClick={openWorkspaceInEditor}
        >
          <Code2Icon />
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {problem ? (
          <ProblemAlert problem={problem} />
        ) : root === null ? (
          <Spinner />
        ) : (
          <TreeRows
            entries={root}
            parentPath=""
            depth={0}
            expanded={expanded}
            cache={cache}
            loading={loading}
            nodeProblems={nodeProblems}
            onToggleFolder={toggleFolder}
            onOpenFile={openFile}
          />
        )}
      </div>
    </div>
  );
}
