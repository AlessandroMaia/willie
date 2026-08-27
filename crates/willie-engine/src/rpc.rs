//! JSON-RPC client over a `LineTransport`.
//!
//! One reader thread owns the peer's decoded line stream. For each line
//! it routes a [`Response`] to the call waiting on that id, fans a
//! [`Notification`] out to every live subscriber, and keeps anything that
//! is no envelope at all as a stray line for a failure report. Calls own
//! only the write half, so a call sending a request never blocks the
//! reader and a blocking read never blocks a send. When the peer closes
//! its stdout the reader wakes every waiter with the closed-stdout error.

use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    process::ChildStdin,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};
use willie_proto::rpc::{Notification, Request, Response};

use crate::{error::EngineError, process::LineTransport};

/// How many non-envelope lines are worth quoting when a call fails.
const STRAY_LINES: usize = 8;

/// The one message the engine reports when the daemon closes its stdout,
/// whether that is seen before a call or while one is waiting.
const CLOSED_STDOUT: &str = "daemon closed its stdout";

/// Calls waiting for a response, and whether the stream has ended. Kept
/// together so a call can check for the end and register atomically: the
/// reader cannot slip the closed verdict in between.
#[derive(Debug, Default)]
struct Inbox {
    waiters: HashMap<u64, Sender<Response>>,
    closed: bool,
}

#[derive(Debug)]
pub struct RpcClient {
    input: Option<ChildStdin>,
    timeout: Duration,
    next_id: u64,
    inbox: Arc<Mutex<Inbox>>,
    stray: Arc<Mutex<VecDeque<String>>>,
    subscribers: Arc<Mutex<Vec<Sender<Notification>>>>,
}

impl RpcClient {
    #[must_use]
    pub fn new(transport: LineTransport, timeout: Duration) -> Self {
        let (input, lines) = transport.split();
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let stray = Arc::new(Mutex::new(VecDeque::new()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        {
            let inbox = Arc::clone(&inbox);
            let stray = Arc::clone(&stray);
            let subscribers = Arc::clone(&subscribers);
            // A failed spawn leaves no router: calls then time out and no
            // notification is delivered, which is the correct fail-closed
            // behaviour when the host is this far out of threads.
            let _ = std::thread::Builder::new()
                .name("willie-rpc-router".into())
                .spawn(move || {
                    run_reader(&lines, &inbox, &stray, &subscribers)
                });
        }
        Self {
            input,
            timeout,
            next_id: 1,
            inbox,
            stray,
            subscribers,
        }
    }

    /// Changes the timeout applied to calls made from now on.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// A live subscription to every [`Notification`] the reader sees from
    /// now on. Notifications with no subscriber are dropped; a subscriber
    /// whose receiver is gone is dropped by the reader.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<Notification> {
        let (tx, rx) = mpsc::channel();
        lock(&self.subscribers).push(tx);
        rx
    }

    /// The last lines that were not JSON-RPC envelopes, oldest first.
    /// When the daemon never starts, `wsl.exe`'s own message is all the
    /// engine has to explain the failure with.
    #[must_use]
    pub fn stray_lines(&self) -> Vec<String> {
        lock(&self.stray).iter().cloned().collect()
    }

    /// [`Self::stray_lines`] as one block of text, empty when the peer
    /// only ever spoke the protocol.
    #[must_use]
    pub fn stray_text(&self) -> String {
        self.stray_lines().join("\n").trim().to_owned()
    }

    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| std::io::Error::other("input closed"))?;
        input.write_all(line.as_bytes())?;
        input.write_all(b"\n")?;
        input.flush()
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
        if self.input.is_none() {
            return Err(EngineError::Protocol(CLOSED_STDOUT.into()));
        }
        let (tx, rx) = mpsc::channel::<Response>();
        // Register the waiter before sending: a reply that arrives before
        // this call starts waiting must still find a channel to land in.
        {
            let mut inbox = lock(&self.inbox);
            if inbox.closed {
                return Err(EngineError::Protocol(CLOSED_STDOUT.into()));
            }
            inbox.waiters.insert(id, tx);
        }
        // A failed write means the peer is gone; leave the verdict to the
        // reader, which drains the peer's last words into stray lines and
        // then disconnects the wait below with the closed-stdout error.
        let _ = self.send_line(&line);
        match rx.recv_timeout(self.timeout) {
            Ok(response) => {
                let value = response.into_result().map_err(EngineError::Rpc)?;
                serde_json::from_value(value).map_err(|e| {
                    EngineError::Protocol(format!("bad `{method}` result: {e}"))
                })
            }
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.inbox).waiters.remove(&id);
                Err(EngineError::Timeout {
                    method: method.to_owned(),
                })
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(EngineError::Protocol(CLOSED_STDOUT.into()))
            }
        }
    }

    pub fn close(&mut self) {
        // Dropping the peer's stdin lets a well-behaved daemon exit on
        // EOF, which closes its stdout and ends the reader thread.
        self.input = None;
    }
}

