import { FolderTreeIcon, PlusIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { RenameTab } from "@/features/session/rename-tab";
import { SessionsPanel } from "@/features/session/sessions-panel";
import type { Session } from "@/lib/proto";
import { cn } from "@/lib/utils";

interface SessionTabsProps {
  /** Every live session of the current system, agent sessions first —
   * the screen owns that ordering, this strip only renders it. */
  sessions: Session[];
  /** The current system's finished sessions, newest first — forwarded
   * straight to the Sessions panel; the strip itself never reads them
   * beyond that. */
  finished: Session[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNewSession: () => void;
  onNewZsh: () => void;
  onRename: (id: string, label: string | null) => void;
  onResume: (id: string) => void;
}

/**
 * One tab per live session: an agent tab carries a live dot and the
 * session's own name, and renames in place on a double-click; a shell
 * tab always reads "$ zsh" and never renames. The tree toggle is
 * still an inert placeholder; Task 14 wires it up. The Sessions panel
 * at the right end replaces Task 12's disabled placeholder.
 */
export function SessionTabs({
  sessions,
  finished,
  activeId,
  onSelect,
  onNewSession,
  onNewZsh,
  onRename,
  onResume,
}: SessionTabsProps) {
  return (
    <div
      role="tablist"
      className="flex items-center gap-1 overflow-x-auto border-b pb-1.5"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Workspace tree"
        disabled
      >
        <FolderTreeIcon />
      </Button>

      {sessions.map((session) => {
        const active = session.id === activeId;

        if (session.kind === "shell") {
          return (
            <button
              key={session.id}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => onSelect(session.id)}
              className={cn(
                "flex items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 py-1 text-sm",
                active
                  ? "bg-muted text-foreground"
                  : "text-muted-foreground hover:bg-muted/50",
              )}
            >
              $ zsh
            </button>
          );
        }

        return (
          <RenameTab
            key={session.id}
            session={session}
            active={active}
            onSelect={onSelect}
            onRename={onRename}
          />
        );
      })}

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button variant="ghost" size="icon-sm" aria-label="New tab" />
          }
        >
          <PlusIcon />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onClick={onNewSession}>
            New session
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onNewZsh}>New zsh</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <SessionsPanel
        live={sessions}
        finished={finished}
        onOpen={onSelect}
        onResume={onResume}
      />
    </div>
  );
}
