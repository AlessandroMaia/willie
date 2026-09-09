import { describe, expect, it } from "vitest";
import { compactTokens, contextTone } from "@/lib/domain/usage";

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

describe("compactTokens", () => {
  it("leaves a count under a thousand as it is", () => {
    expect(compactTokens(0)).toBe("0");
    expect(compactTokens(999)).toBe("999");
  });

  it("keeps one decimal below ten thousand, where it still carries", () => {
    expect(compactTokens(1000)).toBe("1.0k");
    expect(compactTokens(4200)).toBe("4.2k");
    expect(compactTokens(9949)).toBe("9.9k");
  });

  it("drops the decimal past ten thousand", () => {
    expect(compactTokens(12004)).toBe("12k");
    expect(compactTokens(184320)).toBe("184k");
  });

  it("carries to millions", () => {
    expect(compactTokens(1088547)).toBe("1.1M");
    expect(compactTokens(23400000)).toBe("23M");
  });
});
