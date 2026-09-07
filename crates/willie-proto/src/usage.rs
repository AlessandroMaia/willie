//! The `usage.*` namespace: a snapshot of token usage across providers,
//! sessions and projects. Read-only from the client's point of view; the
//! daemon assembles it from plugin-owned counters.

use serde::{Deserialize, Serialize};
use willie_core::id::{ProjectId, SessionId};

pub mod method {
    pub const SNAPSHOT: &str = "usage.snapshot";
}

/// One provider's usage, keyed by its id. Left minimal on purpose: source 1
/// fills `windows` in later without a breaking change, since every field
/// defaults on an absent or since-expanded provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub id: String,
    #[serde(default)]
    pub windows: Vec<String>,
}

/// A session's token count and, once the harness reports one, how full its
/// context window is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub id: SessionId,
    pub tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pct: Option<u8>,
}

/// A project's token count, summed across its sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectUsage {
    pub id: ProjectId,
    pub tokens: u64,
}

/// The full picture returned by `usage.snapshot`: usage broken down by
/// provider, session and project, as of `fetched_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub providers: Vec<ProviderUsage>,
    pub sessions: Vec<SessionUsage>,
    pub projects: Vec<ProjectUsage>,
    pub fetched_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_with_no_providers_round_trips() {
        let snapshot = UsageSnapshot {
            providers: Vec::new(),
            sessions: Vec::new(),
            projects: Vec::new(),
            fetched_at: "2026-09-06T00:00:00Z".into(),
        };
        let v = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(v["providers"], serde_json::json!([]));
        let back: UsageSnapshot = serde_json::from_value(v).unwrap();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn session_usage_omits_a_missing_context_pct() {
        let usage = SessionUsage {
            id: SessionId::new(),
            tokens: 42,
            context_pct: None,
        };
        let v = serde_json::to_value(&usage).unwrap();
        assert!(v.get("context_pct").is_none());
    }

    #[test]
    fn session_usage_serialises_a_present_context_pct() {
        let usage = SessionUsage {
            id: SessionId::new(),
            tokens: 42,
            context_pct: Some(87),
        };
        let v = serde_json::to_value(&usage).unwrap();
        assert_eq!(v["context_pct"], 87);
        let back: SessionUsage = serde_json::from_value(v).unwrap();
        assert_eq!(usage, back);
    }
}
