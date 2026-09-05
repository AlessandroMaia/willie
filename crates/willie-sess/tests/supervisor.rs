//! Integration: the real `willie-sess` binary against a fake harness.
//! Linux only (PTY, fork); runs inside the distribution via
//! `just test-linux`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use willie_core::{
    id::{ProjectId, SessionId},
    session::{SessionEvent, SessionEventKind, SessionSpec},
};

pub fn sess_bin() -> String {
    willie_linux::paths::test_binary(env!("CARGO_BIN_EXE_willie-sess"))
}

pub fn scratch(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir()
        .join(format!("willie-sess-{name}-{}-{nanos}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A shell script that answers `--version` and otherwise runs `body`.
pub fn fake_harness(root: &Path, body: &str) -> PathBuf {
    let bin = root.join("claude");
    fs::write(
        &bin,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '1.2.3 (fake)'; exit 0; fi\n{body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Writes `spec.json` into `root/session` and returns its path.
pub fn write_spec(root: &Path, argv: &[&str], workspace: &Path) -> PathBuf {
    let dir = root.join("session");
    fs::create_dir_all(&dir).unwrap();
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned());
    env.insert("HOME".to_owned(), root.to_string_lossy().into_owned());
    env.insert("TERM".to_owned(), "xterm-256color".to_owned());
    env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    let spec = SessionSpec {
        id: SessionId::new(),
        project_id: ProjectId::new(),
        harness: "claude-code".into(),
        workspace: workspace.to_string_lossy().into_owned(),
        socket: root.join("s.sock").to_string_lossy().into_owned(),
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        env,
        created_at: "1".into(),
        willie_version: willie_core::VERSION.into(),
        resumed_from: None,
        capabilities: willie_core::sandbox::CapabilitySet::default(),
    };
    let path = dir.join("spec.json");
    fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
    path
}

pub fn read_events(spec: &Path) -> Vec<SessionEvent> {
    let path = spec.with_file_name("events.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Polls `pred` until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    pred()
}

/// Runs `willie-sess run --spec` and returns (exit code, stdout line).
pub fn launch(spec: &Path) -> (i32, String) {
    launch_with_env(spec, &[])
}

/// The same, with the test-only variables the supervisor reads
/// (`docs/TESTING.md`).
pub fn launch_with_env(spec: &Path, env: &[(&str, &str)]) -> (i32, String) {
    let mut command = Command::new(sess_bin());
    command.args(["run", "--spec", &spec.to_string_lossy()]);
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command.output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
    )
}

fn exited(events: &[SessionEvent]) -> Option<(Option<i32>, Option<i32>)> {
    events.iter().find_map(|e| match e.kind {
        SessionEventKind::Exited { code, signal } => Some((code, signal)),
        _ => None,
    })
}

#[test]
fn a_harness_that_exits_is_reported_started_then_exited() {
    let root = scratch("exit7");
    let bin = fake_harness(&root, "echo hello; exit 7");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "stdout: {line}");
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    assert!(pid > 1);
    assert!(wait_until(Duration::from_secs(5), || {
        exited(&read_events(&spec)).is_some()
    }));
    let events = read_events(&spec);
    assert_eq!(events[0].kind, SessionEventKind::Created);
    assert!(matches!(
        events[1].kind,
        SessionEventKind::SandboxApplied { .. }
    ));
    assert!(matches!(events[2].kind, SessionEventKind::Started { .. }));
    assert_eq!(exited(&events), Some((Some(7), None)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_binary_is_harness_exec_failed_before_any_start() {
    let root = scratch("nobin");
    let spec = write_spec(&root, &["/nonexistent/claude"], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail harness_exec_failed: "), "{line}");
    let events = read_events(&spec);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::Started { .. }))
    );
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        SessionEventKind::Failed { code, .. } if code == "harness_exec_failed"
    )));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_deleted_workspace_is_harness_exec_failed_naming_the_directory() {
    let root = scratch("nocwd");
    let bin = fake_harness(&root, "exit 0");
    let gone = root.join("gone");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &gone);
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail harness_exec_failed: "), "{line}");
    assert!(line.contains("gone"), "{line}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_unreadable_spec_fails_closed() {
    let root = scratch("badspec");
    let spec = root.join("spec.json");
    fs::write(&spec, "{ nope").unwrap();
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail spec_invalid: "), "{line}");
    let _ = fs::remove_dir_all(&root);
}

use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
};

use willie_linux::wire;
use willie_proto::supervisor::{CloseReason, Closed, Hello, Role, Status};

#[derive(Debug)]
pub struct TestClient {
    pub stream: UnixStream,
    decoder: wire::Decoder,
}

impl TestClient {
    pub fn connect(socket: &Path, role: Role, rows: u16, cols: u16) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut c = Self {
            stream,
            decoder: wire::Decoder::new(),
        };
        c.send(
            &wire::encode_json(wire::HELLO, &Hello { role, rows, cols })
                .unwrap(),
        );
        c
    }

    /// Connect without saying hello, to break the protocol on purpose.
    pub fn connect_silent(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            stream,
            decoder: wire::Decoder::new(),
        }
    }

    pub fn send(&mut self, frame: &[u8]) {
        self.stream.write_all(frame).unwrap();
    }

    /// Next frame, or `None` on EOF/timeout.
    pub fn next_frame(&mut self) -> Option<wire::Frame> {
        loop {
            if let Some(f) = self.decoder.pop() {
                return Some(f);
            }
            let mut buf = [0u8; 8192];
            match self.stream.read(&mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.decoder.push(&buf[..n]),
            }
        }
    }

    /// Concatenated OUTPUT until `pred` holds on the total or the stream
    /// ends; returns what was seen.
    pub fn output_until(&mut self, pred: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let mut seen = Vec::new();
        while !pred(&seen) {
            match self.next_frame() {
                Some(f) if f.kind == wire::OUTPUT => seen.extend(f.payload),
                Some(_) => {}
                None => break,
            }
        }
        seen
    }

    /// Skips frames until a CLOSED arrives.
    pub fn closed(&mut self) -> Option<Closed> {
        loop {
            let f = self.next_frame()?;
            if f.kind == wire::CLOSED {
                return wire::decode_json(&f.payload).ok();
            }
        }
    }
}

