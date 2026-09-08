import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "@/lib/ipc";
import type { CapabilityInfo, Project, Session, Snapshot } from "@/lib/proto";

const STATUS: EngineStatus = {
  engine_version: "0.1.0",
  wsl: {
    installed: true,
    version: "2.6.1.0",
    meets_minimum: true,
    minimum: "2.4.4",
  },
  distro: { registered: true, running: true, install_dir: "C:\\x" },
  distro_error: null,
  daemon: {
    state: "running",
    willie_version: "0.1.0",
    image_version: "0.1.0+abc",
  },
  doctor: { checks: [] },
  image_available: true,
};

const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn(), setSandbox: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
  sandbox: { catalogue: vi.fn(async (): Promise<CapabilityInfo[]> => []) },
}));

vi.mock("@/lib/ipc", () => ipc);

let SandboxScreen: typeof import("@/features/sandbox/sandbox-screen").SandboxScreen;
let useCurrentSystem: typeof import("@/store/use-current-system").useCurrentSystem;

function project(over: Partial<Project> = {}): Project {
  return {
    id: "proj_1",
    name: "willie",
    slug: "willie",
    source: "C:\\github\\willie",
    workspace: "/home/willie/projects/willie",
    branch: "main",
    state: { state: "ready" },
    source_present: true,
    created_at: "1",
    sandbox: {},
    ...over,
  };
}

function session(over: Partial<Session> & { id: string }): Session {
  return {
    project_id: "proj_1",
    harness: "claude-code",
    workspace: "/home/willie/projects/willie",
    state: { state: "running" },
    created_at: "1",
    clients: 0,
    ...over,
  };
}

function snapshot(
  sessions: Session[] = [],
  projects: Project[] = [project()],
): Snapshot {
  return { seq: 1, projects, jobs: [], sessions };
}

const CATALOGUE: CapabilityInfo[] = [
  {
    capability: "project_rw",
    display_name: "project.rw",
    consequence: "the session edits the project, which is why it exists",
    implemented: true,
    default_enabled: true,
  },
  {
    capability: "agent_state",
    display_name: "agent.state",
    consequence: "anything the agent runs can use the harness's login",
    implemented: true,
    default_enabled: true,
  },
];

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.projects.snapshot.mockResolvedValue(snapshot());
  ipc.sandbox.catalogue.mockResolvedValue(CATALOGUE);
  ({ SandboxScreen } = await import("@/features/sandbox/sandbox-screen"));
  ({ useCurrentSystem } = await import("@/store/use-current-system"));
});

/* `useCurrentSystem` backs onto a module-level singleton, the same one
 * `SandboxScreen` reads — imported fresh above right after
 * `vi.resetModules()`, so this probe's `setSystem` reaches the exact
 * store instance the rendered screen is subscribed to. Mirrors
 * `status-bar.test.tsx`'s `focus()` helper. */
function switchSystem(id: string) {
  const hook = renderHook(() => useCurrentSystem());
  act(() => {
    hook.result.current.setSystem(id);
  });
}

/* A minimal one-route tree, the same `validateSearch` shape
 * `app/router.tsx` gives `/sandbox` — enough for `useSearch({ from:
 * "/sandbox" })` to resolve without pulling in the whole app graph
 * (and its xterm-backed Session screen) just to render this screen. */
