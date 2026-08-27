//! The session domain: what a session is, the launch spec the supervisor
//! executes without knowing what a harness is, and the event log both
//! sides agree on. Pure: the daemon applies events to build its index,
//! the supervisor only produces them. No I/O.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::{ProjectId, SessionId};

/// `Failed` has the shape of `ProjectState::Failed` so the app reuses
/// the same chip and inline remediation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    Creating,
    Running,
    Stopping,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Failed {
        code: String,
        message: String,
        remediation: String,
    },
}

impl SessionState {
    /// A supervisor is expected to be answering on the socket.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Running | Self::Stopping)
    }

    /// Nothing more will happen to this session.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Exited { .. } | Self::Failed { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    /// Harness id, e.g. `claude-code`.
    pub harness: String,
    /// The working directory handed to the harness.
    pub workspace: String,
    pub state: SessionState,
    /// Epoch seconds as a string, like jobs.
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// The harness process while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Attached terminals.
    #[serde(default)]
    pub clients: u32,
}

/// `spec.json`: immutable once written by the daemon. Everything the
/// supervisor needs to launch is resolved here — the binary path is
/// `argv[0]`, the environment is complete, the socket path is fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSpec {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub harness: String,
    pub workspace: String,
    pub socket: String,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub created_at: String,
    pub willie_version: String,
}

