import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionScreen } from "@/features/session/session-screen";
import type { EngineStatus } from "@/lib/ipc";
import type { Project, Session, Snapshot } from "@/lib/proto";
import { useFocusedSession } from "@/store/use-focused-session";
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

/* The bridge is the only I/O in the frontend; `xterm` is mocked the
 * same way — a fake standing in for the real terminal so the effect
 * inside `SessionTerminal` runs without touching a real canvas or
 * renderer, which jsdom cannot provide. */
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

const ipc = vi.hoisted(() => ({
  engine: { status: vi.fn(), onStatus: vi.fn(async () => () => {}) },
  projects: { snapshot: vi.fn() },
  onDaemonEvent: vi.fn(async () => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
  sessions: { open: vi.fn(), resume: vi.fn() },
  sessionTerminal: {
    open: vi.fn(async () => {}),
    input: vi.fn(async () => {}),
    resize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  },
  onSessionOutput: vi.fn(async () => () => {}),
}));

vi.mock("@/lib/ipc", () => ipc);

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

function session(over: Partial<Session> & { id: string }): Session {
  return {
    project_id: "proj_1",
    harness: "claude-code",
    workspace: "/home/willie/projects/willie",
    state: { state: "running" },
    created_at: "1",
    clients: 0,
    ...over,
  };
}

function snapshot(sessions: Session[] = []): Snapshot {
  return { seq: 1, projects: [project()], jobs: [], sessions };
}

/* A probe that renders alongside the screen so a test can observe what
 * the active tab told `useFocusedSession` without reaching into the
 * screen's own internal state. */
function FocusedProbe() {
  const { sessionId } = useFocusedSession();
  return <div data-testid="focused">{sessionId ?? "none"}</div>;
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  ipc.engine.status.mockResolvedValue(STATUS);
  ipc.ui.prefs.mockResolvedValue({ current_project: null });
  ipc.projects.snapshot.mockResolvedValue(snapshot());
});

describe("SessionScreen", () => {
  it("n_live_sessions_render_n_tabs_each_with_its_own_terminal", async () => {
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({ id: "sess_agent_1", label: "first" }),
        session({ id: "sess_agent_2", label: "second", created_at: "2" }),
        session({ id: "sess_shell_1", kind: "shell", created_at: "3" }),
      ]),
    );

    render(<SessionScreen />);

    expect(await screen.findAllByRole("tab")).toHaveLength(3);
    expect(ipc.sessionTerminal.open).toHaveBeenCalledTimes(3);
    expect(ipc.sessionTerminal.open).toHaveBeenCalledWith("sess_agent_1");
    expect(ipc.sessionTerminal.open).toHaveBeenCalledWith("sess_agent_2");
    expect(ipc.sessionTerminal.open).toHaveBeenCalledWith("sess_shell_1");
  });

  it("inactive_tabs_stay_mounted_and_only_toggle_hidden", async () => {
    const user = userEvent.setup();
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({ id: "sess_agent_1", label: "first" }),
        session({ id: "sess_agent_2", label: "second", created_at: "2" }),
        session({ id: "sess_agent_3", label: "third", created_at: "3" }),
      ]),
    );

    const { container } = render(<SessionScreen />);
    await screen.findByRole("tab", { name: /third/ });

    /* Each terminal's container carries `title={sessionName}` (see
     * `SessionTerminal`), so its ancestor tabpanel is found the same
     * way regardless of which tab is currently active. */
    function panelFor(label: string): HTMLElement {
      const marker = container.querySelector(`[title="${label}"]`);
      const panel = marker?.closest('[role="tabpanel"]');
      if (!panel) throw new Error(`no tabpanel hosting "${label}"`);
      return panel as HTMLElement;
    }

    function panelCount(): number {
      return container.querySelectorAll('[role="tabpanel"]').length;
    }

    await vi.waitFor(() => {
      expect(panelCount()).toBe(3);
      expect(panelFor("third").hasAttribute("hidden")).toBe(false);
    });
    expect(panelFor("first").hasAttribute("hidden")).toBe(true);
    expect(panelFor("second").hasAttribute("hidden")).toBe(true);

    const callsBeforeSwitch = ipc.sessionTerminal.open.mock.calls.length;
    expect(callsBeforeSwitch).toBe(3);

    await user.click(screen.getByRole("tab", { name: /first/ }));

    await vi.waitFor(() => {
      expect(panelFor("first").hasAttribute("hidden")).toBe(false);
    });
    expect(panelFor("third").hasAttribute("hidden")).toBe(true);
    expect(panelFor("second").hasAttribute("hidden")).toBe(true);
    /* Still mounted, never remounted: the same three terminals opened
     * once each, switching tabs never asks for a fresh one. */
    expect(panelCount()).toBe(3);
    expect(ipc.sessionTerminal.open.mock.calls.length).toBe(callsBeforeSwitch);
  });

  it("plus_offers_a_new_session_and_a_new_zsh", async () => {
    const user = userEvent.setup();
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([session({ id: "sess_agent_1", label: "first" })]),
    );

    render(<SessionScreen />);
    await screen.findByRole("tab", { name: /first/ });

    await user.click(screen.getByRole("button", { name: "New tab" }));
    await user.click(
      await screen.findByRole("menuitem", { name: "New session" }),
    );

    expect(ipc.sessions.open).toHaveBeenCalledWith("proj_1");

    await user.click(screen.getByRole("button", { name: "New tab" }));
    await user.click(await screen.findByRole("menuitem", { name: "New zsh" }));

    expect(ipc.sessions.open).toHaveBeenCalledWith("proj_1", "shell");
  });

  it("switching_tabs_sets_the_focused_session", async () => {
    const user = userEvent.setup();
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({ id: "sess_agent_1", label: "first" }),
        session({ id: "sess_agent_2", label: "second", created_at: "2" }),
      ]),
    );

    render(
      <>
        <SessionScreen />
        <FocusedProbe />
      </>,
    );
    await screen.findByRole("tab", { name: /first/ });

    await vi.waitFor(() =>
      expect(screen.getByTestId("focused").textContent).toBe("sess_agent_2"),
    );

    await user.click(screen.getByRole("tab", { name: /first/ }));

    await vi.waitFor(() =>
      expect(screen.getByTestId("focused").textContent).toBe("sess_agent_1"),
    );
  });

  it("a_system_without_sessions_shows_the_empty_state_with_resume_when_one_finished", async () => {
    const user = userEvent.setup();
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({
          id: "sess_done_1",
          label: "old work",
          state: { state: "exited", code: 0, signal: null },
          finished_at: "2",
        }),
      ]),
    );

    render(<SessionScreen />);

    expect(await screen.findByText("No live sessions")).toBeDefined();
    expect(
      screen.getByRole("button", { name: "Resume old work" }),
    ).toBeDefined();

    await user.click(screen.getByRole("button", { name: "New session" }));
    expect(ipc.sessions.open).toHaveBeenCalledWith("proj_1");

    await user.click(screen.getByRole("button", { name: "Resume old work" }));
    expect(ipc.sessions.resume).toHaveBeenCalledWith("proj_1", "sess_done_1");
  });

  it("a_failed_new_session_shows_the_problem_instead_of_failing_silently", async () => {
    const user = userEvent.setup();
    ipc.sessions.open.mockRejectedValueOnce({
      code: "shell_unavailable",
      message: "zsh is not installed in the distro",
      remediation: "",
    });

    render(<SessionScreen />);

    await user.click(
      await screen.findByRole("button", { name: "New session" }),
    );

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("shell_unavailable");
    expect(alert.textContent).toContain("zsh is not installed in the distro");
  });
});
