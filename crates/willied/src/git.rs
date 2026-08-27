//! Thin wrapper over the `git` binary. The daemon runs inside the
//! distribution; every path here is already a Linux path.

use std::{path::Path, process::Command};

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

#[derive(Debug, Clone)]
pub struct CloneOutput {
    pub log: String,
}

#[must_use]
pub fn is_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

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

/// Reads the branch `HEAD` points at. Uses `symbolic-ref`, not
/// `rev-parse --abbrev-ref`, so it resolves even on an unborn branch (a
/// repository with zero commits still has a `HEAD` symbolic ref). A true
/// detached `HEAD` — a raw commit checked out, not a branch — makes
/// `symbolic-ref` fail; that failure is the signal for
/// `source_detached_head`.
pub fn current_branch(repo: &Path) -> Result<String, GitError> {
    match run(repo, &["symbolic-ref", "--short", "HEAD"]) {
        Ok(out) => Ok(out.trim().to_owned()),
        Err(_) => Err(GitError::new(
            "source_detached_head",
            "the checkout is on a detached HEAD; check out a branch first",
        )),
    }
}

/// Whether `HEAD` resolves to a commit. False on an unborn branch (a
/// freshly `git init`'d repository with no commit yet); never panics.
#[must_use]
pub fn has_commits(repo: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Reads the `HEAD` commit's author name and email, trimmed. The caller
/// must ensure `repo` `has_commits`; on an unborn branch this fails with
/// `git_failed`.
pub fn head_author(repo: &Path) -> Result<(String, String), GitError> {
    let name = run(repo, &["log", "-1", "--format=%an"])?;
    let email = run(repo, &["log", "-1", "--format=%ae"])?;
    Ok((name.trim().to_owned(), email.trim().to_owned()))
}

pub fn is_clean(repo: &Path) -> Result<bool, GitError> {
    Ok(run(repo, &["status", "--porcelain"])?.trim().is_empty())
}

pub fn clone(src: &Path, dst: &Path) -> Result<CloneOutput, GitError> {
    let src = src.to_string_lossy();
    let dst = dst.to_string_lossy();
    let output = Command::new("git")
        .args(["clone", "--", &src, &dst])
        .output()
        .map_err(|e| GitError::new("git_failed", e.to_string()))?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(CloneOutput { log })
    } else {
        Err(GitError::new("git_failed", log))
    }
}

#[must_use]
pub fn head_contains(repo: &Path, commit: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-git-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_repo(dir: &Path) {
        run(dir, &["init", "-b", "main"]).unwrap();
        run(dir, &["config", "user.email", "t@t"]).unwrap();
        run(dir, &["config", "user.name", "t"]).unwrap();
        fs::write(dir.join("f.txt"), "hi").unwrap();
        run(dir, &["add", "."]).unwrap();
        run(dir, &["commit", "-m", "init"]).unwrap();
    }

    #[test]
    fn clone_reports_branch_and_cleanliness() {
        let src = scratch("src");
        init_repo(&src);
        assert!(is_repo(&src));
        assert_eq!(current_branch(&src).unwrap(), "main");
        assert!(is_clean(&src).unwrap());

        let dst = scratch("dst").join("clone");
        let out = clone(&src, &dst).unwrap();
        assert!(is_repo(&dst));
        assert!(!out.log.is_empty());

        fs::write(src.join("f.txt"), "changed").unwrap();
        assert!(!is_clean(&src).unwrap());

        let _ = fs::remove_dir_all(src.parent().unwrap());
        let _ = fs::remove_dir_all(dst.parent().unwrap());
    }

    #[test]
    fn a_non_repo_directory_is_not_a_repo() {
        let dir = scratch("plain");
        assert!(!is_repo(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unborn_branch_has_no_commits_but_resolves_its_name() {
        let dir = scratch("unborn");
        run(&dir, &["init", "-b", "main"]).unwrap();
        assert!(is_repo(&dir));
        assert!(!has_commits(&dir));
        assert_eq!(current_branch(&dir).unwrap(), "main");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_commit_makes_has_commits_true() {
        let dir = scratch("committed");
        init_repo(&dir);
        assert!(has_commits(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_detached_head_is_reported_as_such() {
        let dir = scratch("detached");
        init_repo(&dir);
        let head = run(&dir, &["rev-parse", "HEAD"]).unwrap();
        run(&dir, &["checkout", head.trim()]).unwrap();
        let err = current_branch(&dir).unwrap_err();
        assert_eq!(err.code, "source_detached_head");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_author_reads_the_head_commits_author() {
        let dir = scratch("author");
        init_repo(&dir);
        let (name, email) = head_author(&dir).unwrap();
        assert_eq!(name, "t");
        assert_eq!(email, "t@t");
        let _ = fs::remove_dir_all(&dir);
    }
}
