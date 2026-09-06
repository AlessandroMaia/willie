//! The session domain: what a session is, the launch spec the supervisor
//! executes without knowing what a harness is, and the event log both
//! sides agree on. Pure: the daemon applies events to build its index,
//! the supervisor only produces them. No I/O.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    id::{ProjectId, SessionId},
    sandbox::CapabilitySet,
};

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
    /// The session this one continues, if it was opened as a resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<SessionId>,
    /// What the sandbox applied and refused for this session; empty until
    /// its events are folded, and on a session from a pre-part-2 log.
    #[serde(default)]
    pub sandbox: SandboxState,
}

/// What the sandbox reported for one session. Empty until the
/// `sandbox_applied` event is folded; a session from a pre-part-2 log
/// leaves it default, and a log written before denials were recorded
/// leaves `denied` and `degraded` empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxState {
    /// The mechanisms measured to apply.
    #[serde(default)]
    pub applied: Vec<String>,
    /// The required-optional mechanisms this kernel does not offer.
    #[serde(default)]
    pub unavailable: Vec<String>,
    /// What the sandbox refused, one row per (class, name); repeats
    /// accumulate into the row's count.
    #[serde(default)]
    pub denied: Vec<Denied>,
    /// The mechanisms that fell back to their closed direction while the
    /// session ran, each named once.
    #[serde(default)]
    pub degraded: Vec<String>,
}

