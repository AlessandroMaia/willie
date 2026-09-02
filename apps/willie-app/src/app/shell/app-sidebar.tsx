import { Link, useLocation } from "@tanstack/react-router";
import { ROUTES, shortcutLabel } from "@/app/routes";
import { Kbd } from "@/components/ui/kbd";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/components/ui/sidebar";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/* This span exists only to carry hover/focus for a planned entry's
 * tooltip trigger, wrapped around its disabled button; it takes no
 * other keyboard action itself. */
// biome-ignore lint/a11y/noNoninteractiveTabindex: see comment above
const disabledEntryTrigger = <span className="block" tabIndex={0} />;

export function AppSidebar() {
  const pathname = useLocation({ select: (location) => location.pathname });

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="h-12 justify-center px-4">
        <span className="truncate font-semibold tracking-tight group-data-[collapsible=icon]:hidden">
          Willie
        </span>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu>
            {ROUTES.map((entry) => (
              <SidebarMenuItem key={entry.id}>
                {entry.available ? (
                  <SidebarMenuButton
                    render={<Link to={entry.path} />}
                    isActive={pathname === entry.path}
                    /* `isActive` drives the button's own state styling
                     * (Base UI renders it as a bare `data-active`, no
                     * value); tests and any external styling that reads
                     * the active row want the conventional
                     * `data-active="true"`, so it is set explicitly
                     * too and wins the merge over the state-derived
                     * one. */
                    data-active={pathname === entry.path ? "true" : undefined}
                    tooltip={{
                      children: (
                        <>
                          {entry.label}{" "}
                          <Kbd>{shortcutLabel(entry.shortcut)}</Kbd>
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
                ) : (
                  /* A disabled control receives no pointer or focus
                   * events, so a tooltip trigger wrapped directly around
                   * it (via `SidebarMenuButton`'s own `tooltip` prop)
                   * would never open — and in the icon-collapsed sidebar
                   * that tooltip is the row's only label. The tooltip
                   * hangs instead on a plain, focusable wrapper around
                   * the inert, still genuinely disabled button. */
                  <Tooltip>
                    <TooltipTrigger render={disabledEntryTrigger}>
                      <SidebarMenuButton
                        disabled
                        className="pointer-events-none"
                      >
                        <entry.icon />
                        <span>{entry.label}</span>
                      </SidebarMenuButton>
                    </TooltipTrigger>
                    <TooltipContent side="right">
                      Not available yet
                    </TooltipContent>
                  </Tooltip>
                )}
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>

      <SidebarRail />
    </Sidebar>
  );
}
