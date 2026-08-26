//! The request loop: one JSON-RPC object per line in, one per line out.
//! Any malformed line produces an error response (id 0) and the loop goes
//! on; EOF or `daemon.shutdown` ends it.

use std::{
    io::{self, BufRead, Write},
    time::Instant,
};

use willie_proto::{
    daemon::{DoctorReport, method},
    rpc::{Request, Response, RpcError},
};

use crate::handlers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Eof,
    Shutdown,
}

pub struct Server {
    started: Instant,
    doctor: fn() -> DoctorReport,
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
    pub fn new(doctor: fn() -> DoctorReport) -> Self {
        Self {
            started: Instant::now(),
            doctor,
            shutting_down: false,
        }
    }

    pub fn dispatch(&mut self, req: Request) -> Response {
        let id = req.id;
        let result = match req.method.as_str() {
            method::HELLO => handlers::hello(req.params),
            method::HEALTH => handlers::health(self.started),
            method::DOCTOR => handlers::doctor(self.doctor),
            method::SHUTDOWN => {
                self.shutting_down = true;
                Ok(serde_json::Value::Null)
            }
            other => Err(RpcError::new(
                "method_not_found",
                format!("unknown method `{other}`"),
            )
            .with_remediation("see docs/PROTOCOL.md for the daemon.* methods")),
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

    pub fn serve<R: BufRead, W: Write>(
        &mut self,
        reader: R,
        mut writer: W,
    ) -> io::Result<ExitReason> {
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
            let text =
                serde_json::to_string(&response).map_err(io::Error::other)?;
            writeln!(writer, "{text}")?;
            writer.flush()?;
            if self.shutting_down {
                return Ok(ExitReason::Shutdown);
            }
        }
        Ok(ExitReason::Eof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use willie_proto::daemon::{
        CheckStatus, DoctorCheck, DoctorReport, Hello, HelloReply, method,
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

    fn line(method: &str, params: serde_json::Value) -> String {
        serde_json::to_string(&Request::new(1, method, params).unwrap())
            .unwrap()
    }

    fn roundtrip(input: &str) -> (ExitReason, Vec<Response>) {
        let mut out = Vec::new();
        let reason = Server::new(fake_doctor)
            .serve(Cursor::new(input), &mut out)
            .unwrap();
        let responses = String::from_utf8(out)
            .unwrap()
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
}