/// One thing the sandbox refused, merged over the session by
/// (`class`, `name`): `count` accumulates, `first_at` stays at the first
/// refusal and `last_at` advances to the latest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denied {
    /// What kind of thing was refused: `syscall` now, `terminal` later.
    pub class: String,
    /// The refused thing within its class, e.g. the syscall name.
    pub name: String,
    /// How many times over the session, every repeat counted.
    pub count: u64,
    /// Epoch seconds as a string, like the events.
    pub first_at: String,
    pub last_at: String,
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
    /// Persisted so a re-adopted session keeps its lineage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<SessionId>,
    /// The policy this session runs under, resolved once by the daemon
    /// before anything is spawned. Defaulted so a spec written before
    /// sandboxing is re-adopted rather than rejected.
    #[serde(default)]
    pub capabilities: CapabilitySet,
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
    /// What the supervisor applied around the harness, once per session,
    /// before `started`. A session that ran with less says so forever.
    SandboxApplied {
        mechanisms: Vec<String>,
        #[serde(default)]
        unavailable: Vec<String>,
    },
    /// The sandbox refused something `count` times since the last such
    /// event for the same (class, name); the session merges repeats.
    SandboxDenied {
        class: String,
        name: String,
        count: u64,
    },
    /// A mechanism fell back to its closed direction mid-session; the
    /// message says why, the session remembers only the mechanism.
    SandboxDegraded {
        mechanism: String,
        message: String,
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
        SessionEventKind::SandboxApplied {
            mechanisms,
            unavailable,
        } => {
            session.sandbox.applied = mechanisms.clone();
            session.sandbox.unavailable = unavailable.clone();
        }
        SessionEventKind::SandboxDenied { class, name, count } => {
            if let Some(d) = session
                .sandbox
                .denied
                .iter_mut()
                .find(|d| d.class == *class && d.name == *name)
            {
                d.count += count;
                d.last_at = event.at.clone();
            } else {
                session.sandbox.denied.push(Denied {
                    class: class.clone(),
                    name: name.clone(),
                    count: *count,
                    first_at: event.at.clone(),
                    last_at: event.at.clone(),
                });
            }
        }
        SessionEventKind::SandboxDegraded { mechanism, .. } => {
            if !session.sandbox.degraded.iter().any(|m| m == mechanism) {
                session.sandbox.degraded.push(mechanism.clone());
            }
        }
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
        resumed_from: spec.resumed_from,
        sandbox: SandboxState::default(),
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
        "harness_cannot_resume" => {
            "open a fresh session instead; this harness cannot continue \
             a conversation"
        }
        "session_already_live" => {
            "use the running session, or stop it first, then resume"
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
        "sandbox_backend_missing" => {
            "the image lacks the namespace helper: rebuild and reinstall \
             the distribution (`just distro-build`, `just distro-install`)"
        }
        "sandbox_apply_failed" => {
            "the message names the path; the helper writes its own \
             complaint to the session's terminal, so attach to see it, \
             then run `willie doctor`"
        }
        // Both also travel as failure events: the daemon refuses a
        // policy with its own remediation, but the supervisor's
        // resolution-time refusal takes its hint from here.
        "sandbox_profile_invalid" => {
            "the message names the path and where it leads; correct it \
             in the project's Sandbox settings"
        }
        "sandbox_capability_unsupported" => {
            "the message names the capability; remove it from the \
             project's Sandbox settings, this version cannot apply it"
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
            resumed_from: None,
            capabilities: CapabilitySet::default(),
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
    fn resumed_from_defaults_to_none_and_round_trips_when_set() {
        let s = from_log(&spec(), &[]);
        assert_eq!(s.resumed_from, None);
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("resumed_from").is_none());
        let back: Session = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);

        let mut resumed = s;
        resumed.resumed_from = Some(SessionId::new());
        let v = serde_json::to_value(&resumed).unwrap();
        assert_eq!(
            v["resumed_from"],
            serde_json::to_value(resumed.resumed_from.unwrap()).unwrap()
        );
        let back: Session = serde_json::from_value(v).unwrap();
        assert_eq!(back.resumed_from, resumed.resumed_from);
    }

    #[test]
    fn from_log_carries_resumed_from_from_the_spec() {
        let mut origin = spec();
        origin.resumed_from = Some(SessionId::new());
        let s = from_log(&origin, &[]);
        assert_eq!(s.resumed_from, origin.resumed_from);
    }

    /// The applied event does not touch the lifecycle fields (state, pid,
    /// clients); it only fills the sandbox record.
    #[test]
    fn a_sandbox_applied_event_fills_the_sandbox_record_only() {
        let spec = spec();
        let mut session = from_log(&spec, &[]);
        let lifecycle = session.state.clone();

        apply_event(
            &mut session,
            &SessionEvent {
                at: "2".into(),
                kind: SessionEventKind::SandboxApplied {
                    mechanisms: vec!["namespaces".into(), "mounts".into()],
                    unavailable: vec!["landlock".into()],
                },
            },
        );

        assert_eq!(session.state, lifecycle);
        assert_eq!(session.sandbox.applied, ["namespaces", "mounts"]);
        assert_eq!(session.sandbox.unavailable, ["landlock"]);
    }

    /// A session from a pre-part-2 log carries no sandbox field; it
    /// defaults to empty rather than failing to deserialise.
    #[test]
    fn a_session_without_a_sandbox_field_defaults_to_empty() {
        let mut v = serde_json::to_value(from_log(&spec(), &[])).unwrap();
        v.as_object_mut().unwrap().remove("sandbox");
        let back: Session = serde_json::from_value(v).unwrap();
        assert!(back.sandbox.applied.is_empty());
        assert!(back.sandbox.unavailable.is_empty());
    }

    #[test]
    fn a_sandbox_applied_event_serialises_with_its_mechanisms() {
        let text = serde_json::to_string(&SessionEventKind::SandboxApplied {
            mechanisms: vec!["namespaces".into()],
            unavailable: vec![],
        })
        .unwrap();

        assert_eq!(
            text,
            r#"{"kind":"sandbox_applied","mechanisms":["namespaces"],"unavailable":[]}"#
        );
    }

    /// The event gained `unavailable` in part 2. A part-1 `sandbox_applied`
    /// line was written without it, so the old shape gets a regression
    /// test (docs/TESTING.md): it must still parse, with `unavailable`
    /// defaulting to empty rather than failing to deserialise.
    #[test]
    fn a_part_one_sandbox_applied_line_without_unavailable_still_parses() {
        let back: SessionEventKind = serde_json::from_str(
            r#"{"kind":"sandbox_applied","mechanisms":["namespaces","mounts"]}"#,
        )
        .unwrap();

        assert_eq!(
            back,
            SessionEventKind::SandboxApplied {
                mechanisms: vec!["namespaces".into(), "mounts".into()],
                unavailable: vec![],
            }
        );
    }

    /// A denial event merges into the record by (class, name): the count
    /// accumulates and last_at advances, first_at stays. A second syscall
    /// is a separate row.
    #[test]
    fn sandbox_denied_events_merge_by_class_and_name() {
        let mut s = from_log(&spec(), &[]);
        apply_event(
            &mut s,
            &ev(
                "5",
                SessionEventKind::SandboxDenied {
                    class: "syscall".into(),
                    name: "unshare".into(),
                    count: 1,
                },
            ),
        );
        apply_event(
            &mut s,
            &ev(
                "9",
                SessionEventKind::SandboxDenied {
                    class: "syscall".into(),
                    name: "unshare".into(),
                    count: 3,
                },
            ),
        );
        apply_event(
            &mut s,
            &ev(
                "9",
                SessionEventKind::SandboxDenied {
                    class: "syscall".into(),
                    name: "ptrace".into(),
                    count: 1,
                },
            ),
        );
        assert_eq!(s.sandbox.denied.len(), 2);
        let unshare = s
            .sandbox
            .denied
            .iter()
            .find(|d| d.name == "unshare")
            .unwrap();
        assert_eq!(unshare.count, 4);
        assert_eq!(unshare.first_at, "5");
        assert_eq!(unshare.last_at, "9");
    }

    #[test]
    fn a_sandbox_degraded_event_is_recorded() {
        let mut s = from_log(&spec(), &[]);
        apply_event(
            &mut s,
            &ev(
                "2",
                SessionEventKind::SandboxDegraded {
                    mechanism: "seccomp".into(),
                    message: "notify thread died".into(),
                },
            ),
        );
        assert_eq!(s.sandbox.degraded, ["seccomp"]);
    }

    /// Neither event touches the lifecycle fields.
    #[test]
    fn denial_and_degraded_events_leave_the_lifecycle_alone() {
        let mut s =
            from_log(&spec(), &[ev("1", SessionEventKind::Started { pid: 7 })]);
        let before = s.state.clone();
        apply_event(
            &mut s,
            &ev(
                "2",
                SessionEventKind::SandboxDenied {
                    class: "syscall".into(),
                    name: "bpf".into(),
                    count: 1,
                },
            ),
        );
        apply_event(
            &mut s,
            &ev(
                "3",
                SessionEventKind::SandboxDegraded {
                    mechanism: "seccomp".into(),
                    message: "x".into(),
                },
            ),
        );
        assert_eq!(s.state, before);
        assert_eq!(s.pid, Some(7));
    }

    #[test]
    fn a_session_without_denied_or_degraded_defaults_them() {
        let mut v = serde_json::to_value(from_log(&spec(), &[])).unwrap();
        v["sandbox"].as_object_mut().unwrap().remove("denied");
        v["sandbox"].as_object_mut().unwrap().remove("degraded");
        let back: Session = serde_json::from_value(v).unwrap();
        assert!(
            back.sandbox.denied.is_empty() && back.sandbox.degraded.is_empty()
        );
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

    /// A code with no arm of its own falls through to the generic hint,
    /// which names no action for it: that is exactly what this test
    /// exists to catch, so it compares against the fall-through rather
    /// than only asking for a non-empty string. `supervisor_timeout` is
    /// the one code whose own answer is that same sentence, because for
    /// a supervisor that never answered, opening the session again is
    /// the action.
    #[test]
    fn every_documented_code_has_a_remediation_that_names_an_action() {
        let generic = remediation_for("a_code_no_arm_matches");
        let answered_by_the_generic = ["supervisor_timeout"];

        for code in [
            "project_not_ready",
            "harness_cannot_resume",
            "session_already_live",
            "harness_not_installed",
            "git_identity_missing",
            "supervisor_spawn_failed",
            "supervisor_timeout",
            "harness_exec_failed",
            "sandbox_backend_missing",
            "sandbox_apply_failed",
            "sandbox_profile_invalid",
            "sandbox_capability_unsupported",
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
            if !answered_by_the_generic.contains(&code) {
                assert_ne!(r, generic, "{code} has no remediation of its own");
            }
        }
    }
}
