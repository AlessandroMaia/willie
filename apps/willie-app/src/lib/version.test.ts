import { describe, expect, it } from "vitest";
import { formatVersion } from "./version";

describe("formatVersion", () => {
  it("prefixes a known version with v", () => {
    expect(formatVersion("0.1.0")).toBe("v0.1.0");
  });

  it("explains when the version is unknown", () => {
    expect(formatVersion(null)).toBe("version unavailable");
  });
});
