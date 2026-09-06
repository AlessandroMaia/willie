//! The request the supervisor hands its in-namespace stage, and the
//! report that stage hands back. Pure data: one JSON line each, framed by
//! the newline the way the event log is. Phases 2 and 3 add fields to
//! both; a message written before them deserialises with those absent.

use serde::{Deserialize, Serialize};

/// What the in-namespace stage must do before it execs the harness.
/// Phases 2 and 3 add fields (the seccomp program, the Landlock rules);
/// a message written before them deserialises with those absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The harness command the stage execs.
    pub argv: Vec<String>,
    pub rlimits: Rlimits,
}

/// The three resource limits, each applied as `min(value, current hard
/// limit)` so a stricter host is never loosened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rlimits {
    pub nproc: u64,
    pub nofile: u64,
    pub core: u64,
}

impl Rlimits {
    /// 4096 processes, 65536 descriptors, no core dump. Counted per user
    /// namespace, so only a stage inside the session's own namespace can
    /// set them without bounding the whole distribution's `willie` user.
    pub const DEFAULT: Rlimits = Rlimits {
        nproc: 4096,
        nofile: 65536,
        core: 0,
    };
}

/// The stage's one-line answer, before it execs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Report {
    /// The mechanisms that took effect, and the required-optional ones
    /// (phase 3's Landlock) this kernel does not offer.
    Applied {
        mechanisms: Vec<String>,
        #[serde(default)]
        unavailable: Vec<String>,
    },
    /// The stage refused before applying anything it could not undo.
    Refused { code: String, message: String },
}

/// A mechanism this build always requires. Absent from a report, the
/// session refuses. Phase 2 adds `"seccomp"`.
pub const REQUIRED: &[&str] = &["namespaces", "mounts", "rlimits"];

/// The first required mechanism the report does not name, or `None` when
/// every one is present.
#[must_use]
pub fn required_missing(applied: &[String]) -> Option<&'static str> {
    REQUIRED
        .iter()
        .copied()
        .find(|req| !applied.iter().any(|m| m == req))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_limits_are_the_documented_three() {
        assert_eq!(Rlimits::DEFAULT.nproc, 4096);
        assert_eq!(Rlimits::DEFAULT.nofile, 65536);
        assert_eq!(Rlimits::DEFAULT.core, 0);
    }

    #[test]
    fn a_request_is_one_json_line_that_round_trips() {
        let req = Request {
            argv: vec!["/bin/claude".into(), "--continue".into()],
            rlimits: Rlimits::DEFAULT,
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn an_applied_report_names_its_mechanisms_and_the_unavailable_ones() {
        let rep = Report::Applied {
            mechanisms: vec![
                "namespaces".into(),
                "mounts".into(),
                "rlimits".into(),
            ],
            unavailable: vec![],
        };
        let line = serde_json::to_string(&rep).unwrap();
        assert!(line.contains("\"result\":\"applied\""), "{line}");
        assert_eq!(serde_json::from_str::<Report>(&line).unwrap(), rep);
    }

    #[test]
    fn a_refused_report_carries_a_code_and_a_message() {
        let rep = Report::Refused {
            code: "sandbox_apply_failed".into(),
            message: "not inside the sandbox namespace".into(),
        };
        let line = serde_json::to_string(&rep).unwrap();
        assert!(line.contains("\"result\":\"refused\""), "{line}");
        assert_eq!(serde_json::from_str::<Report>(&line).unwrap(), rep);
    }

    /// A report written before phases 2 and 3 add fields still parses.
    #[test]
    fn an_applied_report_without_unavailable_defaults_it() {
        let rep: Report = serde_json::from_str(
            r#"{"result":"applied","mechanisms":["namespaces"]}"#,
        )
        .unwrap();
        assert_eq!(
            rep,
            Report::Applied {
                mechanisms: vec!["namespaces".into()],
                unavailable: vec![],
            }
        );
    }

    #[test]
    fn required_missing_names_the_first_gap_or_none() {
        let full = [
            "namespaces".to_owned(),
            "mounts".to_owned(),
            "rlimits".to_owned(),
        ];
        assert_eq!(required_missing(&full), None);
        let short = ["namespaces".to_owned(), "rlimits".to_owned()];
        assert_eq!(required_missing(&short), Some("mounts"));
        assert_eq!(required_missing(&[]), Some("namespaces"));
    }
}
