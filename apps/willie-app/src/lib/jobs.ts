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

/* `started_at` is the daemon's whole-second stamp, so two jobs on one
 * project can share it and the tiebreak falls to `id` — a ULID, which is
 * time-ordered across milliseconds and random only within one. A
 * project's jobs are serialised one at a time, so a same-second pair is
 * still milliseconds apart and the larger id is the newer job. */
function isNewer(a: Job, b: Job): boolean {
  if (a.started_at !== b.started_at) return a.started_at > b.started_at;
  return a.id > b.id;
}

/* The harness install is a tool job (no project_id), so `latestJobFor`
 * never surfaces it; the Dashboard tracks it by kind instead. */
export function latestInstallJob(jobs: Job[]): Job | undefined {
  let latest: Job | undefined;
  for (const j of jobs) {
    if (j.kind !== "install_harness") continue;
    if (!latest || isNewer(j, latest)) latest = j;
  }
  return latest;
}
