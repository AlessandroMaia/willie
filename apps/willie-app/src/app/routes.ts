import {
  ActivityIcon,
  BlocksIcon,
  FolderGit2Icon,
  GaugeIcon,
  GitBranchIcon,
  LayersIcon,
  type LucideIcon,
  SettingsIcon,
  ShieldIcon,
  TerminalIcon,
  WrenchIcon,
} from "lucide-react";
import type { SetupEntry } from "@/lib/domain/setup-entries";

export type { SetupEntry };

export type ScreenPath = "/session" | "/sandbox" | "/profiles" | "/usage";

export interface NavEntry {
  id: string;
  label: string;
  icon: LucideIcon;
  path: ScreenPath;
  /** In the shortcuts library's syntax; `Mod` is Ctrl on Windows. */
  shortcut: string;
}

/** The sidebar's four screens: every system has all of them, so
 * unlike the old five-entry sidebar none is ever disabled. The router
 * (`app/router.tsx`) owns the screens themselves; this list never
 * imports a feature. */
export const ROUTES: readonly NavEntry[] = [
  {
    id: "session",
    label: "Session",
    icon: TerminalIcon,
    path: "/session",
    shortcut: "Mod+1",
  },
  {
    id: "sandbox",
    label: "Sandbox",
    icon: ShieldIcon,
    path: "/sandbox",
    shortcut: "Mod+2",
  },
  {
    id: "profiles",
    label: "Profiles",
    icon: LayersIcon,
    path: "/profiles",
    shortcut: "Mod+3",
  },
  {
    id: "usage",
    label: "Usage",
    icon: GaugeIcon,
    path: "/usage",
    shortcut: "Mod+4",
  },
];

export type SetupPath = `/setup/${SetupEntry}`;

interface SetupEntryInfo {
  id: SetupEntry;
  label: string;
  description: string;
  icon: LucideIcon;
  path: SetupPath;
}

/** The setup drawer's own entries: global, not scoped to any one
 * system, so they carry no shortcut and live outside the sidebar's
 * four screens. */
export const SETUP_ENTRIES: readonly SetupEntryInfo[] = [
  {
    id: "engine",
    label: "Engine",
    description: "WSL, the distribution and the daemon's health.",
    icon: ActivityIcon,
    path: "/setup/engine",
  },
  {
    id: "tools",
    label: "Tools",
    description: "Install and update the managed toolchain.",
    icon: WrenchIcon,
    path: "/setup/tools",
  },
  {
    id: "plugins",
    label: "Plugins",
    description: "Enable or disable what runs across every system.",
    icon: BlocksIcon,
    path: "/setup/plugins",
  },
  {
    id: "profile-store",
    label: "Profile store",
    description: "Create and edit configuration profiles.",
    icon: GitBranchIcon,
    path: "/setup/profile-store",
  },
  {
    id: "systems",
    label: "Systems",
    description: "Add, discover and manage every system.",
    icon: FolderGit2Icon,
    path: "/setup/systems",
  },
  {
    id: "settings",
    label: "Settings",
    description: "Theme and other app-wide preferences.",
    icon: SettingsIcon,
    path: "/setup/settings",
  },
];

/** "Mod+1" → "Ctrl+1": Willie runs on Windows only. */
export function shortcutLabel(shortcut: string): string {
  return shortcut.replace("Mod", "Ctrl");
}
