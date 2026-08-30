//! Request and result types for the `session.*` namespace.

use serde::{Deserialize, Serialize};
use willie_core::{
    id::{ProjectId, SessionId},
    session::Session,
};

pub mod method {
    pub const CREATE: &str = "session.create";
    pub const STOP: &str = "session.stop";
    pub const LIST: &str = "session.list";
}

/// `user.name`/`user.email` read from the Windows git configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    pub project_id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_identity: Option<GitIdentity>,
    #[serde(default)]
    pub resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateResult {
    pub session: Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdParams {
    pub id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionList {
    pub sessions: Vec<Session>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_params_accept_a_missing_identity() {
        let p: CreateParams = serde_json::from_value(serde_json::json!({
            "project_id": ProjectId::new()
        }))
        .unwrap();
        assert!(p.git_identity.is_none());
        let p: CreateParams = serde_json::from_value(serde_json::json!({
            "project_id": ProjectId::new(),
            "git_identity": { "name": "A", "email": "a@x" }
        }))
        .unwrap();
        assert_eq!(p.git_identity.unwrap().email, "a@x");
    }

    #[test]
    fn create_params_default_to_a_fresh_session() {
        let p: CreateParams = serde_json::from_value(serde_json::json!({
            "project_id": ProjectId::new()
        }))
        .unwrap();
        assert!(!p.resume);
        let p: CreateParams = serde_json::from_value(serde_json::json!({
            "project_id": ProjectId::new(), "resume": true
        }))
        .unwrap();
        assert!(p.resume);
    }
}
