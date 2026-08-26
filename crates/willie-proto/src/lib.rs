//! Wire types for the Willie control protocol.
//!
//! Messages are JSON-RPC 2.0 objects, one per line (ndjson). The same
//! messages travel over the engine's stdio pipe to the daemon, over the
//! daemon's Unix socket to local clients and, later, over TCP. This crate
//! only defines the shapes; it performs no I/O.
//!
//! Compatibility rule: every message type ignores unknown fields and gives
//! new fields a default, so an older client can talk to a newer daemon.

use serde::{Deserialize, Serialize};

/// Protocol revision. Bumped only for incompatible changes; additive
/// changes keep the number.
pub const PROTOCOL_VERSION: u32 = 1;

/// First request on any connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Human-readable client name, e.g. `willie-engine`.
    pub client: String,
    /// Willie version the client was built from.
    pub willie_version: String,
    /// Protocol revision the client speaks.
    pub protocol_version: u32,
}

/// Reply to [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloReply {
    /// Willie version the daemon was built from.
    pub willie_version: String,
    /// Protocol revision the daemon speaks.
    pub protocol_version: u32,
}

/// Error payload carried in JSON-RPC error responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    /// Stable `snake_case` code, e.g. `session_not_found`.
    pub code: String,
    /// One sentence describing what went wrong.
    pub message: String,
    /// What the caller can do about it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl Hello {
    /// Builds a hello for the current Willie version.
    #[must_use]
    pub fn for_client(client: impl Into<String>) -> Self {
        Self {
            client: client.into(),
            willie_version: willie_core::VERSION.to_owned(),
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips_through_json() {
        let hello = Hello::for_client("willie-engine");
        let json = serde_json::to_string(&hello).unwrap();
        let back: Hello = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{"willie_version":"9.9.9","protocol_version":1,
                       "added_later":true}"#;
        let reply: HelloReply = serde_json::from_str(json).unwrap();
        assert_eq!(reply.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn remediation_is_omitted_when_absent() {
        let err = RpcError {
            code: "session_not_found".into(),
            message: "no such session".into(),
            remediation: None,
        };
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("remediation"));
    }
}
