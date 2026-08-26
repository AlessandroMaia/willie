//! Willie daemon.
//!
//! Runs inside the Willie WSL distribution as an unprivileged user and owns
//! projects, sessions, plugins and the SQLite index. Speaks the control
//! protocol over stdio (to the engine). Only builds its real entry point on
//! Linux; elsewhere it is a stub so the workspace still type-checks.

#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod git;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod handlers;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod jobs;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod outbound;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod server;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod state;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod store;

use std::process::ExitCode;

const EXIT_USAGE: u8 = 2;

fn version_line() -> String {
    format!("willied {}", willie_core::VERSION)
}

fn usage() -> ExitCode {
    eprintln!("usage: willied --stdio | --version");
    ExitCode::from(EXIT_USAGE)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Some("--stdio") => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut server = server::Server::new(willie_linux::doctor::run_all);
            match server.serve(stdin.lock(), stdout.lock()) {
                Ok(reason) => {
                    eprintln!("willied: exiting ({reason:?})");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("willied: transport error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    // The server is still compiled and unit-tested here; only the entry
    // point is Linux-specific.
    let _ = server::Server::new;
    eprintln!(
        "{} runs only inside the Willie Linux distribution",
        version_line()
    );
    usage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_binary_and_version() {
        assert_eq!(version_line(), format!("willied {}", willie_core::VERSION));
    }
}
