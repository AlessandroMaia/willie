//! The request loop: one JSON-RPC object per line in, one per line out.
//! Any malformed line produces an error response (id 0) and the loop goes
//! on; EOF or `daemon.shutdown` ends it. The loop only reads: every reply
//! leaves through the single-writer [`Outbound`], so responses never
//! interleave with the job and project events it also carries.

use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender},
    },
    time::Instant,
};

use willie_proto::{
    daemon::{DoctorReport, method as daemon},
    job::method as job,
    project::method as project,
    rpc::{Request, Response, RpcError},
    sandbox::method as sandbox,
    session::method as session,
    state::method as state_method,
    tool::method as tool,
};

use crate::{
    handlers, outbound::Outbound, projects::Ops, sessions::SessionOps,
    state::State,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Eof,
    Shutdown,
}

/// Where a request came from: the engine's stdio pipe, or a local socket
/// client. `daemon.shutdown` is the engine's alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Stdio,
    Socket,
}

/// One request reaching the dispatcher, and the way its reply leaves. The
/// stdin reader and every socket connection feed the same channel, so a
/// single dispatcher answers both and the daemon keeps its one-request-at-
/// a-time invariant. A stdio reply leaves through [`Outbound`] (stdout) and
/// carries no `reply`; a socket reply goes back through its connection's
/// sender.
pub enum Inbound {
    Line {
        text: String,
        origin: Origin,
        reply: Option<Sender<Response>>,
    },
    StdinClosed,
}