/// Recovers a poisoned lock instead of propagating the panic: a panic in
/// one call must not take the whole engine's RPC client down with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn remember_stray(stray: &Mutex<VecDeque<String>>, line: &str) {
    let text = line.trim();
    if text.is_empty() {
        return;
    }
    let mut buf = lock(stray);
    if buf.len() == STRAY_LINES {
        buf.pop_front();
    }
    buf.push_back(text.to_owned());
}

/// Owns the peer's line stream for the life of the client. Classifies
/// each line and, when the stream ends, marks the inbox closed and drops
/// every waiter's channel so each blocked call wakes with the
/// closed-stdout error.
fn run_reader(
    lines: &Receiver<String>,
    inbox: &Mutex<Inbox>,
    stray: &Mutex<VecDeque<String>>,
    subscribers: &Mutex<Vec<Sender<Notification>>>,
) {
    while let Ok(line) = lines.recv() {
        if let Ok(response) = serde_json::from_str::<Response>(&line) {
            // Removing the waiter releases the lock before the send, and
            // a response with no waiter is one a call already gave up on.
            let waiter = lock(inbox).waiters.remove(&response.id);
            if let Some(tx) = waiter {
                let _ = tx.send(response);
            }
        } else if let Ok(notification) =
            serde_json::from_str::<Notification>(&line)
        {
            // Fan out, dropping any subscriber whose receiver is gone.
            lock(subscribers)
                .retain(|tx| tx.send(notification.clone()).is_ok());
        } else {
            remember_stray(stray, &line);
        }
    }
    let mut inbox = lock(inbox);
    inbox.closed = true;
    inbox.waiters.clear();
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, Write},
        time::{Duration, Instant},
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

    /// A notification the peer sends while a call is in flight reaches a
    /// subscriber: the reader fans it out instead of the call swallowing
    /// it, which is what lets the app watch the daemon's event stream.
    #[test]
    fn a_notification_reaches_a_subscriber_during_a_call() {
        if std::env::var_os("WILLIE_PEER_MODE").is_some() {
            // Peer: on the first request, emit a notification, then reply.
            announce_ready();
            let stdin = std::io::stdin();
            let mut out = std::io::stdout();
            for line in stdin.lock().lines().map_while(Result::ok) {
                let req: Request = serde_json::from_str(&line).unwrap();
                let notice = serde_json::json!({
                    "jsonrpc": "2.0", "method": "state.event",
                    "params": {"seq": 1}
                });
                writeln!(out, "{}", serde_json::to_string(&notice).unwrap())
                    .unwrap();
                let resp = Response::ok(
                    req.id,
                    serde_json::json!({"echo": req.method}),
                )
                .unwrap();
                writeln!(out, "{}", serde_json::to_string(&resp).unwrap())
                    .unwrap();
                out.flush().unwrap();
            }
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::a_notification_reaches_a_subscriber_during_a_call",
            "WILLIE_PEER_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_secs(5));
        let events = client.subscribe();
        let _: serde_json::Value = client.call("daemon.health", ()).unwrap();
        let ev = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(ev.method, "state.event");
        client.close();
        child.wait().unwrap();
    }

    /// Answers every request, but chatters ten non-envelope lines first:
    /// what `wsl.exe` does when it writes its own message to the same
    /// stdout the daemon speaks on.
    fn noisy_peer_main() {
        announce_ready();
        let stdin = std::io::stdin();
        let mut out = std::io::stdout();
        for i in 0..10 {
            writeln!(out, "willied: noise {i}").unwrap();
        }
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

    /// The kept window is small on purpose: a failure report needs the
    /// last thing the peer said, not its whole history.
    #[test]
    fn stray_lines_keep_the_last_eight_non_json_lines() {
        if std::env::var_os("WILLIE_NOISY_MODE").is_some() {
            noisy_peer_main();
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::stray_lines_keep_the_last_eight_non_json_lines",
            "WILLIE_NOISY_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_secs(5));
        let echo: Echo = client.call("daemon.health", ()).unwrap();
        assert_eq!(echo.echo, "daemon.health");
        assert_eq!(
            client.stray_lines(),
            [
                "willied: noise 2",
                "willied: noise 3",
                "willied: noise 4",
                "willied: noise 5",
                "willied: noise 6",
                "willied: noise 7",
                "willied: noise 8",
                "willied: noise 9",
            ]
        );
        assert!(client.stray_text().ends_with("noise 9"));
        client.close();
        child.wait().unwrap();
    }

    /// A notification is a valid envelope: it belongs to the protocol,
    /// not to the text a failure report should quote.
    #[test]
    fn notifications_are_not_kept_as_stray_lines() {
        if std::env::var_os("WILLIE_PEER_MODE").is_some() {
            peer_main();
            return;
        }
        let mut child = spawn_peer(
            "rpc::tests::notifications_are_not_kept_as_stray_lines",
            "WILLIE_PEER_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        let mut client = RpcClient::new(transport, Duration::from_secs(5));
        let _: Echo = client.call("daemon.health", ()).unwrap();
        assert_eq!(client.stray_lines(), ["willied: not json, just noise"]);
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
