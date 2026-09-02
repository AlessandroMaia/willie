//! The daemon's in-memory truth: projects (mirroring the TOML files),
//! jobs (transient), and a monotonic sequence for the event stream.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

use willie_core::{
    id::{JobId, ProjectId, SessionId},
    project::Project,
    session::Session,
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
    pub sessions: BTreeMap<SessionId, Session>,
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
            sessions: BTreeMap::new(),
            seq: 0,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.seq,
            projects: self.projects.values().cloned().collect(),
            jobs: self.jobs.values().cloned().collect(),
            sessions: self.sessions.values().cloned().collect(),
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

    #[must_use]
    pub fn upsert_session(&mut self, session: Session) -> Event {
        let seq = self.bump();
        self.sessions.insert(session.id, session.clone());
        Event {
            seq,
            kind: EventKind::SessionChanged { session },
        }
    }

    /// Ids of the project's sessions that are still live.
    #[must_use]
    pub fn live_session_ids_for(&self, project: &ProjectId) -> Vec<SessionId> {
        self.sessions
            .values()
            .filter(|s| s.project_id == *project && s.state.is_live())
            .map(|s| s.id)
            .collect()
    }
}

/// Recovers a poisoned lock instead of panicking, matching the discipline
/// in `jobs` and `projects`: one worker's panic must not wedge state.
/// Re-exported at the crate root as `crate::lock` for the session code.
pub(crate) fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
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
    use willie_core::{project::ProjectState, sandbox::SandboxProfile};

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
            sandbox: SandboxProfile::default(),
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

    #[test]
    fn a_session_upsert_bumps_seq_and_shows_in_the_snapshot() {
        use willie_core::session::{Session, SessionState};
        let mut s = State::default();
        let sess = Session {
            id: SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: "/w".into(),
            state: SessionState::Running,
            created_at: "t".into(),
            started_at: None,
            finished_at: None,
            pid: Some(3),
            clients: 0,
            resumed_from: None,
        };
        let ev = s.upsert_session(sess.clone());
        assert_eq!(ev.seq, 1);
        assert_eq!(s.snapshot().sessions.len(), 1);
        assert_eq!(s.live_session_ids_for(&sess.project_id), vec![sess.id]);
    }
}
