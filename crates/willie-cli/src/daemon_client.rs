//! A client for the daemon's own local socket
//! (`willie_linux::paths::daemon_socket`): one connection, one request in
//! flight at a time, each request/reply a single ndjson line — the same
//! framing `willied`'s socket server speaks
//! (`crates/willied/src/main.rs`'s `serve_connection`). Unix-socket only.

use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};
use willie_proto::rpc::{Request, Response, RpcError};

/// A read or a write stalls this long before the CLI gives up and
/// reports `daemon_timeout`, rather than hanging forever on a wedged
/// daemon — one that accepted the connection but never drains it, as
/// much as one that never replies.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Every `daemon_transport` error shares this remediation: none of them
/// name a specific fix (unlike `daemon_unreachable`'s "open the app" or
/// `daemon_timeout`'s "try again"), so `doctor` is the generic next
/// step.
const TRANSPORT_REMEDIATION: &str =
    "the daemon connection failed; run willie doctor and try again";

/// A live connection to the daemon's socket.
pub struct Client {
    stream: BufReader<UnixStream>,
    /// The next request id; incremented before each `call`, so the first
    /// request is id 1.
    id: u64,
}

impl Client {
    /// Connects and sets the read and write timeouts every call runs
    /// under. A missing socket or nothing listening on it is
    /// `daemon_unreachable`; any other failure to connect is
    /// `daemon_transport`.
    pub fn connect(socket: &Path) -> Result<Self, RpcError> {
        let stream = UnixStream::connect(socket).map_err(connect_error)?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(transport_error)?;
        stream
            .set_write_timeout(Some(READ_TIMEOUT))
            .map_err(transport_error)?;
        Ok(Self {
            stream: BufReader::new(stream),
            id: 0,
        })
    }

    /// One request, one reply: writes a `Request` line, reads exactly one
    /// response line back, and collapses the envelope into a plain
    /// `Result`. A stalled write or read is `daemon_timeout`; any other
    /// I/O failure, a line that closed early, or a reply that doesn't
    /// decode is `daemon_transport`.
    ///
    /// This client is strictly single-in-flight: one request per
    /// connection, and `?` tears the whole call down on any error, so a
    /// reply is trusted to answer the request just written without
    /// checking `response.id` against the request's id. A future change
    /// that pipelines several in-flight requests on one `Client` would
    /// need to add that check back.
    pub fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, RpcError> {
        self.id += 1;
        let request =
            Request::new(self.id, method, params).map_err(encode_error)?;
        let line = serde_json::to_string(&request).map_err(encode_error)?;
        let socket = self.stream.get_mut();
        writeln!(socket, "{line}").map_err(timeout_or_transport)?;
        socket.flush().map_err(timeout_or_transport)?;

        let mut reply = String::new();
        let n = self
            .stream
            .read_line(&mut reply)
            .map_err(timeout_or_transport)?;
        if n == 0 {
            return Err(RpcError::new(
                "daemon_transport",
                "the daemon closed the connection without replying",
            )
            .with_remediation(TRANSPORT_REMEDIATION));
        }
        let response: Response =
            serde_json::from_str(&reply).map_err(decode_error)?;
        let value = response.into_result()?;
        serde_json::from_value(value).map_err(decode_error)
    }
}

fn connect_error(e: io::Error) -> RpcError {
    match e.kind() {
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound => {
            RpcError::new(
                "daemon_unreachable",
                format!("cannot reach the daemon's socket: {e}"),
            )
            .with_remediation(
                "open the Willie app; the daemon runs while it is open",
            )
        }
        _ => transport_error(e),
    }
}

/// A stalled read or write surfaces as `WouldBlock` or `TimedOut`
/// depending on the platform; both mean the same thing here — the
/// daemon stayed quiet, whether the CLI was waiting on a reply or
/// waiting for the daemon to drain what it wrote.
fn timeout_or_transport(e: io::Error) -> RpcError {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => RpcError::new(
            "daemon_timeout",
            "the daemon did not answer in ten seconds",
        )
        .with_remediation(
            "the daemon did not answer in time; run willie doctor if \
             it repeats",
        ),
        _ => transport_error(e),
    }
}

fn transport_error(e: io::Error) -> RpcError {
    RpcError::new("daemon_transport", format!("cannot reach the daemon: {e}"))
        .with_remediation(TRANSPORT_REMEDIATION)
}

fn encode_error(e: serde_json::Error) -> RpcError {
    RpcError::new(
        "daemon_transport",
        format!("cannot encode the request: {e}"),
    )
    .with_remediation(TRANSPORT_REMEDIATION)
}

fn decode_error(e: serde_json::Error) -> RpcError {
    RpcError::new(
        "daemon_transport",
        format!("cannot decode the daemon's reply: {e}"),
    )
    .with_remediation(TRANSPORT_REMEDIATION)
}
