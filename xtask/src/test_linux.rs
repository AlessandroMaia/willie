//! `test-linux`: build the Linux crates' test binaries for musl and run
//! them inside the registered `willie` distribution.
//!
//! `cargo test --workspace` on the Windows host never executes a line
//! behind `cfg(target_os = "linux")`. This cross-compiles the test
//! harnesses with `cargo zigbuild test --no-run` (static musl binaries)
//! and runs each through `wsl.exe -d <distro> --exec`, straight from the
//! DrvFs `target/` directory. Opt-in through `WILLIE_TEST_DISTRO` so the
//! default gate on a machine without the distribution stays green.

use std::{env, path::Path, process::Command};

use crate::{TaskResult, linux};

const CRATES: &[&str] =
    &["willied", "willie-linux", "willie-cli", "willie-sess"];

pub fn run(root: &Path, _args: &[String]) -> TaskResult {
    let Some(distro) = env::var_os("WILLIE_TEST_DISTRO") else {
        eprintln!(
            "skip: set WILLIE_TEST_DISTRO to a registered distribution \
             to run the Linux tests"
        );
        return Ok(());
    };
    let distro = distro.to_string_lossy().into_owned();

    let binaries = build_test_binaries(root)?;
    if binaries.is_empty() {
        return Err("no test binaries were produced".into());
    }
    let mut failures = Vec::new();
    for binary in &binaries {
        let wsl_path = to_drvfs(root, binary)?;
        eprintln!("running {wsl_path} in {distro}");
        let status = Command::new("wsl.exe")
            .args(["-d", &distro, "--exec", &wsl_path, "--test-threads=1"])
            .status()
            .map_err(|e| format!("cannot run wsl.exe: {e}"))?;
        if !status.success() {
            failures.push(wsl_path);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("failing test binaries: {}", failures.join(", ")))
    }
}

/// `cargo zigbuild test --no-run` for the musl target, parsing the JSON
/// messages for the produced test executables.
fn build_test_binaries(root: &Path) -> Result<Vec<String>, String> {
    let mut cmd = Command::new("cargo-zigbuild");
    cmd.current_dir(root).args([
        "test",
        "--locked",
        "--no-run",
        "--target",
        linux::TARGET,
        "--message-format=json",
    ]);
    for c in CRATES {
        cmd.args(["-p", c]);
    }
    if let Some(python) = linux::python_with_ziglang() {
        cmd.env("CARGO_ZIGBUILD_PYTHON_PATH", python);
    }
    let output = cmd.output().map_err(|e| {
        format!(
            "cannot run `cargo-zigbuild test`: {e} \
             (python -m pip install --user cargo-zigbuild ziglang)"
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "cargo zigbuild test --no-run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut binaries = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if msg["reason"] == "compiler-artifact"
            && msg["profile"]["test"] == true
            && let Some(exe) = msg["executable"].as_str()
        {
            binaries.push(exe.to_owned());
        }
    }
    Ok(binaries)
}

fn to_drvfs(root: &Path, binary: &str) -> Result<String, String> {
    let rel = Path::new(binary)
        .strip_prefix(root)
        .map_err(|_| format!("{binary} is not under {}", root.display()))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    let root_wsl =
        willie_core::paths::windows_to_drvfs(&root.to_string_lossy())
            .ok_or_else(|| format!("cannot map {} to DrvFs", root.display()))?;
    Ok(format!("{root_wsl}/{rel}"))
}
