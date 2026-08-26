//! `willie` — the stateless CLI used inside the distribution.
//!
//! Talks to the daemon over its Unix socket and to session supervisors over
//! theirs. Windows Terminal tabs run `willie attach <session>` through
//! `wsl.exe`. Holds no state of its own. Linux only.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;

fn version_line() -> String {
    format!("willie {}", willie_core::VERSION)
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
            eprintln!("usage: willie --version");
            eprintln!("attach and doctor are not implemented yet");
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
        assert_eq!(version_line(), format!("willie {}", willie_core::VERSION));
    }
}
