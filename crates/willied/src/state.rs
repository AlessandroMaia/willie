//! The daemon's in-memory truth: projects (mirroring the TOML files),
//! jobs (transient), and a monotonic sequence for the event stream.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

use willie_core::{
    id::{JobId, ProjectId},
    project::Project,
};
use willie_proto::{
    job::Job,
    state::{Event, EventKind, Snapshot},
};

use crate::{outbound::Outbound, store};

#[derive(Debug, Default)]
pub struct State {
    pub projects: BTreeMap<ProjectId, Project>,
    pub jobs: BTreeMap<JobId, Job>,
    pub seq: u64,
}

impl State {
    #[must_use]
    pub fn load(state_dir: &Path) -> Self {
        let mut projects = BTreeMap::new();
        for p in store::load_all(state_dir) {
            projects.insert(p.id, p);
        }
        Self {
            projects,
            jobs: BTreeMap::new(),
            seq: 0,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.seq,
            projects: self.projects.values().cloned().collect(),
            jobs: self.jobs.values().cloned().collect(),
            // The daemon does not track sessions yet; a later task adds
            // the field to `State` and populates this from it.
            sessions: Vec::new(),
        }
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    #[must_use]
    pub fn upsert_project(&mut self, p: Project) -> Event {
        let seq = self.bump();
        self.projects.insert(p.id, p.clone());
        Event {
            seq,
            kind: EventKind::ProjectChanged { project: p },
        }
    }

    #[must_use]
    pub fn remove_project(&mut self, id: &ProjectId) -> Event {
        let seq = self.bump();
        self.projects.remove(id);
        Event {
            seq,
            kind: EventKind::ProjectRemoved { id: *id },
        }
    }

    #[must_use]
    pub fn upsert_job(&mut self, job: Job) -> Event {
        let seq = self.bump();
        self.jobs.insert(job.id, job.clone());
        Event {
            seq,
            kind: EventKind::JobChanged { job },
        }
    }
}

/// Recovers a poisoned lock instead of panicking, matching the discipline
/// in `jobs` and `projects`: one worker's panic must not wedge state.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Commits a state mutation and broadcasts its event while still holding
/// the lock, so the event stream's `seq` order always matches the order
/// mutations committed in -- even when several worker threads mutate at
/// once. `send_event` only enqueues, so holding the lock across it never
/// blocks on I/O.
pub fn emit(
    state: &Mutex<State>,
    out: &Outbound,
    mutate: impl FnOnce(&mut State) -> Event,
) {
    let mut guard = lock(state);
    let event = mutate(&mut guard);
    out.send_event(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_core::project::ProjectState;

    fn proj() -> Project {
        Project {
            id: ProjectId::new(),
            name: "x".into(),
            slug: "x".into(),
            source: r"C:\x".into(),
            workspace: "/home/willie/projects/x".into(),
            branch: "main".into(),
            state: ProjectState::Preparing,
            source_present: true,
            created_at: "t".into(),
        }
    }

    #[test]
    fn each_mutation_gets_the_next_seq_and_the_snapshot_agrees() {
        let mut s = State::default();
        let p = proj();
        let e1 = s.upsert_project(p.clone());
        let e2 = s.remove_project(&p.id);
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(s.snapshot().seq, 2);
        assert!(s.snapshot().projects.is_empty());
    }
}
