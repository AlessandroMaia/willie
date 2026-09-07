//! Session lifecycle: create (fail-closed), stop, list, and re-adopt live
//! supervisors when the daemon restarts. The daemon writes the spec and
//! spawns the supervisor; the supervisor owns the PTY and outlives it.

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use willie_core::{
    id::SessionId,
    project::ProjectState,
    sandbox::{CapabilitySet, SandboxProfile},
    session::{
        Session, SessionEvent, SessionEventKind, SessionKind, SessionSpec,
        apply_event, from_log,
    },
};
use willie_harness::{Harness, LaunchMode, Resume};
use willie_linux::paths::{SUPERVISOR_BIN, session_socket, sessions_run_dir};
use willie_plugin_api::CoreEvent;

use crate::{
    harness, identity, jobs::Runner, plugins::PluginHost, projects::OpError,
    session_store, state, state::State,
};

/// Shared inputs a session operation needs.
pub struct SessionOps {
    state: Arc<Mutex<State>>,
    out: crate::outbound::Outbound,
    state_dir: PathBuf,
    run_dir: PathBuf,
    home: PathBuf,
    clock: fn() -> String,
    // The same runner the project ops submit to: `create` reads its
    // per-project busy set to refuse a session while a job is in flight.
    runner: Arc<Runner>,
    // The daemon's plugin host, shared with the server. `None` in the unit
    // tests that build session ops without one; the real daemon wires it so
    // a session's start and exit reach the plugins as `CoreEvent`s.
    plugin_host: Option<Arc<Mutex<PluginHost>>>,
}

impl std::fmt::Debug for SessionOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionOps").finish_non_exhaustive()
    }
}

impl SessionOps {
    #[must_use]
    pub fn new(
        state: Arc<Mutex<State>>,
        out: crate::outbound::Outbound,
        state_dir: PathBuf,
        run_dir: PathBuf,
        home: PathBuf,
        clock: fn() -> String,
        runner: Arc<Runner>,
    ) -> Self {
        Self {
            state,
            out,
            state_dir,
            run_dir,
            home,
            clock,
            runner,
            plugin_host: None,
        }
    }

    /// Shares the daemon's plugin host with the session path, so a session's
    /// start and exit are fanned to the plugins as `CoreEvent`s. The daemon
    /// calls this; the unit tests leave it unset.
    #[must_use]
    pub fn with_plugin_host(mut self, host: Arc<Mutex<PluginHost>>) -> Self {
        self.plugin_host = Some(host);
        self
    }

    /// Fans one `CoreEvent` to the plugin host, if the daemon wired one. The
    /// host catches a plugin panic at its own boundary, so this never
    /// unwinds; the lock is recovered rather than unwrapped.
    fn notify_plugins(&self, ev: CoreEvent) {
        notify_plugin_host(&self.plugin_host, ev);
    }

