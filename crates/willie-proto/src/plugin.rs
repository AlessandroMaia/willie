//! The `plugin.*` namespace: the host's registry of compiled-in plugins,
//! their enablement, and status. A plugin's own methods (`profile.*`, …)
//! are not named here — the daemon routes them through `plugin.handle`
//! after splitting `<id>.<method>` (see `willie-plugin-api`).

use serde::{Deserialize, Serialize};
use willie_core::id::ProjectId;

pub mod method {
    pub const LIST: &str = "plugin.list";
    pub const ENABLE: &str = "plugin.enable";
    pub const DISABLE: &str = "plugin.disable";
}

/// Wire copy of `willie_plugin_api::Scope`. The proto owns this
/// serialisable shape; the daemon maps the plugin API's `Scope` onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Global,
    PerProject,
}

/// Whether a plugin is enabled: a bare flag for a `Global` plugin, or the
/// set of projects it is enabled in for a `PerProject` one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enablement {
    Global(bool),
    PerProject(Vec<ProjectId>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginStatus {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub enabled: Enablement,
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnableParams {
    pub id: String,
    /// `None` enables the plugin globally; `Some` enables it for that
    /// project only. Only meaningful for a `PerProject`-scoped plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_globally_enabled_plugin_status_round_trips() {
        let status = PluginStatus {
            id: "usage".into(),
            name: "Usage".into(),
            scope: Scope::Global,
            enabled: Enablement::Global(true),
            degraded: false,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["scope"], "global");
        assert_eq!(v["enabled"], serde_json::json!({ "global": true }));
        let back: PluginStatus = serde_json::from_value(v).unwrap();
        assert_eq!(status, back);
    }

    #[test]
    fn a_per_project_plugin_status_round_trips() {
        let project = ProjectId::new();
        let status = PluginStatus {
            id: "profiles".into(),
            name: "Profiles".into(),
            scope: Scope::PerProject,
            enabled: Enablement::PerProject(vec![project]),
            degraded: true,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["scope"], "per_project");
        assert_eq!(
            v["enabled"],
            serde_json::json!({ "per_project": [project] })
        );
        let back: PluginStatus = serde_json::from_value(v).unwrap();
        assert_eq!(status, back);
    }

    #[test]
    fn enable_params_default_project_id_to_none_for_a_global_enable() {
        let p: EnableParams =
            serde_json::from_value(serde_json::json!({ "id": "usage" }))
                .unwrap();
        assert_eq!(p.id, "usage");
        assert!(p.project_id.is_none());
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("project_id").is_none());
    }
}
