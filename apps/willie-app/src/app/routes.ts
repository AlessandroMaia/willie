import type { ComponentType } from "react";
import { DashboardScreen } from "@/features/health/dashboard-screen";
import { ProjectsScreen } from "@/features/projects/projects-screen";
import { SessionsScreen } from "@/features/sessions/sessions-screen";

/** One entry per screen the shell can show. Adding a screen is adding
 * a row here: the shell renders whatever this list contains, so no
 * navigation code names an individual screen. */
export interface Route {
  id: string;
  label: string;
  screen: ComponentType;
}

export const ROUTES: Route[] = [
  { id: "dashboard", label: "Dashboard", screen: DashboardScreen },
  { id: "projects", label: "Projects", screen: ProjectsScreen },
  { id: "sessions", label: "Sessions", screen: SessionsScreen },
];
