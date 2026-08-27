import { describe, expect, it } from "vitest";
import { latestJobFor } from "./jobs";
import type { Job } from "./proto";

/* `startedAt` mirrors the wire: the daemon stamps `started_at` as whole
 * epoch seconds in decimal, not as a calendar timestamp. */
const job = (id: string, startedAt: string, projectId = "proj_1"): Job => ({
  id,
  kind: "sync_to_windows",
  project_id: projectId,
  state: { state: "running" },
  started_at: startedAt,
  log_tail: "",
});

describe("latestJobFor", () => {
  it("breaks a started_at tie on the higher job id, in either order", () => {
    const older = job("job_01AAAAAAAAAAAAAAAAAAAAAAAA", "1756300000");
    const newer = job("job_01BBBBBBBBBBBBBBBBBBBBBBBB", "1756300000");
    expect(latestJobFor([older, newer], "proj_1")?.id).toBe(newer.id);
    expect(latestJobFor([newer, older], "proj_1")?.id).toBe(newer.id);
  });

  it("prefers a later started_at over a higher id", () => {
    const earlierHigherId = job("job_01ZZZZZZZZZZZZZZZZZZZZZZZZ", "1756300000");
    const laterLowerId = job("job_01AAAAAAAAAAAAAAAAAAAAAAAA", "1756300001");
    expect(latestJobFor([earlierHigherId, laterLowerId], "proj_1")?.id).toBe(
      laterLowerId.id,
    );
  });

  it("ignores jobs that belong to another project", () => {
    const mine = job("job_01AAAAAAAAAAAAAAAAAAAAAAAA", "1756300000");
    const other = job("job_01ZZZZZZZZZZZZZZZZZZZZZZZZ", "1756300001", "proj_2");
    expect(latestJobFor([mine, other], "proj_1")?.id).toBe(mine.id);
  });

  it("returns undefined when no job matches the project", () => {
    expect(latestJobFor([], "proj_1")).toBeUndefined();
  });
});
