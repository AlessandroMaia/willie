import { createMemoryHistory } from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "@/lib/ipc";
import type { Snapshot } from "@/lib/proto";

/* The Session screen (Task 12) hosts a real `SessionTerminal` for every
 * live session, and this suite resets the module graph for every test
 * (see the `beforeEach` below) — without this, the real `@xterm/xterm`
 * would be re-imported and re-initialised from scratch on every case.
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

const SNAPSHOT: Snapshot = { seq: 1, projects: [], jobs: [], sessions: [] };

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Every function the shell or any `/setup/*` screen may
 * call on mount is here; the rest are inert (mirrors shell.test.tsx). */
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
  profiles: {
    list: vi.fn(async () => []),
    create: vi.fn(),
    readFragment: vi.fn(),
    writeFragment: vi.fn(),
    check: vi.fn(),
    apply: vi.fn(),
    setRemote: vi.fn(),
    push: vi.fn(),
    pull: vi.fn(),
  },
  usage: {
    snapshot: vi.fn(async () => ({
      providers: [],
      sessions: [],
      projects: [],
      fetched_at: "",
    })),
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

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
  }),
}));

let App: typeof import("@/app/app").App;
let createAppRouter: typeof import("@/app/router").createAppRouter;

/* `useEngineStatus`, `useSnapshot` and `useCurrentSystem` back onto
 * module-level singleton stores; reset the module graph before every
 * test so none of them carries state from the test before it (same
 * reasoning as shell.test.tsx). */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ({ App } = await import("@/app/app"));
  ({ createAppRouter } = await import("@/app/router"));
});

function renderApp(path = "/session") {
  const router = createAppRouter(
    createMemoryHistory({ initialEntries: [path] }),
  );
  render(<App router={router} />);
  return router;
}

describe("the setup drawer", () => {
  it("the_drawer_lists_the_six_setup_entries_and_navigates", async () => {
    const user = userEvent.setup();
    const router = renderApp();
    await screen.findByRole("link", { name: /Session/ });

    await user.click(screen.getByRole("button", { name: "Open setup" }));

    const labels = [
      "Engine",
      "Tools",
      "Plugins",
      "Profile store",
      "Systems",
      "Settings",
    ];
    for (const label of labels) {
      expect(
        screen.getByRole("button", { name: new RegExp(label) }),
      ).toBeDefined();
    }

    await user.click(screen.getByRole("button", { name: /Tools/ }));

    await vi.waitFor(() =>
      expect(router.state.location.pathname).toBe("/setup/tools"),
    );
    /* The drawer closed: none of its rows are on screen any more. */
    expect(screen.queryByRole("button", { name: /Engine/ })).toBeNull();
  });
});
