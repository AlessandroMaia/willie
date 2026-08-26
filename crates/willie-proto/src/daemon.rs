//! Messages of the `daemon.*` namespace.

use serde::{Deserialize, Serialize};

/// Method names of the `daemon.*` namespace.
pub mod method {
    pub const HELLO: &str = "daemon.hello";
    pub const HEALTH: &str = "daemon.health";
    pub const DOCTOR: &str = "daemon.doctor";
    pub const SHUTDOWN: &str = "daemon.shutdown";
    pub const ALL: &[&str] = &[HELLO, HEALTH, DOCTOR, SHUTDOWN];
}

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

impl Hello {
    /// Builds a hello for the current Willie version.
    #[must_use]
    pub fn for_client(client: impl Into<String>) -> Self {
        Self {
            client: client.into(),
            willie_version: willie_core::VERSION.to_owned(),
            protocol_version: crate::PROTOCOL_VERSION,
        }
    }
}

/// Reply to [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloReply {
    /// Willie version the daemon was built from.
    pub willie_version: String,
    /// Protocol revision the daemon speaks.
    pub protocol_version: u32,
    /// Version of the distro image the daemon is running in, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distro_image_version: Option<String>,
}

/// Result of `daemon.health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub pid: u32,
    pub uptime_secs: u64,
    pub willie_version: String,
}

/// Outcome of a single [`DoctorCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Fail,
    Skip,
}

/// One diagnostic check reported by `daemon.doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    #[serde(default)]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// Result of `daemon.doctor`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// True unless a required check failed; optional failures don't count.
    #[must_use]
    pub fn healthy(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|c| c.required && c.status == CheckStatus::Fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_healthy_unless_a_required_check_fails() {
        let ok = DoctorCheck {
            name: "a".into(),
            status: CheckStatus::Ok,
            detail: String::new(),
            remediation: None,
            required: true,
        };
        let optional_fail = DoctorCheck {
            name: "b".into(),
            status: CheckStatus::Fail,
            detail: String::new(),
            remediation: None,
            required: false,
        };
        let required_fail = DoctorCheck {
            name: "c".into(),
            status: CheckStatus::Fail,
            detail: String::new(),
            remediation: None,
            required: true,
        };
        assert!(
            DoctorReport {
                checks: vec![ok.clone(), optional_fail.clone()]
            }
            .healthy()
        );
        assert!(
            !DoctorReport {
                checks: vec![ok, optional_fail, required_fail]
            }
            .healthy()
        );
    }

    #[test]
    fn check_status_uses_lowercase_wire_names() {
        assert_eq!(
            serde_json::to_string(&CheckStatus::Skip).unwrap(),
            "\"skip\""
        );
    }

    #[test]
    fn method_names_are_namespaced() {
        assert!(method::ALL.iter().all(|m| m.starts_with("daemon.")));
    }

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
        assert_eq!(reply.protocol_version, crate::PROTOCOL_VERSION);
    }
}
