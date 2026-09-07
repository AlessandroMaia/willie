import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UsageSnapshot } from "@/lib/proto";

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Mirrors the shape of every other panel test's hoisted
 * fake (see profiles-panel.test.tsx). */
const ipc = vi.hoisted(() => ({
  usage: { snapshot: vi.fn() },
}));

vi.mock("@/lib/ipc", () => ipc);

let UsagePanel: typeof import("@/plugins/usage/usage-panel").UsagePanel;

function snapshot(overrides: Partial<UsageSnapshot> = {}): UsageSnapshot {
  return {
    providers: [],
    sessions: [],
    projects: [],
    fetched_at: "2026-09-06T00:00:00Z",
    ...overrides,
  };
}

beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ({ UsagePanel } = await import("@/plugins/usage/usage-panel"));
});

afterEach(() => {
  vi.useRealTimers();
});

describe("UsagePanel", () => {
  it("renders a session's context percentage, tone and token total", async () => {
    ipc.usage.snapshot.mockResolvedValue(
      snapshot({
        sessions: [{ id: "sess_1", tokens: 12345, context_pct: 42 }],
      }),
    );

    render(<UsagePanel />);

    expect(await screen.findByText(/42%/)).toBeDefined();
    expect(
      screen.getByText(/42%/).closest("[data-tone]")?.getAttribute("data-tone"),
    ).toBe("ok");
    expect(screen.getByText(/12345/)).toBeDefined();
  });

  it('reads "no usage yet" for a session with no context_pct', async () => {
    ipc.usage.snapshot.mockResolvedValue(
      snapshot({
        sessions: [{ id: "sess_1", tokens: 100, context_pct: null }],
      }),
    );

    render(<UsagePanel />);

    const badge = await screen.findByText("no usage yet");
    expect(badge.closest("[data-tone]")?.getAttribute("data-tone")).toBe(
      "muted",
    );
  });

  it("colours a high context percentage as an error", async () => {
    ipc.usage.snapshot.mockResolvedValue(
      snapshot({
        sessions: [{ id: "sess_1", tokens: 1, context_pct: 95 }],
      }),
    );

    render(<UsagePanel />);

    expect(await screen.findByText(/95%/)).toBeDefined();
    expect(
      screen.getByText(/95%/).closest("[data-tone]")?.getAttribute("data-tone"),
    ).toBe("error");
  });

  it("renders the per-project token summary", async () => {
    ipc.usage.snapshot.mockResolvedValue(
      snapshot({
        projects: [
          { id: "proj_1", tokens: 500 },
          { id: "proj_2", tokens: 250 },
        ],
      }),
    );

    render(<UsagePanel />);

    expect(await screen.findByText("proj_1")).toBeDefined();
    expect(screen.getByText(/500/)).toBeDefined();
    expect(await screen.findByText("proj_2")).toBeDefined();
    expect(screen.getByText(/250/)).toBeDefined();
  });

  it("shows a problem alert when the snapshot call fails", async () => {
    ipc.usage.snapshot.mockRejectedValue({
      code: "daemon_unreachable",
      message: "the daemon is not running",
      remediation: "start the daemon",
    });

    render(<UsagePanel />);

    expect(await screen.findByText("daemon_unreachable")).toBeDefined();
  });

  it("polls for a fresh snapshot while mounted", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    ipc.usage.snapshot.mockResolvedValue(snapshot());

    render(<UsagePanel />);

    expect(ipc.usage.snapshot).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(5000);

    expect(ipc.usage.snapshot.mock.calls.length).toBeGreaterThan(1);
  });
});
