//! The daemon's in-memory truth: projects (mirroring the TOML files),
//! jobs (transient), and a monotonic sequence for the event stream.
//!
//! Scaffolding until the daemon wires this module in (Task 9).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::{collections::BTreeMap, path::Path};

use willie_core::{
    id::{JobId, ProjectId},
    project::Project,
};
use willie_proto::{
    job::Job,
    state::{Event, EventKind, Snapshot},
};

use crate::store;

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
