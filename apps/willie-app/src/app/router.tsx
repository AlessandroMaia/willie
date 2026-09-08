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
import { PluginsScreen } from "@/features/plugins/plugins-screen";
import { ProfileStoreScreen } from "@/features/profile-store/profile-store-screen";
import { ProfilesScreen } from "@/features/profiles/profiles-screen";
import { ProjectsScreen } from "@/features/projects/projects-screen";
import { SettingsScreen } from "@/features/settings/settings-screen";
import { ToolsScreen } from "@/features/tools/tools-screen";
import { UsageScreen } from "@/features/usage/usage-screen";

/* A stale hash never shows a blank: it lands on the current system's
 * Session screen, the app's new home. */
function NotFound() {
  return <Navigate to="/session" replace />;
}

/** What Tasks 12/15 replace: a screen this task only gives an address
 * to, not a route that does not resolve. */
function ScreenPlaceholder({ title }: { title: string }) {
  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-2">
      <h1 className="font-semibold text-lg">{title}</h1>
      <p className="text-muted-foreground text-sm">Coming soon.</p>
    </div>
  );
}

function SessionScreen() {
  return <ScreenPlaceholder title="Session" />;
}

function SandboxScreen() {
  return <ScreenPlaceholder title="Sandbox" />;
}

const rootRoute = createRootRoute({
  component: Shell,
  notFoundComponent: NotFound,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/session" });
  },
});

const sessionRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/session",
  component: SessionScreen,
});

const sandboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sandbox",
  component: SandboxScreen,
  /* Read by Task 15's system aggregate; the footer's governance
   * segment (Task 11) links here with `?session=<id>` today. */
  validateSearch: (search: Record<string, unknown>): { session?: string } => ({
    session: typeof search.session === "string" ? search.session : undefined,
  }),
});

const profilesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/profiles",
  component: ProfilesScreen,
});

const usageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/usage",
  component: UsageScreen,
});

const setupEngineRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/engine",
  component: DashboardScreen,
});

const setupToolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/tools",
  component: ToolsScreen,
});

const setupPluginsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/plugins",
  component: PluginsScreen,
});

const setupProfileStoreRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/profile-store",
  component: ProfileStoreScreen,
});

const setupSystemsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/systems",
  component: ProjectsScreen,
});

const setupSettingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/setup/settings",
  component: SettingsScreen,
});

/* The old five-screen paths, kept as redirects so a bookmark or a
 * stale hash from before the system-scoped shell still lands
 * somewhere real. */
const dashboardRedirect = createRoute({
  getParentRoute: () => rootRoute,
  path: "/dashboard",
  beforeLoad: () => {
    throw redirect({ to: "/setup/engine" });
  },
});

const projectsRedirect = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects",
  beforeLoad: () => {
    throw redirect({ to: "/setup/systems" });
  },
});

const sessionsRedirect = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sessions",
  beforeLoad: () => {
    throw redirect({ to: "/session" });
  },
});

const toolsRedirect = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tools",
  beforeLoad: () => {
    throw redirect({ to: "/setup/tools" });
  },
});

const pluginsRedirect = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plugins",
  beforeLoad: () => {
    throw redirect({ to: "/setup/plugins" });
  },
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  sessionRoute,
  sandboxRoute,
  profilesRoute,
  usageRoute,
  setupEngineRoute,
  setupToolsRoute,
  setupPluginsRoute,
  setupProfileStoreRoute,
  setupSystemsRoute,
  setupSettingsRoute,
  dashboardRedirect,
  projectsRedirect,
  sessionsRedirect,
  toolsRedirect,
  pluginsRedirect,
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
