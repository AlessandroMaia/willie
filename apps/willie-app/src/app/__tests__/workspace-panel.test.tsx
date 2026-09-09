import { createMemoryHistory } from "@tanstack/react-router";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "@/app/app";
import { createAppRouter } from "@/app/router";
import type { EngineStatus } from "@/lib/ipc";
import type { Snapshot } from "@/lib/proto";
import { resetStores } from "@/test-support/reset-stores";

/* jsdom has no canvas for `@xterm/xterm`; the same fake the other shell
 * suites use stands in. */
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
    image_version: "0.1.0",
  },
  doctor: { checks: [] },
  image_available: true,
};

const SNAPSHOT: Snapshot = {
  seq: 1,
  projects: [
    {
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
    },
  ],
  jobs: [],
  sessions: [],
};

const SHELL = {
  id: "sess_shell_1",
  project_id: "proj_1",
  harness: "shell",
  workspace: "/home/willie/projects/willie",
  kind: "shell" as const,
  state: { state: "running" as const },
  created_at: "1",
  clients: 0,
};

const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: {
    snapshot: vi.fn(),
    tree: vi.fn(),
    readFile: vi.fn(),
    roots: vi.fn(async () => []),
    openInExplorer: vi.fn(),
    openInEditor: vi.fn(),
    updateFromWindows: vi.fn(),
    rename: vi.fn(),
    relocate: vi.fn(),
    remove: vi.fn(),
    discover: vi.fn(async () => []),
    setRoots: vi.fn(),
  },
  sessions: {
    open: vi.fn(async () => ({ session: SHELL })),
    resume: vi.fn(async () => ({ session: SHELL })),
    attach: vi.fn(async () => {}),
    stop: vi.fn(async () => {}),
  },
  sessionTerminal: {
    open: vi.fn(async () => {}),
    input: vi.fn(async () => {}),
    resize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  },
  tools: { list: vi.fn(async () => ({ tools: [] })) },
  plugins: { list: vi.fn(async () => []), enable: vi.fn(), disable: vi.fn() },
  profiles: {
    list: vi.fn(async () => []),
    check: vi.fn(),
    apply: vi.fn(),
    create: vi.fn(),
    readFragment: vi.fn(),
    writeFragment: vi.fn(),
    setRemote: vi.fn(),
    push: vi.fn(),
    pull: vi.fn(),
  },
  sandbox: { catalogue: vi.fn(async () => []) },
  usage: { snapshot: vi.fn() },
  dialogs: { pickFolder: vi.fn(async () => null) },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
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

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.ui.prefs.mockResolvedValue({ current_project: "proj_1" });
  ipc.projects.tree.mockResolvedValue({ branch: "main", entries: [] });
  ipc.usage.snapshot.mockResolvedValue({
    providers: [],
    sessions: [],
    projects: [],
    fetched_at: "1",
  });
});

function renderApp(path = "/session") {
  render(
    <App
      router={createAppRouter(createMemoryHistory({ initialEntries: [path] }))}
    />,
  );
}

async function openPanel(user: ReturnType<typeof userEvent.setup>) {
  /* The toggle is disabled until the current system resolves, and the
   * preference read that resolves it is asynchronous. */
  const toggle = await screen.findByRole("button", {
    name: "Workspace panel",
  });
  await vi.waitFor(() => expect(toggle.hasAttribute("disabled")).toBe(false));
  await user.click(toggle);

  return toggle;
}

describe("the workspace panel", () => {
  /* The panel belongs to the shell, not to a route: that is what puts
   * it on all four screens, and what took it out of the Session
   * screen, whose business is agent conversations. */
  it("the_panel_is_mounted_by_the_shell_not_by_the_session_screen", async () => {
    const user = userEvent.setup();
    renderApp("/usage");
    await screen.findByRole("link", { name: /Usage/ });

    await openPanel(user);

    const panel = await screen.findByRole("complementary", {
      name: "Workspace panel",
    });
    const inset = document.querySelector('[data-slot="sidebar-inset"]');

    expect(inset?.contains(panel)).toBe(true);
  });

  it.each(["/session", "/sandbox", "/profiles", "/usage"])(
    "the_panel_opens_from_the_header_on_%s",
    async (path) => {
      const user = userEvent.setup();
      renderApp(path);

      const toggle = await openPanel(user);

      expect(toggle.getAttribute("aria-pressed")).toBe("true");
      expect(
        await screen.findByRole("complementary", { name: "Workspace panel" }),
      ).toBeDefined();
    },
  );

  it("the_tree_lists_the_workspace_with_its_git_status", async () => {
    ipc.projects.tree.mockResolvedValue({
      branch: "main",
      entries: [
        { name: "crates", kind: "dir", git: "M" },
        { name: "AGENTS.md", kind: "file" },
      ],
    });
    const user = userEvent.setup();
    renderApp("/session");

    await openPanel(user);

    expect(await screen.findByText("crates")).toBeDefined();
    expect(screen.getByText("AGENTS.md")).toBeDefined();
    expect(screen.getByText("main")).toBeDefined();
  });

  /* The tree had its only control in the Session screen's tab strip.
   * The header owns it now, so the strip has no business with it. */
  it("the_tab_strip_has_no_tree_toggle", async () => {
    renderApp("/session");
    await screen.findByRole("link", { name: /Session/ });

    expect(screen.queryByRole("button", { name: "Workspace tree" })).toBeNull();
  });

  /* One shell per system, resolved by kind and system rather than by
   * who started it: a shell already running is adopted, never doubled. */
  it("the_shell_tab_reuses_the_systems_live_shell", async () => {
    ipc.projects.snapshot.mockResolvedValue({
      ...SNAPSHOT,
      sessions: [SHELL],
    });
    const user = userEvent.setup();
    renderApp("/session");
    await openPanel(user);

    await user.click(screen.getByRole("tab", { name: "Shell" }));

    await vi.waitFor(() =>
      expect(ipc.sessionTerminal.open).toHaveBeenCalledWith("sess_shell_1"),
    );
    expect(ipc.sessions.open).not.toHaveBeenCalled();
  });

  it("the_shell_tab_starts_one_when_the_system_has_none", async () => {
    const user = userEvent.setup();
    renderApp("/session");
    await openPanel(user);

    await user.click(screen.getByRole("tab", { name: "Shell" }));

    await vi.waitFor(() =>
      expect(ipc.sessions.open).toHaveBeenCalledWith("proj_1", "shell"),
    );
  });

  /* Opening the panel is not asking for a shell. Until the tab is
   * picked, nothing is started. */
  it("the_panel_starts_no_shell_until_the_tab_is_opened", async () => {
    const user = userEvent.setup();
    renderApp("/session");

    await openPanel(user);

    expect(ipc.sessions.open).not.toHaveBeenCalled();
  });

  it("a_refused_shell_shows_its_remediation_not_a_terminal", async () => {
    ipc.sessions.open.mockRejectedValue({
      code: "sandbox_backend_missing",
      message: "bwrap is not available in the distribution.",
      remediation: "Run `willie doctor` and reinstall the distribution.",
    });
    const user = userEvent.setup();
    renderApp("/session");
    await openPanel(user);

    await user.click(screen.getByRole("tab", { name: "Shell" }));

    expect(await screen.findByText(/reinstall the distribution/)).toBeDefined();
    expect(ipc.sessionTerminal.open).not.toHaveBeenCalled();
  });
});
