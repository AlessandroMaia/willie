import { FolderTreeIcon, PanelRightIcon, PlusIcon } from "lucide-react";
import { TONE_DOT } from "@/components/tone";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { sessionName } from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";
import { cn } from "@/lib/utils";

interface SessionTabsProps {
  /** Every live session of the current system, agent sessions first —
   * the screen owns that ordering, this strip only renders it. */
  sessions: Session[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNewSession: () => void;
  onNewZsh: () => void;
}

/**
 * One tab per live session: an agent tab carries a live dot and the
 * session's own name, a shell tab always reads "$ zsh" — renaming a
 * shell session never changes that label. The tree toggle and the
 * Sessions-panel button on either end are inert placeholders; Tasks 14
 * and 13 wire them up.
 */
export function SessionTabs({
  sessions,
  activeId,
  onSelect,
  onNewSession,
  onNewZsh,
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
        const isShell = session.kind === "shell";

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
            {isShell ? (
              "$ zsh"
            ) : (
              <>
                <span className={cn("size-1.5 rounded-full", TONE_DOT.ok)} />
                {sessionName(session)}
              </>
            )}
          </button>
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

      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Sessions"
        disabled
        className="ml-auto"
      >
        <PanelRightIcon />
      </Button>
    </div>
  );
}
