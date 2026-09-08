import { describe, expect, it } from "vitest";
import { glyphFor, resolveCurrent } from "@/lib/domain/systems";
import type { Project } from "@/lib/proto";

function project(id: string, name: string): Project {
  return {
    id,
    name,
    slug: name,
    source: `C:\\github\\${name}`,
    workspace: `/home/willie/projects/${name}`,
    branch: "main",
    state: { state: "ready" },
    source_present: true,
    created_at: "1",
    sandbox: {},
  };
}

describe("resolveCurrent", () => {
  it("resolveCurrent_prefers_the_saved_system_then_the_first_then_null", () => {
    const willie = project("proj_1", "willie");
    const other = project("proj_2", "other-system");

    expect(resolveCurrent("proj_2", [willie, other])).toBe(other);
    expect(resolveCurrent("proj_missing", [willie, other])).toBe(willie);
    expect(resolveCurrent(null, [willie, other])).toBe(willie);
    expect(resolveCurrent(undefined, [])).toBeNull();
  });
});

describe("glyphFor", () => {
  it("takes the initials of up to two words, or the first two letters of one", () => {
    expect(glyphFor("Willie")).toBe("WI");
    expect(glyphFor("Web Autenticacao")).toBe("WA");
    expect(glyphFor("  spaced   name  ")).toBe("SN");
  });
});
