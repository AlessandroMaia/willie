//! Request and result types for the `project.*` namespace.

use serde::{Deserialize, Serialize};
use willie_core::{
    id::{JobId, ProjectId},
    project::Project,
};

pub mod method {
    pub const LIST: &str = "project.list";
    pub const ADD: &str = "project.add";
    pub const REMOVE: &str = "project.remove";
    pub const SYNC_TO_WINDOWS: &str = "project.sync_to_windows";
    pub const UPDATE_FROM_WINDOWS: &str = "project.update_from_windows";
    pub const RELOCATE: &str = "project.relocate";
    pub const RENAME: &str = "project.rename";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectList {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddParams {
    pub windows_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddResult {
    pub project_id: ProjectId,
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveParams {
    pub id: ProjectId,
    #[serde(default)]
    pub delete_workspace: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdParams {
    pub id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelocateParams {
    pub id: ProjectId,
    pub windows_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameParams {
    pub id: ProjectId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRef {
    pub job_id: JobId,
}
