//! Shared harness for the willied integration tests: spawns the real
//! `willied --stdio` binary inside the distribution against a private,
//! hermetic state and workspaces directory, and speaks ndjson over its
//! stdio. Test helpers may unwrap freely.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

/// The Linux path of the built `willied`. Cross-compiled on Windows the
/// baked `CARGO_BIN_EXE_willied` is a `C:\...` path, so map it to the
/// DrvFs mount the distribution runs it from; a native Linux build already
/// hands us a usable path.
pub fn willied_bin() -> String {
    let raw = env!("CARGO_BIN_EXE_willied");
    willie_core::paths::windows_to_drvfs(raw).unwrap_or_else(|| raw.to_owned())
}

/// Whether `git` can be run; prints the one skip reason when it cannot.
pub fn git_available() -> bool {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => true,
        _ => {
            eprintln!("skip: git is not available");
            false
        }
    }
}

/// A unique scratch root under the system temp dir (ext4 `/tmp` inside the
/// distribution), cleared first so a rerun starts clean.
pub fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "willie-it-{name}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    root
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Initialises a git repository with one commit at `dir`.
pub fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "t@t"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("f.txt"), "hi").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "init"]);
}

/// Runs `git -C dir args`, asserting success (source setup, not the daemon).
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A running daemon child plus a background reader that turns its stdout
/// into a stream of parsed JSON lines the test consumes with a timeout.
pub struct Daemon {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
}

impl Daemon {
    pub fn start(state_dir: &Path, workspaces_dir: &Path) -> Daemon {
        let mut child = Command::new(willied_bin())
            .arg("--stdio")
            .env("WILLIE_STATE_DIR", state_dir)
            .env("WILLIE_PROJECTS_DIR", workspaces_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        Daemon {
            child,
            stdin,
            rx,
            next_id: 0,
        }
    }

    /// Sends a request, returning the id it was given.
    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        id
    }

    /// The next parsed line, or `None` on timeout / stream close.
    pub fn recv(&self, timeout: Duration) -> Option<Value> {
        match self.rx.recv_timeout(timeout) {
            Ok(v) => Some(v),
            Err(RecvTimeoutError::Timeout)
            | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Reads until the response to `id` arrives (returning it), handing
    /// every other line to `on_other` in receive order. Panics on timeout.
    pub fn wait_response(
        &self,
        id: u64,
        deadline: Duration,
        mut on_other: impl FnMut(&Value),
    ) -> Value {
        let until = Instant::now() + deadline;
        while let Some(remaining) = until.checked_duration_since(Instant::now())
        {
            let Some(v) = self.recv(remaining) else { break };
            if is_response_to(&v, id) {
                return v;
            }
            on_other(&v);
        }
        panic!("no response to id {id} within {deadline:?}");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// True when `v` is a JSON-RPC response carrying this request `id`.
pub fn is_response_to(v: &Value, id: u64) -> bool {
    v.get("id").and_then(Value::as_u64) == Some(id) && v.get("method").is_none()
}

/// True when `v` is a `state.event` notification.
pub fn is_state_event(v: &Value) -> bool {
    v.get("method").and_then(Value::as_str) == Some("state.event")
}
