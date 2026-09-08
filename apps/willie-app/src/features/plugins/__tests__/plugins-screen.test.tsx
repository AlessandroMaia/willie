import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PluginStatus, Snapshot } from "@/lib/proto";

const emptySnapshot = (): Snapshot => ({
  seq: 1,
  projects: [],
  jobs: [],
  sessions: [],
});

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Mirrors the shape of every other screen test's hoisted
 * fake (see shell.test.tsx, tools-screen.test.tsx). */
const ipc = vi.hoisted(() => ({
  plugins: { list: vi.fn(), enable: vi.fn(), disable: vi.fn() },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

let PluginsScreen: typeof import("@/features/plugins/plugins-screen").PluginsScreen;

/* `useSnapshot` backs onto a module-level singleton store, so a
 * snapshot left behind by one render would still be there for the
 * next test's first synchronous render. Reset the module graph and
 * re-import the screen fresh for every case. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(emptySnapshot());
  ({ PluginsScreen } = await import("@/features/plugins/plugins-screen"));
});

function globalPlugin(overrides: Partial<PluginStatus> = {}): PluginStatus {
  return {
    id: "usage",
    name: "Usage",
    scope: "global",
    enabled: { global: true },
    degraded: false,
    ...overrides,
  };
}

function perProjectPlugin(overrides: Partial<PluginStatus> = {}): PluginStatus {
  return {
    id: "profiles",
    name: "Profiles",
    scope: "per_project",
    enabled: { per_project: ["proj_1"] },
    degraded: false,
    ...overrides,
  };
}

/* The real profiles plugin's id is singular — "profile" — matching
 * `crates/willie-plugins/profiles/src/lib.rs`'s manifest, unlike the
 * generic `perProjectPlugin` fixture above (its "profiles" id is
 * incidental, not a stand-in for the real plugin). */
function profilesPlugin(overrides: Partial<PluginStatus> = {}): PluginStatus {
  return {
    id: "profile",
    name: "Configuration profiles",
    scope: "per_project",
    enabled: { per_project: ["proj_1"] },
    degraded: false,
    ...overrides,
  };
}

describe("PluginsScreen", () => {
  it("lists plugins from plugin.list on mount", async () => {
    ipc.plugins.list.mockResolvedValue([globalPlugin(), perProjectPlugin()]);

    render(<PluginsScreen />);

    expect(await screen.findByText("Usage")).toBeDefined();
    expect(screen.getByText("Profiles")).toBeDefined();
    expect(ipc.plugins.list).toHaveBeenCalledTimes(1);
  });

  it("toggles an enabled global plugin off", async () => {
    const user = userEvent.setup();
    ipc.plugins.list.mockResolvedValue([
      globalPlugin({
        enabled: { global: true },
      }),
    ]);
    ipc.plugins.disable.mockResolvedValue(
      globalPlugin({ enabled: { global: false } }),
    );

    render(<PluginsScreen />);
    const toggle = await screen.findByRole("checkbox", { name: /Enabled/ });
    expect(toggle.hasAttribute("data-checked")).toBe(true);

    await user.click(toggle);

    expect(ipc.plugins.disable).toHaveBeenCalledWith("usage");
    expect(ipc.plugins.enable).not.toHaveBeenCalled();
  });

  it("toggles a disabled global plugin on", async () => {
    const user = userEvent.setup();
    ipc.plugins.list.mockResolvedValue([
      globalPlugin({
        enabled: { global: false },
      }),
    ]);
    ipc.plugins.enable.mockResolvedValue(
      globalPlugin({ enabled: { global: true } }),
    );

    render(<PluginsScreen />);
    const toggle = await screen.findByRole("checkbox", { name: /Enabled/ });
    expect(toggle.hasAttribute("data-checked")).toBe(false);

    await user.click(toggle);

    expect(ipc.plugins.enable).toHaveBeenCalledWith("usage");
    expect(ipc.plugins.disable).not.toHaveBeenCalled();
  });

  it("shows a note instead of a toggle for a per-project plugin", async () => {
    ipc.plugins.list.mockResolvedValue([perProjectPlugin()]);

    render(<PluginsScreen />);

    expect(await screen.findByText("Profiles")).toBeDefined();
    expect(screen.getByText(/enabled per project/i)).toBeDefined();
    expect(screen.queryByRole("checkbox")).toBeNull();
  });

  it("shows an error chip for a degraded plugin", async () => {
    ipc.plugins.list.mockResolvedValue([globalPlugin({ degraded: true })]);

    render(<PluginsScreen />);

    expect(await screen.findByText("Usage")).toBeDefined();
    expect(screen.getByText("error")).toBeDefined();
  });

  it("does not show an error chip for a healthy plugin", async () => {
    ipc.plugins.list.mockResolvedValue([globalPlugin({ degraded: false })]);

    render(<PluginsScreen />);

    expect(await screen.findByText("Usage")).toBeDefined();
    expect(screen.queryByText("error")).toBeNull();
  });

  it("the_plugins_screen_no_longer_mounts_the_panels", async () => {
    ipc.plugins.list.mockResolvedValue([
      profilesPlugin(),
      globalPlugin({ enabled: { global: true } }),
    ]);

    render(<PluginsScreen />);

    expect(await screen.findByText("Configuration profiles")).toBeDefined();
    expect(await screen.findByText("Usage")).toBeDefined();
    /* Neither panel's own heading appears: this screen keeps only the
     * plugin list and its enable/disable state now — applying a
     * profile and viewing usage moved to each system's own screens. */
    expect(screen.queryByRole("heading", { name: "Profiles" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Session usage" })).toBeNull();
  });
});
