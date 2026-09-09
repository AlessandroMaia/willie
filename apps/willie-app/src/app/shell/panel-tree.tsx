import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { FailureChip } from "@/components/failure-chip";
import { ProblemAlert } from "@/components/problem-alert";
import { TONE_TEXT } from "@/components/tone";
import { Spinner } from "@/components/ui/spinner";
import { joinPath, sortEntries } from "@/lib/domain/tree";
import type { Problem } from "@/lib/ipc";
import { projects as projectsApi } from "@/lib/ipc";
import { asProblem } from "@/lib/problem";
import type { TreeEntry } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { useWorkspacePanel } from "@/store/use-workspace-panel";

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
 * The workspace panel's Tree tab. Its root loads once the tab is
 * mounted and every folder's children fetch lazily on first expand,
 * then stay cached for as long as the panel stays open — collapsing a
 * folder never re-fetches it, and a failed subfolder gets its own
 * inline `FailureChip` rather than blanking the rest of an otherwise
 * healthy tree.
 */
export function PanelTree() {
  const { system } = useCurrentSystem();
  const { setBranch, openFile } = useWorkspacePanel();
  const [root, setRoot] = useState<TreeEntry[] | null>(null);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [cache, setCache] = useState<Map<string, TreeEntry[]>>(new Map());
  const [loading, setLoading] = useState<Set<string>>(new Set());
  const [nodeProblems, setNodeProblems] = useState<Map<string, Problem>>(
    new Map(),
  );

  const systemId = system?.id ?? null;
  const lastSystemIdRef = useRef(systemId);

  /* A new system invalidates every cached path and the tree read
   * against the system just left — never a stale listing under the
   * next system's name. A ref guard instead of a dependency array:
   * this must fire exactly once per distinct `systemId`. */
  useEffect(() => {
    if (lastSystemIdRef.current === systemId) return;
    lastSystemIdRef.current = systemId;
    setRoot(null);
    setProblem(null);
    setExpanded(new Set());
    setCache(new Map());
    setLoading(new Set());
    setNodeProblems(new Map());
  });

  useEffect(() => {
    if (systemId === null || root !== null) return;
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
  }, [systemId, root, setBranch]);

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

  if (problem) return <ProblemAlert problem={problem} />;
  if (root === null) return <Spinner />;

  return (
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
  );
}
