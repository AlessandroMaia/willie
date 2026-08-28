//! Integration: `tool.install` runs a harness installer as a job and
//! re-detection then makes `daemon.doctor` report it installed. Linux
//! only -- it drives the real `willied --stdio` binary.
#![cfg(target_os = "linux")]
// Test setup may unwrap freely; see `crates/willied/tests/sessions.rs`.
#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use serde_json::json;

#[test]
fn tool_install_runs_the_installer_and_marks_the_harness_present() {
    let root = common::scratch("tool-install");
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".local/bin")).unwrap();
    // The fake installer writes a fake `claude` into the session PATH,
    // standing in for the real `curl -fsSL https://claude.ai/install.sh
    // | bash` (`willie_harness::ClaudeCode::installer`).
    let installer = format!(
        "mkdir -p {home}/.local/bin && printf '#!/bin/sh\\necho \
         5.0.0\\n' > {home}/.local/bin/claude && chmod +x \
         {home}/.local/bin/claude",
        home = home.display()
    );
    let mut d = common::Daemon::start_with_env(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
        &[("WILLIE_HARNESS_INSTALLER", &installer)],
    );
    let mut done = false;
    let install_id =
        d.send("tool.install", json!({ "harness": "claude-code" }));
    let resp = d.wait_response(install_id, Duration::from_secs(30), |ev| {
        if ev["params"]["kind"] == "job_changed"
            && ev["params"]["job"]["state"]["state"] == "done"
        {
            done = true;
        }
    });
    assert!(resp["result"]["job_id"].is_string(), "{resp}");
    assert!(common::wait_until(Duration::from_secs(30), || done || {
        // Once the job is done, doctor now reports the harness.
        let doctor_id = d.send("daemon.doctor", json!({}));
        let rep = d.wait_response(doctor_id, Duration::from_secs(5), |_| {});
        rep["result"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "Claude Code" && c["status"] == "ok")
    }));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tool_install_refuses_a_second_call_while_one_is_running() {
    let root = common::scratch("tool-install-busy");
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // An installer that blocks until the test lets it finish, so the
    // second `tool.install` call can be observed while the first is
    // still running.
    let marker = root.join("go");
    let installer =
        format!("while [ ! -f {} ]; do sleep 0.1; done", marker.display());
    let mut d = common::Daemon::start_with_env(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
        &[("WILLIE_HARNESS_INSTALLER", &installer)],
    );
    let first_id = d.send("tool.install", json!({ "harness": "claude-code" }));
    let first = d.wait_response(first_id, Duration::from_secs(10), |_| {});
    assert!(first["result"]["job_id"].is_string(), "{first}");
    let second_id = d.send("tool.install", json!({ "harness": "claude-code" }));
    let second = d.wait_response(second_id, Duration::from_secs(10), |_| {});
    assert_eq!(second["error"]["code"], "tool_busy", "{second}");
    std::fs::write(&marker, "").unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tool_install_refuses_when_the_harness_is_already_present() {
    let root = common::scratch("tool-install-present");
    let home = root.join("home");
    let bin_dir = home.join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("claude");
    std::fs::write(&bin, "#!/bin/sh\necho '1.0.0 (fake)'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .unwrap();
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
    );
    let install_id =
        d.send("tool.install", json!({ "harness": "claude-code" }));
    let resp = d.wait_response(install_id, Duration::from_secs(10), |_| {});
    assert_eq!(resp["error"]["code"], "harness_already_installed", "{resp}");
    let _ = std::fs::remove_dir_all(&root);
}
