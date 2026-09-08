import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Snapshot } from "@/lib/proto";

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

function snapshot(): Snapshot {
  return { seq: 1, projects: [project()], jobs: [], sessions: [] };
}

/* The bridge is the only I/O this component reaches: everything
 * `useCurrentSystem` needs on mount, plus the read/editor calls the
 * preview itself makes. `@/app/*` stays out of reach here (Biome's
 * layering rule), so this suite drives `useFilePreview` directly
 * instead of going through the tree drawer. */
const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn(), readFile: vi.fn(), openInEditor: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  ui: {
    prefs: vi.fn(async () => ({ current_project: null })),
    setPrefs: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

let FilePreview: typeof import("@/features/session/file-preview").FilePreview;
let useFilePreview: typeof import("@/store/use-file-preview").useFilePreview;
let useTreeDrawer: typeof import("@/store/use-tree-drawer").useTreeDrawer;

/* Stands in for a tree row's click: opens a file directly through the
 * store, the same call `tree-drawer.tsx` makes on a file row. `open`
 * additionally opens the tree drawer's own store — `useTreeDrawer` is
 * a plain store, not a component, so reading it here does not reach
 * into `@/app/*` (Biome's layering rule stays satisfied). */
function OpenFileHarness({
  path,
  openDrawer = false,
}: {
  path: string;
  openDrawer?: boolean;
}) {
  const { openFile } = useFilePreview();
  const { open, toggle } = useTreeDrawer();

  useEffect(() => {
    openFile(path);
  }, [path, openFile]);

  useEffect(() => {
    if (openDrawer && !open) toggle();
  }, [openDrawer, open, toggle]);

  return <FilePreview />;
}

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.projects.snapshot.mockResolvedValue(snapshot());
  ipc.editorAvailable.mockResolvedValue(true);
  ipc.projects.openInEditor.mockResolvedValue(undefined);
  ({ FilePreview } = await import("@/features/session/file-preview"));
  ({ useFilePreview } = await import("@/store/use-file-preview"));
  ({ useTreeDrawer } = await import("@/store/use-tree-drawer"));
});

describe("the file preview", () => {
  it("expand_makes_the_preview_fill_the_width_right_of_the_tree", async () => {
    const user = userEvent.setup();
    ipc.projects.readFile.mockResolvedValue({
      content: "line one\n",
      truncated: false,
    });

    render(<OpenFileHarness path="notes.md" openDrawer />);

    const heading = await screen.findByText("notes.md");
    const panel = heading.closest('[data-slot="file-preview"]');
    if (!panel) throw new Error("no file-preview panel rendered");
    await vi.waitFor(() =>
      expect(panel.getAttribute("style")).toContain("left: var(--tree-width)"),
    );
    expect(panel.className).not.toContain("right-0");

    await user.click(screen.getByRole("button", { name: "Expand preview" }));

    expect(panel.className).toContain("right-0");
    expect(panel.getAttribute("style")).toContain("left: var(--tree-width)");
  });

  it("open_in_vs_code_from_the_preview_passes_the_file", async () => {
    const user = userEvent.setup();
    ipc.projects.readFile.mockResolvedValue({
      content: "line one\n",
      truncated: false,
    });

    render(<OpenFileHarness path="notes.md" />);
    await screen.findByText("notes.md");

    const button = (await screen.findByRole("button", {
      name: "Open in VS Code",
    })) as HTMLButtonElement;
    await vi.waitFor(() => expect(button.disabled).toBe(false));

    await user.click(button);

    expect(ipc.projects.openInEditor).toHaveBeenCalledWith(
      "/home/willie/projects/willie",
      "notes.md",
    );
  });

  it("truncated_content_shows_the_512_kib_line", async () => {
    ipc.projects.readFile.mockResolvedValue({
      content: "a".repeat(100),
      truncated: true,
    });

    render(<OpenFileHarness path="big.log" />);

    await screen.findByText("big.log");
    expect(await screen.findByText("showing the first 512 KiB")).toBeDefined();
  });

  it("file_not_text_shows_a_failure_chip", async () => {
    ipc.projects.readFile.mockRejectedValue({
      code: "file_not_text",
      message: "the file is not text",
      remediation: "",
    });

    render(<OpenFileHarness path="image.png" />);

    await screen.findByText("image.png");
    expect(await screen.findByText("file_not_text")).toBeDefined();
    expect(screen.getByText("the file is not text")).toBeDefined();
  });
});
