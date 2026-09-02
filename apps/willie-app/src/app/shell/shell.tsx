import { Outlet, useLocation } from "@tanstack/react-router";
import { ShellHotkeys } from "@/app/hotkeys";
import { AppSidebar } from "@/app/shell/app-sidebar";
import { StatusBar } from "@/app/shell/status-bar";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";

/** The root route's component: sidebar, the screen in a scroll area,
 * the status bar. `isolate` gives Base UI's portals their own stacking
 * context. */
export function Shell() {
  const pathname = useLocation({ select: (location) => location.pathname });

  return (
    <SidebarProvider className="isolate h-svh">
      <AppSidebar />
      <SidebarInset className="flex min-h-0 flex-col">
        {/* The router only resets the window's scroll on navigation,
         * but the screen scrolls inside this viewport, not the window;
         * keying it by path remounts it fresh so a new screen always
         * opens at its own top instead of the previous screen's offset. */}
        <ScrollArea key={pathname} className="min-h-0 flex-1">
          <div className="p-6">
            <Outlet />
          </div>
        </ScrollArea>
        <StatusBar />
      </SidebarInset>
      <ShellHotkeys />
    </SidebarProvider>
  );
}
