import { createMemoryHistory } from "@tanstack/react-router";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "@/app/app";
import { createAppRouter } from "@/app/router";
import type { EngineStatus } from "@/lib/ipc";
import type { Snapshot } from "@/lib/proto";
import { resetStores } from "@/test-support/reset-stores";

/* The Session screen hosts a real `SessionTerminal` for every live
 * session, and jsdom has no canvas for `@xterm/xterm` to render into.
 * A fake stands in, same shape `session-screen.test.tsx` uses. */
vi.mock("@xterm/xterm", () => {
  class FakeTerminal {
    rows = 24;
    cols = 80;
    options: Record<string, unknown> = {};
    attachCustomKeyEventHandler() {}
    loadAddon() {}
    open() {}
    onData() {}
    write() {}
    dispose() {}
  }
  return { Terminal: FakeTerminal };
});

vi.mock("@xterm/addon-fit", () => {
  class FakeFitAddon {
    fit() {}
  }
  return { FitAddon: FakeFitAddon };
});

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
 * test fakes. Every function the shell or its four screens may call
 * on mount is here; the rest are inert. */
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
    setSandbox: vi.fn(),
    cancelJob: vi.fn(),
    setRoots: vi.fn(),
    discover: vi.fn(),
    openInExplorer: vi.fn(),
    openInEditor: vi.fn(),
  },
  sessions: { open: vi.fn(), resume: vi.fn(), attach: vi.fn(), stop: vi.fn() },
  sessionTerminal: {
    open: vi.fn(),
    input: vi.fn(),
    resize: vi.fn(),
    close: vi.fn(),
  },
  tools: {
    install: vi.fn(),
    update: vi.fn(),
    list: vi.fn(async () => ({ tools: [] })),
  },
  plugins: {
    list: vi.fn(async () => []),
    enable: vi.fn(),
    disable: vi.fn(),
  },
  sandbox: { catalogue: vi.fn(async () => []) },
  dialogs: { pickFolder: vi.fn(async () => null) },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  ui: {
    prefs: vi.fn(async () => ({ current_project: null })),
    setPrefs: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

/* The header's window controls talk to this module directly, not
 * through the ipc bridge above; every test here renders the full
 * shell, so it needs a stub too, even though none of these tests
 * assert on it (header.test.tsx does). */
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
  }),
}));

/* `useEngineStatus`, `useSnapshot` and `useCurrentSystem` back onto
 * module-level singleton stores, so a value left behind by one test's
 * render would otherwise still be there for the next test's first
 * synchronous render. Reset them for every case, so no test depends on
 * running before or after another. */
beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
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
  it("opens on Session and marks it active in the sidebar", async () => {
    renderApp();

    const link = await screen.findByRole("link", { name: /Session/ });

    expect(link.getAttribute("aria-current")).toBe("page");
    expect(link.getAttribute("href")).toBe("/session");
  });

  it("navigates by clicking a sidebar entry", async () => {
    const user = userEvent.setup();
    const router = renderApp();
    await screen.findByRole("link", { name: /Session/ });

    await user.click(screen.getByRole("link", { name: /Sandbox/ }));

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/sandbox"),
    );
    /* This suite's snapshot has no projects, so the real Sandbox screen
     * shows its own "no system yet" empty state rather than a
     * system header — proof enough that the route actually resolved to
     * the screen, not a stale placeholder. */
    expect(await screen.findByText("No system yet")).toBeDefined();
  });

  it("navigates with Mod+2", async () => {
    const user = userEvent.setup();
    const router = renderApp();
    await screen.findByRole("link", { name: /Session/ });

    await user.keyboard("{Control>}2{/Control}");

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/sandbox"),
    );
  });

  it("toggles the sidebar exactly once on Mod+B", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByRole("link", { name: /Session/ });
    const sidebar = document.querySelector("[data-slot='sidebar'][data-state]");
    expect(sidebar?.getAttribute("data-state")).toBe("expanded");

    await user.keyboard("{Control>}b{/Control}");

    expect(sidebar?.getAttribute("data-state")).toBe("collapsed");
  });

  it("the_sidebar_has_the_four_screens_with_ctrl_1_to_4", async () => {
    renderApp();
    await screen.findByRole("link", { name: /Session/ });

    const expected: [RegExp, string][] = [
      [/Session/, "Ctrl+1"],
      [/Sandbox/, "Ctrl+2"],
      [/Profiles/, "Ctrl+3"],
      [/Usage/, "Ctrl+4"],
    ];

    for (const [name, shortcut] of expected) {
      const link = screen.getByRole("link", { name });
      expect(within(link).getByText(shortcut)).toBeDefined();
    }
  });

  it("sends a stale hash to the current system's Session home", async () => {
    const router = renderApp("/settings");

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/session"),
    );
  });

  it("old_routes_redirect_to_their_new_homes", async () => {
    const cases: [string, string][] = [
      ["/dashboard", "/setup/engine"],
      ["/projects", "/setup/systems"],
      ["/sessions", "/session"],
      ["/tools", "/setup/tools"],
      ["/plugins", "/setup/plugins"],
    ];

    for (const [from, to] of cases) {
      ipc.engine.status.mockResolvedValue(STATUS);
      ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
      const router = createAppRouter(
        createMemoryHistory({ initialEntries: [from] }),
      );
      const { unmount } = render(<App router={router} />);

      await vi.waitFor(() => expect(router.state.location.pathname).toBe(to));

      unmount();
    }
  });

  /* The footer carries what changes while you work. The daemon's
   * version is not that: it belongs to the Engine screen, one click
   * away through the headline beside it. */
  it("puts the engine headline and the live count in the status bar, and leaves the version to the Engine screen", async () => {
    renderApp();

    expect(await screen.findByText("Engine running")).toBeDefined();
    expect(await screen.findByText("1 live")).toBeDefined();
    expect(screen.queryByText(/willied 0\.1\.0/)).toBeNull();
  });

  it("names the first failing part and hides the count when the daemon is stopped", async () => {
    renderApp("/", { ...STATUS, daemon: { state: "stopped" } });

    expect(await screen.findByText("Daemon stopped")).toBeDefined();
    expect(screen.queryByText(/live session/)).toBeNull();
    expect(ipc.projects.snapshot).not.toHaveBeenCalled();
  });

  /* One ground behind the whole window, with the screen floating on it
   * as a rounded card: no rule runs the window's full height, which is
   * what the `inset` variant is for. The margins and the rounding come
   * with it; what this pins is the variant itself. */
  it("the_screen_floats_as_a_card_on_the_sidebar_ground", async () => {
    renderApp();
    await screen.findByRole("link", { name: /Session/ });

    const sidebar = document.querySelector("[data-slot='sidebar'][data-state]");

    expect(sidebar?.getAttribute("data-variant")).toBe("inset");
  });

  /* The generated sidebar is `position: fixed` with `inset-y-0` and a
   * `h-svh` of its own. Inside the body row's containing block that
   * height wins over `bottom: 0`, so the sidebar ends one header plus
   * one status bar below the row: it paints over the status bar's left
   * edge and gives the window a scrollbar it must never have. */
  it("the_sidebar_is_as_tall_as_the_body_row_not_the_window", async () => {
    renderApp();
    await screen.findByRole("link", { name: /Session/ });

    const container = document.querySelector('[data-slot="sidebar-container"]');

    expect(container?.className).toContain("h-full");
    expect(container?.className).not.toContain("h-svh");
  });
});
