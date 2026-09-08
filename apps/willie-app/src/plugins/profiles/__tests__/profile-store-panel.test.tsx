import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProfileSummary } from "@/lib/proto";
import { ProfileStorePanel } from "@/plugins/profiles/profile-store-panel";
import { resetStores } from "@/test-support/reset-stores";

/* The bridge is the only I/O in the frontend and the only module a test
 * fakes. Mirrors the shape of every other panel/screen test's hoisted
 * fake (see shell.test.tsx, plugins-screen.test.tsx). */
const ipc = vi.hoisted(() => ({
  profiles: {
    list: vi.fn(),
    create: vi.fn(),
    readFragment: vi.fn(),
    writeFragment: vi.fn(),
    setRemote: vi.fn(),
    push: vi.fn(),
    pull: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

function summary(overrides: Partial<ProfileSummary> = {}): ProfileSummary {
  return { name: "acme", fragments_active: [], ...overrides };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.profiles.readFragment.mockResolvedValue({ content: "" });
});

/** Renders the panel and selects `name` from the profile list, then
 * waits for its three fragment editors to finish loading
 * (`readFragment` resolves for each) before the caller interacts with
 * anything below them. */
async function renderAndSelectProfile(name: string) {
  const user = userEvent.setup();
  render(<ProfileStorePanel />);
  await user.click(
    await screen.findByRole("button", { name: `Select ${name}` }),
  );
  await screen.findByLabelText("Settings");
  return user;
}

describe("ProfileStorePanel", () => {
  it("lists profiles from profile.list and creates one", async () => {
    ipc.profiles.list.mockResolvedValue([summary({ name: "acme" })]);
    ipc.profiles.create.mockResolvedValue(summary({ name: "beta" }));
    const user = userEvent.setup();

    render(<ProfileStorePanel />);

    expect(await screen.findByText("acme")).toBeDefined();

    await user.type(screen.getByLabelText("New profile"), "beta");
    await user.click(screen.getByRole("button", { name: "New profile" }));

    expect(ipc.profiles.create).toHaveBeenCalledWith("beta");
    expect(await screen.findByText("beta")).toBeDefined();
  });

  it("edits a fragment and saves", async () => {
    ipc.profiles.list.mockResolvedValue([summary()]);
    ipc.profiles.readFragment.mockImplementation((_name, fragment) =>
      Promise.resolve({
        content: fragment === "settings" ? "old settings" : "",
      }),
    );
    ipc.profiles.writeFragment.mockResolvedValue({ content: "new settings" });

    const user = await renderAndSelectProfile("acme");
    const settingsBox = screen.getByLabelText(
      "Settings",
    ) as HTMLTextAreaElement;
    expect(settingsBox.value).toBe("old settings");

    await user.clear(settingsBox);
    await user.type(settingsBox, "new settings");
    await user.click(screen.getByRole("button", { name: "Save Settings" }));

    expect(ipc.profiles.writeFragment).toHaveBeenCalledWith(
      "acme",
      "settings",
      "new settings",
    );
  });

  it("sets a remote and surfaces a profile_sync_conflict on pull", async () => {
    ipc.profiles.list.mockResolvedValue([summary()]);
    ipc.profiles.setRemote.mockResolvedValue({});
    ipc.profiles.pull.mockRejectedValue({
      code: "profile_sync_conflict",
      message: "profile `acme` has diverged from its remote",
      remediation: "resolve it in a terminal, then pull again",
    });

    const user = await renderAndSelectProfile("acme");
    await user.type(
      screen.getByLabelText("Remote URL"),
      "git@host:profiles/acme.git",
    );
    await user.click(screen.getByRole("button", { name: "Set remote" }));

    expect(ipc.profiles.setRemote).toHaveBeenCalledWith(
      "acme",
      "git@host:profiles/acme.git",
    );

    await user.click(screen.getByRole("button", { name: "Pull" }));

    expect(await screen.findByText("profile_sync_conflict")).toBeDefined();
    expect(
      screen.getByText(/profile `acme` has diverged from its remote/),
    ).toBeDefined();
  });

  it("never offers a project selector or an apply flow", async () => {
    ipc.profiles.list.mockResolvedValue([summary()]);

    await renderAndSelectProfile("acme");

    expect(screen.queryByLabelText("Project")).toBeNull();
    expect(screen.queryByRole("button", { name: "Check" })).toBeNull();
  });
});
