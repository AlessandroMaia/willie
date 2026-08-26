//! Willie daemon.
//!
//! Runs inside the Willie WSL distribution as an unprivileged user and owns
//! projects, sessions, plugins and the SQLite index. Speaks the control
//! protocol over stdio (to the engine) and over a Unix socket (to local
//! clients). Only builds its real entry point on Linux; elsewhere it is a
//! stub so the workspace still type-checks.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;

fn version_line() -> String {
    format!("willied {}", willie_core::VERSION)
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
            eprintln!("usage: willied --version");
            eprintln!("the daemon itself is not implemented yet");
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
        assert_eq!(version_line(), format!("willied {}", willie_core::VERSION));
    }
}
