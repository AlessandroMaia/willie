import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Session, Snapshot, UsageSnapshot } from "@/lib/proto";

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. `projects.snapshot`/`onDaemonEvent` back `useSnapshot`,
 * which the panel now reads to map a usage session row to the project
 * it belongs to (mirrors the shape of every other panel test's
 * hoisted fake — see profile-store-panel.test.tsx). */
const ipc = vi.hoisted(() => ({
  usage: { snapshot: vi.fn() },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

let UsagePanel: typeof import("@/plugins/usage/usage-panel").UsagePanel;

function usageSnapshot(overrides: Partial<UsageSnapshot> = {}): UsageSnapshot {
  return {
    providers: [],
    sessions: [],
    projects: [],
    fetched_at: "2026-09-06T00:00:00Z",
    ...overrides,
  };
}

function daemonSession(id: string, projectId: string): Session {
  return {
    id,
    project_id: projectId,
    harness: "claude-code",
    workspace: `/home/willie/projects/${projectId}`,
    state: { state: "running" },
    created_at: "1",
    clients: 1,
  };
}

function daemonSnapshot(sessions: Session[] = []): Snapshot {
  return { seq: 1, projects: [], jobs: [], sessions };
}

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.projects.snapshot.mockResolvedValue(daemonSnapshot());
  ({ UsagePanel } = await import("@/plugins/usage/usage-panel"));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("UsagePanel", () => {
  it("renders a session's context percentage, tone and token total", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      daemonSnapshot([daemonSession("sess_1", "proj_1")]),
    );
    ipc.usage.snapshot.mockResolvedValue(
      usageSnapshot({
        sessions: [{ id: "sess_1", tokens: 12345, context_pct: 42 }],
      }),
    );

    render(<UsagePanel projectId="proj_1" />);

    expect(await screen.findByText(/42%/)).toBeDefined();
    expect(
      screen.getByText(/42%/).closest("[data-tone]")?.getAttribute("data-tone"),
    ).toBe("ok");
    expect(screen.getByText(/12345/)).toBeDefined();
  });

  it('reads "no usage yet" for a session with no context_pct', async () => {
    ipc.projects.snapshot.mockResolvedValue(
      daemonSnapshot([daemonSession("sess_1", "proj_1")]),
    );
    ipc.usage.snapshot.mockResolvedValue(
      usageSnapshot({
        sessions: [{ id: "sess_1", tokens: 100, context_pct: null }],
      }),
    );

    render(<UsagePanel projectId="proj_1" />);

    const badge = await screen.findByText("no usage yet");
    expect(badge.closest("[data-tone]")?.getAttribute("data-tone")).toBe(
      "muted",
    );
  });

  it("colours a high context percentage as an error", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      daemonSnapshot([daemonSession("sess_1", "proj_1")]),
    );
    ipc.usage.snapshot.mockResolvedValue(
      usageSnapshot({
        sessions: [{ id: "sess_1", tokens: 1, context_pct: 95 }],
      }),
    );

    render(<UsagePanel projectId="proj_1" />);

    expect(await screen.findByText(/95%/)).toBeDefined();
    expect(
      screen.getByText(/95%/).closest("[data-tone]")?.getAttribute("data-tone"),
    ).toBe("error");
  });

  it("filters out a session that belongs to another project", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      daemonSnapshot([daemonSession("sess_1", "proj_2")]),
    );
    ipc.usage.snapshot.mockResolvedValue(
      usageSnapshot({
        sessions: [{ id: "sess_1", tokens: 12345, context_pct: 42 }],
      }),
    );

    render(<UsagePanel projectId="proj_1" />);

    await screen.findByText("Session usage");
    expect(screen.queryByText("sess_1")).toBeNull();
  });

  it("shows only the current project's row in the per-project totals", async () => {
    ipc.usage.snapshot.mockResolvedValue(
      usageSnapshot({
        projects: [
          { id: "proj_1", tokens: 500 },
          { id: "proj_2", tokens: 250 },
        ],
      }),
    );

    render(<UsagePanel projectId="proj_1" />);

    expect(await screen.findByText("proj_1")).toBeDefined();
    expect(screen.getByText(/500/)).toBeDefined();
    expect(screen.queryByText("proj_2")).toBeNull();
  });

  it("shows a problem alert when the snapshot call fails", async () => {
    ipc.usage.snapshot.mockRejectedValue({
      code: "daemon_unreachable",
      message: "the daemon is not running",
      remediation: "start the daemon",
    });

    render(<UsagePanel projectId="proj_1" />);

    expect(await screen.findByText("daemon_unreachable")).toBeDefined();
  });

  it("polls for a fresh snapshot while mounted", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    ipc.usage.snapshot.mockResolvedValue(usageSnapshot());

    render(<UsagePanel projectId="proj_1" />);

    expect(ipc.usage.snapshot).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(5000);

    expect(ipc.usage.snapshot.mock.calls.length).toBeGreaterThan(1);
  });
});
