import { DashboardScreen } from "@/features/health/dashboard-screen";
import { ProjectsScreen } from "@/features/projects/projects-screen";
import { SessionsScreen } from "@/features/sessions/sessions-screen";

/** One entry per screen the shell can show. Adding a screen is adding
 * a row here: the shell renders whatever this list contains, so no
 * navigation code names an individual screen. `as const` keeps each
 * `id` a literal, so `Route["id"]` below is the three-value union
 * rather than `string` — a typo in the active tab is a compile error
 * again, the way it was before this list existed. */
export const ROUTES = [
  { id: "dashboard", label: "Dashboard", screen: DashboardScreen },
  { id: "projects", label: "Projects", screen: ProjectsScreen },
  { id: "sessions", label: "Sessions", screen: SessionsScreen },
] as const;

export type Route = (typeof ROUTES)[number];
