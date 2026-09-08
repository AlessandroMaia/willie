import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SystemSelector } from "@/app/shell/system-selector";
import { SidebarProvider } from "@/components/ui/sidebar";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Snapshot } from "@/lib/proto";
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

function project(id: string, name: string): Project {
  return {
    id,
    name,
    slug: name,
    source: `C:\\github\\${name}`,
    workspace: `/home/willie/projects/${name}`,
    branch: "main",
    state: { state: "ready" },
    source_present: true,
    created_at: "1",
    sandbox: {},
  };
}

const WILLIE = project("proj_1", "willie");
const OTHER = project("proj_2", "other-system");

const SNAPSHOT: Snapshot = {
  seq: 1,
  projects: [WILLIE, OTHER],
  jobs: [],
  sessions: [
    {
      id: "sess_1",
      project_id: "proj_2",
      harness: "claude-code",
      workspace: OTHER.workspace,
      state: { state: "running" },
      created_at: "1",
      clients: 1,
    },
  ],
};

const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

/* `useEngineStatus`, `useSnapshot` and `useCurrentSystem` back onto
 * module-level singleton stores; reset them before every test so none
 * carries state from the test before it. */
beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
});

function renderSelector() {
  return render(
    <SidebarProvider>
      <SystemSelector />
    </SidebarProvider>,
  );
}

describe("the system selector", () => {
  it("the_selector_lists_systems_with_a_live_dot_and_filters_by_name", async () => {
    const user = userEvent.setup();
    renderSelector();
    await screen.findByRole("button", { name: "Switch system" });

    await user.click(screen.getByRole("button", { name: "Switch system" }));

    const willieRow = await screen.findByRole("button", { name: /willie/i });
    const otherRow = screen.getByRole("button", { name: /other-system/i });

    /* Only the live project's row carries a "live" dot; the current,
     * not-live one carries "not running" instead. */
    expect(within(otherRow).getByRole("img", { name: "live" })).toBeDefined();
    expect(
      within(willieRow).getByRole("img", { name: "not running" }),
    ).toBeDefined();
    expect(screen.getAllByRole("img", { name: "live" })).toHaveLength(1);

    await user.type(screen.getByPlaceholderText("Find a system…"), "other");

    expect(screen.queryByRole("button", { name: /willie/i })).toBeNull();
    expect(screen.getByRole("button", { name: /other-system/i })).toBeDefined();
  });

  it("choosing_a_system_persists_the_preference", async () => {
    const user = userEvent.setup();
    renderSelector();
    await screen.findByRole("button", { name: "Switch system" });

    await user.click(screen.getByRole("button", { name: "Switch system" }));
    await user.click(
      await screen.findByRole("button", { name: /other-system/i }),
    );

    expect(ipc.ui.setPrefs).toHaveBeenCalledWith({
      current_project: "proj_2",
    });
  });

  it("a_saved_system_wins_over_the_first_even_when_the_snapshot_arrives_first", async () => {
    let resolvePrefs: (prefs: { current_project: string }) => void = () => {};
    ipc.ui.prefs.mockReturnValue(
      new Promise((resolve) => {
        resolvePrefs = resolve;
      }),
    );

    renderSelector();

    /* The snapshot resolves on its own; the preference read is the
     * only thing still pending. Until it settles the trigger says it
     * is loading — never "No system", which is a fact about the
     * registry, and never the first project, which `resolveCurrent`
     * would otherwise pick. */
    expect(await screen.findByText("Loading…")).toBeDefined();
    expect(screen.queryByText("No system")).toBeNull();
    expect(screen.queryByText("willie")).toBeNull();
    expect(screen.queryByText("other-system")).toBeNull();

    resolvePrefs({ current_project: "proj_2" });

    expect(await screen.findByText("other-system")).toBeDefined();
    expect(screen.queryByText("willie")).toBeNull();
  });

  it("a_failed_preference_read_falls_back_to_the_first_system", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    ipc.ui.prefs.mockRejectedValue(new Error("boom"));

    renderSelector();

    expect(await screen.findByText("willie")).toBeDefined();
    expect(warn).toHaveBeenCalledTimes(1);

    warn.mockRestore();
  });
});
