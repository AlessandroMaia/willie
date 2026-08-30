//! Integration: the real `willie attach` against a fake supervisor that
//! speaks the frame protocol. Linux only (Unix sockets), run by
//! `just test-linux`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use willie_linux::wire;
use willie_proto::supervisor::{CloseReason, Closed, Hello, Role};

fn binary_path() -> String {
    let compiled = env!("CARGO_BIN_EXE_willie");
    willie_core::paths::windows_to_drvfs(compiled)
        .unwrap_or_else(|| compiled.to_owned())
}

fn scratch_socket(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("willie-attach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("s.sock")
}

/// Reads frames from `stream` until one of `kind` arrives.
fn read_frame(
    stream: &mut UnixStream,
    dec: &mut wire::Decoder,
    kind: u8,
) -> wire::Frame {
    let mut buf = [0u8; 4096];
    loop {
        if let Some(f) = dec.pop() {
            if f.kind == kind {
                return f;
            }
            continue;
        }
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "client closed before sending a {kind} frame");
        dec.push(&buf[..n]);
    }
}

/// A fake supervisor: accepts one client, checks its hello, sends
/// `hi\n`, waits for the input `abc`, then closes with `closed`.
fn fake_supervisor(
    socket: PathBuf,
    expect: Hello,
    closed: Closed,
) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(&socket).unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut dec = wire::Decoder::new();
        let hello = read_frame(&mut stream, &mut dec, wire::HELLO);
        let got: Hello = wire::decode_json(&hello.payload).unwrap();
        assert_eq!(got, expect);
        stream
            .write_all(&wire::encode(wire::OUTPUT, b"hi\n"))
            .unwrap();
        let input = read_frame(&mut stream, &mut dec, wire::INPUT);
        assert_eq!(input.payload, b"abc");
        stream
            .write_all(&wire::encode_json(wire::CLOSED, &closed).unwrap())
            .unwrap();
    })
}

fn run_attach(socket: &Path, closed: Closed) -> (i32, String, String) {
    let server = fake_supervisor(
        socket.to_path_buf(),
        Hello {
            role: Role::Terminal,
            rows: 24,
            cols: 80,
        },
        closed,
    );
    let mut child = Command::new(binary_path())
        .args([
            "attach",
            "--no-raw",
            "--size",
            "24x80",
            &socket.to_string_lossy(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    thread::sleep(Duration::from_millis(200));
    stdin.write_all(b"abc").unwrap();
    stdin.flush().unwrap();
    let out = child.wait_with_output().unwrap();
    server.join().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn output_reaches_stdout_and_a_non_zero_exit_is_reported_with_exit_1() {
    let socket = scratch_socket("exit7");
    let (code, stdout, stderr) = run_attach(
        &socket,
        Closed {
            reason: CloseReason::Exited,
            code: Some(7),
            signal: None,
        },
    );
    assert_eq!(stdout, "hi\n");
    assert!(stderr.contains("session exited with code 7"), "{stderr}");
    assert_eq!(code, 1);
}

#[test]
fn a_clean_exit_and_a_stop_close_the_tab_with_exit_0() {
    let socket = scratch_socket("exit0");
    let (code, _, stderr) = run_attach(
        &socket,
        Closed {
            reason: CloseReason::Exited,
            code: Some(0),
            signal: None,
        },
    );
    assert_eq!(code, 0, "{stderr}");
    let socket = scratch_socket("stopped");
    let (code, _, stderr) = run_attach(
        &socket,
        Closed {
            reason: CloseReason::Stopped,
            code: None,
            signal: Some(9),
        },
    );
    assert_eq!(code, 0);
    assert!(stderr.contains("session stopped"), "{stderr}");
}

#[test]
fn too_slow_tells_the_user_to_reattach_and_exits_1() {
    let socket = scratch_socket("slow");
    let (code, _, stderr) = run_attach(
        &socket,
        Closed {
            reason: CloseReason::TooSlow,
            code: None,
            signal: None,
        },
    );
    assert_eq!(code, 1);
    assert!(stderr.contains("reattach"), "{stderr}");
}

#[test]
fn a_missing_socket_is_exit_1_with_a_plain_message() {
    let out = Command::new(binary_path())
        .args(["attach", "--no-raw", "/nonexistent/sess_x.sock"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("is not running"), "{stderr}");
    assert!(out.stdout.is_empty());
}

#[test]
fn host_mode_forwards_input_and_resize_and_streams_output_raw() {
    use willie_proto::hostterm;
    let sock = scratch_socket("host");
    let listener = UnixListener::bind(&sock).unwrap();

    let mut child = Command::new(binary_path())
        .args(["attach", sock.to_str().unwrap(), "--host"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let (mut server, _) = listener.accept().unwrap();
    let mut dec = wire::Decoder::new();

    // hello names the terminal role.
    let hello = read_frame(&mut server, &mut dec, wire::HELLO);
    let hello: Hello = wire::decode_json(&hello.payload).unwrap();
    assert_eq!(hello.role, Role::Terminal);

    let mut stdin = child.stdin.take().unwrap();

    // an input host-frame becomes a wire INPUT frame.
    stdin.write_all(&hostterm::encode_input(b"ls\n")).unwrap();
    let f = read_frame(&mut server, &mut dec, wire::INPUT);
    assert_eq!(f.payload, b"ls\n");

    // a resize host-frame becomes a wire RESIZE frame (rows, cols).
    stdin.write_all(&hostterm::encode_resize(40, 120)).unwrap();
    let f = read_frame(&mut server, &mut dec, wire::RESIZE);
    assert_eq!(wire::decode_resize(&f.payload), Some((40, 120)));

    // a supervisor OUTPUT frame reaches the child's stdout raw.
    server
        .write_all(&wire::encode(wire::OUTPUT, b"hi there"))
        .unwrap();
    let mut out = [0u8; 8];
    child.stdout.as_mut().unwrap().read_exact(&mut out).unwrap();
    assert_eq!(&out, b"hi there");

    // EOF on stdin is a clean detach; the session keeps running. The real
    // supervisor closes the socket on `detach` with no `closed` frame
    // (`willie-sess` breaks out of its loop and shuts the stream down), so
    // mirror that here: a `LEAVING` flag missed on the host EOF path would
    // make the output thread race this shutdown and report the connection
    // lost instead of a clean detach.
    drop(stdin);
    let _ = read_frame(&mut server, &mut dec, wire::DETACH);
    server.shutdown(std::net::Shutdown::Both).unwrap();

    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("connection to the session lost"),
        "{stderr}"
    );
}
