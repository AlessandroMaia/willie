import type { Job } from "./proto";

/* The UI never computes project or job truth: this is a read-only
 * projection over the job list the snapshot hands the store — the
 * newest job for one project, nothing invented. */
export function latestJobFor(jobs: Job[], projectId: string): Job | undefined {
  let latest: Job | undefined;
  for (const job of jobs) {
    if (job.project_id !== projectId) continue;
    if (!latest || isNewer(job, latest)) latest = job;
  }
  return latest;
}

/* `started_at` is whole-second resolution, so two jobs on the same
 * project can share a timestamp; the tiebreak falls back to `id` — a
 * ULID, monotonic by creation, so the lexicographically larger id was
 * minted later and is the newer job. */
function isNewer(a: Job, b: Job): boolean {
  if (a.started_at !== b.started_at) return a.started_at > b.started_at;
  return a.id > b.id;
}
