import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Change, ProfileSummary } from "@/lib/proto";

/* The bridge is the only I/O in the frontend and the only module a test
 * fakes. Mirrors the shape of every other panel test's hoisted fake
 * (see profile-store-panel.test.tsx). */
const ipc = vi.hoisted(() => ({
  profiles: {
    list: vi.fn(),
    check: vi.fn(),
    apply: vi.fn(),
  },
  plugins: { enable: vi.fn(), disable: vi.fn(), list: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

let ApplyPanel: typeof import("@/plugins/profiles/apply-panel").ApplyPanel;

function summary(overrides: Partial<ProfileSummary> = {}): ProfileSummary {
  return { name: "acme", fragments_active: [], ...overrides };
}

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.profiles.list.mockResolvedValue([summary()]);
  ({ ApplyPanel } = await import("@/plugins/profiles/apply-panel"));
});

describe("ApplyPanel", () => {
  it("enables the profiles plugin for the projectId prop, not a chosen one", async () => {
    const user = userEvent.setup();
    ipc.plugins.enable.mockResolvedValue({
      id: "profile",
      name: "Configuration profiles",
      scope: "per_project",
      enabled: { per_project: ["proj_1"] },
      degraded: false,
    });

    render(<ApplyPanel projectId="proj_1" />);
    await screen.findByText("acme");

    await user.click(
      screen.getByRole("button", { name: /Enable profiles for this system/ }),
    );

    expect(ipc.plugins.enable).toHaveBeenCalledWith("profile", "proj_1");
  });

  it("runs check against the projectId prop for the chosen profile", async () => {
    const user = userEvent.setup();
    const changes: Change[] = [
      { path: "settings.json", kind: "merge", after: "{}" },
    ];
    ipc.profiles.check.mockResolvedValue({ changes });

    render(<ApplyPanel projectId="proj_1" />);
    await screen.findByRole("option", { name: "acme" });
    await user.selectOptions(screen.getByLabelText("Profile"), "acme");
    await user.click(screen.getByRole("button", { name: "Check" }));

    expect(ipc.profiles.check).toHaveBeenCalledWith("acme", "proj_1");
    expect(await screen.findByText("settings.json")).toBeDefined();
    expect(screen.getByText(/1 to merge/)).toBeDefined();
  });

  it("applies against the projectId prop and shows the backup path", async () => {
    const user = userEvent.setup();
    const changes: Change[] = [
      { path: "settings.json", kind: "merge", after: "{}" },
    ];
    ipc.profiles.check.mockResolvedValue({ changes });
    ipc.profiles.apply.mockResolvedValue({
      changes,
      backup_path: "/home/willie/projects/acme/.willie-bak/1",
    });

    render(<ApplyPanel projectId="proj_1" />);
    await screen.findByRole("option", { name: "acme" });
    await user.selectOptions(screen.getByLabelText("Profile"), "acme");
    await user.click(screen.getByRole("button", { name: "Check" }));
    await screen.findByText("settings.json");
    await user.click(screen.getByRole("button", { name: "Confirm & apply" }));

    expect(ipc.profiles.apply).toHaveBeenCalledWith("acme", "proj_1");
    expect(
      await screen.findByText(
        /Backup saved at \/home\/willie\/projects\/acme\/\.willie-bak\/1/,
      ),
    ).toBeDefined();
  });

  it("holds Check until a profile is chosen", async () => {
    render(<ApplyPanel projectId="proj_1" />);
    await screen.findByText("acme");

    expect(
      (screen.getByRole("button", { name: "Check" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });
});
