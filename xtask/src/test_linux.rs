//! `test-linux`: build the Linux crates' test binaries for musl and run
//! them inside the registered `willie` distribution.
//!
//! `cargo test --workspace` on the Windows host never executes a line
//! behind `cfg(target_os = "linux")`. This cross-compiles the test
//! harnesses with `cargo zigbuild test --no-run` (static musl binaries),
//! **stages them inside the distribution** and runs each through
//! `wsl.exe -d <distro> --exec`. Opt-in through `WILLIE_TEST_DISTRO` so
//! the default gate on a machine without the distribution stays green.
//!
//! The staging is not tidiness. Running a binary straight from the
//! Windows mount is not reliable: measured on 2026-09-05, the identical
//! bytes of one debug test binary faulted before `main` from the mount,
//! every time, and ran correctly the moment they were copied into the
//! distribution's own filesystem. Reading from the mount is fine, which
//! is why the copy works; only executing is not. The tests that launch
//! a workspace binary of their own find it in the same staging
//! directory, named by `WILLIE_TEST_BIN_DIR`.
//!
//! `willie-plugin-profiles` carries no `cfg(target_os = "linux")` code —
//! its `cargo test --workspace` pass on the host already exercises every
//! line, `git` included, because Windows happens to have its own `git` on
//! `PATH`. It is included here anyway: its profiles are real git
//! repositories the daemon manages for real inside the distribution, and
//! this is what proves the same code against the distribution's own
//! `git`, not a Windows stand-in.
//!
//! `willie-plugin-usage` is included for the same reason: `usage.snapshot`
//! reads a harness's real session log directory (a Windows host proves
//! only that forward slashes also work as path separators there), so
//! this is what proves the plugin's reads against the distribution's own
//! filesystem.

use std::{env, path::Path, process::Command};

use crate::{TaskResult, linux};

const CRATES: &[&str] = &[
    "willied",
    "willie-linux",
    "willie-cli",
    "willie-sess",
    "willie-plugin-profiles",
    "willie-plugin-usage",
];

/// Where the binaries are copied to inside the distribution. On the
/// distribution's own filesystem, and emptied on every run so a stale
/// binary can never be the thing under test.
const STAGE: &str = "/tmp/willie-test-bins";

/// Copy every binary into the distribution, replacing whatever a
/// previous run left. Fails the whole run rather than testing a
/// binary that did not arrive.
fn stage_script(stage: &str, sources: &[String]) -> String {
    let mut script = format!("set -e\nrm -rf '{stage}'\nmkdir -p '{stage}'\n");
    for source in sources {
        script.push_str(&format!("cp '{source}' '{stage}/'\n"));
    }
    script.push_str(&format!("chmod +x '{stage}'/*\n"));
    script
}

/// Run one staged binary, telling it where its siblings are.
fn run_script(stage: &str, binary: &str) -> String {
    let name = binary.rsplit('/').next().unwrap_or(binary);
    format!(
        "WILLIE_TEST_BIN_DIR='{stage}' exec '{stage}/{name}' \
         --test-threads=1"
    )
}

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
    // The workspace binaries some tests launch travel with the test
    // binaries: they cannot be executed from the mount either, and a
    // test that spawns one must find the staged copy.
    let mut sources: Vec<String> = Vec::new();
    for binary in &binaries {
        sources.push(to_drvfs(root, binary)?);
    }
    let built = root.join("target").join(linux::TARGET).join("debug");
    for name in linux::BINARIES {
        let path = built.join(name);
        if path.is_file() {
            sources.push(to_drvfs(root, &path.to_string_lossy())?);
        }
    }

    eprintln!("staging {} binaries in {distro}:{STAGE}", sources.len());
    let staged = Command::new("wsl.exe")
        .args(["-d", &distro, "--exec", "sh", "-c"])
        .arg(stage_script(STAGE, &sources))
        .status()
        .map_err(|e| format!("cannot run wsl.exe: {e}"))?;
    if !staged.success() {
        return Err(format!(
            "cannot stage the test binaries in {distro} ({staged})"
        ));
    }

    let mut failures = Vec::new();
    for binary in &binaries {
        let wsl_path = to_drvfs(root, binary)?;
        eprintln!("running {wsl_path} in {distro}");
        let status = Command::new("wsl.exe")
            .args(["-d", &distro, "--exec", "sh", "-c"])
            .arg(run_script(STAGE, &wsl_path))
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
