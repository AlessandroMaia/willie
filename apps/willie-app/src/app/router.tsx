import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  type RouterHistory,
  redirect,
} from "@tanstack/react-router";
import { Shell } from "@/app/shell/shell";
import { DashboardScreen } from "@/features/health/dashboard-screen";
import { ProjectsScreen } from "@/features/projects/projects-screen";
import { SessionsScreen } from "@/features/sessions/sessions-screen";
import { ToolsScreen } from "@/features/tools/tools-screen";

/* A stale hash (an old bookmark, a screen that no longer exists) never
 * shows a blank: it lands on the Dashboard like a fresh start. */
function NotFound() {
  return <Navigate to="/dashboard" replace />;
}

const rootRoute = createRootRoute({
  component: Shell,
  notFoundComponent: NotFound,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/dashboard" });
  },
});

const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dashboard",
  component: DashboardScreen,
});

const projectsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects",
  component: ProjectsScreen,
});

const sessionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sessions",
  component: SessionsScreen,
});

const toolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tools",
  component: ToolsScreen,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  dashboardRoute,
  projectsRoute,
  sessionsRoute,
  toolsRoute,
]);

/** Hash history by default: it survives a dev-server reload and needs
 * no real URL under Tauri. Tests pass a memory history. */
export function createAppRouter(history: RouterHistory = createHashHistory()) {
  return createRouter({ routeTree, history });
}

export type AppRouter = ReturnType<typeof createAppRouter>;

declare module "@tanstack/react-router" {
  interface Register {
    router: AppRouter;
  }
}
