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

/// `git remote add origin <url>`, or `set-url` if `origin` is already
/// configured — idempotent, so pointing an existing profile at a new
/// remote is just calling this again.
pub fn set_remote(repo: &Path, url: &str) -> Result<(), GitError> {
    let existing = run(repo, &["remote"])?;
    let verb = if existing.lines().any(|line| line.trim() == "origin") {
        "set-url"
    } else {
        "add"
    };
    run(repo, &["remote", verb, "origin", url])?;
    Ok(())
}

/// `git push -u origin HEAD` — publishes the current branch and records
/// it as the upstream, so a later [`pull`] needs no branch name.
pub fn push(repo: &Path) -> Result<(), GitError> {
    run(repo, &["push", "-u", "origin", "HEAD"])?;
    Ok(())
}

/// `git pull --ff-only`. A divergent history — the local branch is not
/// an ancestor of upstream, so no fast-forward exists — is reported with
/// the code `pull_conflict` rather than `git_failed`, so the plugin can
/// map it onto its own `profile_sync_conflict` instead of a generic
/// fault; nothing else about the repository changes; a failed pull
/// leaves it exactly where it was.
pub fn pull(repo: &Path) -> Result<(), GitError> {
    match run(repo, &["pull", "--ff-only"]) {
        Ok(_) => Ok(()),
        Err(e) if is_pull_conflict(&e.message) => {
            Err(GitError::new("pull_conflict", e.message))
        }
        Err(e) => Err(e),
    }
}

/// Whether a failed `pull --ff-only`'s stderr tail describes a
/// divergent-history failure rather than some other git fault (an
/// unreachable remote, no `origin` configured, …). Git's own wording for
/// this case has stayed stable across versions.
fn is_pull_conflict(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("not possible to fast-forward")
        || lower.contains("non-fast-forward")
        || lower.contains("would be overwritten by merge")
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

    // --------------------------------------------------- remote sync

    /// A fresh bare repository, standing in for the private remote a
    /// profile is pushed to and pulled from — a real `git` transport
    /// (a filesystem path), not a fake.
    fn bare_remote(name: &str) -> std::path::PathBuf {
        let dir = scratch(&format!("remote-{name}"));
        fs::create_dir_all(&dir).unwrap();
        // `-b main` matches `init`'s own default branch: a bare repo's
        // `HEAD` otherwise follows whatever `init.defaultBranch` this
        // machine's git config happens to carry, which a clone's
        // checkout follows — if that default is not `main`, cloning
        // sees an empty working tree even though `main` itself has
        // every commit.
        run(&dir, &["init", "-q", "--bare", "-b", "main"]).unwrap();
        dir
    }

    fn remote_url(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// `git clone <remote> <dest>` plus a fixed local identity, matching
    /// [`init`] — a stand-in for the same profile checked out on the
    /// user's other machine.
    fn clone_repo(remote: &Path, dest: &Path) {
        let output = Command::new("git")
            .arg("clone")
            .arg("-q")
            .arg(remote)
            .arg(dest)
            .output()
            .unwrap();
        assert!(output.status.success());
        run(dest, &["config", "user.name", "Willie"]).unwrap();
        run(dest, &["config", "user.email", "willie@localhost"]).unwrap();
    }

    #[test]
    fn set_remote_adds_then_updates_origin() {
        let repo = scratch("set-remote");
        init(&repo).unwrap();

        set_remote(&repo, "https://example.invalid/first.git").unwrap();
        let listed = run(&repo, &["remote", "-v"]).unwrap();
        assert!(listed.contains("first.git"), "{listed}");

        // A second call with a different URL updates it in place rather
        // than failing on "remote origin already exists".
        set_remote(&repo, "https://example.invalid/second.git").unwrap();
        let listed = run(&repo, &["remote", "-v"]).unwrap();
        assert!(listed.contains("second.git"), "{listed}");
        assert!(!listed.contains("first.git"), "{listed}");

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn set_remote_then_push_publishes_and_a_clone_sees_it() {
        let repo = scratch("push");
        let remote = bare_remote("push");
        init(&repo).unwrap();
        fs::write(repo.join("f.txt"), "hi").unwrap();
        commit_all(&repo, "first").unwrap();

        set_remote(&repo, &remote_url(&remote)).unwrap();
        push(&repo).unwrap();

        let clone = scratch("push-clone");
        clone_repo(&remote, &clone);
        assert!(clone.join("f.txt").is_file());

        let _ = fs::remove_dir_all(&repo);
        let _ = fs::remove_dir_all(&remote);
        let _ = fs::remove_dir_all(&clone);
    }

    #[test]
    fn pull_fast_forwards_from_a_clone_that_pushed_ahead() {
        let a = scratch("pull-a");
        let remote = bare_remote("pull");
        init(&a).unwrap();
        fs::write(a.join("f.txt"), "hi").unwrap();
        commit_all(&a, "first").unwrap();
        set_remote(&a, &remote_url(&remote)).unwrap();
        push(&a).unwrap();

        // A second machine clones, adds a commit, and pushes ahead.
        let b = scratch("pull-b");
        clone_repo(&remote, &b);
        fs::write(b.join("g.txt"), "bye").unwrap();
        commit_all(&b, "second").unwrap();
        push(&b).unwrap();

        // `a` fast-forwards to see it.
        pull(&a).unwrap();
        assert!(a.join("g.txt").is_file());

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        let _ = fs::remove_dir_all(&remote);
    }

    #[test]
    fn pull_reports_pull_conflict_on_divergent_history() {
        let a = scratch("conflict-a");
        let remote = bare_remote("conflict");
        init(&a).unwrap();
        fs::write(a.join("f.txt"), "hi").unwrap();
        commit_all(&a, "first").unwrap();
        set_remote(&a, &remote_url(&remote)).unwrap();
        push(&a).unwrap();

        // `b` clones, commits, and pushes ahead of `a`.
        let b = scratch("conflict-b");
        clone_repo(&remote, &b);
        fs::write(b.join("g.txt"), "from-b").unwrap();
        commit_all(&b, "from-b").unwrap();
        push(&b).unwrap();

        // `a` also commits locally, without pulling first: its history
        // now diverges from the pushed-ahead `origin/main`.
        fs::write(a.join("h.txt"), "from-a").unwrap();
        commit_all(&a, "from-a").unwrap();

        let err = pull(&a).unwrap_err();
        assert_eq!(err.code, "pull_conflict");

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
        let _ = fs::remove_dir_all(&remote);
    }
}
