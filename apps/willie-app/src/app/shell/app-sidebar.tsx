import { Link, useLocation } from "@tanstack/react-router";
import { ROUTES, shortcutLabel } from "@/app/routes";
import { SystemActionsMenu } from "@/app/shell/system-actions-menu";
import { SystemSelector } from "@/app/shell/system-selector";
import { Kbd } from "@/components/ui/kbd";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/components/ui/sidebar";

/** The system-scoped shell's sidebar: the current system (its selector
 * and "…" actions) up top, then the four screens every system has.
 * Nothing global belongs here — everything machine-wide lives behind
 * the header's settings button. */
export function AppSidebar() {
  const pathname = useLocation({ select: (location) => location.pathname });

  /* The generated sidebar is `fixed inset-y-0 h-svh`, and that height
   * wins over `bottom: 0` inside the shell's body row: a full window
   * tall, it ends a header plus a status bar below the row, covering
   * the status bar and scrolling the window. It fills the row it
   * lives in instead. */
  return (
    <Sidebar collapsible="icon" className="h-full">
      <SidebarHeader className="flex-row items-center gap-1 px-2 py-2">
        <SystemSelector />
        <SystemActionsMenu />
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Screens</SidebarGroupLabel>
          <SidebarMenu>
            {ROUTES.map((entry) => (
              <SidebarMenuItem key={entry.id}>
                <SidebarMenuButton
                  render={<Link to={entry.path} />}
                  isActive={pathname === entry.path}
                  tooltip={{
                    children: (
                      <>
                        {entry.label} <Kbd>{shortcutLabel(entry.shortcut)}</Kbd>
                      </>
                    ),
                  }}
                >
                  <entry.icon />
                  <span>{entry.label}</span>
                  <Kbd className="ml-auto group-data-[collapsible=icon]:hidden">
                    {shortcutLabel(entry.shortcut)}
                  </Kbd>
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>

      <SidebarRail />
    </Sidebar>
  );
}
