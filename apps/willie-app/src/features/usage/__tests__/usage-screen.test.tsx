import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { UsageScreen } from "@/features/usage/usage-screen";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Session, Snapshot } from "@/lib/proto";
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

function session(): Session {
  return {
    id: "sess_1",
    project_id: "proj_1",
    harness: "claude-code",
    workspace: "/home/willie/projects/willie",
    state: { state: "running" },
    created_at: "1",
    clients: 1,
  };
}

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes: the engine/projects/ui triad backs `useCurrentSystem`
 * (mirrors profiles-screen.test.tsx), `usage.snapshot` backs the panel
 * it mounts. */
const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
  usage: { snapshot: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

function snapshot(projects: Project[] = [project()]): Snapshot {
  return { seq: 1, projects, jobs: [], sessions: [session()] };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(snapshot());
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.usage.snapshot.mockResolvedValue({
    providers: [],
    sessions: [{ id: "sess_1", tokens: 42, context_pct: 10 }],
    projects: [{ id: "proj_1", tokens: 42 }],
    fetched_at: "2026-09-08T00:00:00Z",
  });
});

describe("UsageScreen", () => {
  it("mounts the usage panel scoped to the current system", async () => {
    render(<UsageScreen />);

    expect(await screen.findByText("sess_1")).toBeDefined();
  });

  it("offers to add a system when there is none yet", async () => {
    ipc.projects.snapshot.mockResolvedValue(snapshot([]));

    render(<UsageScreen />);

    expect(await screen.findByText("No system yet")).toBeDefined();
    expect(ipc.usage.snapshot).not.toHaveBeenCalled();
  });
});
