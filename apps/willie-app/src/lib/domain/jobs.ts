import type { Job } from "@/lib/proto";

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

/* The Tools screen tracks both kinds a tool action can start
 * (`install_harness` for a missing tool, `update_harness` for one
 * already present) as a single lineage: only one can ever be running
 * at a time (the daemon refuses a second tool job outright), so the
 * newest of either kind is always the one worth showing. */
export function latestToolJob(jobs: Job[]): Job | undefined {
  let latest: Job | undefined;
  for (const j of jobs) {
    if (j.kind !== "install_harness" && j.kind !== "update_harness") continue;
    if (!latest || isNewer(j, latest)) latest = j;
  }
  return latest;
}

/* `log_tail` is a multi-line buffer the daemon keeps appending to as
 * the job runs, often with trailing blank lines; only the last real
 * line is worth showing beside a check or a tool row. */
export function lastLogLine(job: Job): string {
  const lines = job.log_tail.split("\n");
  while (lines.length > 0 && lines[lines.length - 1]?.trim() === "") {
    lines.pop();
  }
  return lines[lines.length - 1] ?? "";
}
