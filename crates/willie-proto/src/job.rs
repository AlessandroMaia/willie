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
    pub project_id: ProjectId,
    pub state: JobState,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub log_tail: String,
}
