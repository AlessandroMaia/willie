//! Integration tests that spawn the real `willie` binary.
//!
//! `doctor_json_prints_a_report` only runs on Linux, so it stays inert on
//! this dev machine and starts exercising the real binary once it runs
//! inside the distribution.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io;
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
};

#[cfg(target_os = "linux")]
use willie_core::{
    id::ProjectId,
    project::{Project, ProjectState},
    sandbox::{Capability, CapabilitySet, Explained, SandboxProfile, Source},
};
#[cfg(target_os = "linux")]
use willie_proto::{
    daemon::{DoctorReport, HelloReply, method as daemon},
    project::{ProjectList, method as project},
    rpc::{Request, Response},
    sandbox::{ExplainParams, ExplainResult, method as sandbox},
};

fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(binary_path()).args(args).output()
}

/// Like `run`, but with extra environment variables set on the child —
/// `sandbox explain`'s daemon tests point the CLI at a private, temporary
/// run dir rather than the real `/run/willie`.
#[cfg(target_os = "linux")]
fn run_env(args: &[&str], env: &[(&str, &str)]) -> io::Result<Output> {
    let mut cmd = Command::new(binary_path());
    cmd.args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output()
}

/// `CARGO_BIN_EXE_willie` is a path Cargo bakes in at compile time. When
/// the musl test binary is cross-compiled on Windows and then run inside
/// WSL (`just test-linux`), that compiled-in path is still Windows-style
/// and does not resolve from the Linux side; translate it to the DrvFs
/// mount first. The native Windows test never takes this branch, so its
/// own compiled-in Windows path is used unchanged.
#[cfg(target_os = "linux")]
fn binary_path() -> String {
    willie_linux::paths::test_binary(env!("CARGO_BIN_EXE_willie"))
}

#[cfg(not(target_os = "linux"))]
fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_willie")
}

#[cfg(not(target_os = "linux"))]
#[test]
fn the_stub_exits_with_the_usage_code() {
    let out = run(&[]).unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("usage:"));
    assert!(out.stdout.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn doctor_json_prints_a_report() {
    let out = run(&["doctor", "--json"]).unwrap();
    let code = out.status.code();
    assert!(code == Some(0) || code == Some(3));
    let report: DoctorReport = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!report.checks.is_empty());
}

/// A scratch run dir, mirroring `attach.rs`'s `scratch_socket`: a private
/// temporary directory the daemon socket (or, for the no-daemon test,
/// nothing at all) lives under, so the test never touches the real
/// `/run/willie`.
#[cfg(target_os = "linux")]
fn scratch_run_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("willie-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One request in, one reply out: reads a line, asserts its method,
/// answers with `result` as the `Response`'s result, and returns the
/// parsed request so a caller can inspect its params (`sandbox.explain`
/// carries the resolved project id). Every fake daemon below is built
/// from calls to this, one per step it answers before it stops — a
/// daemon that stops early leaves the rest of the exchange to the real
/// assertion: whatever the CLI does with no further reply.
#[cfg(target_os = "linux")]
fn answer_one<T: serde::Serialize>(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    expect_method: &str,
    result: T,
) -> Request {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let req: Request = serde_json::from_str(&line).unwrap();
    assert_eq!(req.method, expect_method);
    let resp = Response::ok(req.id, result).unwrap();
    writeln!(writer, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
    req
}

#[cfg(target_os = "linux")]
fn hello_reply() -> HelloReply {
    HelloReply {
        willie_version: willie_core::VERSION.to_owned(),
        protocol_version: willie_proto::PROTOCOL_VERSION,
        distro_image_version: None,
    }
}

#[cfg(target_os = "linux")]
fn project_list_with(id: ProjectId, slug: &str) -> ProjectList {
    ProjectList {
        projects: vec![Project {
            id,
            name: "Demo".into(),
            slug: slug.to_owned(),
            source: r"C:\github\demo".into(),
            workspace: "/home/willie/projects/demo".into(),
            branch: "main".into(),
            state: ProjectState::Ready,
            source_present: true,
            created_at: "2026-09-07T00:00:00Z".into(),
            sandbox: SandboxProfile::default(),
            sandbox_problem: None,
        }],
    }
}

#[cfg(target_os = "linux")]
fn explain_result() -> ExplainResult {
    ExplainResult {
        entries: vec![Explained {
            capability: Capability::ProjectRw,
            enabled: true,
            source: Source::Default,
        }],
        capabilities: CapabilitySet::default(),
    }
}

/// Accepts one client and answers `daemon.hello`, `project.list` and
/// `sandbox.explain` in order, each as one JSON-RPC line in, one line
/// out — mirroring the real socket's framing
/// (`crates/willied/tests/socket.rs`), not `attach.rs`'s frame protocol,
/// which is a different wire entirely. `project.list` answers with one
/// project at `slug`; the happy path calls this with `"demo"` (matching
/// the CLI argument), and the slug-mismatch test calls it with a slug
/// the CLI's argument does not match, so the exchange stops there.
#[cfg(target_os = "linux")]
fn fake_daemon(
    run_dir: &Path,
    project_id: ProjectId,
    slug: &str,
    answer_explain: bool,
) -> JoinHandle<()> {
    let socket = willie_linux::paths::daemon_socket(run_dir);
    let listener = UnixListener::bind(&socket).unwrap();
    let slug = slug.to_owned();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;

        answer_one(&mut reader, &mut writer, daemon::HELLO, hello_reply());
        answer_one(
            &mut reader,
            &mut writer,
            project::LIST,
            project_list_with(project_id, &slug),
        );
        if answer_explain {
            let req = answer_one(
                &mut reader,
                &mut writer,
                sandbox::EXPLAIN,
                explain_result(),
            );
            let params: ExplainParams =
                serde_json::from_value(req.params).unwrap();
            assert_eq!(params.project_id, project_id);
        }
    })
}

