//! Integration: the real `willied --stdio` binary answers `project.add`,
//! streams the add job to `Done`, and reports the project `Ready` with a
//! `windows` remote in its ext4 workspace. Runs only inside the
//! distribution (musl), so it is a `target_os = "linux"` file exercised by
//! `just test-linux`.
#![cfg(target_os = "linux")]

mod common;

use std::{
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// Records whether a `state.event` reports its job reaching `Done`, and
/// fails the test loudly if the job failed instead.
fn note_job_done(ev: &Value, done: &mut bool) {
    let params = &ev["params"];
    if params["kind"] == "job_changed" {
        match params["job"]["state"]["state"].as_str() {
            Some("done") => *done = true,
            Some("failed") => panic!("the add job failed: {params}"),
            _ => {}
        }
    }
}

#[test]
fn add_replies_then_the_project_is_ready_with_a_windows_remote() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("projects-add");
    let src = root.join("src");
    common::init_repo(&src);
    let mut d =
        common::Daemon::start(&root.join("state"), &root.join("workspaces"));

    // project.add replies with an AddResult; events may precede the reply.
    let mut job_done = false;
    let add_id = d.send(
        "project.add",
        json!({ "windows_path": src.to_string_lossy() }),
    );
    let resp = d.wait_response(add_id, Duration::from_secs(30), |ev| {
        if common::is_state_event(ev) {
            note_job_done(ev, &mut job_done);
        }
    });
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("add errored: {resp}"));
    let project_id = result["project_id"].as_str().unwrap().to_owned();
    assert!(result["job_id"].is_string(), "add result: {result}");

    // Wait for the job's `Done` event if it has not arrived already.
    let until = Instant::now() + Duration::from_secs(30);
    while !job_done && Instant::now() < until {
        if let Some(ev) = d.recv(Duration::from_secs(5))
            && common::is_state_event(&ev)
        {
            note_job_done(&ev, &mut job_done);
        }
    }
    assert!(job_done, "the add job never reached Done");

    // state.snapshot shows exactly one project, Ready, and its workspace
    // carries the `windows` remote pointing back at the source.
    let snap_id = d.send("state.snapshot", json!({}));
    let snap = d.wait_response(snap_id, Duration::from_secs(15), |_| {});
    let projects = snap["result"]["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1, "one project expected: {snap}");
    let p = &projects[0];
    assert_eq!(p["id"].as_str(), Some(project_id.as_str()));
    assert_eq!(p["state"]["state"], "ready", "{p}");
    let workspace = p["workspace"].as_str().unwrap();
    let remotes = common::git(Path::new(workspace), &["remote"]);
    assert!(
        remotes.split_whitespace().any(|r| r == "windows"),
        "workspace remotes: {remotes}"
    );
}

#[test]
fn a_non_repo_source_is_a_coded_error_and_no_project_appears() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("projects-nonrepo");
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut d =
        common::Daemon::start(&root.join("state"), &root.join("workspaces"));

    let id = d.send(
        "project.add",
        json!({ "windows_path": plain.to_string_lossy() }),
    );
    let resp = d.wait_response(id, Duration::from_secs(15), |_| {});
    let err = resp
        .get("error")
        .unwrap_or_else(|| panic!("expected an error: {resp}"));
    assert_eq!(err["code"], "not_a_git_repository", "{err}");

    // Nothing was registered: fast validation refused before any project.
    let snap_id = d.send("state.snapshot", json!({}));
    let snap = d.wait_response(snap_id, Duration::from_secs(15), |_| {});
    assert!(
        snap["result"]["projects"].as_array().unwrap().is_empty(),
        "no project should have appeared: {snap}"
    );
}
