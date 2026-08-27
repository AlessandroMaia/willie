//! Integration: the real `willie-sess` binary against a fake harness.
//! Linux only (PTY, fork); runs inside the distribution via
//! `just test-linux`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use willie_core::{
    id::{ProjectId, SessionId},
    session::{SessionEvent, SessionEventKind, SessionSpec},
};

pub fn sess_bin() -> String {
    let raw = env!("CARGO_BIN_EXE_willie-sess");
    willie_core::paths::windows_to_drvfs(raw).unwrap_or_else(|| raw.to_owned())
}

pub fn scratch(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir()
        .join(format!("willie-sess-{name}-{}-{nanos}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A shell script that answers `--version` and otherwise runs `body`.
pub fn fake_harness(root: &Path, body: &str) -> PathBuf {
    let bin = root.join("claude");
    fs::write(
        &bin,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '1.2.3 (fake)'; exit 0; fi\n{body}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Writes `spec.json` into `root/session` and returns its path.
pub fn write_spec(root: &Path, argv: &[&str], workspace: &Path) -> PathBuf {
    let dir = root.join("session");
    fs::create_dir_all(&dir).unwrap();
    let mut env = BTreeMap::new();
    env.insert("PATH".to_owned(), "/usr/local/bin:/usr/bin:/bin".to_owned());
    env.insert("HOME".to_owned(), root.to_string_lossy().into_owned());
    env.insert("TERM".to_owned(), "xterm-256color".to_owned());
    env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    let spec = SessionSpec {
        id: SessionId::new(),
        project_id: ProjectId::new(),
        harness: "claude-code".into(),
        workspace: workspace.to_string_lossy().into_owned(),
        socket: root.join("s.sock").to_string_lossy().into_owned(),
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        env,
        created_at: "1".into(),
        willie_version: willie_core::VERSION.into(),
    };
    let path = dir.join("spec.json");
    fs::write(&path, serde_json::to_string(&spec).unwrap()).unwrap();
    path
}

pub fn read_events(spec: &Path) -> Vec<SessionEvent> {
    let path = spec.with_file_name("events.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Polls `pred` until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + timeout;
    while Instant::now() < until {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    pred()
}

/// Runs `willie-sess run --spec` and returns (exit code, stdout line).
pub fn launch(spec: &Path) -> (i32, String) {
    let out = Command::new(sess_bin())
        .args(["run", "--spec", &spec.to_string_lossy()])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_owned(),
    )
}

fn exited(events: &[SessionEvent]) -> Option<(Option<i32>, Option<i32>)> {
    events.iter().find_map(|e| match e.kind {
        SessionEventKind::Exited { code, signal } => Some((code, signal)),
        _ => None,
    })
}

#[test]
fn a_harness_that_exits_is_reported_started_then_exited() {
    let root = scratch("exit7");
    let bin = fake_harness(&root, "echo hello; exit 7");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 0, "stdout: {line}");
    let pid: u32 = line.strip_prefix("ok ").unwrap().parse().unwrap();
    assert!(pid > 1);
    assert!(wait_until(Duration::from_secs(5), || {
        exited(&read_events(&spec)).is_some()
    }));
    let events = read_events(&spec);
    assert_eq!(events[0].kind, SessionEventKind::Created);
    assert!(matches!(events[1].kind, SessionEventKind::Started { .. }));
    assert_eq!(exited(&events), Some((Some(7), None)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_binary_is_harness_exec_failed_before_any_start() {
    let root = scratch("nobin");
    let spec = write_spec(&root, &["/nonexistent/claude"], &root);
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail harness_exec_failed: "), "{line}");
    let events = read_events(&spec);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, SessionEventKind::Started { .. }))
    );
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        SessionEventKind::Failed { code, .. } if code == "harness_exec_failed"
    )));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_deleted_workspace_is_harness_exec_failed_naming_the_directory() {
    let root = scratch("nocwd");
    let bin = fake_harness(&root, "exit 0");
    let gone = root.join("gone");
    let spec = write_spec(&root, &[&bin.to_string_lossy()], &gone);
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail harness_exec_failed: "), "{line}");
    assert!(line.contains("gone"), "{line}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_unreadable_spec_fails_closed() {
    let root = scratch("badspec");
    let spec = root.join("spec.json");
    fs::write(&spec, "{ nope").unwrap();
    let (code, line) = launch(&spec);
    assert_eq!(code, 1);
    assert!(line.starts_with("fail spec_invalid: "), "{line}");
    let _ = fs::remove_dir_all(&root);
}
