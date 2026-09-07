//! Integration: the daemon's local socket answers a second client while
//! the engine's stdio pipe is open, refuses daemon.shutdown, and the
//! socket file is gone once the daemon exits. Distro-only (Unix sockets,
//! the real binary).
#![cfg(target_os = "linux")]
// The helpers below are not `#[test]` fns, so they fall outside clippy's
// `allow-unwrap-in-tests`; test setup may unwrap freely.
#![allow(clippy::unwrap_used)]

mod common;

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// Connect to the daemon's socket, retrying while the daemon binds it
/// (up to ~2 s after `scan()` returns). Fails loudly if it never appears.
fn connect(socket: &Path) -> UnixStream {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            return stream;
        }
        assert!(
            Instant::now() < until,
            "the daemon never bound {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// One request/response over a socket connection: write the JSON line,
/// read exactly one reply line back, and parse it.
fn call(
    writer: &mut UnixStream,
    reader: &mut impl BufRead,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    writeln!(writer, "{line}").unwrap();
    writer.flush().unwrap();
    let mut buf = String::new();
    let n = reader.read_line(&mut buf).unwrap();
    assert!(n > 0, "the socket closed before replying to {method}");
    serde_json::from_str(&buf).unwrap()
}

/// A socket client is a second origin alongside the engine's stdio pipe:
/// it says hello and asks health and both are results with matching ids;
/// `daemon.shutdown` over the socket is refused with `method_not_served`
/// and the stdio daemon keeps answering; and once the daemon exits (stdin
/// closed) the socket file is removed.
#[test]
fn the_local_socket_serves_refuses_shutdown_and_is_removed_on_exit() {
    let root = common::scratch("willied-socket");
    let state_dir = root.join("state");
    let workspaces = root.join("ws");
    let run_dir = root.join("run");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();

    let mut daemon =
        common::Daemon::start_with(&state_dir, &workspaces, &run_dir, &home);

    let socket = willie_linux::paths::daemon_socket(&run_dir);
    let stream = connect(&socket);
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let hello_params =
        serde_json::to_value(willie_proto::daemon::Hello::for_client("willie"))
            .unwrap();
    let hello = call(&mut writer, &mut reader, 1, "daemon.hello", hello_params);
    assert_eq!(hello["id"].as_u64(), Some(1));
    assert!(
        hello.get("result").is_some(),
        "hello over the socket: {hello}"
    );

    let health = call(&mut writer, &mut reader, 2, "daemon.health", json!({}));
    assert_eq!(health["id"].as_u64(), Some(2));
    assert!(
        health.get("result").is_some(),
        "health over the socket: {health}"
    );

    // daemon.shutdown over the socket is refused, not obeyed.
    let refused =
        call(&mut writer, &mut reader, 3, "daemon.shutdown", json!({}));
    assert_eq!(
        refused["error"]["code"].as_str(),
        Some("method_not_served"),
        "shutdown over the socket should be refused: {refused}"
    );

    // The engine's stdio daemon still answers after the socket's refusal.
    let sid = daemon.send("daemon.health", json!({}));
    let resp = daemon.wait_response(sid, Duration::from_secs(5), |_| {});
    assert!(
        resp.get("result").is_some(),
        "stdio health after the socket shutdown: {resp}"
    );

    // A stdio `daemon.shutdown` is the engine's own graceful exit: the
    // serve loop ends and the daemon removes its socket on the way out.
    // (The fixture's `Drop` sends SIGKILL, which by design bypasses that
    // cleanup, so the graceful path is what proves the removal.)
    let sd = daemon.send("daemon.shutdown", json!({}));
    let bye = daemon.wait_response(sd, Duration::from_secs(5), |_| {});
    assert!(
        bye.get("result").is_some(),
        "stdio shutdown should be obeyed: {bye}"
    );
    let gone = common::wait_until(Duration::from_secs(5), || !socket.exists());
    assert!(
        gone,
        "the socket {} was not removed on exit",
        socket.display()
    );

    drop(daemon);
}
