import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Minimize2Icon,
  MinusIcon,
  PanelLeftIcon,
  SettingsIcon,
  SquareIcon,
  XIcon,
} from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { useSidebar } from "@/components/ui/sidebar";
import { useSetupDrawer } from "@/store/use-setup-drawer";

interface HeaderProps {
  /** Task 9's `useCurrentSystem()` supplies this; until then the
   * centre shows the bare app name. */
  systemName?: string;
}

function SidebarToggle() {
  const { toggleSidebar } = useSidebar();

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label="Toggle sidebar"
      onClick={toggleSidebar}
    >
      <PanelLeftIcon />
    </Button>
  );
}

function SettingsButton() {
  const { openAt } = useSetupDrawer();

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label="Open setup"
      onClick={() => openAt()}
    >
      <SettingsIcon />
    </Button>
  );
}

interface DragRegionTitleProps {
  systemName?: string;
}

function DragRegionTitle({ systemName }: DragRegionTitleProps) {
  return (
    <div
      data-tauri-drag-region
      className="flex flex-1 items-center justify-center gap-1 text-xs"
    >
      <span data-tauri-drag-region className="font-medium">
        Willie
      </span>
      {systemName && (
        <span data-tauri-drag-region className="text-muted-foreground">
          &middot; {systemName}
        </span>
      )}
    </div>
  );
}

interface WindowControlProps {
  label: string;
  destructive?: boolean;
  onClick: () => void;
  children: ReactNode;
}

function WindowControl({
  label,
  destructive,
  onClick,
  children,
}: WindowControlProps) {
  return (
    <Button
      variant={destructive ? "destructive" : "ghost"}
      size="icon-sm"
      aria-label={label}
      onClick={onClick}
    >
      {children}
    </Button>
  );
}

/** Reflects `isMaximized()` on mount and on every resize, so the glyph
 * never lies after the user drags an edge or double-clicks the title
 * area instead of using this control. */
function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;

    win.isMaximized().then((value) => {
      if (!cancelled) setMaximized(value);
    });

    win
      .onResized(() => {
        win.isMaximized().then((value) => {
          if (!cancelled) setMaximized(value);
        });
      })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return (
    <div className="flex items-center">
      <WindowControl
        label="Minimize"
        onClick={() => getCurrentWindow().minimize()}
      >
        <MinusIcon />
      </WindowControl>
      <WindowControl
        label={maximized ? "Restore" : "Maximize"}
        onClick={() => getCurrentWindow().toggleMaximize()}
      >
        {maximized ? <Minimize2Icon /> : <SquareIcon />}
      </WindowControl>
      <WindowControl
        label="Close"
        destructive
        onClick={() => getCurrentWindow().close()}
      >
        <XIcon />
      </WindowControl>
    </div>
  );
}

/** The 36px frameless title bar: the sidebar and setup drawer toggles
 * on the left, a draggable centre carrying the app and system name,
 * Willie's own window controls on the right. With `decorations:
 * false` (Task 7) this is the only way to move or close the window. */
export function Header({ systemName }: HeaderProps) {
  return (
    <header className="flex h-9 shrink-0 items-center gap-1 border-b bg-sidebar px-1">
      <SidebarToggle />
      <SettingsButton />
      <DragRegionTitle systemName={systemName} />
      <WindowControls />
    </header>
  );
}
