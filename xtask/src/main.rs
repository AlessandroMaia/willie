//! Development tasks, invoked as `cargo xtask <command>`.
//!
//! Anything the `justfile` needs beyond a single native command lives here
//! so it is written once, in Rust, and behaves the same on every host.

mod doctor;
mod linux;
mod refs;

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

/// Result type for tasks: the message is printed to stderr and the task
/// exits with status 1.
pub type TaskResult = Result<(), String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    let outcome = match args.first().map(String::as_str) {
        Some("doctor-tools") => doctor::run(&root),
        Some("check-refs") => refs::run(&root, &args[1..]),
        Some("build-linux") => linux::build(&root, &args[1..]),
        Some("help") | Some("--help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n{USAGE}")),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
usage: cargo xtask <command>

  doctor-tools           verify the local toolchain, print install hints
  check-refs [--list F]  fail if versioned content matches the denylist
  build-linux [--debug]  cross-compile willied, willie-sess and willie
                         for x86_64-unknown-linux-musl
";

/// Root of the workspace: the parent of this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
