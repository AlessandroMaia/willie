import { Outlet, useLocation } from "@tanstack/react-router";
import { ShellHotkeys } from "@/app/hotkeys";
import { AppSidebar } from "@/app/shell/app-sidebar";
import { Header } from "@/app/shell/header";
import { SetupDrawer } from "@/app/shell/setup-drawer";
import { StatusBar } from "@/app/shell/status-bar";
import { WorkspacePanel } from "@/app/shell/workspace-panel";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useCurrentSystem } from "@/store/use-current-system";

/** The root route's component: the frameless header, then a body row
 * of sidebar + screen, then the status bar — three grid rows so the
 * header and status bar always span the full window width. `isolate`
 * gives Base UI's portals their own stacking context. */
export function Shell() {
  const pathname = useLocation({ select: (location) => location.pathname });
  const { system } = useCurrentSystem();
  return (
    <SidebarProvider className="isolate grid h-svh grid-cols-[minmax(0,1fr)] grid-rows-[var(--header-height)_1fr_auto]">
      <Header systemName={system?.name} />
      {/* The generated sidebar is `position: fixed; inset-y-0`, which
       * without a new containing block would place it flush against
       * the window's top edge, under the header. `will-change-transform`
       * (already used for the same reason in ui/toast.tsx) turns this
       * row into that containing block, so the fixed sidebar spans
       * exactly this row instead of the whole window. */}
      <div className="flex min-h-0 min-w-0 will-change-transform">
        <AppSidebar />
        <SidebarInset className="flex min-h-0 flex-col overflow-hidden">
          {/* The screen scrolls inside this viewport, not the window,
           * and the panel is its column rather than a layer over it:
           * opening the panel narrows the screen instead of covering
           * it. Keying the scroll area by path remounts it fresh, so a
           * new screen always opens at its own top instead of the
           * previous screen's offset. */}
          {/* `min-w-0` on the screen, or its content's intrinsic
           * width wins and the panel is pushed off the window. */}
          <div className="flex min-h-0 flex-1">
            <ScrollArea key={pathname} className="min-h-0 min-w-0 flex-1">
              <div className="p-6">
                <Outlet />
              </div>
            </ScrollArea>
            <WorkspacePanel />
          </div>
        </SidebarInset>
      </div>
      <StatusBar />
      <ShellHotkeys />
      <SetupDrawer />
    </SidebarProvider>
  );
}