pub struct Server {
    started: Instant,
    doctor: fn() -> DoctorReport,
    state: Arc<Mutex<State>>,
    ops: Ops,
    sessions: SessionOps,
    out: Outbound,
    shutting_down: bool,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl Server {
    #[must_use]
    pub fn new(
        doctor: fn() -> DoctorReport,
        state: Arc<Mutex<State>>,
        ops: Ops,
        sessions: SessionOps,
        out: Outbound,
    ) -> Self {
        Self {
            started: Instant::now(),
            doctor,
            state,
            ops,
            sessions,
            out,
            shutting_down: false,
        }
    }

    /// Trips every in-flight job's cancel flag. Called once the serve loop
    /// has returned, before the writer thread is joined.
    pub fn shutdown(&self) {
        self.ops.shutdown();
    }

    pub fn dispatch(&mut self, req: Request) -> Response {
        let id = req.id;
        let result = match req.method.as_str() {
            daemon::HELLO => handlers::hello(req.params),
            daemon::HEALTH => handlers::health(self.started),
            daemon::DOCTOR => handlers::doctor(self.doctor),
            daemon::SHUTDOWN => {
                self.shutting_down = true;
                Ok(serde_json::Value::Null)
            }
            project::ADD => handlers::project_add(&self.ops, req.params),
            project::REMOVE => handlers::project_remove(&self.ops, req.params),
            project::SYNC_TO_WINDOWS => {
                handlers::project_sync(&self.ops, req.params)
            }
            project::UPDATE_FROM_WINDOWS => {
                handlers::project_update(&self.ops, req.params)
            }
            project::RELOCATE => {
                handlers::project_relocate(&self.ops, req.params)
            }
            project::RENAME => handlers::project_rename(&self.ops, req.params),
            project::SET_SANDBOX => {
                handlers::project_set_sandbox(&self.ops, req.params)
            }
            project::LIST => handlers::project_list(&self.state),
            job::LIST => handlers::job_list(&self.state),
            job::GET => handlers::job_get(&self.state, req.params),
            job::CANCEL => handlers::job_cancel(&self.ops, req.params),
            tool::INSTALL => handlers::tool_install(&self.ops, req.params),
            session::CREATE => {
                handlers::session_create(&self.sessions, req.params)
            }
            session::STOP => handlers::session_stop(&self.sessions, req.params),
            session::LIST => handlers::session_list(&self.sessions),
            sandbox::EXPLAIN => {
                handlers::sandbox_explain(&self.state, req.params)
            }
            state_method::SNAPSHOT => {
                handlers::state_snapshot(&self.ops, &self.state)
            }
            other => Err(RpcError::new(
                "method_not_found",
                format!("unknown method `{other}`"),
            )
            .with_remediation("see docs/PROTOCOL.md for the served methods")),
        };
        match result {
            Ok(value) => Response {
                jsonrpc: "2.0".to_owned(),
                id,
                result: Some(value),
                error: None,
            },
            Err(err) => Response::err(id, err),
        }
    }

    /// Dispatch a request knowing where it came from. `daemon.shutdown` is
    /// the engine's alone: over a socket it is refused with
    /// `method_not_served` and the daemon keeps running; over stdio it is
    /// unchanged. Every other method is origin-agnostic.
    pub fn dispatch_from(&mut self, origin: Origin, req: Request) -> Response {
        if origin == Origin::Socket && req.method == daemon::SHUTDOWN {
            return Response::err(
                req.id,
                RpcError::new(
                    "method_not_served",
                    "`daemon.shutdown` is the engine's; the daemon stops \
                     with the app",
                )
                .with_remediation("restart the daemon from the Dashboard"),
            );
        }
        self.dispatch(req)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    /// The request loop. One channel carries lines from every origin — the
    /// stdin reader and one thread per socket connection — so a single
    /// dispatcher serialises them all. A stdio reply leaves through the
    /// single-writer [`Outbound`] (stdout); a socket reply goes back to its
    /// connection. The loop ends when stdin closes or a stdio
    /// `daemon.shutdown` trips the flag.
    pub fn serve_inbound(&mut self, rx: Receiver<Inbound>) -> ExitReason {
        for msg in rx {
            match msg {
                Inbound::StdinClosed => return ExitReason::Eof,
                Inbound::Line {
                    text,
                    origin,
                    reply,
                } => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    let response = match serde_json::from_str::<Request>(&text)
                    {
                        Ok(req) => self.dispatch_from(origin, req),
                        Err(e) => Response::err(
                            0,
                            RpcError::new(
                                "invalid_request",
                                format!("not a JSON-RPC request: {e}"),
                            ),
                        ),
                    };
                    let shutting = self.shutting_down;
                    match origin {
                        Origin::Stdio => self.out.send_response(response),
                        Origin::Socket => {
                            if let Some(tx) = reply {
                                let _ = tx.send(response);
                            }
                        }
                    }
                    if shutting {
                        return ExitReason::Shutdown;
                    }
                }
            }
        }
        ExitReason::Eof
    }

    /// A thin stdio wrapper the unit tests drive: it pushes each reader
    /// line onto a channel as a `Stdio` origin, closes it at EOF, and runs
    /// [`Self::serve_inbound`]. The real daemon feeds the channel from two
    /// origins in `main` (a stdin reader thread and one thread per socket
    /// connection), but the stdio behaviour is identical. Test-only: the
    /// daemon no longer reads stdin in place.
    #[cfg(test)]
    pub fn serve<R: std::io::BufRead>(
        &mut self,
        reader: R,
    ) -> std::io::Result<ExitReason> {
        let (tx, rx) = std::sync::mpsc::channel();
        for line in reader.lines() {
            let line = line?;
            if tx
                .send(Inbound::Line {
                    text: line,
                    origin: Origin::Stdio,
                    reply: None,
                })
                .is_err()
            {
                break;
            }
        }
        let _ = tx.send(Inbound::StdinClosed);
        drop(tx);
        Ok(self.serve_inbound(rx))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Write};
    use std::sync::{Arc, Mutex};

    use willie_proto::daemon::{
        CheckStatus, DoctorCheck, DoctorReport, Hello, HelloReply, method,
    };
    use willie_proto::rpc::{Request, Response};

    use super::*;
    use crate::{
        jobs::Runner, outbound::Outbound, projects::Ops, sessions::SessionOps,
        state::State,
    };

    fn fake_doctor() -> DoctorReport {
        DoctorReport {
            checks: vec![DoctorCheck {
                name: "fake".into(),
                status: CheckStatus::Ok,
                detail: String::new(),
                remediation: None,
                required: true,
            }],
        }
    }

    fn clock() -> String {
        "t".to_owned()
    }

    fn line(method: &str, params: serde_json::Value) -> String {
        serde_json::to_string(&Request::new(1, method, params).unwrap())
            .unwrap()
    }

    /// A `Write` sink the test can read back after the writer thread drains.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);
    impl SharedBuf {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }
    impl Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Builds a `Server` over a readable stdout sink. The two callers
    /// share it (the skill forbids duplicated fixtures): `roundtrip`
    /// drives it through the stdin path, and the origin tests call
    /// `dispatch_from` on it directly.
    fn test_server() -> (Server, SharedBuf, std::thread::JoinHandle<()>) {
        let buf = SharedBuf::default();
        let (out, handle) = Outbound::spawn(buf.clone());
        let state = Arc::new(Mutex::new(State::default()));
        let runner = Runner::new(Arc::clone(&state), out.clone(), clock);
        let ops = Ops::new(
            Arc::clone(&state),
            runner,
            std::env::temp_dir().join("willie-server-test-state"),
            std::env::temp_dir().join("willie-server-test-ws"),
            clock,
            out.clone(),
        );
        let sessions = SessionOps::new(
            Arc::clone(&state),
            out.clone(),
            std::env::temp_dir().join("willie-server-test-state"),
            std::env::temp_dir().join("willie-server-test-run"),
            std::env::temp_dir().join("willie-server-test-home"),
            clock,
            ops.runner_handle(),
        );
        let server =
            Server::new(fake_doctor, Arc::clone(&state), ops, sessions, out);
        (server, buf, handle)
    }

    fn shutdown_request() -> Request {
        Request::new(1, method::SHUTDOWN, serde_json::json!({})).unwrap()
    }

    fn roundtrip(input: &str) -> (ExitReason, Vec<Response>) {
        let (mut server, buf, handle) = test_server();
        let reason = server.serve(Cursor::new(input)).unwrap();
        // Drop every `Outbound` sender so the writer thread drains and ends.
        drop(server);
        handle.join().unwrap();
        let responses = buf
            .contents()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        (reason, responses)
    }

    #[test]
    fn hello_answers_with_the_daemon_version_and_protocol() {
        let (_, resp) = roundtrip(&line(
            method::HELLO,
            serde_json::to_value(Hello::for_client("test")).unwrap(),
        ));
        let reply: HelloReply =
            serde_json::from_value(resp[0].clone().into_result().unwrap())
                .unwrap();
        assert_eq!(reply.willie_version, willie_core::VERSION);
        assert_eq!(reply.protocol_version, willie_proto::PROTOCOL_VERSION);
    }

    #[test]
    fn hello_with_another_protocol_version_is_refused() {
        let hello = serde_json::json!({
            "client": "x",
            "willie_version": "0.0.0",
            "protocol_version": 999
        });
        let (_, resp) = roundtrip(&line(method::HELLO, hello));
        assert_eq!(
            resp[0].clone().into_result().unwrap_err().code,
            "protocol_version_mismatch"
        );
    }

    #[test]
    fn unknown_methods_and_bad_json_get_error_responses_and_the_loop_continues()
    {
        let input = format!(
            "{}\nnot json at all\n{}\n",
            line("nope.x", serde_json::json!({})),
            line(method::HEALTH, serde_json::json!({}))
        );
        let (reason, resp) = roundtrip(&input);
        assert_eq!(reason, ExitReason::Eof);
        assert_eq!(resp.len(), 3);
        assert_eq!(
            resp[0].clone().into_result().unwrap_err().code,
            "method_not_found"
        );
        assert_eq!(
            resp[1].clone().into_result().unwrap_err().code,
            "invalid_request"
        );
        assert!(resp[2].clone().into_result().is_ok());
    }

    #[test]
    fn doctor_uses_the_injected_checks() {
        let (_, resp) = roundtrip(&line(method::DOCTOR, serde_json::json!({})));
        let report: DoctorReport =
            serde_json::from_value(resp[0].clone().into_result().unwrap())
                .unwrap();
        assert_eq!(report.checks[0].name, "fake");
    }

    /// `daemon.shutdown` from a socket client is refused, and the server
    /// is not marked shutting down; from stdio it still ends the loop
    /// (covered by `shutdown_replies_then_stops_serving`).
    #[test]
    fn shutdown_over_the_socket_is_refused_not_obeyed() {
        let (mut server, _buf, handle) = test_server();
        let resp = server.dispatch_from(Origin::Socket, shutdown_request());
        assert_eq!(resp.into_result().unwrap_err().code, "method_not_served");
        assert!(!server.is_shutting_down());
        drop(server);
        handle.join().unwrap();
    }

    #[test]
    fn shutdown_replies_then_stops_serving() {
        let input = format!(
            "{}\n{}\n",
            line(method::SHUTDOWN, serde_json::json!({})),
            line(method::HEALTH, serde_json::json!({}))
        );
        let (reason, resp) = roundtrip(&input);
        assert_eq!(reason, ExitReason::Shutdown);
        assert_eq!(resp.len(), 1, "nothing is processed after shutdown");
    }

    #[test]
    fn eof_ends_the_loop_cleanly() {
        let (reason, resp) = roundtrip("");
        assert_eq!(reason, ExitReason::Eof);
        assert!(resp.is_empty());
    }

    #[test]
    fn an_unknown_project_id_is_a_coded_error_not_a_panic() {
        let (_, resp) = roundtrip(&line(
            project::RENAME,
            serde_json::json!({
                "id": "proj_00000000000000000000000000",
                "name": "x"
            }),
        ));
        assert_eq!(
            resp[0].clone().into_result().unwrap_err().code,
            "project_not_found"
        );
    }

    /// `sandbox.explain` answers the same not-found code its neighbours
    /// use for an id the daemon has never seen.
    #[test]
    fn sandbox_explain_of_an_unknown_project_is_project_not_found() {
        let (_, resp) = roundtrip(&line(
            sandbox::EXPLAIN,
            serde_json::json!({
                "project_id": "proj_00000000000000000000000000"
            }),
        ));
        assert_eq!(
            resp[0].clone().into_result().unwrap_err().code,
            "project_not_found"
        );
    }

    /// `project.set_sandbox` is dispatched and answers the same
    /// not-found code every other `project.*` method uses.
    #[test]
    fn set_sandbox_of_an_unknown_project_is_project_not_found() {
        let (_, resp) = roundtrip(&line(
            project::SET_SANDBOX,
            serde_json::json!({
                "project_id": "proj_00000000000000000000000000",
                "profile": {}
            }),
        ));
        assert_eq!(
            resp[0].clone().into_result().unwrap_err().code,
            "project_not_found"
        );
    }
}
