import { describe, expect, it } from "vitest";
import type { Project, Snapshot } from "./proto";
import { applyEvent, needsResnapshot } from "./state";

const proj = (id: string, name: string): Project => ({
  id,
  name,
  slug: name,
  source: `C:\\x\\${name}`,
  workspace: `/home/willie/projects/${name}`,
  branch: "main",
  state: { state: "ready" },
  source_present: true,
  created_at: "t",
});

const empty: Snapshot = { seq: 0, projects: [], jobs: [] };

describe("daemon store", () => {
  it("applies a project_changed event and bumps seq", () => {
    const next = applyEvent(empty, {
      seq: 1,
      kind: "project_changed",
      project: proj("proj_1", "a"),
    });
    expect(next.seq).toBe(1);
    expect(next.projects).toHaveLength(1);
  });

  it("replaces a project on a second change, not duplicates it", () => {
    const one = applyEvent(empty, {
      seq: 1,
      kind: "project_changed",
      project: proj("proj_1", "a"),
    });
    const two = applyEvent(one, {
      seq: 2,
      kind: "project_changed",
      project: { ...proj("proj_1", "a"), name: "renamed" },
    });
    expect(two.projects).toHaveLength(1);
    expect(two.projects[0]?.name).toBe("renamed");
  });

  it("keeps a changed project's index instead of moving it to the end", () => {
    const withTwo = applyEvent(
      applyEvent(empty, {
        seq: 1,
        kind: "project_changed",
        project: proj("proj_1", "a"),
      }),
      { seq: 2, kind: "project_changed", project: proj("proj_2", "b") },
    );
    const renamed = applyEvent(withTwo, {
      seq: 3,
      kind: "project_changed",
      project: { ...proj("proj_1", "a"), name: "renamed" },
    });
    expect(renamed.projects.map((p) => p.id)).toEqual(["proj_1", "proj_2"]);
    expect(renamed.projects[0]?.name).toBe("renamed");
  });

  it("does not mutate the snapshot it is given", () => {
    const withOne = applyEvent(empty, {
      seq: 1,
      kind: "project_changed",
      project: proj("proj_1", "a"),
    });
    const before = structuredClone(withOne);
    applyEvent(withOne, {
      seq: 2,
      kind: "project_changed",
      project: { ...proj("proj_1", "a"), name: "renamed" },
    });
    expect(withOne).toEqual(before);
  });

  it("removes a project", () => {
    const one = applyEvent(empty, {
      seq: 1,
      kind: "project_changed",
      project: proj("proj_1", "a"),
    });
    const gone = applyEvent(one, {
      seq: 2,
      kind: "project_removed",
      id: "proj_1",
    });
    expect(gone.projects).toHaveLength(0);
  });

  it("flags a resnapshot on a seq gap", () => {
    expect(
      needsResnapshot(empty, { seq: 5, kind: "project_removed", id: "x" }),
    ).toBe(true);
    expect(
      needsResnapshot(empty, { seq: 1, kind: "project_removed", id: "x" }),
    ).toBe(false);
  });
});
