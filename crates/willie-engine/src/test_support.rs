//! Test-only helpers shared by tests that drive a peer process: spawning
//! this test binary again as a scripted peer, and a readiness handshake
//! so the parent never races the test harness's own startup banner.

use std::{
    io::Write,
    process::{Child, Command, Stdio},
    time::Duration,
};

use crate::process::LineTransport;

/// Line the peer sends once its stdout carries only scripted output.
pub(crate) const READY: &str = "@@willie-ready@@";

/// Re-runs this test binary with `--exact qualified_test_name`; setting
/// `mode_env=1` tells the re-entrant test body to act as the peer.
pub(crate) fn spawn_peer(qualified_test_name: &str, mode_env: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", qualified_test_name])
        .env(mode_env, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// Like [`spawn_peer`] but pipes stderr too, for tests that assert on
/// what the peer wrote there instead of discarding it.
pub(crate) fn spawn_peer_with_stderr(
    qualified_test_name: &str,
    mode_env: &str,
) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", qualified_test_name])
        .env(mode_env, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The message `wsl.exe` writes when the distribution is missing, in the
/// shape the engine has to survive: its own text, not the daemon's.
pub(crate) const DISTRO_NOT_FOUND: &str = concat!(
    "There is no distribution with the supplied name. ",
    "Error code: Wsl/Service/WSL_E_DISTRO_NOT_FOUND"
);

/// Peer mode that imitates `wsl.exe` refusing to start the daemon: one
/// UTF-16LE line on stdout, nothing on stderr, exit code 127. Writing the
/// handle directly bypasses the harness's capture hook so the bytes reach
/// the real pipe.
pub(crate) fn write_utf16_message_and_exit() -> ! {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in format!("{DISTRO_NOT_FOUND}\r\n").encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let mut out = std::io::stdout();
    let _ = out.write_all(&bytes);
    let _ = out.flush();
    std::process::exit(127);
}

/// Called by the peer once it is ready to be driven.
pub(crate) fn announce_ready() {
    let mut out = std::io::stdout();
    writeln!(out, "{READY}").unwrap();
    out.flush().unwrap();
}

/// The harness prints its own startup banner to the real stdout before
/// this body runs, with no flag to silence it; drain lines until the
/// peer's readiness marker instead of counting them.
pub(crate) fn await_ready(transport: &mut LineTransport) {
    loop {
        match transport.recv_line(Duration::from_secs(5)).unwrap() {
            Some(line) if line == READY => return,
            Some(_) => continue,
            None => panic!("peer exited before signalling ready"),
        }
    }
}
