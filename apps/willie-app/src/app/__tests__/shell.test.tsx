import { createMemoryHistory } from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "@/lib/ipc";
import type { Snapshot } from "@/lib/proto";

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

const SNAPSHOT: Snapshot = {
  seq: 1,
  projects: [],
  jobs: [],
  sessions: [
    {
      id: "sess_1",
      project_id: "proj_1",
      harness: "claude-code",
      workspace: "/home/willie/projects/a",
      state: { state: "running" },
      created_at: "1",
      clients: 1,
    },
  ],
};

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Every function the three screens may call on mount is
 * here; the rest are inert. */
const ipc = vi.hoisted(() => ({
  engine: {
    status: vi.fn(),
    onStatus: vi.fn(async () => () => {}),
    installDistro: vi.fn(),
    startDaemon: vi.fn(),
    stopDaemon: vi.fn(),
    doctor: vi.fn(),
  },
  projects: {
    snapshot: vi.fn(),
    roots: vi.fn(async () => []),
    add: vi.fn(),
    remove: vi.fn(),
    syncToWindows: vi.fn(),
    updateFromWindows: vi.fn(),
    relocate: vi.fn(),
    rename: vi.fn(),
    cancelJob: vi.fn(),
    setRoots: vi.fn(),
    discover: vi.fn(),
    openInExplorer: vi.fn(),
  },
  sessions: { open: vi.fn(), resume: vi.fn(), attach: vi.fn(), stop: vi.fn() },
  sessionTerminal: {
    open: vi.fn(),
    input: vi.fn(),
    resize: vi.fn(),
    close: vi.fn(),
  },
  tools: { install: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

let App: typeof import("@/app/app").App;
let createAppRouter: typeof import("@/app/router").createAppRouter;

/* `useEngineStatus` and `useSnapshot` back onto module-level singleton
 * stores, so a status left behind by one test's render would otherwise
 * still be there for the next test's first synchronous render. Reset
 * the module graph and re-import the entry points fresh for every
 * case, so no test depends on running before or after another. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ({ App } = await import("@/app/app"));
  ({ createAppRouter } = await import("@/app/router"));
});

function renderApp(path = "/", status: EngineStatus = STATUS) {
  ipc.engine.status.mockResolvedValue(status);
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  const router = createAppRouter(
    createMemoryHistory({ initialEntries: [path] }),
  );
  render(<App router={router} />);
  return router;
}

describe("the shell", () => {
  it("opens on the Dashboard and marks it active in the sidebar", async () => {
    renderApp();

    const link = await screen.findByRole("link", { name: /Dashboard/ });

    expect(link.getAttribute("aria-current")).toBe("page");
    expect(link.getAttribute("href")).toBe("/dashboard");
  });

  it("navigates by clicking a sidebar entry", async () => {
    const user = userEvent.setup();
    const router = renderApp();
    await screen.findByRole("link", { name: /Dashboard/ });

    await user.click(screen.getByRole("link", { name: /Projects/ }));

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/projects"),
    );
    expect(
      await screen.findByRole("heading", { name: "Projects" }),
    ).toBeDefined();
  });

  it("navigates with Mod+3", async () => {
    const user = userEvent.setup();
    const router = renderApp();
    await screen.findByRole("link", { name: /Dashboard/ });

    await user.keyboard("{Control>}3{/Control}");

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/sessions"),
    );
  });

  it("toggles the sidebar exactly once on Mod+B", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("link", { name: /Dashboard/ });
    const sidebar = document.querySelector("[data-slot='sidebar'][data-state]");
    expect(sidebar?.getAttribute("data-state")).toBe("expanded");

    await user.keyboard("{Control>}b{/Control}");

    expect(sidebar?.getAttribute("data-state")).toBe("collapsed");
  });

  it("shows the planned screens disabled, with no route", async () => {
    renderApp();
    await screen.findByRole("link", { name: /Dashboard/ });

    for (const label of ["Tools", "Plugins", "Settings"]) {
      const button = screen.getByRole("button", { name: new RegExp(label) });
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
    expect(screen.queryByRole("link", { name: /Tools/ })).toBeNull();
  });

  it("shows a tooltip on hover for a planned entry", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("link", { name: /Dashboard/ });

    const wrapper = screen.getByRole("button", { name: /Tools/ })
      .parentElement as HTMLElement;
    await user.hover(wrapper);

    expect(await screen.findByText("Not available yet")).toBeDefined();

    /* Closes the tooltip and waits out its own transition before the
     * test ends, so no pending state update from it echoes into
     * whichever test runs next. */
    await user.unhover(wrapper);
    await vi.waitFor(() =>
      expect(screen.queryByText("Not available yet")).toBeNull(),
    );
  });

  it("sends a stale hash to the Dashboard", async () => {
    const router = renderApp("/tools");

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/dashboard"),
    );
  });

  it("puts the engine headline, the daemon version and the live count in the status bar", async () => {
    renderApp();

    expect(await screen.findByText("Engine running")).toBeDefined();
    expect(await screen.findByText("willied 0.1.0")).toBeDefined();
    expect(await screen.findByText("1 live session")).toBeDefined();
  });

  it("names the first failing part and hides the count when the daemon is stopped", async () => {
    renderApp("/", { ...STATUS, daemon: { state: "stopped" } });

    expect(await screen.findByText("Daemon stopped")).toBeDefined();
    expect(screen.queryByText(/live session/)).toBeNull();
    expect(ipc.projects.snapshot).not.toHaveBeenCalled();
  });
});
