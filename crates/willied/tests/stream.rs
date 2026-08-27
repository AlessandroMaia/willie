//! Integration: with two `project.add` calls in flight, every `state.event`
//! carries a strictly increasing `seq` and every reply id matches a request
//! id. This is the observable proof of the single-writer, seq-ordered event
//! stream. Runs only inside the distribution (musl).
#![cfg(target_os = "linux")]

mod common;

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

#[test]
fn event_seqs_are_monotonic_and_replies_match_requests() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("stream");
    let src1 = root.join("src1");
    let src2 = root.join("src2");
    common::init_repo(&src1);
    common::init_repo(&src2);
    let mut d =
        common::Daemon::start(&root.join("state"), &root.join("workspaces"));

    let id1 = d.send(
        "project.add",
        json!({ "windows_path": src1.to_string_lossy() }),
    );
    let id2 = d.send(
        "project.add",
        json!({ "windows_path": src2.to_string_lossy() }),
    );

    // Collect events until both add jobs have finished (each ends with a
    // `job_changed` -> Done, the last event of its sequence).
    let mut seqs: Vec<u64> = Vec::new();
    let mut replies: HashSet<u64> = HashSet::new();
    let mut done = 0;
    let deadline = Instant::now() + Duration::from_secs(45);
    while done < 2 && Instant::now() < deadline {
        let Some(v) = d.recv(Duration::from_secs(10)) else {
            break;
        };
        if common::is_state_event(&v) {
            let seq = v["params"]["seq"].as_u64().unwrap();
            seqs.push(seq);
            if v["params"]["kind"] == "job_changed"
                && v["params"]["job"]["state"]["state"] == "done"
            {
                done += 1;
            }
        } else if let Some(rid) = v.get("id").and_then(Value::as_u64) {
            replies.insert(rid);
        }
    }

    assert_eq!(done, 2, "both add jobs should finish; seqs = {seqs:?}");
    assert!(seqs.len() >= 8, "each add emits four events; got {seqs:?}");
    for pair in seqs.windows(2) {
        assert!(
            pair[0] < pair[1],
            "event seqs must strictly increase: {seqs:?}"
        );
    }
    assert!(
        replies.contains(&id1) && replies.contains(&id2),
        "replies {replies:?} must include {id1} and {id2}"
    );
}
