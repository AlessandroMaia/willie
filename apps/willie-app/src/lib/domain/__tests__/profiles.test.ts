import { describe, expect, it } from "vitest";
import { EDITABLE_FRAGMENTS, summarizeChanges } from "@/lib/domain/profiles";
import type { Change } from "@/lib/proto";

const change = (path: string, kind: Change["kind"]): Change => ({
  path,
  kind,
  after: "content",
});

describe("summarizeChanges", () => {
  it("counts a mixed change list by kind", () => {
    const changes = [
      change("settings.json", "merge"),
      change("CLAUDE.md", "create"),
      change("mcp.json", "merge"),
      change(".claude/settings.json", "overwrite"),
    ];

    expect(summarizeChanges(changes)).toEqual({
      create: 1,
      merge: 2,
      overwrite: 1,
    });
  });

  it("zero-fills a kind that never appears", () => {
    expect(summarizeChanges([change("a", "create")])).toEqual({
      create: 1,
      merge: 0,
      overwrite: 0,
    });
  });

  it("is all zero for an empty change list", () => {
    expect(summarizeChanges([])).toEqual({
      create: 0,
      merge: 0,
      overwrite: 0,
    });
  });
});

describe("EDITABLE_FRAGMENTS", () => {
  it("offers exactly the three name-only fragments", () => {
    expect(EDITABLE_FRAGMENTS.map((f) => f.id)).toEqual([
      "settings",
      "instructions",
      "mcp",
    ]);
  });
});
