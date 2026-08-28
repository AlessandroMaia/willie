//! The request loop: one JSON-RPC object per line in, one per line out.
//! Any malformed line produces an error response (id 0) and the loop goes
//! on; EOF or `daemon.shutdown` ends it. The loop only reads: every reply
//! leaves through the single-writer [`Outbound`], so responses never
//! interleave with the job and project events it also carries.

use std::{
    io::{self, BufRead},
    sync::{Arc, Mutex},
    time::Instant,
};

use willie_proto::{
    daemon::{DoctorReport, method as daemon},
    job::method as job,
    project::method as project,
    rpc::{Request, Response, RpcError},
    session::method as session,
    state::method as state_method,
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
            project::LIST => handlers::project_list(&self.state),
            job::LIST => handlers::job_list(&self.state),
            job::GET => handlers::job_get(&self.state, req.params),
            job::CANCEL => handlers::job_cancel(&self.ops, req.params),
            session::CREATE => {
                handlers::session_create(&self.sessions, req.params)
            }
            session::STOP => handlers::session_stop(&self.sessions, req.params),
            session::LIST => handlers::session_list(&self.sessions),
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

    pub fn serve<R: BufRead>(&mut self, reader: R) -> io::Result<ExitReason> {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Request>(&line) {
                Ok(req) => self.dispatch(req),
                Err(e) => Response::err(
                    0,
                    RpcError::new(
                        "invalid_request",
                        format!("not a JSON-RPC request: {e}"),
                    ),
                ),
            };
            self.out.send_response(response);
            if self.shutting_down {
                return Ok(ExitReason::Shutdown);
            }
        }
        Ok(ExitReason::Eof)
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

    fn roundtrip(input: &str) -> (ExitReason, Vec<Response>) {
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
        );
        let mut server =
            Server::new(fake_doctor, Arc::clone(&state), ops, sessions, out);
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
}
