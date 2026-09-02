//! Request and result types for the `sandbox.*` namespace.

use serde::{Deserialize, Serialize};
use willie_core::{
    id::ProjectId,
    sandbox::{Capability, CapabilitySet, Explained},
};

pub mod method {
    pub const EXPLAIN: &str = "sandbox.explain";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainParams {
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainResult {
    pub entries: Vec<Explained>,
    pub capabilities: CapabilitySet,
}

/// One row of the capability catalogue the app renders verbatim: the
/// dotted name and the consequence sentence `Capability` owns, whether
/// this version can apply it, and what layer 1 decided for it.
/// Crosses the IPC boundary (a Tauri command's return value, not a
/// daemon RPC), so it lives here rather than in `willie-core` with the
/// rest of the domain type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInfo {
    pub capability: Capability,
    pub display_name: String,
    pub consequence: String,
    pub implemented: bool,
    /// The harness's own default for this capability, so a row with no
    /// project override shows what layer 1 decided instead of a guess
    /// restated in TypeScript. Per row rather than a `CapabilitySet`
    /// beside the catalogue, because that would make the app map the
    /// set's field names back onto `Capability` — the restatement this
    /// field exists to remove.
    pub default_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_params_read_a_bare_project_id() {
        let p: ExplainParams = serde_json::from_value(serde_json::json!({
            "project_id": ProjectId::new()
        }))
        .unwrap();
        assert!(!p.project_id.to_string().is_empty());
    }

    /// The app never keeps a second copy of the ten user-facing
    /// sentences, nor of layer 1's answer for them: it renders this
    /// JSON verbatim, so the wire shape (snake_case capability name
    /// alongside the two owned strings and the two flags) is the
    /// contract that matters.
    #[test]
    fn capability_info_serialises_the_dotted_name_as_snake_case() {
        let defaults = CapabilitySet {
            agent_state: false,
            ..CapabilitySet::default()
        };
        let info = CapabilityInfo {
            capability: Capability::AgentState,
            display_name: Capability::AgentState.display_name().to_owned(),
            consequence: Capability::AgentState.consequence().to_owned(),
            implemented: Capability::AgentState.is_implemented(),
            default_enabled: defaults.enabled(Capability::AgentState),
        };

        let value = serde_json::to_value(&info).unwrap();

        assert_eq!(value["capability"], "agent_state");
        assert_eq!(value["display_name"], "agent.state");
        assert_eq!(value["implemented"], true);
        assert_eq!(value["default_enabled"], false);
    }
}