    /// Resolve fail-closed, write the spec, spawn the supervisor and wait
    /// for its readiness. The returned session is `Running` once the
    /// supervisor reports its pid; a control connection then folds the
    /// supervisor's later events into the session.
    pub fn create(
        &self,
        params: willie_proto::session::CreateParams,
    ) -> Result<Session, OpError> {
        let (project, live, resumed_from) = {
            let s = crate::lock(&self.state);
            let project =
                s.projects.get(&params.project_id).cloned().ok_or_else(
                    || crate::projects::not_found_err(params.project_id),
                )?;
            // Only meaningful when `params.resume`; computed alongside the
            // project lookup so both read the same lock acquisition.
            let live = s.sessions.values().any(|se| {
                se.project_id == params.project_id && se.state.is_live()
            });
            let resumed_from = s
                .sessions
                .values()
                .filter(|se| {
                    se.project_id == params.project_id && se.state.is_terminal()
                })
                .max_by(|a, b| a.created_at.cmp(&b.created_at))
                .map(|se| se.id);
            (project, live, resumed_from)
        };
        if let Some(problem) = &project.sandbox_problem {
            return Err(OpError::from_problem(problem));
        }
        if !matches!(project.state, ProjectState::Ready) {
            return Err(OpError::coded(
                "project_not_ready",
                "the project is not ready",
            ));
        }
        // A project with a job in flight may be a `remove` deleting its
        // workspace: the project stays `Ready` while `remove_dir_all` runs
        // in the background, so starting a supervisor now would give the
        // harness a cwd that is being torn out from under it. Refuse until
        // the job finishes, reusing the project-side `project_busy` error.
        if self.runner.is_busy(&params.project_id) {
            return Err(crate::projects::busy_err());
        }
        // Fail closed means fail before anything happens: resolution
        // needs only the profile already in hand, so it runs before
        // `--version` is spawned and before `identity::ensure` may write
        // `~/.gitconfig`. A refusal after a filesystem side effect is
        // not the daemon refusing early, it is the daemon refusing late.
        let capabilities = session_capabilities(
            harness::claude().default_capabilities(),
            &project.sandbox,
            &self.home,
        )?;
        let (mode, resumed_from) = resume_decision(
            params.resume,
            harness::claude().capabilities().resume,
            live,
            resumed_from,
        )?;
        let installed =
            harness::detect_claude(&self.home).ok_or_else(|| {
                OpError::coded(
                    "harness_not_installed",
                    "Claude Code is not installed",
                )
            })?;
        let source_linux = crate::projects::source_to_linux(&project.source);
        identity::ensure(
            &self.home,
            params.git_identity.as_ref(),
            source_linux.as_deref().map(std::path::Path::new),
        )
        .map_err(|e| OpError::coded(e.code(), e.message()))?;

        let id = SessionId::new();
        let socket = session_socket(&self.run_dir, &id.to_string());
        let launch = harness::claude().launch(
            &installed.path,
            std::path::Path::new(&project.workspace),
            &self.home,
            mode,
        );
        let spec = SessionSpec {
            id,
            project_id: project.id,
            harness: harness::claude().id().to_owned(),
            workspace: project.workspace.clone(),
            socket: socket.to_string_lossy().into_owned(),
            argv: launch.argv,
            env: launch.env,
            created_at: (self.clock)(),
            willie_version: willie_core::VERSION.to_owned(),
            resumed_from,
            kind: SessionKind::Agent,
            capabilities,
        };
        let dir =
            session_store::write_spec(&self.state_dir, &spec).map_err(|e| {
                OpError::coded("supervisor_spawn_failed", &e.to_string())
            })?;
        let _ = std::fs::create_dir_all(sessions_run_dir(&self.run_dir));

        let mut session = from_log(&spec, &[]);
        state::emit(&self.state, &self.out, |s| {
            s.upsert_session(session.clone())
        });

        let ready = match self.spawn_supervisor(&dir) {
            Ok(ready) => ready,
            Err(e) => {
                // The spawn never reported readiness (a timeout, or the
                // launcher could not even start): fold the failure onto the
                // session we already emitted so the index shows it Failed,
                // not stuck Creating forever.
                apply_event(
                    &mut session,
                    &SessionEvent {
                        at: (self.clock)(),
                        kind: SessionEventKind::Failed {
                            code: e.code.clone(),
                            message: e.message.clone(),
                        },
                    },
                );
                state::emit(&self.state, &self.out, |s| {
                    s.upsert_session(session.clone())
                });
                return Err(e);
            }
        };
        match ready {
            Ready::Ok(pid) => {
                apply_event(
                    &mut session,
                    &SessionEvent {
                        at: (self.clock)(),
                        kind: SessionEventKind::Started { pid },
                    },
                );
                state::emit(&self.state, &self.out, |s| {
                    s.upsert_session(session.clone())
                });
                self.watch(id, &socket);
                self.notify_plugins(CoreEvent::SessionStarted {
                    session_id: id,
                    project_id: project.id,
                });
                Ok(session)
            }
            Ready::Fail { code, text } => {
                apply_event(
                    &mut session,
                    &SessionEvent {
                        at: (self.clock)(),
                        kind: SessionEventKind::Failed {
                            code: code.clone(),
                            message: text.clone(),
                        },
                    },
                );
                state::emit(&self.state, &self.out, |s| {
                    s.upsert_session(session.clone())
                });
                Err(OpError::coded_owned(code, text))
            }
        }
    }

