//! S1: the stdio transport through `wsl.exe --exec`, measured against a
//! registered distribution.
//!
//! Every test that needs a distribution skips — printing one reason line
//! and passing — when `WILLIE_TEST_DISTRO` is unset or names a
//! distribution that is not registered. Point it at a scratch
//! distribution to measure:
//!
//! ```text
//! $env:WILLIE_TEST_DISTRO = "willie-spike"
//! cargo test -p willie-engine --locked --test wsl_stdio -- --nocapture
//! ```

#![cfg(windows)]

use std::time::{Duration, Instant};

use willie_engine::{
    process::WslProcess,
    wsl::{WslCli, WslExec},
};

/// The distribution to measure against, or `None` after printing the one
/// reason the measurement is being skipped.
fn test_distro() -> Option<String> {
    let Ok(name) = std::env::var("WILLIE_TEST_DISTRO") else {
        eprintln!("skip: set WILLIE_TEST_DISTRO to a registered distribution");
        return None;
    };
    match WslCli.list() {
        Ok(list) if list.iter().any(|d| d.eq_ignore_ascii_case(&name)) => {
            Some(name)
        }
        Ok(_) => {
            eprintln!("skip: distribution `{name}` is not registered");
            None
        }
        Err(err) => {
            eprintln!("skip: cannot list distributions: {err}");
            None
        }
    }
}

#[test]
fn round_trip_latency_through_cat_is_under_50ms_median() {
    let Some(distro) = test_distro() else {
        return;
    };
    let mut proc =
        WslProcess::spawn(&WslExec::new(&distro, "/bin/cat")).unwrap();
    let mut transport = proc.transport().unwrap();
    let mut samples = Vec::new();
    for i in 0..100 {
        let line = format!("{{\"id\":{i},\"text\":\"Versão ✓ ✔\"}}");
        let started = Instant::now();
        transport.send_line(&line).unwrap();
        let echoed = transport
            .recv_line(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        samples.push(started.elapsed());
        assert_eq!(echoed, line, "bytes must pass through untouched");
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let max = samples[samples.len() - 1];
    eprintln!("S1 latency: median {median:?}, max {max:?}");
    assert!(
        median < Duration::from_millis(50),
        "median {median:?} too slow"
    );
    transport.close_input();
    assert_eq!(proc.wait().unwrap(), 0, "cat exits 0 on EOF");
}

#[test]
fn wsl_errors_decode_to_readable_text() {
    if WslCli.version().is_err() {
        eprintln!("skip: wsl.exe not available");
        return;
    }
    let err = WslCli.terminate("willie-does-not-exist-xyz").unwrap_err();
    let text = err.to_string();
    assert!(
        !text.contains('\u{fffd}'),
        "UTF-16 output was mis-decoded: {text}"
    );
    assert!(
        text.chars().filter(|c| c.is_alphabetic()).count() > 10,
        "stderr looks empty: {text}"
    );
}

#[test]
fn child_stdout_is_raw_utf8_not_utf16() {
    let Some(distro) = test_distro() else {
        return;
    };
    let exec = WslExec::new(&distro, "/bin/echo").arg("olá ✓");
    let mut proc = WslProcess::spawn(&exec).unwrap();
    let mut transport = proc.transport().unwrap();
    let first = transport.recv_line(Duration::from_secs(5)).unwrap();
    assert_eq!(first.as_deref(), Some("olá ✓"));
}
