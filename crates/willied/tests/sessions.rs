//! Integration: the real `willied --stdio` creates, lists, stops and
//! re-adopts sessions against a fake harness. Linux only.
#![cfg(target_os = "linux")]
// The `fake_home` helper is not a `#[test]` fn, so it falls outside
// clippy's `allow-unwrap-in-tests`; test setup may unwrap freely.
#![allow(clippy::unwrap_used)]

mod common;

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use willie_linux::wire;
use willie_proto::supervisor::{Hello, Role};

/// A home shaped like the one the image provisions: an executable fake
/// `claude` under `home/.local/bin`, and the harness's state directory
/// with the two links into it (`distro/provision.sh`). Returns the home
/// path to pass as WILLIE_HOME.
///
/// The state directory is not decoration: `agent.state` is on by default
/// for Claude Code, and the sandbox binds it without tolerance, so a
/// home without it is a home no real session would ever find.
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
    let state = home.join(".willie/agent-state/claude");
    std::fs::create_dir_all(state.join("dot-claude")).unwrap();
    std::fs::write(state.join("claude.json"), b"{}\n").unwrap();
    std::os::unix::fs::symlink(
        ".willie/agent-state/claude/dot-claude",
        home.join(".claude"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        ".willie/agent-state/claude/claude.json",
        home.join(".claude.json"),
    )
    .unwrap();
    home
}

/// The supervisor pid for a session: the parent of the pid the session
/// reports, which is the namespace helper's monitor — `willie-sess`
/// forks the helper, and the harness is two levels below it. Parsed
/// from `/proc/<pid>/stat`, whose `comm` field may hold spaces or
/// parentheses, so the numeric fields are read after the last `)`.
fn supervisor_pid_of(monitor_pid: u64) -> u64 {
    let stat =
        std::fs::read_to_string(format!("/proc/{monitor_pid}/stat")).unwrap();
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
    let monitor_pid = session["pid"].as_u64().unwrap();

    // SIGKILL the supervisor (the monitor's parent) so it dies without
    // running finish(): its named socket file lingers, so the daemon must
    // finalise off the control reader ending, not off the file existing.
    let supervisor = supervisor_pid_of(monitor_pid);
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

/// Reads frames off a fresh terminal connection for up to `timeout`,
/// accumulating `OUTPUT` payloads (a late terminal still gets the
/// supervisor's buffered replay) until the text contains `needle`.
fn output_contains(
    stream: &mut UnixStream,
    timeout: Duration,
    needle: &str,
) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut dec = wire::Decoder::new();
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    let until = Instant::now() + timeout;
    loop {
        while let Some(f) = dec.pop() {
            if f.kind == wire::OUTPUT {
                acc.push_str(&String::from_utf8_lossy(&f.payload));
                if acc.contains(needle) {
                    return true;
                }
            }
            if f.kind == wire::CLOSED {
                return false;
            }
        }
        if Instant::now() >= until {
            return false;
        }
        match stream.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => dec.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return false,
        }
    }
}

/// A `kind: "shell"` session runs an interactive zsh under `ZDOTDIR=
/// /etc/willie/zsh`: its terminal shows Willie's own prompt
/// (`<user>@willie`), proof the image's `.zshrc` is the one that ran,
/// not zsh's own defaults. Distro-only: needs the real `/usr/bin/zsh`
/// the image installs for this feature (`just distro-build`, `just
/// distro-install`). Uses `fake_home` (not a bare directory): a shell
/// session resolves the same `agent_state` capability an agent session
/// does, so the sandbox binds the private login state the same way and
/// needs it to exist, exactly as the agent tests above already require.
#[test]
fn a_shell_session_shows_the_willie_prompt() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-shell-prompt");
    let src = root.join("src");
    common::init_repo(&src);
    let home = fake_home(&root, "exec cat");
    let run_dir = root.join("run");
    let mut d = common::Daemon::start_with(
        &root.join("state"),
        &root.join("workspaces"),
        &run_dir,
        &home,
    );
    let pid = common::add_ready_project(&mut d, &src);
    let create_id = d.send(
        "session.create",
        json!({ "project_id": pid, "kind": "shell",
            "git_identity": { "name": "T", "email": "t@x" } }),
    );
    let resp = d.wait_response(create_id, Duration::from_secs(30), |_| {});
    let session = &resp["result"]["session"];
    assert_eq!(session["state"]["state"], "running", "{resp}");
    assert_eq!(session["kind"], "shell", "{resp}");
    let sid = session["id"].as_str().unwrap().to_owned();

    let socket = willie_linux::paths::session_socket(&run_dir, &sid);
    let shows_prompt = common::wait_until(Duration::from_secs(15), || {
        let Ok(mut stream) = UnixStream::connect(&socket) else {
            return false;
        };
        let hello = Hello {
            role: Role::Terminal,
            rows: 24,
            cols: 80,
        };
        if stream
            .write_all(&wire::encode_json(wire::HELLO, &hello).unwrap())
            .is_err()
        {
            return false;
        }
        output_contains(&mut stream, Duration::from_secs(2), "@willie")
    });
    assert!(shows_prompt, "the shell prompt never showed @willie");

    let stop_id = d.send("session.stop", json!({ "id": sid }));
    d.wait_response(stop_id, Duration::from_secs(5), |_| {});
    let _ = std::fs::remove_dir_all(&root);
}

/// A real agent session, with a fake `claude` that plants its own
/// first-prompt record where Claude Code really keeps one -- under
/// `$HOME/.claude/projects/<pwd with '/' turned into '-'>`, the same
/// escaping `ClaudeCode::escape_workspace` does -- ends up titled from
/// `session.list`, proving the lazy hook against the real sandboxed
/// launch (cwd `--chdir`'d to the workspace, `agent_state` binding the
/// log directory rw), not just the unit-level plumbing.
#[test]
fn a_running_agent_session_gets_its_first_prompt_as_title() {
    if !common::git_available() {
        return;
    }
    let root = common::scratch("sess-title");
    let src = root.join("src");
    common::init_repo(&src);
    // A short delay before writing the log: the daemon records `started_at`
    // as soon as the supervisor reports the harness's pid, essentially the
    // moment this script starts, so the log's modified time must land
    // safely after that instant for `pick_log_for`'s `mtime >= started_at`
    // to accept it.
    let home = fake_home(
        &root,
        r#"sleep 1
dir="$HOME/.claude/projects/$(pwd | tr '/' '-')"
mkdir -p "$dir"
printf '%s\n' '{"type":"user","message":{"role":"user","content":"fix the flaky login test"}}' > "$dir/session.jsonl"
exec cat"#,
    );
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
    assert_eq!(
        resp["result"]["session"]["state"]["state"], "running",
        "{resp}"
    );
    let sid = resp["result"]["session"]["id"].as_str().unwrap().to_owned();

    assert!(
        common::wait_until(Duration::from_secs(15), || {
            let list_id = d.send("session.list", json!({}));
            let list = d.wait_response(list_id, Duration::from_secs(5), |_| {});
            list["result"]["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| {
                    s["id"] == sid && s["title"] == "fix the flaky login test"
                })
        }),
        "the session never picked up its first-prompt title"
    );

    let stop_id = d.send("session.stop", json!({ "id": sid }));
    d.wait_response(stop_id, Duration::from_secs(5), |_| {});
    let _ = std::fs::remove_dir_all(&root);
}