    /// Spawn `willie-sess run --spec` and read its one readiness line from
    /// the launcher's stdout within a fixed budget. `WILLIE_SESS_BIN`
    /// overrides the supervisor path so the tests use their freshly-built
    /// binary instead of the installed one.
    fn spawn_supervisor(
        &self,
        dir: &std::path::Path,
    ) -> Result<Ready, OpError> {
        let bin = std::env::var("WILLIE_SESS_BIN")
            .unwrap_or_else(|_| SUPERVISOR_BIN.to_owned());
        let mut child = Command::new(bin)
            .args(["run", "--spec", &dir.join("spec.json").to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                OpError::coded("supervisor_spawn_failed", &e.to_string())
            })?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(OpError::coded(
                "supervisor_spawn_failed",
                "no supervisor stdout",
            ));
        };
        // The launcher process exits as soon as the grandchild answers, so
        // read the single readiness line on a helper thread and give up
        // after a bounded wait rather than block the request forever.
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let line = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(line) => line,
            Err(_) => {
                // Reap the launcher so it never lingers as a zombie under
                // the daemon; the helper thread unblocks once the child's
                // stdout pipe closes.
                let _ = child.kill();
                let _ = child.wait();
                return Err(OpError::coded(
                    "supervisor_timeout",
                    "no readiness reply",
                ));
            }
        };
        let _ = child.wait();
        Ok(parse_ready(line.trim()))
    }

    /// Adopt a running supervisor: connect a control client, fold its
    /// events into the session, and finalise the session when the control
    /// reader ends. A no-op off Linux, where there is no supervisor.
    fn watch(&self, id: SessionId, socket: &std::path::Path) {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (id, socket);
        }
        #[cfg(target_os = "linux")]
        {
            let Ok(mut control) = crate::control::connect(socket) else {
                return;
            };
            let state = Arc::clone(&self.state);
            let out = self.out.clone();
            let handle = control.watch(move |ev| {
                state::emit(&state, &out, |s| {
                    let mut session = s
                        .sessions
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| placeholder(id));
                    apply_event(&mut session, &ev);
                    s.upsert_session(session)
                });
            });
            // Finalisation keys off the reader thread ENDING, not off the
            // socket file: a supervisor that is SIGKILLed or crashes leaves
            // its named socket behind, so file existence never signals its
            // death. The reader ends on a `closed` frame (clean, a terminal
            // event already folded) or on EOF/reset (abrupt). On an abrupt
            // end the session is still non-terminal; probe the socket once
            // and, if nothing answers, fail it closed with `supervisor_lost`
            // so it never lingers `Running` and never blocks `project.remove`.
            let state = Arc::clone(&self.state);
            let out = self.out.clone();
            let state_dir = self.state_dir.clone();
            let socket = socket.to_path_buf();
            let plugin_host = self.plugin_host.clone();
            let _ = thread::Builder::new()
                .name("session-finalise".to_owned())
                .spawn(move || {
                    if let Some(handle) = handle {
                        let _ = handle.join();
                    }
                    let terminal = crate::lock(&state)
                        .sessions
                        .get(&id)
                        .is_some_and(|s| s.state.is_terminal());
                    // The session exited if a terminal event was folded, or
                    // if the follow-up probe finds no live supervisor and we
                    // finalise it lost. A socket that still answers means it
                    // is alive despite the reader ending: no exit event then.
                    let exited = if terminal {
                        true
                    } else {
                        let answers = crate::control::connect(&socket)
                            .and_then(|mut c| c.status())
                            .is_ok();
                        if !answers {
                            finalise_lost(&state, &out, &state_dir, id);
                        }
                        !answers
                    };
                    if exited {
                        notify_plugin_host(
                            &plugin_host,
                            CoreEvent::SessionExited { session_id: id },
                        );
                    }
                });
        }
    }

    /// Ask the session's supervisor to stop its harness.
    #[cfg(target_os = "linux")]
    pub fn stop(&self, id: SessionId) -> Result<(), OpError> {
        let socket = session_socket(&self.run_dir, &id.to_string());
        let mut control = crate::control::connect(&socket).map_err(|_| {
            OpError::coded("session_not_running", "the session is not running")
        })?;
        control.stop().map_err(|_| {
            OpError::coded("session_not_running", "the session is gone")
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn stop(&self, id: SessionId) -> Result<(), OpError> {
        let _ = id;
        Err(OpError::coded(
            "session_not_running",
            "sessions run only inside the distribution",
        ))
    }

    #[must_use]
    pub fn list(&self) -> Vec<Session> {
        crate::lock(&self.state)
            .sessions
            .values()
            .cloned()
            .collect()
    }

    /// Applies a user-chosen label: trims it, clears it on a blank or
    /// absent value, and refuses one over 120 characters before anything
    /// is touched. Persists a `Renamed` event to the session's own
    /// append-only log *before* folding it into memory and emitting the
    /// change, so a crash between the two can never show a label that is
    /// lost the next time the daemon re-adopts the session from its log.
    pub fn rename(
        &self,
        params: willie_proto::session::RenameParams,
    ) -> Result<Session, OpError> {
        let willie_proto::session::RenameParams { id, label } = params;
        let label =
            label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty());
        if let Some(l) = &label
            && l.chars().count() > 120
        {
            return Err(OpError {
                code: "invalid_params".to_owned(),
                message: "the label is longer than 120 characters".to_owned(),
                remediation: "shorten the name".to_owned(),
            });
        }
        let mut session = crate::lock(&self.state)
            .sessions
            .get(&id)
            .cloned()
            .ok_or_else(|| {
            OpError::coded("session_not_found", "no such session")
        })?;
        let event = SessionEvent {
            at: (self.clock)(),
            kind: SessionEventKind::Renamed { label },
        };
        session_store::append_event(&self.state_dir, &id.to_string(), &event)
            .map_err(|e| OpError {
            code: "state_write_failed".to_owned(),
            message: e.to_string(),
            remediation: "check the daemon's state directory \
                              permissions and try again"
                .to_owned(),
        })?;
        apply_event(&mut session, &event);
        state::emit(&self.state, &self.out, |s| {
            s.upsert_session(session.clone())
        });
        Ok(session)
    }

    /// At start, load every session directory, adopt the ones whose socket
    /// answers, finalise the rest from their logs.
    pub fn scan(&self) {
        #[cfg(target_os = "linux")]
        for (spec, events) in session_store::load_all(&self.state_dir) {
            let mut session = from_log(&spec, &events);
            if session.state.is_terminal() {
                crate::lock(&self.state)
                    .sessions
                    .insert(session.id, session);
                continue;
            }
            let socket = PathBuf::from(&spec.socket);
            match crate::control::connect(&socket).and_then(|mut c| c.status())
            {
                Ok(status) => {
                    session.state = willie_core::session::SessionState::Running;
                    session.pid = Some(status.pid);
                    session.clients = status.clients;
                    state::emit(&self.state, &self.out, |s| {
                        s.upsert_session(session.clone())
                    });
                    self.watch(session.id, &socket);
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&socket);
                    finalise_lost(
                        &self.state,
                        &self.out,
                        &self.state_dir,
                        session.id,
                    );
                }
            }
        }
    }
}

