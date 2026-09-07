import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Toaster } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { Snapshot } from "@/lib/proto";

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

const LAUNCH_FAILED = {
  code: "editor_launch_failed",
  message: "the OS refused to start VS Code",
  remediation: "try opening the workspace from VS Code directly",
};

/* The bridge is the only I/O in the frontend and the only module a test
 * fakes. Every function this screen may call on mount is here; the rest
 * are inert. */
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
  tools: { install: vi.fn() },
  sandbox: { catalogue: vi.fn(async () => []) },
  dialogs: { pickFolder: vi.fn(async () => null) },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
}));

vi.mock("@/lib/ipc", () => ipc);

let ProjectsScreen: typeof import("@/features/projects/projects-screen").ProjectsScreen;

/* `useSnapshot` backs onto a module-level singleton store, so a snapshot
 * left behind by one render would still be there for the next test's
 * first synchronous render. Reset the module graph and re-import the
 * screen fresh for every case. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.editorAvailable.mockResolvedValue(true);
  ipc.projects.openInEditor.mockResolvedValue(undefined);
  ({ ProjectsScreen } = await import("@/features/projects/projects-screen"));
});

function renderScreen() {
  render(
    <TooltipProvider>
      <Toaster>
        <ProjectsScreen />
      </Toaster>
    </TooltipProvider>,
  );
}

describe("opening a project in VS Code", () => {
  it("calls the bridge with the project's workspace", async () => {
    const user = userEvent.setup();
    renderScreen();
    await screen.findByText("willie");

    await user.click(screen.getByRole("button", { name: /More actions/ }));
    await user.click(
      await screen.findByRole("menuitem", { name: "Open in VS Code" }),
    );

    expect(ipc.projects.openInEditor).toHaveBeenCalledWith(
      "/home/willie/projects/willie",
    );
  });

  it("disables the menu item when the host reports no editor", async () => {
    ipc.editorAvailable.mockResolvedValue(false);
    const user = userEvent.setup();
    renderScreen();
    await screen.findByText("willie");

    await user.click(screen.getByRole("button", { name: /More actions/ }));
    const item = await screen.findByRole("menuitem", {
      name: "Open in VS Code",
    });

    expect(item.hasAttribute("data-disabled")).toBe(true);
  });

  it("sets the row problem when the launch is rejected", async () => {
    ipc.projects.openInEditor.mockRejectedValue(LAUNCH_FAILED);
    const user = userEvent.setup();
    renderScreen();
    await screen.findByText("willie");

    await user.click(screen.getByRole("button", { name: /More actions/ }));
    await user.click(
      await screen.findByRole("menuitem", { name: "Open in VS Code" }),
    );

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("editor_launch_failed");
  });
});
