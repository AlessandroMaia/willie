import { describe, expect, it } from "vitest";
import { badgeFor } from "@/features/sessions/session-state-badge";

describe("badgeFor", () => {
  it("maps each state to a label and a tone", () => {
    expect(badgeFor({ state: "running" })).toEqual({
      label: "running",
      tone: "ok",
    });
    expect(badgeFor({ state: "creating" }).tone).toBe("pending");
    expect(
      badgeFor({
        state: "failed",
        code: "supervisor_lost",
        message: "m",
        remediation: "r",
      }).tone,
    ).toBe("error");
    expect(badgeFor({ state: "exited", code: 0, signal: null })).toEqual({
      label: "exited 0",
      tone: "muted",
    });
    expect(badgeFor({ state: "exited", code: 2, signal: null }).tone).toBe(
      "error",
    );
  });

  it("marks a signal-terminated exit as an error, not a clean exit", () => {
    const badge = badgeFor({ state: "exited", code: null, signal: 9 });

    expect(badge.label).toContain("signal");
    expect(badge.tone).toBe("error");
  });
});
