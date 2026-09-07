import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Change, ProfileSummary, Project, Snapshot } from "@/lib/proto";

/* The bridge is the only I/O in the frontend and the only module a test
 * fakes. Mirrors the shape of every other panel/screen test's hoisted
 * fake (see shell.test.tsx, plugins-screen.test.tsx). */
const ipc = vi.hoisted(() => ({
  profiles: {
    list: vi.fn(),
    create: vi.fn(),
    readFragment: vi.fn(),
    writeFragment: vi.fn(),
    check: vi.fn(),
    apply: vi.fn(),
    setRemote: vi.fn(),
    push: vi.fn(),
    pull: vi.fn(),
  },
  plugins: { list: vi.fn(), enable: vi.fn(), disable: vi.fn() },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

let ProfilesPanel: typeof import("@/plugins/profiles/profiles-panel").ProfilesPanel;

const emptySnapshot = (): Snapshot => ({
  seq: 1,
  projects: [],
  jobs: [],
  sessions: [],
});

function project(overrides: Partial<Project> = {}): Project {
  return {
    id: "proj_1",
    name: "Acme",
    slug: "acme",
    source: "C:\\src\\acme",
    workspace: "/home/willie/projects/acme",
    branch: "main",
    state: { state: "ready" },
    source_present: true,
    created_at: "1",
    sandbox: {},
    ...overrides,
  };
}

function summary(overrides: Partial<ProfileSummary> = {}): ProfileSummary {
  return { name: "acme", fragments_active: [], ...overrides };
}

/* `useSnapshot` (a module-level singleton store) and the panel's own
 * component both need a fresh module graph, same reasoning as
 * plugins-screen.test.tsx: a snapshot or a list left behind by one
 * test's render would still be there for the next test's first
 * synchronous render. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(emptySnapshot());
  ipc.profiles.readFragment.mockResolvedValue({ content: "" });
  ({ ProfilesPanel } = await import("@/plugins/profiles/profiles-panel"));
});

/** Renders the panel and selects `name` from the profile list, then
 * waits for its three fragment editors to finish loading
 * (`readFragment` resolves for each) before the caller interacts with
 * anything below them. */
async function renderAndSelectProfile(name: string) {
  const user = userEvent.setup();
  render(<ProfilesPanel />);
  await user.click(
    await screen.findByRole("button", { name: `Select ${name}` }),
  );
  await screen.findByLabelText("Settings");
  return user;
}

describe("ProfilesPanel", () => {
  it("lists profiles from profile.list and creates one", async () => {
    ipc.profiles.list.mockResolvedValue([summary({ name: "acme" })]);
    ipc.profiles.create.mockResolvedValue(summary({ name: "beta" }));
    const user = userEvent.setup();

    render(<ProfilesPanel />);

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

  it("runs check against a chosen project and shows the changes", async () => {
    ipc.profiles.list.mockResolvedValue([summary()]);
    ipc.projects.snapshot.mockResolvedValue({
      ...emptySnapshot(),
      projects: [project()],
    });
    const changes: Change[] = [
      { path: "settings.json", kind: "merge", after: "{}" },
    ];
    ipc.profiles.check.mockResolvedValue({ changes });

    const user = await renderAndSelectProfile("acme");
    await user.selectOptions(screen.getByLabelText("Project"), "proj_1");
    await user.click(screen.getByRole("button", { name: "Check" }));

    expect(ipc.profiles.check).toHaveBeenCalledWith("acme", "proj_1");
    expect(await screen.findByText("settings.json")).toBeDefined();
    expect(screen.getByText(/1 to merge/)).toBeDefined();
  });

  it("applies and shows the backup path", async () => {
    ipc.profiles.list.mockResolvedValue([summary()]);
    ipc.projects.snapshot.mockResolvedValue({
      ...emptySnapshot(),
      projects: [project()],
    });
    const changes: Change[] = [
      { path: "settings.json", kind: "merge", after: "{}" },
    ];
    ipc.profiles.check.mockResolvedValue({ changes });
    ipc.profiles.apply.mockResolvedValue({
      changes,
      backup_path: "/home/willie/projects/acme/.willie-bak/1",
    });

    const user = await renderAndSelectProfile("acme");
    await user.selectOptions(screen.getByLabelText("Project"), "proj_1");
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
});
