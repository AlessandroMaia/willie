import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SessionScreen } from "@/features/session/session-screen";
import { SessionsPanel } from "@/features/session/sessions-panel";
import type { EngineStatus } from "@/lib/ipc";
import type { Event, Project, Session, Snapshot } from "@/lib/proto";
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

/* Same fake xterm the Session screen's own suite uses (see
 * session-screen.test.tsx): only its shape matters here, never a real
 * canvas or renderer jsdom cannot provide. */
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
  onDaemonEvent: vi.fn(async (_cb: (ev: Event) => void) => () => {}),
  ui: { prefs: vi.fn(), setPrefs: vi.fn() },
  sessions: { open: vi.fn(), resume: vi.fn(), rename: vi.fn() },
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

/* Same probe session-screen.test.tsx uses: renders alongside the
 * screen so a test can read what the active tab told
 * `useFocusedSession` without reaching into the screen's own state. */
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

describe("SessionsPanel", () => {
  it("the_panel_lists_live_then_finished_sessions", async () => {
    const user = userEvent.setup();

    render(
      <SessionsPanel
        live={[session({ id: "sess_live_1", label: "live one" })]}
        finished={[
          session({
            id: "sess_done_1",
            label: "finished one",
            state: { state: "exited", code: 0, signal: null },
            finished_at: "100",
          }),
        ]}
        onOpen={vi.fn()}
        onResume={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Sessions" }));

    const liveRow = await screen.findByText("live one");
    const finishedRow = screen.getByText("finished one");
    /* "Live" before "Finished" is a DOM-order fact, not a labeling
     * one: `compareDocumentPosition` is the only reliable way to
     * assert one node actually precedes the other in the tree. */
    const position = liveRow.compareDocumentPosition(finishedRow);
    expect(Boolean(position & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);

    expect(screen.getByRole("button", { name: "Open" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Resume" })).toBeDefined();
  });

  /* The harness continues the workspace's most recent conversation, so
   * a Resume on an older row would silently reopen a different session
   * than the one it names, and a shell has no conversation at all. */
  it("only_the_latest_finished_agent_session_offers_resume", async () => {
    const user = userEvent.setup();

    render(
      <SessionsPanel
        live={[]}
        finished={[
          session({
            id: "sess_shell_1",
            kind: "shell",
            state: { state: "exited", code: 0, signal: null },
            finished_at: "300",
          }),
          session({
            id: "sess_done_2",
            label: "newest agent",
            state: { state: "exited", code: 0, signal: null },
            finished_at: "200",
          }),
          session({
            id: "sess_done_1",
            label: "older agent",
            state: { state: "exited", code: 0, signal: null },
            finished_at: "100",
          }),
        ]}
        onOpen={vi.fn()}
        onResume={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Sessions" }));
    await screen.findByText("newest agent");

    expect(screen.getAllByRole("button", { name: "Resume" })).toHaveLength(1);
    expect(screen.getByText("only the latest can be resumed")).toBeDefined();
    /* The finished shell is named like its live tab, and says why it
     * has no Resume rather than offering one the daemon would honour
     * as a fresh agent session. */
    expect(screen.getByText("$ zsh")).toBeDefined();
    expect(screen.getByText("no conversation")).toBeDefined();
  });
});

describe("resuming a finished session from the panel", () => {
  it("resume_calls_session_resume_with_the_chosen_target_and_focuses_it", async () => {
    const user = userEvent.setup();
    ipc.projects.snapshot.mockResolvedValue(
      snapshot([
        session({ id: "sess_agent_1", label: "current" }),
        session({
          id: "sess_done_1",
          label: "old work",
          state: { state: "exited", code: 0, signal: null },
          finished_at: "1",
        }),
      ]),
    );

    render(
      <>
        <SessionScreen />
        <FocusedProbe />
      </>,
    );
    await screen.findByRole("tab", { name: /current/ });

    await user.click(screen.getByRole("button", { name: "Sessions" }));
    await user.click(await screen.findByRole("button", { name: "Resume" }));

    expect(ipc.sessions.resume).toHaveBeenCalledWith("proj_1", "sess_done_1");

    /* The bridge call resolving is not what focuses the new tab: the
     * daemon announcing it through `session_changed` is, exactly the
     * same "arrived" rule the Session screen already applies to a
     * freshly opened session. */
    const call = ipc.onDaemonEvent.mock.calls[0];
    if (!call) throw new Error("onDaemonEvent was never subscribed");
    const [onEvent] = call;
    onEvent({
      seq: 2,
      kind: "session_changed",
      session: session({
        id: "sess_new_1",
        label: "old work",
        created_at: "2",
        resumed_from: "sess_done_1",
      }),
    });

    await vi.waitFor(() =>
      expect(screen.getByTestId("focused").textContent).toBe("sess_new_1"),
    );
  });
});
