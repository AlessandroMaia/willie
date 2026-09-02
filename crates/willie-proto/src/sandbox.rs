//! Request and result types for the `sandbox.*` namespace.

use serde::{Deserialize, Serialize};
use willie_core::{
    id::ProjectId,
    sandbox::{CapabilitySet, Explained},
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
}
