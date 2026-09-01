import { describe, expect, it, vi } from "vitest";
import { createStore } from "@/lib/domain/store";
import type { Event, Snapshot } from "@/lib/proto";

function snapshotAt(seq: number): Snapshot {
  return { seq, projects: [], jobs: [], sessions: [] };
}

function projectEvent(seq: number, id: string): Event {
  return {
    seq,
    kind: "project_changed",
    project: {
      id,
      name: id,
      slug: id,
      source: "C:\\src",
      workspace: `/home/willie/projects/${id}`,
      branch: "main",
      state: { state: "ready" },
      source_present: true,
      created_at: "1",
    },
  };
}

function fakeSource(snapshot: Snapshot) {
  let emit: ((ev: Event) => void) | undefined;
  const unlisten = vi.fn();
  return {
    unlisten,
    emit: (ev: Event) => emit?.(ev),
    snapshot: vi.fn(async () => snapshot),
    onEvent: vi.fn(async (cb: (ev: Event) => void) => {
      emit = cb;
      return unlisten;
    }),
  };
}

describe("createStore", () => {
  it("stays idle until something acquires it", () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    expect(store.getState().status).toBe("idle");
    expect(source.snapshot).not.toHaveBeenCalled();
  });

  it("loads a snapshot when acquired", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    store.acquire();
    expect(store.getState().status).toBe("loading");
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
  });

  it("folds an event that extends the snapshot", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    source.emit(projectEvent(2, "proj_a"));
    const state = store.getState();
    expect(state.status === "ready" && state.snapshot.seq).toBe(2);
    expect(state.status === "ready" && state.snapshot.projects).toHaveLength(1);
    expect(source.snapshot).toHaveBeenCalledTimes(1);
  });

  it("refetches when an event skips a sequence number", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    source.emit(projectEvent(7, "proj_a"));
    await vi.waitFor(() => expect(source.snapshot).toHaveBeenCalledTimes(2));
  });

  it("reports a failed snapshot as a Problem", async () => {
    const source = fakeSource(snapshotAt(1));
    source.snapshot.mockRejectedValueOnce({
      code: "daemon_unreachable",
      message: "no",
      remediation: "start it",
    });
    const store = createStore(source);
    store.acquire();
    await vi.waitFor(() => {
      const state = store.getState();
      expect(state.status === "failed" && state.problem.code).toBe(
        "daemon_unreachable",
      );
    });
  });

  it("keeps one subscription for many holders and drops it with the last", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    const releaseA = store.acquire();
    const releaseB = store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    expect(source.onEvent).toHaveBeenCalledTimes(1);
    releaseA();
    expect(source.unlisten).not.toHaveBeenCalled();
    releaseB();
    await vi.waitFor(() => expect(source.unlisten).toHaveBeenCalledTimes(1));
    expect(store.getState().status).toBe("idle");
  });

  it("notifies subscribers on every change", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    const listener = vi.fn();
    store.subscribe(listener);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    expect(listener).toHaveBeenCalled();
  });
});
