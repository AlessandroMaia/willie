import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TreeDrawer } from "@/app/shell/tree-drawer";
import { TONE_TEXT } from "@/components/tone";
import { FilePreview } from "@/features/session/file-preview";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Snapshot, TreeEntry } from "@/lib/proto";
import { useCurrentSystem } from "@/store/use-current-system";
import { useTreeDrawer } from "@/store/use-tree-drawer";
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

/* A second system, only used by `switching_systems_closes_the_open_preview`
 * — `resolveCurrent` picks the first project when nothing is preferred,
 * so `proj_1` starts current and this is the one switched to. */
function otherProject(): Project {
  return {
    id: "proj_2",
    name: "other",
    slug: "other",
    source: "C:\\github\\other",
    workspace: "/home/willie/projects/other",
    branch: "main",
    state: { state: "ready" },
    source_present: true,
    created_at: "1",
    sandbox: {},
  };
}

function snapshot(): Snapshot {
  return {
    seq: 1,
    projects: [project(), otherProject()],
    jobs: [],
    sessions: [],
  };
}

const ROOT_ENTRIES: TreeEntry[] = [
  { name: "src", kind: "dir" },
  { name: "README.md", kind: "file", git: "M" },
  { name: "NOTES.txt", kind: "file", git: "Z" },
];

const SRC_ENTRIES: TreeEntry[] = [{ name: "index.ts", kind: "file" }];

/* The bridge is the only I/O the drawer or the preview may reach:
 * everything `useCurrentSystem` needs on mount plus the tree, file
 * and editor calls. */
const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: {
    snapshot: vi.fn(),
    tree: vi.fn(),
    readFile: vi.fn(),
    openInEditor: vi.fn(),
  },
  onDaemonEvent: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  ui: {
    prefs: vi.fn(async () => ({ current_project: null })),
    setPrefs: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

/* A probe rendered alongside the real components: the toggle button
 * `session-tabs.tsx` normally hosts and the system selector's own
 * system switch, standing in here so the drawer can be opened and the
 * current system changed without pulling in the whole shell. */
function Harness() {
  const { toggle } = useTreeDrawer();
  const { setSystem } = useCurrentSystem();
  return (
    <>
      <button type="button" onClick={toggle}>
        toggle tree
      </button>
      <button type="button" onClick={() => setSystem("proj_2")}>
        switch system
      </button>
      <TreeDrawer />
      <FilePreview />
    </>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.projects.snapshot.mockResolvedValue(snapshot());
  ipc.editorAvailable.mockResolvedValue(true);
  ipc.projects.openInEditor.mockResolvedValue(undefined);
  ipc.projects.tree.mockImplementation(async (_id: string, path?: string) => {
    if (!path) return { entries: ROOT_ENTRIES, branch: "main" };
    if (path === "src") return { entries: SRC_ENTRIES };
    throw new Error(`unexpected path: ${path}`);
  });
});

describe("the tree drawer", () => {
  it("the_tree_loads_the_root_and_expands_folders_lazily", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "toggle tree" }));

    await screen.findByRole("button", { name: "src" });
    expect(screen.queryByText("index.ts")).toBeNull();
    expect(ipc.projects.tree).toHaveBeenCalledTimes(1);
    expect(ipc.projects.tree).toHaveBeenCalledWith("proj_1");

    await user.click(screen.getByRole("button", { name: "src" }));

    await screen.findByRole("button", { name: "index.ts" });
    expect(ipc.projects.tree).toHaveBeenCalledTimes(2);
    expect(ipc.projects.tree).toHaveBeenCalledWith("proj_1", "src");
  });

  it("clicking_a_file_opens_the_preview_beside_the_tree", async () => {
    const user = userEvent.setup();
    ipc.projects.readFile.mockResolvedValue({
      content: "const x = 1;\n",
      truncated: false,
    });
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "toggle tree" }));
    await user.click(await screen.findByRole("button", { name: "src" }));
    await user.click(await screen.findByRole("button", { name: "index.ts" }));

    const heading = await screen.findByText("src/index.ts");
    expect(ipc.projects.readFile).toHaveBeenCalledWith(
      "proj_1",
      "src/index.ts",
    );

    const panel = heading.closest('[data-slot="file-preview"]');
    expect(panel).not.toBeNull();
    expect(panel?.getAttribute("style")).toContain("left: var(--tree-width)");
  });

  it("escape_closes_the_preview_then_the_tree", async () => {
    const user = userEvent.setup();
    ipc.projects.readFile.mockResolvedValue({
      content: "const x = 1;\n",
      truncated: false,
    });
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "toggle tree" }));
    await user.click(await screen.findByRole("button", { name: "src" }));
    await user.click(await screen.findByRole("button", { name: "index.ts" }));
    await screen.findByText("src/index.ts");

    await user.keyboard("{Escape}");
    await vi.waitFor(() =>
      expect(screen.queryByText("src/index.ts")).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "src" })).toBeDefined();

    await user.keyboard("{Escape}");
    await vi.waitFor(() =>
      expect(screen.queryByRole("button", { name: "src" })).toBeNull(),
    );
  });

  it("switching_systems_closes_the_open_preview", async () => {
    const user = userEvent.setup();
    ipc.projects.readFile.mockResolvedValue({
      content: "const x = 1;\n",
      truncated: false,
    });
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "toggle tree" }));
    await user.click(await screen.findByRole("button", { name: "src" }));
    await user.click(await screen.findByRole("button", { name: "index.ts" }));
    await screen.findByText("src/index.ts");
    expect(ipc.projects.readFile).toHaveBeenCalledWith(
      "proj_1",
      "src/index.ts",
    );

    await user.click(screen.getByRole("button", { name: "switch system" }));

    await vi.waitFor(() =>
      expect(screen.queryByText("src/index.ts")).toBeNull(),
    );
    expect(ipc.projects.readFile).not.toHaveBeenCalledWith(
      "proj_2",
      "src/index.ts",
    );
  });

  it("a_git_flag_renders_in_the_warning_tone_and_an_unknown_letter_renders_as_is", async () => {
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "toggle tree" }));
    await screen.findByRole("button", { name: /README\.md/ });

    const known = screen.getByText("M");
    const unknown = screen.getByText("Z");
    expect(known.className).toContain(TONE_TEXT.warning);
    expect(unknown.className).toContain(TONE_TEXT.warning);
  });
});
