import { act, render, renderHook, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EngineStatus } from "@/lib/ipc";
import type { Session, Snapshot, UsageSnapshot } from "@/lib/proto";

/* The Session screen (Task 12) hosts a real `SessionTerminal` for every
 * live session, and this suite resets the module graph for every test
 * (see the `beforeEach` below) — without this, the real `@xterm/xterm`
 * would be re-imported and re-initialised from scratch on every case.
 * A fake stands in, same shape `session-screen.test.tsx` uses. */
vi.mock("@xterm/xterm", () => {
  class FakeTerminal {
    rows = 24;
    cols = 80;
    options: Record<string, unknown> = {};
    attachCustomKeyEventHandler() {}
    loadAddon() {}
    open() {}
    onData() {}
    write() {}
    dispose() {}
  }
  return { Terminal: FakeTerminal };
});

vi.mock("@xterm/addon-fit", () => {
  class FakeFitAddon {
    fit() {}
  }
  return { FitAddon: FakeFitAddon };
});

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

const AGENT_SESSION: Session = {
  id: "sess_1",
  project_id: "proj_1",
  harness: "claude-code",
  workspace: "/home/willie/projects/a",
  kind: "agent",
  state: { state: "running" },
  created_at: "1",
  clients: 1,
  sandbox: {
    applied: ["seccomp", "namespaces"],
    unavailable: [],
    degraded: [],
    denied: [
      {
        class: "syscall",
        name: "ptrace",
        count: 3,
        first_at: "1",
        last_at: "2",
      },
    ],
  },
};

const SHELL_SESSION: Session = {
  ...AGENT_SESSION,
  id: "sess_2",
  kind: "shell",
};

/* Still `creating`: no `sandbox` field at all, mirroring a session that
 * has not reported anything yet — `sandboxOf` normalises this to the
 * all-empty state, and `sandboxPosture` reads it as "unknown". */
const CREATING_SESSION: Session = {
  id: "sess_3",
  project_id: "proj_1",
  harness: "claude-code",
  workspace: "/home/willie/projects/a",
  kind: "agent",
  state: { state: "creating" },
  created_at: "1",
  clients: 0,
};

const SNAPSHOT: Snapshot = {
  seq: 1,
  projects: [],
  jobs: [],
  sessions: [AGENT_SESSION, SHELL_SESSION, CREATING_SESSION],
};

const EMPTY_USAGE: UsageSnapshot = {
  providers: [],
  sessions: [],
  projects: [],
  fetched_at: "1",
};

/* The bridge is the only I/O in the frontend and the only module a
 * test fakes. Every function the shell or its four screens may call
 * on mount is here; the rest are inert. */
const ipc = vi.hoisted(() => ({
  engine: {
    status: vi.fn(),
    onStatus: vi.fn(async () => () => {}),
    installDistro: vi.fn(),
    startDaemon: vi.fn(),
    stopDaemon: vi.fn(),
    doctor: vi.fn(),
  },
  projects: {
    snapshot: vi.fn(),
    roots: vi.fn(async () => []),
    add: vi.fn(),
    remove: vi.fn(),
    syncToWindows: vi.fn(),
    updateFromWindows: vi.fn(),
    relocate: vi.fn(),
    rename: vi.fn(),
    setSandbox: vi.fn(),
    cancelJob: vi.fn(),
    setRoots: vi.fn(),
    discover: vi.fn(),
    openInExplorer: vi.fn(),
    openInEditor: vi.fn(),
  },
  sessions: { open: vi.fn(), resume: vi.fn(), attach: vi.fn(), stop: vi.fn() },
  sessionTerminal: {
    open: vi.fn(),
    input: vi.fn(),
    resize: vi.fn(),
    close: vi.fn(),
  },
  tools: {
    install: vi.fn(),
    update: vi.fn(),
    list: vi.fn(async () => ({ tools: [] })),
  },
  plugins: {
    list: vi.fn(async () => []),
    enable: vi.fn(),
    disable: vi.fn(),
  },
  sandbox: { catalogue: vi.fn(async () => []) },
  usage: { snapshot: vi.fn() },
  dialogs: { pickFolder: vi.fn(async () => null) },
  onDaemonEvent: vi.fn(async () => () => {}),
  onSessionOutput: vi.fn(async () => () => {}),
  editorAvailable: vi.fn(async () => true),
  ui: {
    prefs: vi.fn(async () => ({ current_project: null })),
    setPrefs: vi.fn(),
  },
}));

vi.mock("@/lib/ipc", () => ipc);

/* The header's window controls talk to this module directly, not
 * through the ipc bridge above; every test here renders the full
 * shell, so it needs a stub too. */
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
  }),
}));

let App: typeof import("@/app/app").App;
let createAppRouter: typeof import("@/app/router").createAppRouter;
let useFocusedSession: typeof import("@/store/use-focused-session").useFocusedSession;

/* `useFocusedSession`, `useEngineStatus` and `useSnapshot` back onto
 * module-level singleton stores, so the module graph is reset and
 * re-imported fresh for every case, same as shell.test.tsx — and the
 * `useFocusedSession` import here resolves to the very same singleton
 * the rendered app's status bar reads, since both come from the one
 * module cache built by this test's `vi.resetModules()`. */
