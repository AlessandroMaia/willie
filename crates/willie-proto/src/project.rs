//! Request and result types for the `project.*` namespace.

use serde::{Deserialize, Serialize};
use willie_core::{
    id::{JobId, ProjectId},
    project::Project,
    sandbox::SandboxProfile,
};

pub mod method {
    pub const LIST: &str = "project.list";
    pub const ADD: &str = "project.add";
    pub const REMOVE: &str = "project.remove";
    pub const SYNC_TO_WINDOWS: &str = "project.sync_to_windows";
    pub const UPDATE_FROM_WINDOWS: &str = "project.update_from_windows";
    pub const RELOCATE: &str = "project.relocate";
    pub const RENAME: &str = "project.rename";
    pub const SET_SANDBOX: &str = "project.set_sandbox";
    pub const TREE: &str = "project.tree";
    pub const READ_FILE: &str = "project.read_file";
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

/// Layer 2 in full: `set_sandbox` replaces the project's profile with
/// `profile` rather than merging onto the stored one, so the caller
/// (the dialog, via the screen) always sends the whole edited profile,
/// not just the keys touched in this one save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetSandboxParams {
    pub project_id: ProjectId,
    pub profile: SandboxProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRef {
    pub job_id: JobId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeParams {
    pub id: ProjectId,
    /// Workspace-relative directory to list; absent lists the root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub kind: EntryKind,
    /// `M`/`A`/`D`/`R`/`?`, aggregated up to a directory from any
    /// changed path beneath it; absent when nothing changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeResult {
    pub entries: Vec<TreeEntry>,
    /// The workspace's current branch, when it resolves; carried here so
    /// the UI shows it without a second method call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileParams {
    pub id: ProjectId,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub content: String,
    pub truncated: bool,
}
