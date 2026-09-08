import { describe, expect, it } from "vitest";
import { joinPath, sortEntries } from "@/lib/domain/tree";
import type { TreeEntry } from "@/lib/proto";

const dir = (name: string, git?: string | null): TreeEntry => ({
  name,
  kind: "dir",
  git,
});

const file = (name: string, git?: string | null): TreeEntry => ({
  name,
  kind: "file",
  git,
});

describe("sortEntries", () => {
  it("directories sort before files regardless of name", () => {
    const sorted = sortEntries([file("aardvark.ts"), dir("zebra")]);
    expect(sorted.map((e) => e.name)).toEqual(["zebra", "aardvark.ts"]);
  });

  it("same-kind entries sort case-insensitively by name", () => {
    const sorted = sortEntries([file("Banana.ts"), file("apple.ts")]);
    expect(sorted.map((e) => e.name)).toEqual(["apple.ts", "Banana.ts"]);
  });

  it("never mutates the array it was given", () => {
    const original = [file("b.ts"), file("a.ts")];
    sortEntries(original);
    expect(original.map((e) => e.name)).toEqual(["b.ts", "a.ts"]);
  });
});

describe("joinPath", () => {
  it("an empty base returns the bare name", () => {
    expect(joinPath("", "src")).toBe("src");
  });

  it("a non-empty base joins with a single slash", () => {
    expect(joinPath("src", "index.ts")).toBe("src/index.ts");
  });

  it("strips a trailing slash from the base and a leading dot-slash from the name", () => {
    expect(joinPath("src/", "./index.ts")).toBe("src/index.ts");
  });
});
