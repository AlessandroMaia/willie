//! Background-operation types.

use serde::{Deserialize, Serialize};
use willie_core::id::{JobId, ProjectId};

pub mod method {
    pub const LIST: &str = "job.list";
    pub const GET: &str = "job.get";
    pub const CANCEL: &str = "job.cancel";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Add,
    Remove,
    SyncToWindows,
    UpdateFromWindows,
    Relocate,
    InstallHarness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed {
        code: String,
        message: String,
        remediation: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    /// Absent for jobs that belong to no project (tool installs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    pub state: JobState,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub log_tail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_without_a_project_omits_the_field_and_an_old_shape_still_parses() {
        let job = Job {
            id: JobId::new(),
            kind: JobKind::InstallHarness,
            project_id: None,
            state: JobState::Running,
            started_at: "t".into(),
            finished_at: None,
            log_tail: String::new(),
        };
        let v = serde_json::to_value(&job).unwrap();
        assert!(v.get("project_id").is_none());
        assert_eq!(v["kind"], "install_harness");
        // The shape every job had before tool jobs existed.
        let old = serde_json::json!({
            "id": JobId::new(), "kind": "add", "project_id": ProjectId::new(),
            "state": { "state": "done" }, "started_at": "1", "log_tail": ""
        });
        let back: Job = serde_json::from_value(old).unwrap();
        assert!(back.project_id.is_some());
    }
}
