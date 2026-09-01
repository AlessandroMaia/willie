import { describe, expect, it, vi } from "vitest";
import { createStore } from "@/lib/domain/daemon-snapshot";
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

/* Flushes both the microtask queue and any pending timers, so a
 * promise chain started before this call has had every `.then` and
 * `.catch` handler in it actually run by the time it resolves. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
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
    if (state.snapshot === null) throw new Error("expected a snapshot");
    expect(state.snapshot.seq).toBe(2);
    expect(state.snapshot.projects).toHaveLength(1);
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

  it("reports a failed snapshot as a Problem, with no snapshot yet", async () => {
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
      expect(state.status).toBe("failed");
      expect(state.problem?.code).toBe("daemon_unreachable");
    });
    expect(store.getState().snapshot).toBeNull();
  });

  it("keeps the last snapshot when a later refetch fails", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    source.snapshot.mockRejectedValueOnce({
      code: "daemon_unreachable",
      message: "no",
      remediation: "start it",
    });
    /* A gap forces a resnapshot, which is the rejecting call. */
    source.emit(projectEvent(7, "proj_a"));
    await vi.waitFor(() => expect(store.getState().status).toBe("failed"));
    const state = store.getState();
    expect(state.problem?.code).toBe("daemon_unreachable");
    expect(state.snapshot?.seq).toBe(1);
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
    const state = store.getState();
    expect(state.status).toBe("idle");
    expect(state.snapshot).toBeNull();
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

  it("does not revive if the last holder releases before an in-flight snapshot resolves", async () => {
    const source = fakeSource(snapshotAt(1));
    const store = createStore(source);
    const release = store.acquire();
    expect(source.snapshot).toHaveBeenCalledTimes(1);
    release();
    await flush();
    expect(store.getState().status).toBe("idle");
    expect(store.getState().snapshot).toBeNull();
  });

  it("keeps a dead subscription visible even after a late snapshot resolves", async () => {
    const snapshot = snapshotAt(1);
    let resolveSnapshot: ((s: Snapshot) => void) | undefined;
    const source = {
      snapshot: vi.fn(
        () => new Promise<Snapshot>((resolve) => (resolveSnapshot = resolve)),
      ),
      onEvent: vi.fn(async () => {
        throw {
          code: "daemon_unreachable",
          message: "no",
          remediation: "start it",
        };
      }),
    };
    const store = createStore(source);
    store.acquire();

    /* The subscription rejects first; the snapshot fetch is still
     * in flight. */
    await vi.waitFor(() => {
      expect(store.getState().status).toBe("failed");
      expect(store.getState().problem?.code).toBe("daemon_unreachable");
    });

    /* The in-flight snapshot resolves afterward. It must not erase
     * the only sign that daemon events stopped arriving. */
    resolveSnapshot?.(snapshot);
    await flush();

    const state = store.getState();
    expect(state.status).toBe("failed");
    expect(state.problem?.code).toBe("daemon_unreachable");
    expect(state.snapshot?.seq).toBe(1);
  });

  it("acquire, release, then acquire again leaves exactly one live subscription", async () => {
    const snapshot = snapshotAt(1);
    const firstUnlisten = vi.fn();
    const secondUnlisten = vi.fn();
    let calls = 0;
    const source = {
      snapshot: vi.fn(async () => snapshot),
      onEvent: vi.fn(async () => {
        calls += 1;
        return calls === 1 ? firstUnlisten : secondUnlisten;
      }),
    };
    const store = createStore(source);

    const releaseFirst = store.acquire();
    releaseFirst();
    store.acquire();

    await vi.waitFor(() => expect(store.getState().status).toBe("ready"));
    await flush();

    expect(source.onEvent).toHaveBeenCalledTimes(2);
    expect(firstUnlisten).toHaveBeenCalledTimes(1);
    expect(secondUnlisten).not.toHaveBeenCalled();
  });
});
