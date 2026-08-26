//! `build-linux`: cross-compile the Linux binaries from any host.
//!
//! Uses `cargo zigbuild` so no Linux toolchain is needed on Windows. The
//! musl target yields static binaries that do not depend on the glibc
//! version shipped in the distro image.
//!
//! `cargo zigbuild` locates zig by trying `python3 -m ziglang` and then a
//! `zig` executable on `PATH`. On Windows `python3` is frequently the
//! Store's App Execution Alias stub, which fails, while the working
//! interpreter is `python`. To keep `just build-linux` self-sufficient on
//! any machine, this task probes the interpreters itself and points the
//! tool at the one that has `ziglang` (unless the user already chose).

use std::{env, path::Path, process::Command};

use crate::TaskResult;

pub const TARGET: &str = "x86_64-unknown-linux-musl";
pub const PACKAGES: &[&str] = &["willied", "willie-sess", "willie-cli"];
pub const BINARIES: &[&str] = &["willied", "willie-sess", "willie"];

const PYTHON_CANDIDATES: &[&str] = &["python3", "python"];

pub fn build(root: &Path, args: &[String]) -> TaskResult {
    let release = !args.iter().any(|a| a == "--debug");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["zigbuild", "--locked", "--target", TARGET]);
    if release {
        cmd.arg("--release");
    }
    for package in PACKAGES {
        cmd.args(["-p", package]);
    }
    if let Some(python) = python_with_ziglang() {
        cmd.env("CARGO_ZIGBUILD_PYTHON_PATH", python);
    }
    let status = cmd.status().map_err(|e| {
        format!(
            "cannot run `cargo zigbuild`: {e} \
             (python -m pip install --user cargo-zigbuild ziglang)"
        )
    })?;
    if !status.success() {
        return Err(format!("cargo zigbuild exited with {status}"));
    }
    let profile = if release { "release" } else { "debug" };
    for binary in BINARIES {
        println!("target/{TARGET}/{profile}/{binary}");
    }
    Ok(())
}

/// The interpreter to hand to `cargo zigbuild`, or `None` when the user
/// configured one, a `zig` executable is available, or none works.
fn python_with_ziglang() -> Option<&'static str> {
    if env::var_os("CARGO_ZIGBUILD_PYTHON_PATH").is_some()
        || env::var_os("CARGO_ZIGBUILD_ZIG_PATH").is_some()
        || runs(&["zig", "version"])
    {
        return None;
    }
    let outcomes = PYTHON_CANDIDATES
        .iter()
        .map(|python| (*python, runs(&[python, "-m", "ziglang", "version"])));
    pick_python(outcomes)
}

/// First interpreter whose probe succeeded.
fn pick_python<'a>(
    outcomes: impl IntoIterator<Item = (&'a str, bool)>,
) -> Option<&'a str> {
    outcomes
        .into_iter()
        .find_map(|(python, ok)| ok.then_some(python))
}

fn runs(argv: &[&str]) -> bool {
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_package_has_a_binary_name() {
        assert_eq!(PACKAGES.len(), BINARIES.len());
    }

    #[test]
    fn a_failing_python3_stub_falls_through_to_python() {
        let picked = pick_python([("python3", false), ("python", true)]);
        assert_eq!(picked, Some("python"));
    }

    #[test]
    fn no_working_interpreter_means_no_override() {
        assert_eq!(pick_python([("python3", false), ("python", false)]), None);
    }
}
