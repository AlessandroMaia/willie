import { PanelRightIcon } from "lucide-react";
import { useState } from "react";
import { relativeTime } from "@/components/relative-time";
import { TONE_DOT } from "@/components/tone";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { latestResumable, sessionName } from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";
import { cn } from "@/lib/utils";

interface SessionsPanelProps {
  /** The current system's live sessions, tab-strip order (agents
   * first, then shells) — the same list `SessionTabs` renders. */
  live: Session[];
  /** The current system's finished sessions, newest first. */
  finished: Session[];
  /** Selects a live session's tab — the same handler a tab's own click
   * uses, so "Open" is never a second way to pick a tab. */
  onOpen: (id: string) => void;
  /** Starts a new session continuing a finished one. The Session
   * screen's own "arrived" rule focuses it once it lands in the
   * snapshot; this panel never polls for it. */
  onResume: (id: string) => void;
}

/**
 * The current system's sessions, live then finished, in a side sheet
 * off its own trigger at the tab strip's right end. Opening or
 * resuming a row closes the sheet, since either one is about to bring
 * a tab into view behind it. Only one finished row can be resumed —
 * see `latestResumable`: the harness continues the workspace's most
 * recent conversation, so a Resume anywhere else would reopen a
 * different session than the one it names.
 */
export function SessionsPanel({
  live,
  finished,
  onOpen,
  onResume,
}: SessionsPanelProps) {
  const [sheetOpen, setSheetOpen] = useState(false);

  const resumable = latestResumable(finished);

  function openAndClose(id: string): void {
    onOpen(id);
    setSheetOpen(false);
  }

  function resumeAndClose(id: string): void {
    onResume(id);
    setSheetOpen(false);
  }

  return (
    <Sheet open={sheetOpen} onOpenChange={setSheetOpen}>
      <SheetTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Sessions"
            className="ml-auto"
          />
        }
      >
        <PanelRightIcon />
      </SheetTrigger>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Sessions</SheetTitle>
        </SheetHeader>

        <div className="flex flex-col gap-4 overflow-y-auto px-4 pb-4">
          <section className="flex flex-col gap-1">
            <h3 className="text-muted-foreground text-xs uppercase tracking-wide">
              Live
            </h3>
            {live.length === 0 ? (
              <p className="text-muted-foreground text-sm">No live sessions.</p>
            ) : (
              live.map((session) => (
                <div
                  key={session.id}
                  className="flex items-center justify-between gap-2 py-1"
                >
                  <span className="flex min-w-0 items-center gap-1.5 truncate text-sm">
                    <span
                      className={cn(
                        "size-1.5 shrink-0 rounded-full",
                        TONE_DOT.ok,
                      )}
                    />
                    {session.kind === "shell" ? "$ zsh" : sessionName(session)}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => openAndClose(session.id)}
                  >
                    Open
                  </Button>
                </div>
              ))
            )}
          </section>

          <section className="flex flex-col gap-1">
            <h3 className="text-muted-foreground text-xs uppercase tracking-wide">
              Finished
            </h3>
            {finished.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                No finished sessions.
              </p>
            ) : (
              finished.map((session) => (
                <div
                  key={session.id}
                  className="flex items-center justify-between gap-2 py-1"
                >
                  <span className="flex min-w-0 flex-col truncate text-sm">
                    <span className="truncate">
                      {session.kind === "shell"
                        ? "$ zsh"
                        : sessionName(session)}
                    </span>
                    {session.finished_at && (
                      <span className="text-muted-foreground text-xs">
                        {relativeTime(session.finished_at)}
                      </span>
                    )}
                  </span>
                  {session.id === resumable?.id ? (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => resumeAndClose(session.id)}
                    >
                      Resume
                    </Button>
                  ) : (
                    <span className="shrink-0 text-muted-foreground text-xs">
                      {session.kind === "shell"
                        ? "no conversation"
                        : "only the latest can be resumed"}
                    </span>
                  )}
                </div>
              ))
            )}
          </section>
        </div>
      </SheetContent>
    </Sheet>
  );
}
