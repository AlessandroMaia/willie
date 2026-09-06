import { describe, expect, it } from "vitest";
import {
  denials,
  isLive,
  liveCount,
  liveSessions,
  recentTerminal,
  sandboxPosture,
} from "@/lib/domain/sessions";
import type { Denied, SandboxState, Session } from "@/lib/proto";

const s = (
  id: string,
  project_id: string,
  state: Session["state"],
  created_at: string,
  finished_at?: string,
): Session => ({
  id,
  project_id,
  harness: "claude-code",
  workspace: "/w",
  state,
  created_at,
  finished_at,
  clients: 0,
});

const sandbox = (over: Partial<SandboxState>): SandboxState => ({
  applied: [],
  unavailable: [],
  degraded: [],
  denied: [],
  ...over,
});

const denied = (cls: Denied["class"], name: string, count: number): Denied => ({
  class: cls,
  name,
  count,
  first_at: "2026-09-06T00:00:00Z",
  last_at: "2026-09-06T00:05:00Z",
});

describe("session helpers", () => {
  it("counts only live sessions for a project", () => {
    const list = [
      s("a", "p1", { state: "running" }, "3"),
      s("b", "p1", { state: "exited", code: 0, signal: null }, "2"),
      s("c", "p1", { state: "creating" }, "1"),
      s("d", "p2", { state: "running" }, "1"),
    ];
    expect(liveCount(list, "p1")).toBe(2); // running + creating, not exited
    expect(isLive(s("x", "p", { state: "stopping" }, "1"))).toBe(true);
    expect(
      isLive(
        s(
          "x",
          "p",
          { state: "failed", code: "e", message: "m", remediation: "r" },
          "1",
        ),
      ),
    ).toBe(false);
  });

  it("lists live sessions newest first, excluding terminal ones", () => {
    const list = [
      s("old-live", "p", { state: "running" }, "2"),
      s("dead", "p", { state: "exited", code: 0, signal: null }, "9"),
      s("new-live", "p", { state: "running" }, "5"),
    ];
    expect(liveSessions(list).map((x) => x.id)).toEqual([
      "new-live",
      "old-live",
    ]);
  });

  it("returns the most recent finished sessions, capped, newest first", () => {
    const list = [
      s("live", "p", { state: "running" }, "50"),
      s("f-old", "p", { state: "exited", code: 0, signal: null }, "1", "10"),
      s(
        "f-mid",
        "p",
        { state: "failed", code: "e", message: "m", remediation: "r" },
        "1",
        "20",
      ),
      s("f-new", "p", { state: "exited", code: 1, signal: null }, "1", "30"),
    ];
    expect(recentTerminal(list, 2).map((x) => x.id)).toEqual([
      "f-new",
      "f-mid",
    ]);
    expect(recentTerminal(list, 20).map((x) => x.id)).toEqual([
      "f-new",
      "f-mid",
      "f-old",
    ]);
  });
});

describe("sandbox posture", () => {
  it("is full when everything reported applied and nothing is missing", () => {
    const sb = sandbox({ applied: ["namespaces", "mounts", "seccomp"] });
    const session = { ...s("a", "p", { state: "running" }, "1"), sandbox: sb };
    expect(sandboxPosture(session)).toBe("full");
  });

  it("is reduced when a mechanism is unavailable", () => {
    const sb = sandbox({ applied: ["mounts"], unavailable: ["landlock"] });
    const session = { ...s("a", "p", { state: "running" }, "1"), sandbox: sb };
    expect(sandboxPosture(session)).toBe("reduced");
  });

  it("is reduced when a mechanism degraded", () => {
    const sb = sandbox({ applied: ["seccomp"], degraded: ["seccomp"] });
    const session = { ...s("a", "p", { state: "running" }, "1"), sandbox: sb };
    expect(sandboxPosture(session)).toBe("reduced");
  });

  it("is unknown when nothing is applied yet", () => {
    const sb = sandbox({ applied: [] });
    const session = { ...s("a", "p", { state: "creating" }, "1"), sandbox: sb };
    expect(sandboxPosture(session)).toBe("unknown");
  });

  it("is unknown for a session with no sandbox field at all", () => {
    expect(sandboxPosture(s("a", "p", { state: "running" }, "1"))).toBe(
      "unknown",
    );
  });
});

describe("sandbox denials", () => {
  it("sorts by count, highest first, and totals every count", () => {
    const sb = sandbox({
      denied: [
        denied("syscall", "unshare", 3),
        denied("terminal", "clipboard", 5),
      ],
    });
    const session = { ...s("a", "p", { state: "running" }, "1"), sandbox: sb };
    const { items, total } = denials(session);
    expect(items.map((d) => d.name)).toEqual(["clipboard", "unshare"]);
    expect(total).toBe(8);
  });

  it("is empty with a zero total when there are no denials", () => {
    expect(denials(s("a", "p", { state: "running" }, "1"))).toEqual({
      items: [],
      total: 0,
    });
  });
});