/// One line of `events.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Epoch seconds as a string.
    pub at: String,
    #[serde(flatten)]
    pub kind: SessionEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEventKind {
    Created,
    Started {
        pid: u32,
    },
    Attached {
        client: u64,
    },
    Detached {
        client: u64,
    },
    Resized {
        rows: u16,
        cols: u16,
    },
    StopRequested {
        by: String,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Failed {
        code: String,
        message: String,
    },
}

/// Folds one event into the session. Events after a terminal state are
/// ignored: a log can only end once.
pub fn apply_event(session: &mut Session, event: &SessionEvent) {
    if session.state.is_terminal() {
        return;
    }
    match &event.kind {
        SessionEventKind::Created | SessionEventKind::Resized { .. } => {}
        SessionEventKind::Started { pid } => {
            session.state = SessionState::Running;
            session.pid = Some(*pid);
            session.started_at = Some(event.at.clone());
        }
        SessionEventKind::Attached { .. } => session.clients += 1,
        SessionEventKind::Detached { .. } => {
            session.clients = session.clients.saturating_sub(1);
        }
        SessionEventKind::StopRequested { .. } => {
            if session.state.is_live() {
                session.state = SessionState::Stopping;
            }
        }
        SessionEventKind::Exited { code, signal } => {
            session.state = SessionState::Exited {
                code: *code,
                signal: *signal,
            };
            session.finished_at = Some(event.at.clone());
            session.pid = None;
            session.clients = 0;
        }
        SessionEventKind::Failed { code, message } => {
            session.state = SessionState::Failed {
                code: code.clone(),
                message: message.clone(),
                remediation: remediation_for(code).to_owned(),
            };
            session.finished_at = Some(event.at.clone());
            session.pid = None;
            session.clients = 0;
        }
    }
}

/// The session a spec plus its event log describe. An empty log is a
/// session still being created.
#[must_use]
pub fn from_log(spec: &SessionSpec, events: &[SessionEvent]) -> Session {
    let mut session = Session {
        id: spec.id,
        project_id: spec.project_id,
        harness: spec.harness.clone(),
        workspace: spec.workspace.clone(),
        state: SessionState::Creating,
        created_at: spec.created_at.clone(),
        started_at: None,
        finished_at: None,
        pid: None,
        clients: 0,
    };
    for event in events {
        apply_event(&mut session, event);
    }
    session
}

/// The one table of session and tool remediations. Every code the design
/// documents is here; each names an action a person can take.
#[must_use]
pub fn remediation_for(code: &str) -> &'static str {
    match code {
        "project_not_ready" => {
            "wait for the project to be ready, or fix its failure first"
        }
        "harness_not_installed" => "click Install on the Dashboard",
        "git_identity_missing" => {
            "set `git config --global user.name` and `user.email` on \
             Windows, then open the session again"
        }
        "supervisor_spawn_failed" => {
            "run `willie doctor`; reinstall the distribution if the \
             supervisor binary is missing"
        }
        "supervisor_timeout" => {
            "open the session again; run `willie doctor` if it repeats"
        }
        "harness_exec_failed" => {
            "reinstall Claude Code, or remove the project and add it again"
        }
        "session_not_found" => "refresh the Sessions screen",
        "session_not_running" => "nothing to stop; open a new session",
        "sessions_running" => "stop the project's sessions first",
        "supervisor_lost" => "open a new session",
        "harness_already_installed" => "nothing to install",
        "tool_busy" => "wait for the running install to finish",
        "install_failed" => {
            "read the installer output, check the network, then try again"
        }
        _ => "open the session again; run `willie doctor` if it repeats",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ProjectId;

    fn spec() -> SessionSpec {
        SessionSpec {
            id: SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: "/home/willie/projects/x".into(),
            socket: "/run/willie/sessions/s.sock".into(),
            argv: vec!["/home/willie/.local/bin/claude".into()],
            env: [("HOME".to_owned(), "/home/willie".to_owned())]
                .into_iter()
                .collect(),
            created_at: "1".into(),
            willie_version: "0.1.0".into(),
        }
    }

    fn ev(at: &str, kind: SessionEventKind) -> SessionEvent {
        SessionEvent {
            at: at.into(),
            kind,
        }
    }

    #[test]
    fn a_fresh_session_from_a_spec_is_creating() {
        let s = from_log(&spec(), &[]);
        assert_eq!(s.state, SessionState::Creating);
        assert_eq!(s.clients, 0);
        assert!(s.pid.is_none());
    }

    #[test]
    fn started_makes_it_running_with_a_pid_and_start_time() {
        let s = from_log(
            &spec(),
            &[ev("5", SessionEventKind::Started { pid: 42 })],
        );
        assert_eq!(s.state, SessionState::Running);
        assert_eq!(s.pid, Some(42));
        assert_eq!(s.started_at.as_deref(), Some("5"));
    }

    #[test]
    fn attach_and_detach_count_clients_and_never_go_negative() {
        let mut s = from_log(&spec(), &[]);
        apply_event(&mut s, &ev("1", SessionEventKind::Detached { client: 9 }));
        assert_eq!(s.clients, 0);
        apply_event(&mut s, &ev("1", SessionEventKind::Attached { client: 1 }));
        apply_event(&mut s, &ev("1", SessionEventKind::Attached { client: 2 }));
        apply_event(&mut s, &ev("1", SessionEventKind::Detached { client: 1 }));
        assert_eq!(s.clients, 1);
    }

    #[test]
    fn a_stop_request_moves_a_live_session_to_stopping_only() {
        let mut s = from_log(&spec(), &[]);
        apply_event(
            &mut s,
            &ev(
                "1",
                SessionEventKind::StopRequested {
                    by: "daemon".into(),
                },
            ),
        );
        assert_eq!(s.state, SessionState::Creating);
        apply_event(&mut s, &ev("2", SessionEventKind::Started { pid: 1 }));
        apply_event(
            &mut s,
            &ev(
                "3",
                SessionEventKind::StopRequested {
                    by: "daemon".into(),
                },
            ),
        );
        assert_eq!(s.state, SessionState::Stopping);
    }

    #[test]
    fn exited_is_terminal_and_clears_pid_and_clients() {
        let mut s = from_log(
            &spec(),
            &[
                ev("1", SessionEventKind::Started { pid: 7 }),
                ev("1", SessionEventKind::Attached { client: 1 }),
            ],
        );
        apply_event(
            &mut s,
            &ev(
                "9",
                SessionEventKind::Exited {
                    code: Some(7),
                    signal: None,
                },
            ),
        );
        assert_eq!(
            s.state,
            SessionState::Exited {
                code: Some(7),
                signal: None
            }
        );
        assert!(s.state.is_terminal());
        assert!(!s.state.is_live());
        assert_eq!(s.finished_at.as_deref(), Some("9"));
        assert_eq!(s.pid, None);
        assert_eq!(s.clients, 0);
    }

    #[test]
    fn a_failed_event_carries_its_code_and_the_known_remediation() {
        let mut s = from_log(&spec(), &[]);
        apply_event(
            &mut s,
            &ev(
                "2",
                SessionEventKind::Failed {
                    code: "harness_exec_failed".into(),
                    message: "No such file".into(),
                },
            ),
        );
        match &s.state {
            SessionState::Failed {
                code,
                message,
                remediation,
            } => {
                assert_eq!(code, "harness_exec_failed");
                assert_eq!(message, "No such file");
                assert_eq!(remediation, remediation_for("harness_exec_failed"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn events_after_a_terminal_state_are_ignored() {
        let mut s = from_log(
            &spec(),
            &[ev(
                "1",
                SessionEventKind::Exited {
                    code: Some(0),
                    signal: None,
                },
            )],
        );
        apply_event(&mut s, &ev("2", SessionEventKind::Started { pid: 3 }));
        assert!(s.state.is_terminal());
        assert_eq!(s.pid, None);
    }

    #[test]
    fn event_lines_use_a_flat_kind_tag() {
        let line = serde_json::to_string(&ev(
            "12",
            SessionEventKind::Resized {
                rows: 50,
                cols: 160,
            },
        ))
        .unwrap();
        assert_eq!(
            line,
            r#"{"at":"12","kind":"resized","rows":50,"cols":160}"#
        );
        let back: SessionEvent = serde_json::from_str(
            r#"{"at":"3","kind":"exited","code":null,"signal":9}"#,
        )
        .unwrap();
        assert_eq!(
            back.kind,
            SessionEventKind::Exited {
                code: None,
                signal: Some(9)
            }
        );
    }

    #[test]
    fn session_state_serialises_like_project_state() {
        let v = serde_json::to_value(SessionState::Failed {
            code: "x".into(),
            message: "m".into(),
            remediation: "r".into(),
        })
        .unwrap();
        assert_eq!(v["state"], "failed");
        assert_eq!(v["code"], "x");
        let v = serde_json::to_value(SessionState::Exited {
            code: Some(1),
            signal: None,
        })
        .unwrap();
        assert_eq!(v["state"], "exited");
        assert_eq!(v["code"], 1);
    }

    #[test]
    fn every_documented_code_has_a_remediation_that_names_an_action() {
        for code in [
            "project_not_ready",
            "harness_not_installed",
            "git_identity_missing",
            "supervisor_spawn_failed",
            "supervisor_timeout",
            "harness_exec_failed",
            "session_not_found",
            "session_not_running",
            "sessions_running",
            "supervisor_lost",
            "harness_already_installed",
            "tool_busy",
            "install_failed",
        ] {
            let r = remediation_for(code);
            assert!(!r.is_empty() && !r.contains("log"), "{code}: {r}");
        }
    }
}
