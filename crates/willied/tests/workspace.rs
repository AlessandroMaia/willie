//! Integration: `project.tree` and `project.read_file` against a real,
//! on-disk workspace inside the distribution, including a symlink that
//! escapes it. Runs only inside the distribution (musl), so it is a
//! `target_os = "linux"` file exercised by `just test-linux`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

mod common;

use std::{path::Path, time::Duration};

use serde_json::json;

#[test]
fn tree_lists_the_root_and_read_file_refuses_a_symlink_escape() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("workspace-tree");
    let src = root.join("src");
    common::init_repo(&src);

    // A directory outside the workspace, reachable only through a
    // symlink planted inside it once the workspace exists.
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "s").unwrap();

    let mut d =
        common::Daemon::start(&root.join("state"), &root.join("workspaces"));
    let project_id = common::add_ready_project(&mut d, &src);

    let snap = d.snapshot();
    let projects = snap["projects"].as_array().unwrap();
    let p = projects
        .iter()
        .find(|p| p["id"].as_str() == Some(project_id.as_str()))
        .unwrap();
    let workspace = p["workspace"].as_str().unwrap().to_owned();

    std::os::unix::fs::symlink(&outside, Path::new(&workspace).join("escape"))
        .unwrap();

    // The root listing shows the committed file and the branch, and
    // tolerates the symlink entry like any other file.
    let tree_id = d.send("project.tree", json!({ "id": project_id }));
    let resp = d.wait_response(tree_id, Duration::from_secs(10), |_| {});
    let result = resp["result"].clone();
    let entries = result["entries"].as_array().unwrap();
    let names: Vec<&str> =
        entries.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(names.contains(&"f.txt"), "{names:?}");
    assert_eq!(result["branch"].as_str(), Some("main"), "{result}");

    // Reading through the symlink is refused: canonicalising the joined
    // path resolves it outside the workspace.
    let read_id = d.send(
        "project.read_file",
        json!({ "id": project_id, "path": "escape/secret.txt" }),
    );
    let resp = d.wait_response(read_id, Duration::from_secs(10), |_| {});
    assert_eq!(
        resp["error"]["code"].as_str(),
        Some("path_outside_workspace"),
        "{resp}"
    );

    // The committed file itself reads back through project.read_file.
    let read_ok_id = d.send(
        "project.read_file",
        json!({ "id": project_id, "path": "f.txt" }),
    );
    let resp = d.wait_response(read_ok_id, Duration::from_secs(10), |_| {});
    assert_eq!(resp["result"]["content"].as_str(), Some("hi"), "{resp}");
    assert_eq!(resp["result"]["truncated"], false, "{resp}");

    // A FIFO left in the workspace (a build script, or the agent itself)
    // must never be opened blockingly: `read_text` stats it first and
    // refuses it as `file_not_text` before any `File::open` that would
    // block forever with no writer on the other end. This has no reader
    // or writer racing it, so if the daemon ever went back to opening
    // the path directly this call — and the whole single-dispatch daemon
    // behind it — would hang rather than answer, and `wait_response`'s
    // own deadline is what turns that into a clean test failure instead
    // of an actual hang of this test process.
    let mkfifo = std::process::Command::new("mkfifo")
        .arg(Path::new(&workspace).join("notes.md"))
        .status()
        .unwrap();
    assert!(mkfifo.success(), "mkfifo failed: {mkfifo:?}");
    let fifo_id = d.send(
        "project.read_file",
        json!({ "id": project_id, "path": "notes.md" }),
    );
    let resp = d.wait_response(fifo_id, Duration::from_secs(10), |_| {});
    assert_eq!(
        resp["error"]["code"].as_str(),
        Some("file_not_text"),
        "{resp}"
    );
}
