import { describe, expect, it } from "vitest";
import { deniedCount, postureLine } from "@/lib/domain/sandbox";
import type { Denied, SandboxState } from "@/lib/proto";

const denied = (name: string, count: number): Denied => ({
  class: "syscall",
  name,
  count,
  first_at: "1757116800",
  last_at: "1757117100",
});

const sandbox = (over: Partial<SandboxState>): SandboxState => ({
  applied: [],
  unavailable: [],
  degraded: [],
  denied: [],
  ...over,
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
