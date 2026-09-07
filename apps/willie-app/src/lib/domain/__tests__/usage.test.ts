import { describe, expect, it } from "vitest";
import { contextTone } from "@/lib/domain/usage";

describe("contextTone", () => {
  it("is muted when the harness has not reported a context window yet", () => {
    expect(contextTone(null)).toBe("muted");
  });

  it("is ok well below the warning threshold", () => {
    expect(contextTone(0)).toBe("ok");
    expect(contextTone(74)).toBe("ok");
  });

  it("is warning at and past 75%", () => {
    expect(contextTone(75)).toBe("warning");
    expect(contextTone(89)).toBe("warning");
  });

  it("is error at and past 90%", () => {
    expect(contextTone(90)).toBe("error");
    expect(contextTone(100)).toBe("error");
  });
});
