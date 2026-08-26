//! Development tasks, invoked as `cargo xtask <command>`.
//!
//! Anything the `justfile` needs beyond a single native command lives here
//! so it is written once, in Rust, and behaves the same on every host.

mod distro;
mod doctor;
mod linux;
mod refs;
mod test_linux;

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
        Some("test-linux") => test_linux::run(&root, &args[1..]),
        Some("distro") => distro::run(&root, &args[1..]),
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

  doctor-tools                  verify the local toolchain, print hints
  check-refs [--list F]         fail if content matches the denylist
  build-linux [--debug]         cross-compile willied, willie-sess and
                                willie for x86_64-unknown-linux-musl
  test-linux                    build the Linux crates' test binaries
                                and run them inside WILLIE_TEST_DISTRO
  distro pin|fetch|build|clean|install|uninstall
                                manage the distribution image
";

/// Root of the workspace: the parent of this crate's manifest directory.
/// Cargo passes `CARGO_MANIFEST_DIR` to the binary under `cargo run`, so
/// a moved checkout is found without a rebuild; the compiled-in value is
/// only the fallback for a binary started by hand.
fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_holds_the_workspace_manifest() {
        let manifest = workspace_root().join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("[workspace]"), "{}", manifest.display());
    }
}
