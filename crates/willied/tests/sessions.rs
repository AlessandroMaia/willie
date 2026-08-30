//! Integration: the real `willied --stdio` creates, lists, stops and
//! re-adopts sessions against a fake harness. Linux only.
#![cfg(target_os = "linux")]
// The `fake_home` helper is not a `#[test]` fn, so it falls outside
// clippy's `allow-unwrap-in-tests`; test setup may unwrap freely.
#![allow(clippy::unwrap_used)]

mod common;

use std::{path::Path, time::Duration};

use serde_json::{Value, json};

/// Writes an executable fake `claude` under `home/.local/bin` and returns
/// the home path to pass as WILLIE_HOME.
fn fake_home(root: &Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let home = root.join("home");
    let bin_dir = home.join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("claude");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '1.0.0 \
             (fake)'; exit 0; fi\n{body}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .unwrap();
    home
}

/// The supervisor pid for a session: the parent of the harness process the
/// session reports, since `willie-sess` forks the harness directly under
/// itself. Parsed from `/proc/<pid>/stat`, whose `comm` field may hold
/// spaces or parentheses, so the numeric fields are read after the last
/// `)`.
fn supervisor_pid_of(harness_pid: u64) -> u64 {
    let stat =
        std::fs::read_to_string(format!("/proc/{harness_pid}/stat")).unwrap();
    let after = stat.rsplit(')').next().unwrap();
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After the comm come: state, ppid, pgrp, ...
    fields[1].parse().unwrap()
}

