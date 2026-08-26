//! JSON-RPC client over a `LineTransport`: sequential calls, responses
//! matched by id, notifications and stray lines skipped (logged).

use std::{
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};

use serde::{Serialize, de::DeserializeOwned};
use willie_proto::rpc::{Request, Response};

use crate::{error::EngineError, process::LineTransport};

#[derive(Debug)]
pub struct RpcClient {
    transport: LineTransport,
    timeout: Duration,
    next_id: u64,
}

impl RpcClient {
    #[must_use]
    pub fn new(transport: LineTransport, timeout: Duration) -> Self {
        Self {
            transport,
            timeout,
            next_id: 1,
        }
    }

    /// Changes the timeout applied to calls made from now on.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, EngineError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request::new(id, method, params)
            .map_err(|e| EngineError::Protocol(e.to_string()))?;
        let line = serde_json::to_string(&request)
            .map_err(|e| EngineError::Protocol(e.to_string()))?;
        self.transport.send_line(&line)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            // A flooding peer can keep `recv_line` returning instantly
            // from its backlog; check the deadline explicitly too.
            if Instant::now() >= deadline {
                return Err(EngineError::Timeout {
                    method: method.to_owned(),
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.transport.recv_line(remaining) {
                Ok(Some(line)) => line,
                Ok(None) => {
                    return Err(EngineError::Protocol(
                        "daemon closed its stdout".into(),
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(EngineError::Timeout {
                        method: method.to_owned(),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(EngineError::Protocol(
                        "reader thread gone".into(),
                    ));
                }
            };
            let Ok(response) = serde_json::from_str::<Response>(&line) else {
                eprintln!("engine: ignoring non-response line: {line}");
                continue;
            };
            if response.id != id {
                eprintln!(
                    "engine: ignoring response with unexpected id {}",
                    response.id
                );
                continue;
            }
            let value = response.into_result().map_err(EngineError::Rpc)?;
            return serde_json::from_value(value).map_err(|e| {
                EngineError::Protocol(format!("bad `{method}` result: {e}"))
            });
        }
    }

    pub fn close(&mut self) {
        self.transport.close_input();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, Write},
        time::Duration,
    };

    use super::*;
    use crate::test_support::{announce_ready, await_ready, spawn_peer};

    /// The child (this test binary in peer mode) announces readiness,
    /// writes one notification and one garbage line, then answers every
    /// request with `{"result":{"echo":<method>}}`.
    fn peer_main() {
        announce_ready();
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        let notice = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "state.event",
            "params": {},
        });
        writeln!(out, "{}", serde_json::to_string(&notice).unwrap()).unwrap();
        writeln!(out, "willied: not json, just noise").unwrap();
        out.flush().unwrap();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let req: Request = serde_json::from_str(&line).unwrap();
            let result = serde_json::json!({"echo": req.method});
            let resp = Response::ok(req.id, result).unwrap();
            writeln!(out, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
            out.flush().unwrap();
        }
        std::process::exit(0);
    }

    #[derive(serde::Deserialize)]
    struct Echo {
        echo: String,
    }

    #[test]
    fn calls_match_responses_by_id_and_skip_noise() {
        if std::env::var_os("WILLIE_PEER_MODE").is_some() {
            peer_main();
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::calls_match_responses_by_id_and_skip_noise",
            "WILLIE_PEER_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_secs(5));
        let a: Echo = client.call("daemon.health", ()).unwrap();
        let b: Echo = client.call("daemon.doctor", ()).unwrap();
        assert_eq!(a.echo, "daemon.health");
        assert_eq!(b.echo, "daemon.doctor");
        client.close();
        child.wait().unwrap();
    }

    #[test]
    fn a_silent_peer_times_out_with_the_method_name() {
        if std::env::var_os("WILLIE_PEER_MODE").is_some() {
            announce_ready();
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::a_silent_peer_times_out_with_the_method_name",
            "WILLIE_PEER_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_millis(300));
        let err = client
            .call::<_, serde_json::Value>("daemon.health", ())
            .unwrap_err();
        assert!(
            matches!(
                err,
                EngineError::Timeout { ref method } if method == "daemon.health"
            ),
            "{err}"
        );
        child.kill().unwrap();
    }

    /// A peer that never stops chattering must not starve the deadline:
    /// the backlog can keep `recv_line` returning instantly, but the
    /// call still has to give up within roughly its configured budget.
    #[test]
    fn floods_of_notifications_still_time_out() {
        if std::env::var_os("WILLIE_PEER_MODE").is_some() {
            announce_ready();
            let mut out = std::io::stdout();
            let notice = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "state.event",
                "params": {},
            });
            let line = serde_json::to_string(&notice).unwrap();
            loop {
                if writeln!(out, "{line}").is_err() || out.flush().is_err() {
                    break;
                }
            }
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::floods_of_notifications_still_time_out",
            "WILLIE_PEER_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_millis(300));
        let start = Instant::now();
        let err = client
            .call::<_, serde_json::Value>("daemon.health", ())
            .unwrap_err();
        let elapsed = start.elapsed();
        assert!(matches!(err, EngineError::Timeout { .. }), "{err}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        child.kill().unwrap();
    }
}
