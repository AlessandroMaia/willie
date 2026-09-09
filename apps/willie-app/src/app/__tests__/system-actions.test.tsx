import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  SystemActionDialogs,
  SystemActionRows,
} from "@/app/shell/system-actions";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Snapshot } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { resetStores } from "@/test-support/reset-stores";

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
  projects: {
    snapshot: vi.fn(),
    remove: vi.fn(),
    rename: vi.fn(),
    relocate: vi.fn(),
    updateFromWindows: vi.fn(),
    openInExplorer: vi.fn(),
    openInEditor: vi.fn(),
  },
  onDaemonEvent: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  dialogs: { pickFolder: vi.fn(async () => null) },
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

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

function snapshot(projects: Project[]): Snapshot {
  return { seq: 1, projects, jobs: [], sessions: [] };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.ui.prefs.mockResolvedValue({ current_project: "proj_1" });
  ipc.projects.snapshot.mockResolvedValue(
    snapshot([project(), project({ id: "proj_2", name: "other-system" })]),
  );
});

describe("the system actions", () => {
  /* Remove deletes the workspace clone by default, so the dialog may
   * never outlive the system it named: the current system re-resolves
   * on its own whenever the registry changes underneath. */
  it("a_system_change_closes_the_remove_dialog_instead_of_retargeting_it", async () => {
    const user = userEvent.setup();
    /* The rows live in the selector's popover and the dialogs outside
     * it; rendered together here, the pair is what the shell mounts. */
    render(
      <>
        <SystemActionRows />
        <SystemActionDialogs />
      </>,
    );

    await user.click(await screen.findByRole("button", { name: "Remove" }));

    expect(await screen.findByText("Remove “willie”?")).toBeDefined();

    const current = renderHook(() => useCurrentSystem());
    act(() => {
      current.result.current.setSystem("proj_2");
    });

    await vi.waitFor(() =>
      expect(screen.queryByText("Remove “willie”?")).toBeNull(),
    );
    expect(screen.queryByText("Remove “other-system”?")).toBeNull();
    expect(ipc.projects.remove).not.toHaveBeenCalled();
  });
});
