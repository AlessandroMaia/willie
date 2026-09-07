//! The `tool.*` namespace: managed tools, starting with the harness
//! installer. Replies with the same `JobRef` as project jobs.

use serde::{Deserialize, Serialize};

pub use crate::project::JobRef;

pub mod method {
    pub const INSTALL: &str = "tool.install";
    pub const LIST: &str = "tool.list";
    pub const UPDATE: &str = "tool.update";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallParams {
    /// Harness id, e.g. `claude-code`.
    pub harness: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolList {
    pub tools: Vec<ToolStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateParams {
    /// Tool id, e.g. `claude-code`.
    pub tool: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_status_round_trips() {
        let s = ToolStatus {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            installed: true,
            version: Some("2.1.246".into()),
            recorded_version: Some("2.1.246".into()),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<ToolStatus>(&json).unwrap(), s);
    }

    #[test]
    fn an_uninstalled_tool_omits_version_fields() {
        let s = ToolStatus {
            id: "x".into(),
            name: "X".into(),
            installed: false,
            version: None,
            recorded_version: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("version").is_none());
        assert!(v.get("recorded_version").is_none());
    }
}
