import { describe, expect, it } from "vitest";
import {
  counts,
  type DenialRow,
  denialRows,
  deniedCount,
  explain,
  posture,
  postureLine,
} from "@/lib/domain/sandbox";
import type { Denied, SandboxState, Session } from "@/lib/proto";

const denied = (name: string, count: number): Denied => ({
  class: "syscall",
  name,
  count,
  first_at: "1757116800",
  last_at: "1757117100",
});

const deniedAt = (
  cls: Denied["class"],
  name: string,
  count: number,
  lastAt: string,
): Denied => ({ class: cls, name, count, first_at: lastAt, last_at: lastAt });

const sandbox = (over: Partial<SandboxState>): SandboxState => ({
  applied: [],
  unavailable: [],
  degraded: [],
  denied: [],
  ...over,
});

const session = (id: string, sb?: SandboxState): Session => ({
  id,
  project_id: "proj_1",
  harness: "claude-code",
  workspace: "/w",
  state: { state: "running" },
  created_at: "1",
  clients: 0,
  sandbox: sb,
});

describe("postureLine", () => {
  it("joins the applied mechanisms with a middle dot", () => {
    expect(postureLine(sandbox({ applied: ["seccomp", "namespaces"] }))).toBe(
      "seccomp · namespaces",
    );
  });

  it("is empty when nothing has applied yet — callers must check sandboxPosture first, never render this alone", () => {
    expect(postureLine(sandbox({}))).toBe("");
  });
});

describe("deniedCount", () => {
  it("sums every denial's count", () => {
    const sb = sandbox({
      denied: [denied("ptrace", 3), denied("clipboard", 2)],
    });
    expect(deniedCount(sb)).toBe(5);
  });

  it("is zero when nothing was denied", () => {
    expect(deniedCount(sandbox({}))).toBe(0);
  });
});

describe("posture", () => {
  it("posture_unions_the_sessions_mechanisms", () => {
    const s1 = session(
      "sess_1",
      sandbox({ applied: ["seccomp"], unavailable: ["landlock"] }),
    );
    const s2 = session(
      "sess_2",
      sandbox({ applied: ["seccomp", "namespaces"], degraded: ["rlimits"] }),
    );

    expect(posture([s1, s2])).toEqual({
      applied: ["seccomp", "namespaces"],
      unavailable: ["landlock"],
      degraded: ["rlimits"],
    });
  });

  it("contributes nothing for a session with no sandbox report yet", () => {
    const creating = session("sess_3");
    expect(posture([creating])).toEqual({
      applied: [],
      unavailable: [],
      degraded: [],
    });
  });
});

describe("denialRows", () => {
  it("denialRows_flattens_and_sorts_newest_first", () => {
    const s1 = session(
      "sess_1",
      sandbox({
        denied: [
          deniedAt("syscall", "ptrace", 2, "100"),
          deniedAt("terminal", "osc52", 1, "300"),
        ],
      }),
    );
    const s2 = session(
      "sess_2",
      sandbox({ denied: [deniedAt("syscall", "unshare", 4, "200")] }),
    );

    expect(denialRows([s1, s2])).toEqual<DenialRow[]>([
      {
        sessionId: "sess_1",
        class: "terminal",
        name: "osc52",
        count: 1,
        lastAt: "300",
      },
      {
        sessionId: "sess_2",
        class: "syscall",
        name: "unshare",
        count: 4,
        lastAt: "200",
      },
      {
        sessionId: "sess_1",
        class: "syscall",
        name: "ptrace",
        count: 2,
        lastAt: "100",
      },
    ]);
  });

  it("treats a non-numeric last_at as the oldest, never throwing", () => {
    const s1 = session(
      "sess_1",
      sandbox({ denied: [deniedAt("syscall", "ptrace", 1, "not-a-number")] }),
    );
    const s2 = session(
      "sess_2",
      sandbox({ denied: [deniedAt("syscall", "unshare", 1, "50")] }),
    );

    expect(denialRows([s1, s2]).map((r) => r.sessionId)).toEqual([
      "sess_2",
      "sess_1",
    ]);
  });

  it("skips a session with no sandbox report yet", () => {
    expect(denialRows([session("sess_1")])).toEqual([]);
  });
});

describe("counts", () => {
  it("sums syscalls and terminal denials separately and counts distinct sessions", () => {
    const rows: DenialRow[] = [
      {
        sessionId: "a",
        class: "syscall",
        name: "ptrace",
        count: 2,
        lastAt: "1",
      },
      {
        sessionId: "a",
        class: "terminal",
        name: "osc52",
        count: 1,
        lastAt: "2",
      },
      {
        sessionId: "b",
        class: "syscall",
        name: "unshare",
        count: 3,
        lastAt: "3",
      },
    ];

    expect(counts(rows)).toEqual({ syscalls: 5, terminal: 1, sessions: 2 });
  });

  it("is all zero for an empty list", () => {
    expect(counts([])).toEqual({ syscalls: 0, terminal: 0, sessions: 0 });
  });
});

describe("explain", () => {
  it("explains the known syscall denials by name", () => {
    expect(explain("syscall", "prctl")).toBe(
      "the session tried to install its own syscall filter",
    );
    expect(explain("syscall", "ioctl")).toBe(
      "injection into the controlling terminal",
    );
  });

  it("falls back for an unrecognised syscall name without throwing", () => {
    expect(explain("syscall", "some_future_syscall")).toBe(
      "a syscall this sandbox does not allow",
    );
  });

  it("explains every terminal denial the same way regardless of name", () => {
    const line =
      "a sequence that acts on the host, filtered from the terminal output";
    expect(explain("terminal", "osc52")).toBe(line);
    expect(explain("terminal", "some-future-sequence")).toBe(line);
  });
});
