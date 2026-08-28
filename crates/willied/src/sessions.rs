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
    session::{
        Session, SessionEvent, SessionEventKind, SessionSpec, apply_event,
        from_log,
    },
};
use willie_harness::Harness;
use willie_linux::paths::{SUPERVISOR_BIN, session_socket, sessions_run_dir};

use crate::{
    harness, identity, jobs::Runner, projects::OpError, session_store, state,
    state::State,
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
        }
    }

    /// Resolve fail-closed, write the spec, spawn the supervisor and wait
    /// for its readiness. The returned session is `Running` once the
    /// supervisor reports its pid; a control connection then folds the
    /// supervisor's later events into the session.
    pub fn create(
        &self,
        params: willie_proto::session::CreateParams,
    ) -> Result<Session, OpError> {
        let project = {
            let s = crate::lock(&self.state);
            s.projects.get(&params.project_id).cloned().ok_or_else(|| {
                crate::projects::not_found_err(params.project_id)
            })?
        };
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
                    if terminal {
                        return;
                    }
                    let answers = crate::control::connect(&socket)
                        .and_then(|mut c| c.status())
                        .is_ok();
                    if !answers {
                        finalise_lost(&state, &out, &state_dir, id);
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
        state: willie_core::session::SessionState::Running,
        created_at: String::new(),
        started_at: None,
        finished_at: None,
        pid: None,
        clients: 0,
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
    use super::*;

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
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod create_tests {
    use std::sync::{Arc, Mutex, mpsc};

    use willie_core::{
        id::ProjectId,
        project::{Project, ProjectState},
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
            })
            .unwrap_err();
        assert_eq!(err.code, "project_busy");

        // Release the held job so its worker thread ends cleanly.
        drop(tx);
    }
}
