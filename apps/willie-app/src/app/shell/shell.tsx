import { Outlet, useLocation } from "@tanstack/react-router";
import { useEffect } from "react";
import { ShellHotkeys } from "@/app/hotkeys";
import { AppSidebar } from "@/app/shell/app-sidebar";
import { Header } from "@/app/shell/header";
import { SetupDrawer } from "@/app/shell/setup-drawer";
import { StatusBar } from "@/app/shell/status-bar";
import { TreeDrawer } from "@/app/shell/tree-drawer";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { FilePreview } from "@/features/session/file-preview";
import { useCurrentSystem } from "@/store/use-current-system";
import { useFilePreview } from "@/store/use-file-preview";
import { useTreeDrawer } from "@/store/use-tree-drawer";

/** The one route the workspace tree belongs to: it is opened from the
 * Session screen's tab strip and its file preview reads that system's
 * workspace, so neither may hang over another screen. */
const TREE_ROUTE = "/session";

/** The root route's component: the frameless header, then a body row
 * of sidebar + screen, then the status bar — three grid rows so the
 * header and status bar always span the full window width. `isolate`
 * gives Base UI's portals their own stacking context. */
export function Shell() {
  const pathname = useLocation({ select: (location) => location.pathname });
  const { system } = useCurrentSystem();
  const { close: closeTree } = useTreeDrawer();
  const { close: closePreview } = useFilePreview();

  useEffect(() => {
    if (pathname !== TREE_ROUTE) {
      closeTree();
      closePreview();
    }
  }, [pathname, closeTree, closePreview]);

  return (
    <SidebarProvider className="isolate grid h-svh grid-rows-[var(--header-height)_1fr_auto]">
      <Header systemName={system?.name} />
      {/* The generated sidebar is `position: fixed; inset-y-0`, which
       * without a new containing block would place it flush against
       * the window's top edge, under the header. `will-change-transform`
       * (already used for the same reason in ui/toast.tsx) turns this
       * row into that containing block, so the fixed sidebar spans
       * exactly this row instead of the whole window. */}
      <div className="flex min-h-0 will-change-transform">
        <AppSidebar />
        <SidebarInset className="flex min-h-0 flex-col overflow-hidden">
          {/* `SidebarInset` is already `relative` (components/ui/sidebar.tsx),
           * so it is the tree drawer's containing block: `left: 0` there
           * always hugs the sidebar's true current edge, expanded or
           * icon-collapsed, without reading any sidebar-width token.
           * `overflow-hidden` here keeps the drawer's parked (closed)
           * position from ever peeking out from under the sidebar. */}
          <TreeDrawer />
          {/* Beside the drawer, not inside the Session screen: the two
           * are one surface (`| tree | file |`), so the preview shares
           * the drawer's containing block and its edges line up with
           * no padding between them. Both are `null` unless something
           * is open. */}
          <FilePreview />
          {/* The router only resets the window's scroll on navigation,
           * but the screen scrolls inside this viewport, not the window;
           * keying it by path remounts it fresh so a new screen always
           * opens at its own top instead of the previous screen's offset. */}
          <ScrollArea key={pathname} className="min-h-0 flex-1">
            <div className="p-6">
              <Outlet />
            </div>
          </ScrollArea>
        </SidebarInset>
      </div>
      <StatusBar />
      <ShellHotkeys />
      <SetupDrawer />
    </SidebarProvider>
  );
}
