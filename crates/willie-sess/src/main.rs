//! Per-session supervisor.
//!
//! One detached process per session: owns the PTY, builds the sandbox,
//! runs the harness, serves attach clients on a Unix socket and appends to
//! the session's event log. It deliberately outlives the daemon so open
//! terminals keep working while the app is closed. Linux only.

#![deny(clippy::unwrap_used, clippy::expect_used)]

mod cli;
mod events;
mod spec;

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;

fn version_line() -> String {
    format!("willie-sess {}", willie_core::VERSION)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: willie-sess --version");
            eprintln!("the session supervisor is not implemented yet");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!(
        "{} runs only inside the Willie Linux distribution",
        version_line()
    );
    ExitCode::from(EXIT_USAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_binary_and_version() {
        assert_eq!(
            version_line(),
            format!("willie-sess {}", willie_core::VERSION)
        );
    }
}
