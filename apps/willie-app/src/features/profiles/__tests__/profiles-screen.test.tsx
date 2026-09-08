import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProfilesScreen } from "@/features/profiles/profiles-screen";
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

function project(): Project {
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
  };
}

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. `useCurrentSystem` needs the engine/projects/ui triad
 * (mirrors system-selector.test.tsx); the screen itself needs
 * `profiles.list`/`check`/`apply` and `plugins.enable`. */
const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
  profiles: { list: vi.fn(), check: vi.fn(), apply: vi.fn() },
  plugins: { enable: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

function snapshot(projects: Project[] = [project()]): Snapshot {
  return { seq: 1, projects, jobs: [], sessions: [] };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(snapshot());
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.profiles.list.mockResolvedValue([{ name: "acme", fragments_active: [] }]);
});

describe("ProfilesScreen", () => {
  it("the_profiles_screen_applies_to_the_current_system", async () => {
    const user = userEvent.setup();
    ipc.profiles.check.mockResolvedValue({
      changes: [{ path: "settings.json", kind: "merge", after: "{}" }],
    });
    ipc.profiles.apply.mockResolvedValue({
      changes: [{ path: "settings.json", kind: "merge", after: "{}" }],
      backup_path: "/home/willie/projects/willie/.willie-bak/1",
    });

    render(<ProfilesScreen />);
    await screen.findByRole("option", { name: "acme" });
    await user.selectOptions(screen.getByLabelText("Profile"), "acme");
    await user.click(screen.getByRole("button", { name: "Check" }));

    expect(ipc.profiles.check).toHaveBeenCalledWith("acme", "proj_1");

    await screen.findByText("settings.json");
    await user.click(screen.getByRole("button", { name: "Confirm & apply" }));

    expect(ipc.profiles.apply).toHaveBeenCalledWith("acme", "proj_1");
  });

  it("a_disabled_profiles_plugin_offers_to_enable_it_for_this_system", async () => {
    const user = userEvent.setup();
    ipc.profiles.list
      .mockRejectedValueOnce({
        code: "plugin_disabled",
        message: "the profile plugin is not enabled",
        remediation: "",
      })
      .mockResolvedValue([{ name: "acme", fragments_active: [] }]);
    ipc.plugins.enable.mockResolvedValue({
      id: "profile",
      name: "Configuration profiles",
      scope: "per_project",
      enabled: { per_project: ["proj_1"] },
      degraded: false,
    });

    render(<ProfilesScreen />);

    expect(
      await screen.findByText("Profiles are off for this system"),
    ).toBeDefined();
    expect(ipc.profiles.list).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: /Enable/ }));

    expect(ipc.plugins.enable).toHaveBeenCalledWith("profile", "proj_1");
    expect(await screen.findByLabelText("Profile")).toBeDefined();
    expect(ipc.profiles.list.mock.calls.length).toBeGreaterThan(1);
  });

  it("a_refusal_that_is_not_plugin_disabled_is_a_problem_not_the_enable_state", async () => {
    ipc.profiles.list.mockRejectedValue({
      code: "daemon_transport",
      message: "the daemon pipe broke",
      remediation: "click Run doctor",
    });

    render(<ProfilesScreen />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("daemon_transport");
    expect(screen.queryByText("Profiles are off for this system")).toBeNull();
  });

  it("shows a skeleton, not the empty state, while the current-system preference is still loading", async () => {
    let resolvePrefs: (prefs: { current_project: string | null }) => void =
      () => {};
    ipc.ui.prefs.mockReturnValue(
      new Promise((resolve) => {
        resolvePrefs = resolve;
      }),
    );

    render(<ProfilesScreen />);

    expect(screen.queryByText("No system yet")).toBeNull();
    expect(screen.queryByLabelText("Profile")).toBeNull();

    resolvePrefs({ current_project: "proj_1" });
    expect(await screen.findByLabelText("Profile")).toBeDefined();
  });

  it("offers to add a system when there is none yet", async () => {
    ipc.projects.snapshot.mockResolvedValue(snapshot([]));

    render(<ProfilesScreen />);

    expect(await screen.findByText("No system yet")).toBeDefined();
    expect(ipc.profiles.list).not.toHaveBeenCalled();
  });
});
