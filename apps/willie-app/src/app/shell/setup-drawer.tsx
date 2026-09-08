import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { SETUP_ENTRIES, type SetupPath } from "@/app/routes";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useSetupDrawer } from "@/store/use-setup-drawer";

/**
 * The global area behind the header's setup button: six entries, none
 * scoped to any one system, each navigating to its own `/setup/<x>`
 * route and closing the drawer. `openAt(entry)` only highlights and
 * scrolls to that row — the drawer never navigates by itself.
 */
export function SetupDrawer() {
  const { open, entry, close } = useSetupDrawer();
  const navigate = useNavigate();
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open || !entry) return;
    listRef.current
      ?.querySelector(`[data-setup-entry="${entry}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [open, entry]);

  function choose(path: SetupPath) {
    navigate({ to: path });
    close();
  }

  return (
    <Sheet
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
    >
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Setup</SheetTitle>
        </SheetHeader>

        <div
          ref={listRef}
          className="flex flex-col gap-1 overflow-y-auto px-4 pb-4"
        >
          {SETUP_ENTRIES.map((setupEntry) => {
            const Icon = setupEntry.icon;
            return (
              <Button
                key={setupEntry.id}
                data-setup-entry={setupEntry.id}
                variant={setupEntry.id === entry ? "secondary" : "ghost"}
                className="h-auto items-start justify-start gap-3 px-3 py-2.5 text-left"
                onClick={() => choose(setupEntry.path)}
              >
                <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span className="font-medium text-sm">
                    {setupEntry.label}
                  </span>
                  <span className="text-muted-foreground text-xs">
                    {setupEntry.description}
                  </span>
                </span>
              </Button>
            );
          })}
        </div>
      </SheetContent>
    </Sheet>
  );
}