function renderScreen(path = "/sandbox") {
  const rootRoute = createRootRoute();
  const sandboxRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/sandbox",
    component: SandboxScreen,
    validateSearch: (
      search: Record<string, unknown>,
    ): { session?: string } => ({
      session: typeof search.session === "string" ? search.session : undefined,
    }),
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([sandboxRoute]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  return render(<RouterProvider router={router} />);
}

describe("SandboxScreen", () => {
  it("edit_capabilities_opens_the_drawer_with_the_catalogue", async () => {
    renderScreen();
    const user = userEvent.setup();

    await user.click(
      await screen.findByRole("button", { name: "Edit capabilities" }),
    );

    expect(screen.getByRole("dialog")).toBeDefined();
    expect(await screen.findByText("project.rw")).toBeDefined();
  });

  it("a_system_without_denials_shows_the_empty_state", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_1",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [],
          },
        }),
      ]),
    );

    renderScreen();

    expect(await screen.findByText("No denials")).toBeDefined();
    expect(screen.queryByText("syscall filter")).not.toBeNull();
    expect(screen.queryByRole("button", { name: "all" })).toBeNull();
  });

  it("the_screen_filters_the_history_by_session_and_preselects_from_the_query", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_1",
          label: "alpha",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [
              {
                class: "syscall",
                name: "ptrace",
                count: 2,
                first_at: "1",
                last_at: "100",
              },
            ],
          },
        }),
        session({
          id: "sess_2",
          label: "beta",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [
              {
                class: "terminal",
                name: "clipboard",
                count: 1,
                first_at: "1",
                last_at: "200",
              },
            ],
          },
        }),
      ]),
    );

    renderScreen("/sandbox?session=sess_2");

    expect(await screen.findByText("clipboard")).toBeDefined();
    expect(screen.queryByText("ptrace")).toBeNull();
    expect(
      screen.getByRole("button", { name: "beta" }).getAttribute("aria-pressed"),
    ).toBe("true");

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "all" }));

    expect(await screen.findByText("ptrace")).toBeDefined();
    expect(screen.getByText("clipboard")).toBeDefined();
  });

  it("falls back to all when the query names a session with no denials", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_1",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [
              {
                class: "syscall",
                name: "ptrace",
                count: 1,
                first_at: "1",
                last_at: "50",
              },
            ],
          },
        }),
        session({
          id: "sess_2",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [],
          },
        }),
      ]),
    );

    renderScreen("/sandbox?session=sess_2");

    expect(await screen.findByText("ptrace")).toBeDefined();
    expect(
      screen.getByRole("button", { name: "all" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("shows the mechanism chips with a tone per state and the reason on hover", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_1",
          sandbox: {
            applied: ["seccomp"],
            unavailable: ["landlock"],
            degraded: ["rlimits"],
            denied: [],
          },
        }),
      ]),
    );

    renderScreen();

    expect(await screen.findByText("syscall filter")).toBeDefined();
    const unavailable = screen.getByText("path rules");
    expect(unavailable.getAttribute("title")).toMatch(/kernel/);
    const degraded = screen.getByText("limits");
    expect(degraded.getAttribute("title")).toMatch(/degraded/);
  });

  it("shows the no-system empty state and opens the setup drawer", async () => {
    ipc.projects.snapshot.mockResolvedValue(snapshot([], []));

    renderScreen();

    expect(await screen.findByText("No system yet")).toBeDefined();
  });

  it("never offers an allow action anywhere on a denial row", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_1",
          sandbox: {
            applied: ["seccomp"],
            unavailable: [],
            degraded: [],
            denied: [
              {
                class: "syscall",
                name: "ptrace",
                count: 1,
                first_at: "1",
                last_at: "1",
              },
            ],
          },
        }),
      ]),
    );

    renderScreen();

    await screen.findByText("ptrace");
    expect(screen.queryByRole("button", { name: /allow/i })).toBeNull();
  });

  /* A capability profile is edited for exactly one system: if the
   * current system changes while the drawer is open (the daemon is
   * shared — another client can rename, remove, or the selector
   * itself switches), the unsaved edit must never survive to land on
   * the wrong system's profile. */
  it("switching_systems_closes_the_capabilities_drawer_without_saving", async () => {
    const alpha = project({ id: "proj_1", name: "alpha" });
    const beta = project({ id: "proj_2", name: "beta" });
    ipc.ui.prefs.mockResolvedValue({ current_project: "proj_1" });
    ipc.projects.snapshot.mockResolvedValue(snapshot([], [alpha, beta]));

    renderScreen();
    const user = userEvent.setup();

    expect(await screen.findByText("alpha")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Edit capabilities" }));
    await screen.findByText("agent.state");

    /* An unsaved edit: toggled off while alpha is current. */
    await user.click(screen.getByRole("checkbox", { name: /agent\.state/ }));

    switchSystem("proj_2");

    await screen.findByText("beta");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(ipc.projects.setSandbox).not.toHaveBeenCalled();

    /* Reopening shows beta's own (untouched) default, never alpha's
     * stale toggled-off edit — proof the drawer started fresh. */
    await user.click(screen.getByRole("button", { name: "Edit capabilities" }));
    const credential = await screen.findByRole("checkbox", {
      name: /agent\.state/,
    });
    expect(credential.hasAttribute("data-checked")).toBe(true);
  });
});
