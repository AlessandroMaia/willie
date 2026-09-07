//! Snapshot and event types shared by every client.

use serde::{Deserialize, Serialize};
use willie_core::{id::ProjectId, project::Project, session::Session};

use crate::job::Job;
use crate::plugin::PluginStatus;

pub mod method {
    pub const SNAPSHOT: &str = "state.snapshot";
    pub const EVENT: &str = "state.event";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub projects: Vec<Project>,
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub plugins: Vec<PluginStatus>,
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
    SessionChanged { session: Session },
    PluginChanged { plugin: PluginStatus },
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
                project_id: Some(ProjectId::new()),
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

    fn sample_session() -> Session {
        use willie_core::session::SessionState;
        Session {
            id: willie_core::id::SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: "/home/willie/projects/x".into(),
            state: SessionState::Running,
            created_at: "1".into(),
            started_at: Some("1".into()),
            finished_at: None,
            pid: Some(4),
            clients: 1,
            resumed_from: None,
            sandbox: Default::default(),
        }
    }

    #[test]
    fn a_snapshot_without_sessions_or_plugins_still_parses_and_session_events_tag()
     {
        let v = serde_json::json!({ "seq": 1, "projects": [], "jobs": [] });
        let snap: Snapshot = serde_json::from_value(v).unwrap();
        assert!(snap.sessions.is_empty());
        assert!(snap.plugins.is_empty());
        let ev = Event {
            seq: 2,
            kind: EventKind::SessionChanged {
                session: sample_session(),
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "session_changed");
        assert_eq!(v["session"]["state"]["state"], "running");
    }

    #[test]
    fn plugin_changed_event_tags_its_kind() {
        use crate::plugin::{Enablement, Scope};

        let ev = Event {
            seq: 3,
            kind: EventKind::PluginChanged {
                plugin: PluginStatus {
                    id: "usage".into(),
                    name: "Usage".into(),
                    scope: Scope::Global,
                    enabled: Enablement::Global(true),
                    degraded: false,
                },
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "plugin_changed");
        assert_eq!(v["plugin"]["id"], "usage");
        let back: Event = serde_json::from_value(v).unwrap();
        assert_eq!(ev, back);
    }
}
