import {
  ActivityIcon,
  BlocksIcon,
  FolderGit2Icon,
  type LucideIcon,
  SettingsIcon,
  TerminalIcon,
  WrenchIcon,
} from "lucide-react";

export type ScreenPath = "/dashboard" | "/projects" | "/sessions" | "/tools";

interface AvailableEntry {
  id: string;
  label: string;
  icon: LucideIcon;
  available: true;
  path: ScreenPath;
  /** In the shortcuts library's syntax; `Mod` is Ctrl on Windows. */
  shortcut: string;
}

interface PlannedEntry {
  id: string;
  label: string;
  icon: LucideIcon;
  available: false;
}

export type NavEntry = AvailableEntry | PlannedEntry;

/** One row per screen the sidebar shows, in order. A screen that does
 * not exist yet is a row with `available: false`: it has an address
 * and no route, so the shell renders it disabled and registers no
 * shortcut for it. The router (`app/router.tsx`) owns the screens
 * themselves; this list never imports a feature. */
export const ROUTES: readonly NavEntry[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    icon: ActivityIcon,
    available: true,
    path: "/dashboard",
    shortcut: "Mod+1",
  },
  {
    id: "projects",
    label: "Projects",
    icon: FolderGit2Icon,
    available: true,
    path: "/projects",
    shortcut: "Mod+2",
  },
  {
    id: "sessions",
    label: "Sessions",
    icon: TerminalIcon,
    available: true,
    path: "/sessions",
    shortcut: "Mod+3",
  },
  {
    id: "tools",
    label: "Tools",
    icon: WrenchIcon,
    available: true,
    path: "/tools",
    shortcut: "Mod+4",
  },
  { id: "plugins", label: "Plugins", icon: BlocksIcon, available: false },
  { id: "settings", label: "Settings", icon: SettingsIcon, available: false },
];

/** "Mod+1" → "Ctrl+1": Willie runs on Windows only. */
export function shortcutLabel(shortcut: string): string {
  return shortcut.replace("Mod", "Ctrl");
}
