import { CheckIcon, ChevronsUpDownIcon, PlusIcon } from "lucide-react";
import { useMemo, useState } from "react";
import {
  SystemActionDialogs,
  SystemActionRows,
} from "@/app/shell/system-actions";
import { StatusDot } from "@/components/status-dot";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { useSidebar } from "@/components/ui/sidebar";
import { isLive } from "@/lib/domain/sessions";
import { glyphFor } from "@/lib/domain/systems";
import type { Project, Session } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { useEngineStatus } from "@/store/use-engine-status";
import { useSetupDrawer } from "@/store/use-setup-drawer";
import { useSnapshot } from "@/store/use-snapshot";
import { useWorkspacePanel } from "@/store/use-workspace-panel";

function isProjectLive(sessions: Session[], projectId: string): boolean {
  return sessions.some((s) => s.project_id === projectId && isLive(s));
}

/**
 * The current system, up top in the sidebar: glyph, name, and its
 * workspace path with the branch. The branch is the daemon's own
 * `Project.branch`, read as of the last snapshot; the panel tree's
 * live root load overrides it while it is open, since that one is read
 * at the moment the tree was listed. Its menu is a searchable list of
 * every system, each with the same live dot the trigger shows for the
 * current one, "Add system…" which hands off to the setup drawer's
 * Systems section rather than adding one itself, and the system's own
 * actions under a separator — the sidebar header is one button,
 * because collapsed it is one 48px target.
 */
export function SystemSelector() {
  const { system, setSystem, loading } = useCurrentSystem();
  const { branch: panelBranch } = useWorkspacePanel();
  const { status } = useEngineStatus();
  const daemonRunning = status?.daemon.state === "running";
  const { snapshot } = useSnapshot(daemonRunning);
  const { state: sidebarState } = useSidebar();
  const { openAt } = useSetupDrawer();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const projects = snapshot?.projects ?? [];
  const sessions = snapshot?.sessions ?? [];
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return projects;
    return projects.filter((p) => p.name.toLowerCase().includes(needle));
  }, [projects, query]);

  function choose(project: Project): void {
    setSystem(project.id);
    setOpen(false);
    setQuery("");
  }

  function addSystem(): void {
    setOpen(false);
    openAt("systems");
  }

  const collapsed = sidebarState === "collapsed";
  const currentLive = system !== null && isProjectLive(sessions, system.id);
  const branch = panelBranch ?? system?.branch ?? null;

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setQuery("");
      }}
    >
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            aria-label="Switch system"
            className="h-10 min-w-0 flex-1 justify-start gap-2 px-2 group-data-[collapsible=icon]:size-8 group-data-[collapsible=icon]:flex-none group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0"
          />
        }
      >
        <span className="relative flex size-6 shrink-0 items-center justify-center rounded-md bg-sidebar-accent font-medium text-xs">
          {system ? glyphFor(system.name) : "–"}
          {/* Collapsed there is no room beside the glyph: a dot laid out
           * in the row would be pushed past the rail's edge, onto the
           * screen behind it. It rides the glyph's corner instead. */}
          {collapsed && currentLive && (
            <StatusDot
              tone="ok"
              label="live"
              className="absolute -top-0.5 -right-0.5 ring-2 ring-sidebar"
            />
          )}
        </span>
        {!collapsed && (
          <span className="flex min-w-0 flex-1 flex-col text-left">
            {/* "No system" is a fact about the registry, so it may not
             * be shown while the saved preference is still being read
             * — that reads as "you have none" for one paint. */}
            <span className="truncate font-medium text-sm">
              {system?.name ?? (loading ? "Loading…" : "No system")}
            </span>
            {system && (
              <span className="truncate text-muted-foreground text-xs">
                {system.workspace}
                {branch && ` · ${branch}`}
              </span>
            )}
          </span>
        )}
        {!collapsed && currentLive && <StatusDot tone="ok" label="live" />}
        {!collapsed && (
          <ChevronsUpDownIcon className="size-4 shrink-0 text-muted-foreground" />
        )}
      </PopoverTrigger>

      <PopoverContent align="start" className="w-64 gap-1.5 p-1.5">
        <Input
          autoFocus
          placeholder="Find a system…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />

        <div className="flex max-h-64 flex-col gap-0.5 overflow-y-auto">
          {filtered.map((project) => {
            const live = isProjectLive(sessions, project.id);
            return (
              <Button
                key={project.id}
                variant="ghost"
                className="justify-start gap-2 px-2"
                onClick={() => choose(project)}
              >
                <StatusDot
                  tone={live ? "ok" : "muted"}
                  label={live ? "live" : "not running"}
                />
                <span className="truncate">{project.name}</span>
                {project.branch && (
                  <span className="truncate text-muted-foreground text-xs">
                    {project.branch}
                  </span>
                )}
                {system?.id === project.id && (
                  <CheckIcon className="ml-auto size-4" />
                )}
              </Button>
            );
          })}
          {filtered.length === 0 && (
            <p className="px-2 py-1.5 text-muted-foreground text-xs">
              No systems match “{query}”.
            </p>
          )}
        </div>

        <Button
          variant="ghost"
          className="justify-start gap-2 px-2"
          onClick={addSystem}
        >
          <PlusIcon className="size-4" />
          Add system…
        </Button>

        <Separator className="my-0.5" />

        <SystemActionRows />
      </PopoverContent>

      {/* Outside the popover on purpose: it unmounts its content when
       * it closes, and a confirmation raised from a row above has to
       * outlive the row that raised it. */}
      <SystemActionDialogs />
    </Popover>
  );
}
