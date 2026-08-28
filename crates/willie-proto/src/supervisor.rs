//! Payloads of the frames a session supervisor exchanges with its clients
//! (`willie attach` and the daemon). The framing itself lives in
//! `willie-linux::wire`; these are only the JSON shapes.

use serde::{Deserialize, Serialize};

/// Who is connecting. A terminal gets output and replay; the daemon's
/// control connection gets the event stream instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Terminal,
    Control,
}

/// The mandatory first frame of every client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub role: Role,
    /// Terminal size; zeros from a control client.
    pub rows: u16,
    pub cols: u16,
}

/// Answer to a `status` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The harness pid.
    pub pid: u32,
    /// `running` or `stopping`.
    pub state: String,
    /// Attached terminals.
    pub clients: u32,
    pub started_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    /// The harness ended; `code`/`signal` say how.
    Exited,
    /// The daemon asked the session to stop.
    Stopped,
    /// The supervisor received `SIGTERM`/`SIGHUP`.
    Shutdown,
    /// This client's queue overflowed; reattach.
    TooSlow,
    /// This client broke the protocol.
    Protocol,
}

/// The last frame a client receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    pub reason: CloseReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_and_closed_use_snake_case_tags() {
        let h = serde_json::to_value(Hello {
            role: Role::Control,
            rows: 0,
            cols: 0,
        })
        .unwrap();
        assert_eq!(h["role"], "control");
        let c = serde_json::to_string(&Closed {
            reason: CloseReason::TooSlow,
            code: None,
            signal: None,
        })
        .unwrap();
        assert_eq!(c, r#"{"reason":"too_slow"}"#);
        let back: Closed =
            serde_json::from_str(r#"{"reason":"exited","code":7}"#).unwrap();
        assert_eq!(back.reason, CloseReason::Exited);
        assert_eq!(back.code, Some(7));
    }
}
