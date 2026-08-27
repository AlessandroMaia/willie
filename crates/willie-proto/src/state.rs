//! Snapshot and event types shared by every client.

use serde::{Deserialize, Serialize};
use willie_core::{id::ProjectId, project::Project};

use crate::job::Job;

pub mod method {
    pub const SNAPSHOT: &str = "state.snapshot";
    pub const EVENT: &str = "state.event";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub projects: Vec<Project>,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    ProjectChanged { project: Project },
    ProjectRemoved { id: ProjectId },
    JobChanged { job: Job },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::{Job, JobKind, JobState};
    use willie_core::id::JobId;

    #[test]
    fn event_tags_its_kind_and_carries_the_seq() {
        let ev = Event {
            seq: 7,
            kind: EventKind::ProjectRemoved {
                id: ProjectId::new(),
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["seq"], 7);
        assert_eq!(v["kind"], "project_removed");
        let back: Event = serde_json::from_value(v).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn job_state_round_trips_each_variant() {
        for st in [
            JobState::Running,
            JobState::Done,
            JobState::Failed {
                code: "git_failed".into(),
                message: "boom".into(),
                remediation: "retry".into(),
            },
        ] {
            let job = Job {
                id: JobId::new(),
                kind: JobKind::Add,
                project_id: ProjectId::new(),
                state: st.clone(),
                started_at: "t".into(),
                finished_at: None,
                log_tail: String::new(),
            };
            let back: Job =
                serde_json::from_str(&serde_json::to_string(&job).unwrap())
                    .unwrap();
            assert_eq!(job, back);
        }
    }
}
