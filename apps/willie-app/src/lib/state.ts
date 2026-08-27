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
 * does, and every list mutation replaces an existing id in place (or
 * appends it if new) so an id never appears twice and its row does
 * not jump to the end of the list on every update.
 */
export function applyEvent(current: Snapshot, ev: Event): Snapshot {
  const seq = ev.seq;
  switch (ev.kind) {
    case "project_changed": {
      const idx = current.projects.findIndex((p) => p.id === ev.project.id);
      const projects =
        idx === -1
          ? [...current.projects, ev.project]
          : current.projects.map((p, i) => (i === idx ? ev.project : p));
      return { ...current, seq, projects };
    }
    case "project_removed":
      return {
        ...current,
        seq,
        projects: current.projects.filter((p) => p.id !== ev.id),
      };
    case "job_changed": {
      const idx = current.jobs.findIndex((j) => j.id === ev.job.id);
      const jobs =
        idx === -1
          ? [...current.jobs, ev.job]
          : current.jobs.map((j, i) => (i === idx ? ev.job : j));
      return { ...current, seq, jobs };
    }
  }
}