/// Fans one `CoreEvent` to the plugin host if the daemon wired one. Shared
/// by the request-thread `create` path and the background finalise thread,
/// which holds only a clone of the optional host. The host catches a plugin
/// panic at its own boundary, so this never unwinds; the poisoned lock is
/// recovered rather than unwrapped, keeping `willied`'s no-panic discipline.
fn notify_plugin_host(host: &Option<Arc<Mutex<PluginHost>>>, ev: CoreEvent) {
    if let Some(host) = host {
        host.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .on_event(ev);
    }
}

/// The resume guard and mode decision, pulled out of `create` as a pure
/// function so it is unit-testable without a harness/state fixture: a
/// fresh request always launches `Fresh` with no lineage; a resume request
/// is refused fail-closed when the harness cannot continue a conversation
/// or when the project already has a live session, and otherwise launches
/// `Continue` linked to the project's most recent terminal session.
fn resume_decision(
    resume: bool,
    harness_resume: Resume,
    live: bool,
    latest_terminal: Option<SessionId>,
) -> Result<(LaunchMode, Option<SessionId>), OpError> {
    if !resume {
        return Ok((LaunchMode::Fresh, None));
    }
    if harness_resume == Resume::None {
        return Err(OpError::coded(
            "harness_cannot_resume",
            "this harness cannot resume a conversation",
        ));
    }
    if live {
        return Err(OpError::coded(
            "session_already_live",
            "a session for this project is already running; \
             use it or stop it first",
        ));
    }
    Ok((LaunchMode::Continue, latest_terminal))
}

/// Layer 1 from the harness, layer 2 from the project record. A policy
/// that cannot be applied must not become a session that pretends it
/// was, so both refusals happen here, before anything is spawned.
fn session_capabilities(
    defaults: CapabilitySet,
    profile: &SandboxProfile,
    home: &std::path::Path,
) -> Result<CapabilitySet, OpError> {
    Ok(willie_core::sandbox::resolve(
        defaults,
        profile,
        &home.to_string_lossy(),
    )?)
}

#[derive(Debug)]
enum Ready {
    Ok(u32),
    Fail { code: String, text: String },
}

/// Parse the supervisor launcher's one readiness line: `ok <pid>` or
/// `fail <code>: <text>`. Anything else is treated as a spawn failure.
fn parse_ready(line: &str) -> Ready {
    if let Some(pid) = line.strip_prefix("ok ") {
        if let Ok(pid) = pid.trim().parse() {
            return Ready::Ok(pid);
        }
    } else if let Some(rest) = line.strip_prefix("fail ")
        && let Some((code, text)) = rest.split_once(": ")
    {
        return Ready::Fail {
            code: code.to_owned(),
            text: text.to_owned(),
        };
    }
    Ready::Fail {
        code: "supervisor_spawn_failed".to_owned(),
        text: "the supervisor did not report readiness".to_owned(),
    }
}

