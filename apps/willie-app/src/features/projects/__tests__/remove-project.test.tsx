import { render, screen, within } from "@testing-library/react";
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
      source: "C:githubwillie",
      workspace: "/home/willie/projects/willie",
      branch: "main",
      state: { state: "ready" },
      source_present: true,
      created_at: "1",
    },
  ],
  jobs: [],
  sessions: [],
};

const BUSY = {
  code: "project_busy",
  message: "A job is already running",
  remediation: "Wait for it",
};

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Every function this screen may call on mount is here;
 * the rest are inert. */
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
  dialogs: { pickFolder: vi.fn(async () => null) },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

let ProjectsScreen: typeof import("@/features/projects/projects-screen").ProjectsScreen;

/* `useSnapshot` backs onto a module-level singleton store, so a
 * snapshot left behind by one render would still be there for the
 * next test's first synchronous render. Reset the module graph and
 * re-import the screen fresh for every case. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.projects.remove.mockRejectedValue(BUSY);
  ({ ProjectsScreen } = await import("@/features/projects/projects-screen"));
});

describe("removing a project", () => {
  it("keeps the dialog open and shows a synchronous refusal inside it", async () => {
    const user = userEvent.setup();
    render(
      <TooltipProvider>
        <Toaster>
          <ProjectsScreen />
        </Toaster>
      </TooltipProvider>,
    );
    await screen.findByText("willie");

    await user.click(screen.getByRole("button", { name: /More actions/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Remove…" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("project_busy");
    /* The popup stays mounted through an exit transition that never
     * runs in jsdom, so being in the document does not say the dialog
     * is still open — `data-open` does. */
    const stillOpen = screen.getByRole("alertdialog");
    expect(stillOpen.hasAttribute("data-open")).toBe(true);
    expect(stillOpen.contains(alert)).toBe(true);
  });
});