beforeEach(async () => {
  vi.resetModules();
  vi.clearAllMocks();
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.projects.snapshot.mockResolvedValue(SNAPSHOT);
  ipc.usage.snapshot.mockResolvedValue(EMPTY_USAGE);
  ({ App } = await import("@/app/app"));
  ({ createAppRouter } = await import("@/app/router"));
  ({ useFocusedSession } = await import("@/store/use-focused-session"));
});

/* Every test that switches to fake timers must hand real ones back —
 * a test that throws before its own cleanup would otherwise leave
 * later files running against a fake clock. */
afterEach(() => {
  vi.useRealTimers();
});

async function renderApp(path: string) {
  const { createMemoryHistory } = await import("@tanstack/react-router");
  const router = createAppRouter(
    createMemoryHistory({ initialEntries: [path] }),
  );
  render(<App router={router} />);
  await screen.findByText("Engine running");
  return router;
}

function focus(id: string | null) {
  const hook = renderHook(() => useFocusedSession());
  act(() => {
    hook.result.current.setFocused(id);
  });
}

describe("the footer's governance segment", () => {
  it("the_footer_shows_the_focused_sessions_sandbox_and_context", async () => {
    ipc.usage.snapshot.mockResolvedValue({
      ...EMPTY_USAGE,
      sessions: [{ id: "sess_1", tokens: 4200, context_pct: 42 }],
    });
    await renderApp("/session");

    focus("sess_1");

    expect(
      await screen.findByText("sandbox: seccomp · namespaces · 3 denied"),
    ).toBeDefined();
    expect(await screen.findByText("4200 tokens")).toBeDefined();
    expect(screen.getByRole("img", { name: "42% context" })).toBeDefined();
  });

  it("the_footer_hides_governance_on_a_shell_session", async () => {
    await renderApp("/session");

    focus("sess_2");

    await vi.waitFor(() => {
      expect(ipc.usage.snapshot).not.toHaveBeenCalled();
    });
    expect(screen.queryByText(/^sandbox:/)).toBeNull();
  });

  it("the_governance_segment_links_to_the_filtered_sandbox_screen", async () => {
    await renderApp("/session");

    focus("sess_1");

    const link = await screen.findByRole("link", {
      name: /sandbox: seccomp/,
    });
    expect(link.getAttribute("href")).toBe("/sandbox?session=sess_1");
  });

  it("hides governance and never polls usage away from the Session screen", async () => {
    await renderApp("/sandbox");

    focus("sess_1");

    expect(screen.queryByText(/^sandbox:/)).toBeNull();
    expect(ipc.usage.snapshot).not.toHaveBeenCalled();
  });

  it("the_footer_says_no_sandbox_report_while_a_session_has_no_mechanisms_yet", async () => {
    ipc.usage.snapshot.mockResolvedValue({
      ...EMPTY_USAGE,
      sessions: [{ id: "sess_3", tokens: 500, context_pct: 10 }],
    });
    await renderApp("/session");

    focus("sess_3");

    expect(await screen.findByText("sandbox: no sandbox report")).toBeDefined();
    expect(screen.queryByText(/denied/)).toBeNull();
    expect(screen.queryByText(/tokens/)).toBeNull();
    expect(screen.queryByRole("img", { name: /context/ })).toBeNull();
  });
});

describe("the footer's usage poll", () => {
  it("the_footer_polls_usage_every_four_seconds_while_an_agent_session_is_focused", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });

    await renderApp("/session");
    focus("sess_1");
    await screen.findByText(/^sandbox:/);

    expect(ipc.usage.snapshot).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(4000);
    expect(ipc.usage.snapshot).toHaveBeenCalledTimes(2);

    await vi.advanceTimersByTimeAsync(4000);
    expect(ipc.usage.snapshot).toHaveBeenCalledTimes(3);
  });

  it("polling_stops_when_focus_moves_to_a_shell_session", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });

    await renderApp("/session");
    focus("sess_1");
    await screen.findByText(/^sandbox:/);
    await vi.advanceTimersByTimeAsync(4000);

    /* Proves the poll was actually running before the switch — without
     * this, a poll that never started would trivially "stay unchanged"
     * below and the test would pass for the wrong reason. */
    const callsBeforeSwitch = ipc.usage.snapshot.mock.calls.length;
    expect(callsBeforeSwitch).toBeGreaterThanOrEqual(2);

    focus("sess_2");
    expect(screen.queryByText(/^sandbox:/)).toBeNull();

    await vi.advanceTimersByTimeAsync(8000);
    expect(ipc.usage.snapshot.mock.calls.length).toBe(callsBeforeSwitch);
  });

  it("polling_stops_when_the_route_leaves_the_session_screen", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });

    const router = await renderApp("/session");
    focus("sess_1");
    await screen.findByText(/^sandbox:/);
    await vi.advanceTimersByTimeAsync(4000);

    const callsBeforeNavigate = ipc.usage.snapshot.mock.calls.length;
    expect(callsBeforeNavigate).toBeGreaterThanOrEqual(2);

    await act(async () => {
      await router.navigate({ to: "/sandbox" });
    });
    expect(screen.queryByText(/^sandbox:/)).toBeNull();

    await vi.advanceTimersByTimeAsync(8000);
    expect(ipc.usage.snapshot.mock.calls.length).toBe(callsBeforeNavigate);
  });
});