/// A stand-in session for an event that arrives before the daemon has the
/// real one indexed. Its fields are overwritten from the log on the next
/// scan; only the id is load-bearing here.
fn placeholder(id: SessionId) -> Session {
    Session {
        id,
        // No real project: a nil id never attributes this stand-in to a
        // live project until the next scan overwrites it from the log.
        project_id: willie_core::id::ProjectId::nil(),
        harness: String::new(),
        workspace: String::new(),
        kind: SessionKind::Agent,
        state: willie_core::session::SessionState::Running,
        created_at: String::new(),
        started_at: None,
        finished_at: None,
        pid: None,
        clients: 0,
        resumed_from: None,
        label: None,
        title: None,
        sandbox: Default::default(),
    }
}

/// The supervisor stopped answering without a terminal event: fold the
/// session from its log and, if it is still marked live, fail it closed
/// with `supervisor_lost` so the UI never shows a phantom running session.
fn finalise_lost(
    state: &Arc<Mutex<State>>,
    out: &crate::outbound::Outbound,
    state_dir: &std::path::Path,
    id: SessionId,
) {
    let found = session_store::load_all(state_dir)
        .into_iter()
        .find(|(spec, _)| spec.id == id);
    let Some((spec, evs)) = found else { return };
    let mut session = from_log(&spec, &evs);
    if !session.state.is_terminal() {
        apply_event(
            &mut session,
            &SessionEvent {
                at: crate::real_clock_or_zero(),
                kind: SessionEventKind::Failed {
                    code: "supervisor_lost".into(),
                    message: "the supervisor stopped answering".into(),
                },
            },
        );
    }
    state::emit(state, out, |s| s.upsert_session(session));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::outbound::Outbound;
    use willie_core::sandbox::{ExtraPath, PathMode};

    #[test]
    fn ok_line_parses_to_a_pid() {
        assert!(matches!(parse_ready("ok 4321"), Ready::Ok(4321)));
    }

    #[test]
    fn a_fail_line_keeps_its_code_and_text() {
        match parse_ready("fail harness_exec_failed: No such file") {
            Ready::Fail { code, text } => {
                assert_eq!(code, "harness_exec_failed");
                assert_eq!(text, "No such file");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn garbage_is_a_spawn_failure() {
        match parse_ready("who knows") {
            Ready::Fail { code, .. } => {
                assert_eq!(code, "supervisor_spawn_failed");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_fresh_request_launches_fresh_with_no_lineage() {
        let (mode, resumed_from) =
            resume_decision(false, Resume::ById, true, Some(SessionId::new()))
                .unwrap();
        assert_eq!(mode, LaunchMode::Fresh);
        assert_eq!(resumed_from, None);
    }

    #[test]
    fn a_resume_request_launches_continue_linked_to_the_latest_terminal() {
        let latest = SessionId::new();
        let (mode, resumed_from) =
            resume_decision(true, Resume::ById, false, Some(latest)).unwrap();
        assert_eq!(mode, LaunchMode::Continue);
        assert_eq!(resumed_from, Some(latest));
    }

    #[test]
    fn resume_is_refused_when_the_harness_cannot_resume() {
        let err = resume_decision(true, Resume::None, false, None).unwrap_err();
        assert_eq!(err.code, "harness_cannot_resume");
    }

    #[test]
    fn resume_is_refused_while_a_session_is_already_live() {
        let err =
            resume_decision(true, Resume::ById, true, Some(SessionId::new()))
                .unwrap_err();
        assert_eq!(err.code, "session_already_live");
    }

    #[test]
    fn the_live_guard_is_checked_before_the_lineage_is_used() {
        // No prior terminal session at all: resume still launches Continue
        // (the harness's own "nothing to continue" message is the fallback
        // the design accepts), just with no lineage to record.
        let (mode, resumed_from) =
            resume_decision(true, Resume::ById, false, None).unwrap();
        assert_eq!(mode, LaunchMode::Continue);
        assert_eq!(resumed_from, None);
    }

    /// The real harness defaults, so a change to layer 1 is caught
    /// here and not only where it is declared.
    fn claude_defaults() -> CapabilitySet {
        harness::claude().default_capabilities()
    }

    #[test]
    fn a_resolved_policy_keeps_the_project_and_the_harness_defaults() {
        let set = session_capabilities(
            claude_defaults(),
            &SandboxProfile::default(),
            Path::new("/home/willie"),
        )
        .unwrap();

        assert!(set.project_rw);
        assert!(set.agent_state);
        assert!(set.tools_ro);
    }

    #[test]
    fn a_profile_that_disables_the_credential_reaches_the_policy() {
        let profile = SandboxProfile {
            agent_state: Some(false),
            ..SandboxProfile::default()
        };

        let set = session_capabilities(
            claude_defaults(),
            &profile,
            Path::new("/home/willie"),
        )
        .unwrap();

        assert!(!set.agent_state);
        assert!(set.tools_ro);
    }

    #[test]
    fn a_deferred_capability_refuses_the_session_with_its_own_code() {
        let profile = SandboxProfile {
            ssh: Some(true),
            ..SandboxProfile::default()
        };

        let err = session_capabilities(
            claude_defaults(),
            &profile,
            Path::new("/home/willie"),
        )
        .unwrap_err();

        assert_eq!(err.code, "sandbox_capability_unsupported");
        assert!(err.message.contains("ssh"), "{}", err.message);
        assert!(err.remediation.contains("ssh"), "{}", err.remediation);
    }

    #[test]
    fn a_relative_extra_path_refuses_the_session() {
        let profile = SandboxProfile {
            extra_paths: vec![ExtraPath {
                path: "srv/shared".into(),
                mode: PathMode::Ro,
            }],
            ..SandboxProfile::default()
        };

        let err = session_capabilities(
            claude_defaults(),
            &profile,
            Path::new("/home/willie"),
        )
        .unwrap_err();

        assert_eq!(err.code, "sandbox_profile_invalid");
        assert!(err.remediation.contains("absolute"), "{}", err.remediation);
    }

    /// Shared by every `rename` test: a session directory on disk (the
    /// spec `write_spec` would have written for a real `create`) and the
    /// matching in-memory `Session`, folded from an empty log the way
    /// `create` builds its first session.
    fn rename_fixture(state_dir: &Path) -> (SessionOps, SessionId, Session) {
        use willie_core::{id::ProjectId, sandbox::CapabilitySet};

        fn clock() -> String {
            "1".to_owned()
        }

        let id = SessionId::new();
        let spec = SessionSpec {
            id,
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: "/w".into(),
            socket: "/run/willie/sessions/s.sock".into(),
            argv: vec!["/bin/true".into()],
            env: Default::default(),
            created_at: clock(),
            willie_version: willie_core::VERSION.to_owned(),
            resumed_from: None,
            kind: SessionKind::Agent,
            capabilities: CapabilitySet::default(),
        };
        session_store::write_spec(state_dir, &spec).unwrap();
        let session = from_log(&spec, &[]);

        let state = Arc::new(Mutex::new(State::default()));
        crate::lock(&state).sessions.insert(id, session.clone());
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        let ops = SessionOps::new(
            Arc::clone(&state),
            out,
            state_dir.to_path_buf(),
            state_dir.join("run"),
            state_dir.join("home"),
            clock,
            runner,
        );
        (ops, id, session)
    }

    /// A fixture whose id never went through `write_spec`/state insertion.
    fn unknown_id_ops(state_dir: &Path) -> SessionOps {
        fn clock() -> String {
            "1".to_owned()
        }
        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        SessionOps::new(
            Arc::clone(&state),
            out,
            state_dir.to_path_buf(),
            state_dir.join("run"),
            state_dir.join("home"),
            clock,
            runner,
        )
    }

    /// Renaming sets the in-memory label, appends exactly one `Renamed`
    /// line the log's last event parses to, and leaves the change visible
    /// in `state` for the next snapshot/emit to pick up.
    #[test]
    fn rename_sets_the_label_persists_the_event_and_emits_a_change() {
        let state_dir =
            std::env::temp_dir().join("willie-sess-rename-ok-state");
        let _ = std::fs::remove_dir_all(&state_dir);
        let (ops, id, _session) = rename_fixture(&state_dir);

        let session = ops
            .rename(willie_proto::session::RenameParams {
                id,
                label: Some("auth guard".into()),
            })
            .unwrap();
        assert_eq!(session.label.as_deref(), Some("auth guard"));

        // The in-memory session held by `state` agrees.
        assert_eq!(
            crate::lock(&ops.state).sessions.get(&id).unwrap().label,
            Some("auth guard".to_owned())
        );

        let (_, events) = session_store::load_all(&state_dir)
            .into_iter()
            .find(|(spec, _)| spec.id == id)
            .unwrap();
        match &events.last().unwrap().kind {
            SessionEventKind::Renamed { label } => {
                assert_eq!(label.as_deref(), Some("auth guard"));
            }
            other => panic!("{other:?}"),
        }

        let _ = std::fs::remove_dir_all(&state_dir);
    }

    /// An empty (or all-whitespace) label clears an existing one, both in
    /// memory and in the persisted event.
    #[test]
    fn rename_with_an_empty_label_clears_it() {
        let state_dir =
            std::env::temp_dir().join("willie-sess-rename-clear-state");
        let _ = std::fs::remove_dir_all(&state_dir);
        let (ops, id, mut session) = rename_fixture(&state_dir);
        session.label = Some("old".into());
        crate::lock(&ops.state).sessions.insert(id, session);

        let session = ops
            .rename(willie_proto::session::RenameParams {
                id,
                label: Some("   ".into()),
            })
            .unwrap();
        assert_eq!(session.label, None);

        let (_, events) = session_store::load_all(&state_dir)
            .into_iter()
            .find(|(spec, _)| spec.id == id)
            .unwrap();
        match &events.last().unwrap().kind {
            SessionEventKind::Renamed { label } => assert_eq!(*label, None),
            other => panic!("{other:?}"),
        }

        let _ = std::fs::remove_dir_all(&state_dir);
    }

    /// An unknown session id is refused before any event is appended.
    #[test]
    fn renaming_an_unknown_session_is_session_not_found() {
        let state_dir =
            std::env::temp_dir().join("willie-sess-rename-unknown-state");
        let _ = std::fs::remove_dir_all(&state_dir);
        let ops = unknown_id_ops(&state_dir);

        let err = ops
            .rename(willie_proto::session::RenameParams {
                id: SessionId::new(),
                label: Some("x".into()),
            })
            .unwrap_err();
        assert_eq!(err.code, "session_not_found");

        let _ = std::fs::remove_dir_all(&state_dir);
    }

    /// A label past the 120-character cap is refused, and nothing is
    /// persisted or changed.
    #[test]
    fn a_label_over_120_chars_is_invalid_params() {
        let state_dir =
            std::env::temp_dir().join("willie-sess-rename-toolong-state");
        let _ = std::fs::remove_dir_all(&state_dir);
        let (ops, id, _session) = rename_fixture(&state_dir);

        let long = "x".repeat(121);
        let err = ops
            .rename(willie_proto::session::RenameParams {
                id,
                label: Some(long),
            })
            .unwrap_err();
        assert_eq!(err.code, "invalid_params");
        assert_eq!(err.remediation, "shorten the name");
        assert_eq!(
            crate::lock(&ops.state).sessions.get(&id).unwrap().label,
            None
        );

        let _ = std::fs::remove_dir_all(&state_dir);
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod create_tests {
    use std::sync::{Arc, Mutex, mpsc};

    use willie_core::{
        id::ProjectId,
        project::{Project, ProjectState},
        sandbox::SandboxProfile,
        session::SessionKind,
    };
    use willie_proto::{job::JobKind, session::CreateParams};

    use super::SessionOps;
    use crate::{jobs::Runner, outbound::Outbound, state::State};

    fn clock() -> String {
        "t".to_owned()
    }

    fn ready_project() -> Project {
        Project {
            id: ProjectId::new(),
            name: "p".into(),
            slug: "p".into(),
            source: "C:\\src".into(),
            workspace: "/w".into(),
            branch: "main".into(),
            state: ProjectState::Ready,
            source_present: true,
            created_at: clock(),
            sandbox: SandboxProfile::default(),
            sandbox_problem: None,
        }
    }

    /// The workspace-deletion race: `project.remove` runs `remove_dir_all`
    /// in a background job while the project stays `Ready`. A `session.create`
    /// in that window would spawn a supervisor whose cwd is being deleted,
    /// so `create` must refuse `project_busy` once a job is in flight.
    #[test]
    fn create_refuses_a_project_with_a_job_in_flight() {
        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        let project = ready_project();
        let pid = project.id;
        crate::lock(&state).projects.insert(pid, project);

        // Saturate the project's single job slot with a job that blocks
        // until the channel is dropped, mirroring the projects.rs busy
        // test; `submit` inserts `pid` into the busy set synchronously.
        let (tx, rx) = mpsc::channel::<()>();
        let held = runner.submit(
            JobKind::SyncToWindows,
            pid,
            Box::new(move |_| {
                let _ = rx.recv();
                Ok(String::new())
            }),
        );
        assert!(held.is_ok());

        let ops = SessionOps::new(
            Arc::clone(&state),
            out,
            std::env::temp_dir().join("willie-sess-busy-state"),
            std::env::temp_dir().join("willie-sess-busy-run"),
            std::env::temp_dir().join("willie-sess-busy-home"),
            clock,
            Arc::clone(&runner),
        );
        let err = ops
            .create(CreateParams {
                project_id: pid,
                git_identity: None,
                resume: false,
                resume_from: None,
                kind: SessionKind::Agent,
            })
            .unwrap_err();
        assert_eq!(err.code, "project_busy");

        // Release the held job so its worker thread ends cleanly.
        drop(tx);
    }

    /// The live-session guard trips before `create` ever touches the
    /// harness or spawns a supervisor, so this exercises the real `create`
    /// path (not just the extracted decision) without needing a `claude`
    /// binary on this host.
    #[test]
    fn create_refuses_a_resume_while_a_session_is_already_live() {
        use willie_core::{
            id::SessionId,
            session::{Session, SessionKind, SessionState},
        };

        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        let project = ready_project();
        let pid = project.id;
        crate::lock(&state).projects.insert(pid, project);
        let live = Session {
            id: SessionId::new(),
            project_id: pid,
            harness: "claude-code".into(),
            workspace: "/w".into(),
            kind: SessionKind::Agent,
            state: SessionState::Running,
            created_at: clock(),
            started_at: None,
            finished_at: None,
            pid: Some(1),
            clients: 0,
            resumed_from: None,
            label: None,
            title: None,
            sandbox: Default::default(),
        };
        crate::lock(&state).sessions.insert(live.id, live);

        let ops = SessionOps::new(
            Arc::clone(&state),
            out,
            std::env::temp_dir().join("willie-sess-resume-live-state"),
            std::env::temp_dir().join("willie-sess-resume-live-run"),
            std::env::temp_dir().join("willie-sess-resume-live-home"),
            clock,
            Arc::clone(&runner),
        );
        let err = ops
            .create(CreateParams {
                project_id: pid,
                git_identity: None,
                resume: true,
                resume_from: None,
                kind: SessionKind::Agent,
            })
            .unwrap_err();
        assert_eq!(err.code, "session_already_live");
    }

    /// The policy is resolved before the harness is detected and before
    /// `identity::ensure` may write `~/.gitconfig`, so this refusal
    /// needs no `claude` binary on the host — which is exactly the
    /// property being asserted: nothing has happened yet when the
    /// daemon says no.
    #[test]
    fn create_refuses_a_deferred_capability_before_anything_is_touched() {
        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        let mut project = ready_project();
        project.sandbox = SandboxProfile {
            ssh: Some(true),
            ..SandboxProfile::default()
        };
        let pid = project.id;
        crate::lock(&state).projects.insert(pid, project);

        let home = std::env::temp_dir().join("willie-sess-sandbox-home");
        let identity = home.join(".gitconfig");
        let _ = std::fs::remove_file(&identity);

        let ops = SessionOps::new(
            Arc::clone(&state),
            out,
            std::env::temp_dir().join("willie-sess-sandbox-state"),
            std::env::temp_dir().join("willie-sess-sandbox-run"),
            home,
            clock,
            Arc::clone(&runner),
        );
        let err = ops
            .create(CreateParams {
                project_id: pid,
                git_identity: None,
                resume: false,
                resume_from: None,
                kind: SessionKind::Agent,
            })
            .unwrap_err();

        assert_eq!(err.code, "sandbox_capability_unsupported");
        assert!(err.message.contains("ssh"), "{}", err.message);
        assert!(!identity.exists());
    }

    /// A project whose `[sandbox]` table failed to parse carries the
    /// default profile in memory, but the daemon must still refuse to
    /// open a session for it until its owner replaces the table: the
    /// default profile is not what the person wrote, and running under
    /// it silently would hide the problem instead of surfacing it.
    #[test]
    fn create_refuses_a_project_with_a_sandbox_problem() {
        use willie_core::project::SandboxProblem;

        let state = Arc::new(Mutex::new(State::default()));
        let (out, _h) = Outbound::spawn(std::io::sink());
        let runner =
            Arc::new(Runner::new(Arc::clone(&state), out.clone(), clock));
        let mut project = ready_project();
        project.sandbox_problem = Some(SandboxProblem {
            code: "sandbox_profile_invalid".into(),
            message: "the [sandbox] table could not be read: bad".into(),
            remediation: "fix the file".into(),
        });
        let pid = project.id;
        crate::lock(&state).projects.insert(pid, project);

        let ops = SessionOps::new(
            Arc::clone(&state),
            out,
            std::env::temp_dir().join("willie-sess-problem-state"),
            std::env::temp_dir().join("willie-sess-problem-run"),
            std::env::temp_dir().join("willie-sess-problem-home"),
            clock,
            Arc::clone(&runner),
        );
        let err = ops
            .create(CreateParams {
                project_id: pid,
                git_identity: None,
                resume: false,
                resume_from: None,
                kind: SessionKind::Agent,
            })
            .unwrap_err();

        assert_eq!(err.code, "sandbox_profile_invalid");
        assert_eq!(err.remediation, "fix the file");
    }
}