// A lifecycle helper kept beside `exited`.
fn started(events: &[SessionEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e.kind, SessionEventKind::Started { .. }))
}

fn socket_of(root: &Path) -> PathBuf {
    root.join("s.sock")
}

fn kill(pid: u32, signal: i32) {
    // SAFETY: signalling a process this test started.
    unsafe { libc::kill(pid as i32, signal) };
}

#[test]
fn a_terminal_echoes_input_replays_to_a_late_client_and_is_told_why_it_closed()
{
    let root = scratch("echo");
    let bin = fake_harness(&root, "exec cat");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "{line}");
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));

    let mut a = TestClient::connect(&sock, Role::Terminal, 24, 80);
    a.send(&wire::encode(wire::INPUT, b"hello\n"));
    let seen = a.output_until(|s| s.windows(5).any(|w| w == b"hello"));
    assert!(String::from_utf8_lossy(&seen).contains("hello"), "{seen:?}");
    a.send(&wire::encode(wire::DETACH, b""));
    drop(a);
    assert!(wait_until(Duration::from_secs(5), || {
        read_events(&spec)
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::Detached { .. }))
    }));

    // A late client sees the ring first.
    let mut b = TestClient::connect(&sock, Role::Terminal, 24, 80);
    let seen = b.output_until(|s| s.windows(5).any(|w| w == b"hello"));
    assert!(String::from_utf8_lossy(&seen).contains("hello"));

    kill(pid, libc::SIGTERM);
    let closed = b.closed().expect("a CLOSED frame");
    assert_eq!(closed.reason, CloseReason::Exited);
    assert_eq!(closed.signal, Some(libc::SIGTERM));
    assert!(wait_until(Duration::from_secs(5), || !sock.exists()));
    let events = read_events(&spec);
    assert!(events.iter().any(|e| matches!(
        e.kind,
        SessionEventKind::Exited {
            signal: Some(15),
            ..
        }
    )));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_client_whose_first_frame_is_not_hello_is_closed_with_protocol() {
    let root = scratch("protocol");
    let bin = fake_harness(&root, "sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut c = TestClient::connect_silent(&sock);
    c.send(&wire::encode(wire::INPUT, b"x"));
    let closed = c.closed().expect("a CLOSED frame");
    assert_eq!(closed.reason, CloseReason::Protocol);
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn two_terminals_share_the_session_and_the_last_resize_wins() {
    let root = scratch("two");
    let bin = fake_harness(&root, "exec cat");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut a = TestClient::connect(&sock, Role::Terminal, 24, 80);
    let mut b = TestClient::connect(&sock, Role::Terminal, 30, 100);
    a.send(&wire::encode(wire::INPUT, b"ping\n"));
    let seen = b.output_until(|s| s.windows(4).any(|w| w == b"ping"));
    assert!(String::from_utf8_lossy(&seen).contains("ping"));
    b.send(&wire::encode_resize(40, 120));
    assert!(wait_until(Duration::from_secs(5), || {
        read_events(&spec).iter().rev().find_map(|e| match e.kind {
            SessionEventKind::Resized { rows, cols } => Some((rows, cols)),
            _ => None,
        }) == Some((40, 120))
    }));
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_client_that_never_reads_is_dropped_as_too_slow_and_the_session_survives() {
    let root = scratch("slow");
    let bin = fake_harness(&root, "exec yes");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let slow = TestClient::connect(&sock, Role::Terminal, 24, 80);
    // Never read from `slow`: once its 1 MiB queue overflows under the
    // flood from `yes`, the supervisor drops it and logs a `detached`.
    // Detection reads the event log, not the socket — a peer's shutdown
    // does not purge our receive queue, so a slow socket read could never
    // drain a full backlog to observe EOF inside the window.
    let dropped = wait_until(Duration::from_secs(20), || {
        read_events(&spec)
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::Detached { .. }))
    });
    assert!(dropped, "the slow client was never dropped");
    let _ = slow.stream.shutdown(Shutdown::Both);
    let mut fresh = TestClient::connect(&sock, Role::Terminal, 24, 80);
    let seen = fresh.output_until(|s| s.len() > 10);
    assert!(seen.len() > 10, "the session died with the slow client");
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_full_screen_program_gets_a_size_nudge_instead_of_a_replay() {
    let root = scratch("alt");
    let bin = fake_harness(&root, "printf '\\033[?1049hSCREEN'; sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    std::thread::sleep(Duration::from_millis(500));
    let mut c = TestClient::connect(&sock, Role::Terminal, 24, 80);
    // Nothing is replayed; the nudge is visible in the event log.
    assert!(wait_until(Duration::from_secs(5), || {
        let sizes: Vec<(u16, u16)> = read_events(&spec)
            .iter()
            .filter_map(|e| match e.kind {
                SessionEventKind::Resized { rows, cols } => Some((rows, cols)),
                _ => None,
            })
            .collect();
        sizes.ends_with(&[(24, 79), (24, 80)])
    }));
    c.stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let seen = c.output_until(|s| s.windows(6).any(|w| w == b"SCREEN"));
    assert!(
        !String::from_utf8_lossy(&seen).contains("SCREEN"),
        "replayed: {seen:?}"
    );
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_control_client_gets_status_and_live_events() {
    let root = scratch("control");
    let bin = fake_harness(&root, "sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut ctl = TestClient::connect(&sock, Role::Control, 0, 0);
    ctl.send(&wire::encode(wire::STATUS_REQ, b""));
    let f = ctl.next_frame().unwrap();
    assert_eq!(f.kind, wire::STATUS);
    let status: Status = wire::decode_json(&f.payload).unwrap();
    assert_eq!(status.pid, pid);
    assert_eq!(status.state, "running");
    assert_eq!(status.clients, 0);
    let _term = TestClient::connect(&sock, Role::Terminal, 24, 80);
    let attached = loop {
        let f = ctl.next_frame().expect("an EVENT frame");
        if f.kind == wire::EVENT {
            let ev: SessionEvent = wire::decode_json(&f.payload).unwrap();
            if matches!(ev.kind, SessionEventKind::Attached { .. }) {
                break true;
            }
        }
    };
    assert!(attached);
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}

/// The parent of `pid`, from `/proc/<pid>/stat` (field 4).
fn parent_of(pid: u32) -> u32 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after = stat.rsplit(')').next().unwrap();
    after.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn launch_with_grace(spec: &Path, grace_ms: u32) -> (i32, String) {
    let grace = grace_ms.to_string();
    launch_with_env(spec, &[("WILLIE_SESS_STOP_GRACE_MS", &grace)])
}

#[test]
fn a_stop_from_the_daemon_climbs_the_ladder_to_sigkill_and_reports_stopped() {
    let root = scratch("ladder");
    let bin = fake_harness(&root, "trap '' INT TERM; sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch_with_grace(&spec, 200);
    assert_eq!(code, 0, "{line}");
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut term = TestClient::connect(&sock, Role::Terminal, 24, 80);
    let mut ctl = TestClient::connect(&sock, Role::Control, 0, 0);
    ctl.send(&wire::encode(wire::STOP, b""));
    let closed = term.closed().expect("a CLOSED frame");
    assert_eq!(closed.reason, CloseReason::Stopped);
    assert_eq!(closed.signal, Some(libc::SIGKILL));
    let events = read_events(&spec);
    assert!(events.iter().any(|e| matches!(
        &e.kind, SessionEventKind::StopRequested { by } if by == "daemon"
    )));
    assert!(events.iter().any(|e| matches!(
        e.kind,
        SessionEventKind::Exited {
            signal: Some(9),
            ..
        }
    )));
    assert!(wait_until(Duration::from_secs(5), || !sock.exists()));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_stop_is_visible_as_stopping_and_sigterm_ends_a_harness_that_ignores_sigint()
 {
    let root = scratch("stopping");
    let bin = fake_harness(&root, "trap '' INT; sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, _) = launch_with_grace(&spec, 1500);
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut ctl = TestClient::connect(&sock, Role::Control, 0, 0);
    ctl.send(&wire::encode(wire::STOP, b""));
    std::thread::sleep(Duration::from_millis(200));
    ctl.send(&wire::encode(wire::STATUS_REQ, b""));
    let state = loop {
        let f = ctl.next_frame().expect("a STATUS frame");
        if f.kind == wire::STATUS {
            let s: Status = wire::decode_json(&f.payload).unwrap();
            break s.state;
        }
    };
    assert_eq!(state, "stopping");
    assert!(wait_until(Duration::from_secs(5), || {
        read_events(&spec).iter().any(|e| {
            matches!(
                e.kind,
                SessionEventKind::Exited {
                    signal: Some(15),
                    ..
                }
            )
        })
    }));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sigterm_to_the_supervisor_is_a_clean_shutdown() {
    let root = scratch("sigterm");
    // `exec`, so the harness process is the sleep itself. The ladder
    // signals the harness and nothing else, so a shell left in front of
    // it would take the signal while its child slept on inside the
    // namespace, and the session would not end on this rung at all.
    // What this test proves is the shutdown, not which process was
    // signalled: the recorded signal is the same either way, since the
    // helper maps a signal death to an exit code and back. The rung the
    // ladder reaches is settled by the two tests above.
    let bin = fake_harness(&root, "exec sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (_, line) = launch(&spec);
    let harness: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    let sock = socket_of(&root);
    assert!(wait_until(Duration::from_secs(5), || sock.exists()));
    let mut term = TestClient::connect(&sock, Role::Terminal, 24, 80);
    kill(parent_of(harness), libc::SIGTERM);
    let closed = term.closed().expect("a CLOSED frame");
    assert_eq!(closed.reason, CloseReason::Shutdown);
    assert_eq!(closed.signal, Some(libc::SIGINT));
    assert!(wait_until(Duration::from_secs(5), || !sock.exists()));
    let events = read_events(&spec);
    assert!(events.iter().any(|e| matches!(
        &e.kind, SessionEventKind::StopRequested { by } if by == "signal"
    )));
    assert!(events.iter().any(|e| matches!(
        e.kind,
        SessionEventKind::Exited {
            signal: Some(2),
            ..
        }
    )));
    let _ = fs::remove_dir_all(&root);
}

/// The boundary, observed from inside: the harness writes what it can
/// see into the workspace, the one place both sides share.
#[test]
fn a_session_runs_confined_and_sees_neither_the_real_home_nor_windows() {
    let root = scratch("confined");
    fs::write(root.join("secret"), b"outside").unwrap();
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let bin = fake_harness(
        &root,
        "{ cat \"$HOME/secret\" 2>/dev/null && echo home-visible || echo home-private; \
         [ -e /run/WSL ] && echo interop-visible || echo interop-absent; \
         [ -e /mnt/c ] && echo mnt-visible || echo mnt-absent; \
         [ -n \"$WSL_INTEROP\" ] && echo env-leaked || echo env-clean; \
         cat /proc/1/comm; id -u; \
         touch /tmp/left-behind; } > \"$PWD/probe.txt\" 2>&1",
    );
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &ws);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "{line}");
    assert!(wait_until(Duration::from_secs(10), || {
        exited(&read_events(&spec)).is_some()
    }));
    let probe = fs::read_to_string(ws.join("probe.txt")).unwrap();
    assert!(probe.contains("home-private"), "{probe}");
    // The interop socket directory, not the interpreter, is what stops a
    // Windows executable: the kernel holds the interpreter open, so an
    // absent /init proves nothing (decision 0016).
    assert!(probe.contains("interop-absent"), "{probe}");
    assert!(probe.contains("mnt-absent"), "{probe}");
    assert!(probe.contains("env-clean"), "{probe}");
    assert!(probe.contains("bwrap"), "{probe}");
    assert!(!root.join("left-behind").exists());
    assert!(!PathBuf::from("/tmp/left-behind").exists());
    let _ = fs::remove_dir_all(&root);
}

/// A helper that refuses while building the namespace is the one class
/// of failure the supervisor cannot see before running it, and its only
/// channel is the session's terminal, which nothing is attached to yet.
/// Measured against a real installation this arrived as a session that
/// recorded a start and an exit with the cause nowhere: the reason had
/// to be found by rebuilding the argument vector by hand.
#[test]
fn a_helper_that_refuses_after_exec_says_why_instead_of_starting() {
    let root = scratch("helper-refuses");
    let bin = fake_harness(&root, "exit 0");
    let helper = root.join("refuser");
    fs::write(
        &helper,
        "#!/bin/sh\necho \"bwrap: Can't mount on symlink destination /x\" >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);

    let (code, line) = launch_with_env(
        &spec,
        &[("WILLIE_SESS_HELPER_BIN", &helper.to_string_lossy())],
    );

    assert_eq!(code, 1, "{line}");
    assert!(line.starts_with("fail sandbox_apply_failed: "), "{line}");
    assert!(
        line.contains("Can't mount on symlink destination"),
        "{line}"
    );
    let events = read_events(&spec);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::Started { .. })),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::SandboxApplied { .. })),
        "nothing was applied, so nothing may say it was: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            &e.kind,
            SessionEventKind::Failed { code, message }
                if code == "sandbox_apply_failed"
                    && message.contains("Can't mount on symlink destination")
        )),
        "{events:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_applied_mechanisms_are_recorded_before_the_start() {
    let root = scratch("applied");
    let bin = fake_harness(&root, "exit 0");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "{line}");
    assert!(wait_until(Duration::from_secs(5), || {
        exited(&read_events(&spec)).is_some()
    }));
    let events = read_events(&spec);
    let applied = events
        .iter()
        .position(|e| {
            matches!(&e.kind, SessionEventKind::SandboxApplied { mechanisms, .. }
            if mechanisms == &["namespaces".to_owned(), "mounts".to_owned()])
        })
        .expect("a sandbox_applied event");
    let started = events
        .iter()
        .position(|e| matches!(e.kind, SessionEventKind::Started { .. }))
        .expect("a started event");
    assert!(applied < started);
    let _ = fs::remove_dir_all(&root);
}

/// A harness that ends by a signal is recorded by that signal, as it
/// was before the helper stood between it and the supervisor.
#[test]
fn a_harness_killed_by_a_signal_inside_is_recorded_as_that_signal() {
    let root = scratch("sigexit");
    let bin = fake_harness(&root, "kill -TERM $$");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "{line}");
    assert!(wait_until(Duration::from_secs(5), || {
        exited(&read_events(&spec)).is_some()
    }));
    assert_eq!(exited(&read_events(&spec)), Some((None, Some(15))));
    let _ = fs::remove_dir_all(&root);
}

/// The helper is executed some milliseconds before it forks the harness.
/// A stop that arrives with the readiness line used to find nothing
/// behind the monitor and signal the whole group, which killed the
/// session on the first rung. The harness is now resolved before the
/// session is announced ready, so the ladder reaches it: a harness that
/// ignores `SIGINT` survives the first rung and ends on the second,
/// recorded as signal 15 rather than the monitor's own 2.
#[test]
fn a_stop_that_arrives_with_the_readiness_line_still_climbs_the_ladder() {
    let root = scratch("racystop");
    let bin = fake_harness(&root, "trap '' INT; sleep 30");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch_with_grace(&spec, 200);
    assert_eq!(code, 0, "{line}");
    let sock = socket_of(&root);
    assert!(sock.exists(), "the socket is bound before readiness");
    let mut ctl = TestClient::connect(&sock, Role::Control, 0, 0);
    ctl.send(&wire::encode(wire::STOP, b""));
    assert!(wait_until(Duration::from_secs(5), || {
        exited(&read_events(&spec)).is_some()
    }));
    assert_eq!(exited(&read_events(&spec)), Some((None, Some(15))));
    let _ = fs::remove_dir_all(&root);
}

/// The wait for the harness has a ceiling, so a helper that never forks
/// cannot hold a session open for ever. With the ceiling at zero the
/// resolution always gives up — the deadline is tested before the
/// first attempt, so a fast helper cannot make it succeed anyway: the
/// session still starts, and the supervisor says in its log that the
/// promise readiness carries — the harness is running — has just been
/// given up.
#[test]
fn a_resolution_that_gives_up_still_starts_the_session_and_says_so() {
    let root = scratch("nowait");
    let bin = fake_harness(&root, "exec cat");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) =
        launch_with_env(&spec, &[("WILLIE_SESS_HARNESS_WAIT_MS", "0")]);
    assert_eq!(code, 0, "{line}");
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    assert!(started(&read_events(&spec)));
    let log =
        fs::read_to_string(spec.with_file_name("supervisor.log")).unwrap();
    assert!(
        log.contains("no harness appeared behind the helper"),
        "{log}"
    );
    // No stop was asked for, so no rung ever ran: end the session the
    // blunt way, on the monitor the readiness line named.
    kill(pid, libc::SIGKILL);
    let _ = fs::remove_dir_all(&root);
}
