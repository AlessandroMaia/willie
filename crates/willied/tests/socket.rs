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
    process::{Command, Stdio},
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

/// A second daemon started against the same run dir must fail closed —
/// refuse to steal the live socket and exit FAILURE — not hang. The
/// daemon holds several `Outbound` sender clones (`ops`, its `Runner`,
/// `session_ops`), so the failure path has to release all of them before
/// joining the writer thread; releasing only one deadlocks on the join.
/// A hang shows up here as the poll timing out rather than a coded exit.
#[test]
fn a_second_daemon_on_the_same_run_dir_fails_closed_without_hanging() {
    let root = common::scratch("willied-double");
    let state_dir = root.join("state");
    let workspaces = root.join("ws");
    let run_dir = root.join("run");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();

    // The first daemon binds the socket and keeps its stdin open.
    let first =
        common::Daemon::start_with(&state_dir, &workspaces, &run_dir, &home);
    let socket = willie_linux::paths::daemon_socket(&run_dir);
    // Retry-connect proves the socket is bound before the second starts.
    let _ = connect(&socket);

    // A second daemon against the SAME run dir hits the live socket.
    let mut second = Command::new(common::willied_bin())
        .arg("--stdio")
        .env("WILLIE_STATE_DIR", &state_dir)
        .env("WILLIE_PROJECTS_DIR", &workspaces)
        .env("WILLIE_RUN_DIR", &run_dir)
        .env("WILLIE_HOME", &home)
        .env("WILLIE_SESS_BIN", common::sess_bin())
        .env_remove("TERM")
        .env_remove("TZ")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();

    // Poll for it to exit; a deadlock on the join is a timeout, not a hang
    // that blocks the test forever.
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match second.try_wait().unwrap() {
            Some(status) => break Some(status),
            None if Instant::now() >= until => break None,
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };

    let status = match status {
        Some(status) => status,
        None => {
            let _ = second.kill();
            let _ = second.wait();
            drop(first);
            panic!(
                "the second daemon hung on the fail-closed path instead of \
                 exiting"
            );
        }
    };
    assert!(
        !status.success(),
        "the second daemon should fail closed, exited {status:?}"
    );

    drop(first);
    let _ = std::fs::remove_dir_all(&root);
}
