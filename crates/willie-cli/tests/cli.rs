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
    Command::new(env!("CARGO_BIN_EXE_willie"))
        .args(args)
        .output()
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
