//! Integration tests that spawn the real `willie` binary.
//!
//! `doctor_json_prints_a_report` only runs on Linux, so it stays inert on
//! this dev machine and starts exercising the real binary once it runs
//! inside the distribution.

use std::io;
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use willie_proto::daemon::DoctorReport;

fn run(args: &[&str]) -> io::Result<Output> {
    Command::new(binary_path()).args(args).output()
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

/// `sandbox explain` has no transport to the daemon yet (see
/// `fetch_explain`'s doc comment in `src/main.rs`), so this is the only
/// behaviour the subcommand ships with today: it fails closed with a
/// coded, remediated error on stderr and prints nothing on stdout.
#[cfg(target_os = "linux")]
#[test]
fn sandbox_explain_fails_closed_with_no_daemon_to_ask() {
    let out = run(&["sandbox", "explain", "proj_x"]).unwrap();
    assert_ne!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("daemon_unreachable"), "{stderr}");
}