#[test]
fn create_starts_a_session_then_list_and_snapshot_show_it_running() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-create");
    let src = root.join("src");
    common::init_repo(&src);
    let home = fake_home(&root, "exec cat");
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
    );
    let pid = common::add_ready_project(&mut d, &src);
    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(create_id, Duration::from_secs(30), |_| {});
    let session = &resp["result"]["session"];
    assert_eq!(session["state"]["state"], "running", "{resp}");
    let sid = session["id"].as_str().unwrap().to_owned();
    // list shows it
    let list_id = d.send("session.list", json!({}));
    let list = d.wait_response(list_id, Duration::from_secs(5), |_| {});
    assert!(
        list["result"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == sid)
    );
    // stop it; the snapshot eventually shows a terminal state
    let stop_id = d.send("session.stop", json!({ "id": sid }));
    d.wait_response(stop_id, Duration::from_secs(5), |_| {});
    assert!(common::wait_until(Duration::from_secs(10), || {
        let snap = d.snapshot();
        snap["sessions"].as_array().unwrap().iter().any(|s| {
            s["id"] == sid
                && matches!(
                    s["state"]["state"].as_str(),
                    Some("exited" | "stopping")
                )
        })
    }));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn create_without_a_harness_fails_closed_with_harness_not_installed() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-noharness");
    let src = root.join("src");
    common::init_repo(&src);
    let empty = root.join("home"); // no .local/bin/claude
    std::fs::create_dir_all(&empty).unwrap();
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &empty,
    );
    let pid = common::add_ready_project(&mut d, &src);
    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(create_id, Duration::from_secs(10), |_| {});
    assert_eq!(resp["error"]["code"], "harness_not_installed", "{resp}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_restarted_daemon_readopts_a_running_session() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-readopt");
    let src = root.join("src");
    common::init_repo(&src);
    let home = fake_home(&root, "exec cat");
    let (state, ws, run) = (
        root.join("state"),
        root.join("workspaces"),
        root.join("run"),
    );
    let sid = {
        let mut d = common::Daemon::start_with(&state, &ws, &run, &home);
        let pid = common::add_ready_project(&mut d, &src);
        let create_id = d.send(
            "session.create",
            json!({ "project_id": pid,
                "git_identity": { "name": "T", "email": "t@x" } }),
        );
        let resp = d.wait_response(create_id, Duration::from_secs(30), |_| {});
        resp["result"]["session"]["id"].as_str().unwrap().to_owned()
        // d dropped: the daemon exits, the supervisor keeps running
    };
    let mut d = common::Daemon::start_with(&state, &ws, &run, &home);
    assert!(common::wait_until(Duration::from_secs(10), || {
        d.snapshot()["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == sid && s["state"]["state"] == "running")
    }));
    // clean up: stop it
    let stop_id = d.send("session.stop", json!({ "id": sid }));
    d.wait_response(stop_id, Duration::from_secs(5), |_| {});
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn remove_refuses_a_project_with_a_live_session() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-rmguard");
    let src = root.join("src");
    common::init_repo(&src);
    let home = fake_home(&root, "exec cat");
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
    );
    let pid = common::add_ready_project(&mut d, &src);
    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    d.wait_response(create_id, Duration::from_secs(30), |_| {});
    let remove_id = d.send("project.remove", json!({ "id": pid }));
    let resp = d.wait_response(remove_id, Duration::from_secs(5), |_| {});
    assert_eq!(resp["error"]["code"], "sessions_running", "{resp}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resume_continues_the_projects_last_conversation() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-resume");
    let src = root.join("src");
    common::init_repo(&src);
    // A fake harness that mimics a short finished conversation for a fresh
    // session (a brief delay before it exits, so the daemon's control
    // client has time to connect before the exit — an instant exit races
    // that connection and can leave the live `exited` event unobserved
    // until the next rescan) but stays live when launched with
    // `--continue`, so the second resume attempt below finds a live one.
    let home = fake_home(
        &root,
        "if [ \"$1\" = \"--continue\" ]; then exec cat; \
         else sleep 1; exit 0; fi",
    );
    let state_dir = root.join("state");
    let mut d = common::Daemon::start_with(
        &state_dir,
        &root.join("workspaces"),
        &root.join("run"),
        &home,
    );
    let pid = common::add_ready_project(&mut d, &src);

    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(create_id, Duration::from_secs(30), |_| {});
    let first_sid =
        resp["result"]["session"]["id"].as_str().unwrap().to_owned();
    assert!(
        common::wait_until(Duration::from_secs(10), || {
            d.snapshot()["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| {
                    s["id"] == first_sid && s["state"]["state"] == "exited"
                })
        }),
        "the first session never exited on its own"
    );

    let resume_id = d.send(
        "session.create",
        json!({ "project_id": pid, "resume": true,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(resume_id, Duration::from_secs(30), |_| {});
    let session = &resp["result"]["session"];
    assert_eq!(session["state"]["state"], "running", "{resp}");
    let resumed_sid = session["id"].as_str().unwrap().to_owned();
    assert_eq!(session["resumed_from"], first_sid, "{resp}");

    // The written spec confirms the continue-mode launch: the daemon
    // records `--continue` in argv and the lineage even though the
    // protocol's `Session` type never carries argv itself.
    let spec_text = std::fs::read_to_string(
        state_dir
            .join("sessions")
            .join(&resumed_sid)
            .join("spec.json"),
    )
    .unwrap();
    let spec: Value = serde_json::from_str(&spec_text).unwrap();
    assert!(
        spec["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "--continue"),
        "{spec}"
    );
    assert_eq!(spec["resumed_from"], first_sid, "{spec}");

    // The resumed session is now live: a second resume is refused so it
    // never double-drives the same continued conversation.
    let second_id = d.send(
        "session.create",
        json!({ "project_id": pid, "resume": true,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(second_id, Duration::from_secs(10), |_| {});
    assert_eq!(resp["error"]["code"], "session_already_live", "{resp}");

    let stop_id = d.send("session.stop", json!({ "id": resumed_sid }));
    d.wait_response(stop_id, Duration::from_secs(5), |_| {});
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_killed_supervisor_finalises_the_session_and_frees_removal() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-kill");
    let src = root.join("src");
    common::init_repo(&src);
    let home = fake_home(&root, "exec cat");
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &root.join("run"),
        &home,
    );
    let pid = common::add_ready_project(&mut d, &src);
    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid,
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(create_id, Duration::from_secs(30), |_| {});
    let session = &resp["result"]["session"];
    assert_eq!(session["state"]["state"], "running", "{resp}");
    let sid = session["id"].as_str().unwrap().to_owned();
    let harness_pid = session["pid"].as_u64().unwrap();

    // SIGKILL the supervisor (the harness's parent) so it dies without
    // running finish(): its named socket file lingers, so the daemon must
    // finalise off the control reader ending, not off the file existing.
    let supervisor = supervisor_pid_of(harness_pid);
    let killed = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("kill -9 {supervisor}"))
        .status()
        .unwrap();
    assert!(killed.success(), "could not kill supervisor {supervisor}");

    // The session must reach a terminal state within a few seconds.
    assert!(
        common::wait_until(Duration::from_secs(10), || {
            let snap = d.snapshot();
            snap["sessions"].as_array().unwrap().iter().any(|s| {
                s["id"] == sid
                    && matches!(
                        s["state"]["state"].as_str(),
                        Some("failed" | "exited")
                    )
            })
        }),
        "session never became terminal after the supervisor was killed"
    );

    // With no live session left, project.remove is no longer refused.
    let remove_id = d.send("project.remove", json!({ "id": pid }));
    let resp = d.wait_response(remove_id, Duration::from_secs(10), |_| {});
    assert!(resp.get("result").is_some(), "remove was refused: {resp}");
    let _ = std::fs::remove_dir_all(&root);
}
