//! Thin wrapper over the `git` binary, for the profiles plugin's own use.
//!
//! Deliberately its own copy, not `willied::git`: a plugin must never
//! depend on the daemon (AGENTS.md's plugin layering — plugins depend on
//! `willie-plugin-api`, `willie-core` and `willie-harness`, never on the
//! daemon), so the handful of calls a profile's git repository needs are
//! duplicated here rather than shared.

use std::{path::Path, process::Command};

/// A `git` invocation that failed, or could not be run at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError {
    pub code: &'static str,
    pub message: String,
}

impl GitError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Runs `git <args>` with `repo` as `-C`, returning its stdout. A nonzero
/// exit carries stderr's last few lines as the message.
pub fn run(repo: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| GitError::new("git_failed", e.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let tail = String::from_utf8_lossy(&output.stderr);
        let tail: String = tail
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        Err(GitError::new("git_failed", tail))
    }
}

/// `git init` plus a fixed local identity. A profile's history is the
/// daemon's own bookkeeping (fragment edits), not a person's commits, so
/// it must not depend on the distribution's `~/.gitconfig` carrying a
/// resolved identity (0015 is about a project's own workspace, not this).
pub fn init(repo: &Path) -> Result<(), GitError> {
    std::fs::create_dir_all(repo)
        .map_err(|e| GitError::new("git_failed", e.to_string()))?;
    run(repo, &["init", "-q", "-b", "main"])?;
    run(repo, &["config", "user.name", "Willie"])?;
    run(repo, &["config", "user.email", "willie@localhost"])?;
    Ok(())
}

/// Stages every change under `repo` and commits it. A no-op write (the
/// content did not actually change) leaves nothing staged; that is not a
/// failure — `write_fragment` can be called again with identical content
/// without erroring on git's "nothing to commit".
pub fn commit_all(repo: &Path, message: &str) -> Result<(), GitError> {
    run(repo, &["add", "-A"])?;
    let status = run(repo, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(());
    }
    run(repo, &["commit", "-q", "-m", message])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-profiles-git-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn init_creates_the_dir_and_a_committable_repo() {
        let repo = scratch("init");
        init(&repo).unwrap();

        assert!(repo.join(".git").exists());

        fs::write(repo.join("f.txt"), "hi").unwrap();
        commit_all(&repo, "first").unwrap();

        let log = run(&repo, &["log", "--oneline"]).unwrap();
        assert!(log.contains("first"));

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn commit_all_is_a_noop_when_nothing_changed() {
        let repo = scratch("noop");
        init(&repo).unwrap();
        fs::write(repo.join("f.txt"), "hi").unwrap();
        commit_all(&repo, "first").unwrap();

        // Same content again: nothing staged, no "nothing to commit" error.
        fs::write(repo.join("f.txt"), "hi").unwrap();
        commit_all(&repo, "second").unwrap();

        let log = run(&repo, &["log", "--oneline"]).unwrap();
        assert_eq!(log.lines().count(), 1);

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_real_change_produces_a_second_commit() {
        let repo = scratch("change");
        init(&repo).unwrap();
        fs::write(repo.join("f.txt"), "hi").unwrap();
        commit_all(&repo, "first").unwrap();

        fs::write(repo.join("f.txt"), "bye").unwrap();
        commit_all(&repo, "second").unwrap();

        let log = run(&repo, &["log", "--oneline"]).unwrap();
        assert_eq!(log.lines().count(), 2);

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn run_reports_git_failed_for_an_invalid_command() {
        let repo = scratch("invalid");
        fs::create_dir_all(&repo).unwrap();
        let err = run(&repo, &["not-a-git-command"]).unwrap_err();
        assert_eq!(err.code, "git_failed");
        let _ = fs::remove_dir_all(&repo);
    }
}