/// Accepts one client and answers only `daemon.hello` — for the
/// malformed-`proj_` case, where `resolve_project` fails before ever
/// calling `project.list`.
#[cfg(target_os = "linux")]
fn fake_daemon_hello_only(run_dir: &Path) -> JoinHandle<()> {
    let socket = willie_linux::paths::daemon_socket(run_dir);
    let listener = UnixListener::bind(&socket).unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        answer_one(&mut reader, &mut writer, daemon::HELLO, hello_reply());
    })
}

/// The happy path: a fake daemon answers hello, resolves the `demo` slug
/// through `project.list`, then answers `sandbox.explain` — the CLI
/// prints the one capability row on stdout and the heading on stderr.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_explain_prints_the_rows_from_the_daemon() {
    let run_dir = scratch_run_dir("explain-ok");
    let project_id = ProjectId::new();
    let server = fake_daemon(&run_dir, project_id, "demo", true);

    let out = run_env(
        &["sandbox", "explain", "demo"],
        &[("WILLIE_RUN_DIR", run_dir.to_str().unwrap())],
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim_end(), "project.rw\ton\tdefault");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("capability\tstate\tsource"), "{stderr}");
}

/// No socket at all (an empty run dir): the CLI fails closed with
/// `daemon_unreachable` rather than hanging or crashing.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_explain_without_a_daemon_is_unreachable() {
    let run_dir = scratch_run_dir("explain-no-daemon");

    let out = run_env(
        &["sandbox", "explain", "proj_x"],
        &[("WILLIE_RUN_DIR", run_dir.to_str().unwrap())],
    )
    .unwrap();

    assert_ne!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("daemon_unreachable"), "{stderr}");
}

/// A `proj_…`-shaped argument that isn't a valid id is `project_not_found`
/// without ever asking `project.list` — `resolve_project` rejects it
/// itself, so the fake daemon here only needs to answer hello.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_explain_with_a_malformed_proj_id_is_project_not_found() {
    let run_dir = scratch_run_dir("explain-bad-id");
    let server = fake_daemon_hello_only(&run_dir);

    let out = run_env(
        &["sandbox", "explain", "proj_not-a-ulid"],
        &[("WILLIE_RUN_DIR", run_dir.to_str().unwrap())],
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("project_not_found"), "{stderr}");
}

/// A slug that matches no project in `project.list`'s reply is
/// `project_not_found` too — `sandbox.explain` is never reached, so the
/// fake daemon here stops after answering `project.list`.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_explain_with_an_unknown_slug_is_project_not_found() {
    let run_dir = scratch_run_dir("explain-bad-slug");
    let project_id = ProjectId::new();
    let server = fake_daemon(&run_dir, project_id, "demo", false);

    let out = run_env(
        &["sandbox", "explain", "not-demo"],
        &[("WILLIE_RUN_DIR", run_dir.to_str().unwrap())],
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("project_not_found"), "{stderr}");
}
