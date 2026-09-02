import { describe, expect, it } from "vitest";
import {
  isLive,
  liveCount,
  liveSessions,
  recentTerminal,
} from "@/lib/domain/sessions";
import type { Session } from "@/lib/proto";

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
