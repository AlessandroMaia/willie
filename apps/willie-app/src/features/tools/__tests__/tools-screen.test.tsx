import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot, ToolStatus } from "@/lib/proto";

const emptySnapshot = (): Snapshot => ({
  seq: 1,
  projects: [],
  jobs: [],
  sessions: [],
});

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Mirrors the shape of every other screen test's hoisted
 * fake (see shell.test.tsx, remove-project.test.tsx). */
const ipc = vi.hoisted(() => ({
  tools: { install: vi.fn(), update: vi.fn(), list: vi.fn() },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

let ToolsScreen: typeof import("@/features/tools/tools-screen").ToolsScreen;

/* `useSnapshot` backs onto a module-level singleton store, so a
 * snapshot left behind by one render would still be there for the
 * next test's first synchronous render. Reset the module graph and
 * re-import the screen fresh for every case. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(emptySnapshot());
  ({ ToolsScreen } = await import("@/features/tools/tools-screen"));
});

describe("ToolsScreen", () => {
  it("shows an installed tool with its version and an Update button", async () => {
    const installed: ToolStatus = {
      id: "claude-code",
      name: "Claude Code",
      installed: true,
      version: "2.1.246",
    };
    ipc.tools.list.mockResolvedValue({ tools: [installed] });

    render(<ToolsScreen />);

    expect(await screen.findByText("Claude Code")).toBeDefined();
    expect(screen.getByText("installed v2.1.246")).toBeDefined();
    expect(screen.getByRole("button", { name: "Update" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  it("shows a missing tool with an Install button", async () => {
    const missing: ToolStatus = {
      id: "claude-code",
      name: "Claude Code",
      installed: false,
    };
    ipc.tools.list.mockResolvedValue({ tools: [missing] });

    render(<ToolsScreen />);

    expect(await screen.findByText("Claude Code")).toBeDefined();
    expect(screen.getByText("not installed")).toBeDefined();
    expect(screen.getByRole("button", { name: "Install" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Update" })).toBeNull();
  });

  it("shows progress while a tool job runs", async () => {
    const installed: ToolStatus = {
      id: "claude-code",
      name: "Claude Code",
      installed: true,
      version: "2.1.246",
    };
    ipc.tools.list.mockResolvedValue({ tools: [installed] });
    ipc.projects.snapshot.mockResolvedValue({
      seq: 1,
      projects: [],
      sessions: [],
      jobs: [
        {
          id: "job_1",
          kind: "update_harness",
          state: { state: "running" },
          started_at: "1",
          log_tail: "downloading release\n",
        },
      ],
    });

    render(<ToolsScreen />);

    expect(await screen.findByText("downloading release")).toBeDefined();
    expect(screen.getByRole("status")).toBeDefined();
  });

  it("notes a version recorded by an install that no longer matches what is detected", async () => {
    const drifted: ToolStatus = {
      id: "claude-code",
      name: "Claude Code",
      installed: true,
      version: "2.2.0",
      recorded_version: "2.1.246",
    };
    ipc.tools.list.mockResolvedValue({ tools: [drifted] });

    render(<ToolsScreen />);

    expect(await screen.findByText("installed v2.2.0")).toBeDefined();
    expect(screen.getByText("updated outside Willie")).toBeDefined();
  });

  it("does not note a recorded version for a tool that is not installed", async () => {
    const removed: ToolStatus = {
      id: "claude-code",
      name: "Claude Code",
      installed: false,
      version: undefined,
      recorded_version: "1.2.3",
    };
    ipc.tools.list.mockResolvedValue({ tools: [removed] });

    render(<ToolsScreen />);

    expect(await screen.findByText("Claude Code")).toBeDefined();
    expect(screen.getByText("not installed")).toBeDefined();
    expect(screen.getByRole("button", { name: "Install" })).toBeDefined();
    expect(screen.queryByText(/updated outside/i)).toBeNull();
  });
});
