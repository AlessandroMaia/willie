import { describe, expect, it, vi } from "vitest";
import { createEngineStatusStore, summarize } from "@/lib/domain/engine-status";
import type { EngineStatus } from "@/lib/ipc";

const running: EngineStatus = {
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

const stopped: EngineStatus = { ...running, daemon: { state: "stopped" } };

function fakeSource(first: EngineStatus) {
  let push: ((status: EngineStatus) => void) | undefined;
  const unlisten = vi.fn();
  return {
    unlisten,
    push: (status: EngineStatus) => push?.(status),
    status: vi.fn(async () => first),
    onStatus: vi.fn(async (cb: (status: EngineStatus) => void) => {
      push = cb;
      return unlisten;
    }),
  };
}

describe("createEngineStatusStore", () => {
  it("holds nothing until something acquires it", () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);

    expect(store.getState()).toEqual({ status: null, problem: null });
    expect(source.status).not.toHaveBeenCalled();
  });

  it("reads the status once acquired", async () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);

    store.acquire();

    await vi.waitFor(() => expect(store.getState().status).toEqual(running));
    expect(store.getState().problem).toBeNull();
  });

  it("keeps the last status and records the problem when a refresh fails", async () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toEqual(running));

    source.status.mockRejectedValueOnce({
      code: "engine_unreachable",
      message: "no answer",
      remediation: "restart",
    });
    await store.refresh();

    expect(store.getState().status).toEqual(running);
    expect(store.getState().problem?.code).toBe("engine_unreachable");
  });

  it("replaces the status when the engine pushes one and clears the problem", async () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toEqual(running));
    source.status.mockRejectedValueOnce(new Error("boom"));
    await store.refresh();
    expect(store.getState().problem).not.toBeNull();

    source.push(stopped);

    expect(store.getState().status).toEqual(stopped);
    expect(store.getState().problem).toBeNull();
  });

  it("resolves refresh only after the state changed", async () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);
    store.acquire();
    await vi.waitFor(() => expect(store.getState().status).toEqual(running));

    source.status.mockResolvedValueOnce(stopped);
    await store.refresh();

    expect(store.getState().status).toEqual(stopped);
  });

  it("stops listening when the last holder releases", async () => {
    const source = fakeSource(running);
    const store = createEngineStatusStore(source);
    const releaseA = store.acquire();
    const releaseB = store.acquire();
    /* Waiting for the status also drains the microtask that stores the
     * unlisten handle; releasing before that would call nothing. */
    await vi.waitFor(() => expect(store.getState().status).toEqual(running));
    expect(source.onStatus).toHaveBeenCalledTimes(1);

    releaseA();
    expect(source.unlisten).not.toHaveBeenCalled();

    releaseB();
    expect(source.unlisten).toHaveBeenCalledTimes(1);
  });
});

describe("summarize", () => {
  it("says the engine is running, with the daemon version, when every part is ok", () => {
    expect(summarize(running)).toEqual({
      health: "ok",
      headline: "Engine running",
      version: "0.1.0",
    });
  });

  it("names the first part that is not ok, in wsl, distro, daemon, doctor order", () => {
    expect(summarize(stopped).headline).toBe("Daemon stopped");
    expect(
      summarize({
        ...stopped,
        distro: { registered: false, running: true, install_dir: "C:\\x" },
      }).headline,
    ).toBe("Distribution not registered");
    expect(
      summarize({ ...stopped, wsl: { ...running.wsl, installed: false } })
        .headline,
    ).toBe("WSL not installed");
  });

  it("reports a failed daemon with its message and no version", () => {
    const summary = summarize({
      ...running,
      daemon: { state: "failed", code: "daemon_exited", message: "boom" },
    });

    expect(summary).toEqual({
      health: "failed",
      headline: "Daemon failed: boom",
      version: null,
    });
  });

  it("is degraded and unknown before the first status arrives", () => {
    expect(summarize(null)).toEqual({
      health: "degraded",
      headline: "Engine status unknown",
      version: null,
    });
  });
});
