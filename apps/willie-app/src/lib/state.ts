import type { Event, Snapshot } from "./proto";

/**
 * True when `ev` does not extend `current` directly — either an event
 * was missed (a seq gap) or the daemon restarted and re-began its own
 * sequence (`ev.seq` behind `current.seq`). Either way the incremental
 * `applyEvent` history is no longer trustworthy and the caller must
 * fetch a fresh `state_snapshot` instead of applying `ev`.
 */
export function needsResnapshot(current: Snapshot, ev: Event): boolean {
  return ev.seq !== current.seq + 1;
}

/**
 * Pure reducer: folds one daemon event into a snapshot. The UI never
 * computes project or job truth itself — this is the only place that
 * does, and every list mutation is a filter-then-push so an id never
 * appears twice.
 */
export function applyEvent(current: Snapshot, ev: Event): Snapshot {
  const seq = ev.seq;
  switch (ev.kind) {
    case "project_changed": {
      const projects = current.projects.filter((p) => p.id !== ev.project.id);
      projects.push(ev.project);
      return { ...current, seq, projects };
    }
    case "project_removed":
      return {
        ...current,
        seq,
        projects: current.projects.filter((p) => p.id !== ev.id),
      };
    case "job_changed": {
      const jobs = current.jobs.filter((j) => j.id !== ev.job.id);
      jobs.push(ev.job);
      return { ...current, seq, jobs };
    }
  }
}
