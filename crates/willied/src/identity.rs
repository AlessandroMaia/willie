//! The git identity a session's commits use. Resolved once at
//! `session.create`, written to the distro user's global config, and
//! required: a session never commits as nobody.

use std::{path::Path, process::Command};

use willie_proto::session::GitIdentity;

/// Why an identity could not be ensured.
// Read by `session.create`'s error path (Task 8); allow until then so
// the plain (non-test) binary still builds clean.
#[allow(dead_code)]
#[derive(Debug)]
pub struct EnsureError {
    code: &'static str,
    message: String,
}

#[allow(dead_code)]
impl EnsureError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Ensure `<home>/.gitconfig` has a `user.name` and `user.email`, in
/// order: keep an existing one; else write the Windows identity; else the
/// source checkout's; else fail closed.
// Called from `session.create` (Task 8); allow until then.
#[allow(dead_code)]
pub fn ensure(
    home: &Path,
    windows: Option<&GitIdentity>,
    source: Option<&Path>,
) -> Result<(), EnsureError> {
    if read_identity(home).is_some() {
        return Ok(());
    }
    if let Some(id) = windows {
        return write_identity(home, &id.name, &id.email);
    }
    if let Some(src) = source
        && let (Some(name), Some(email)) =
            (git_config(src, "user.name"), git_config(src, "user.email"))
    {
        return write_identity(home, &name, &email);
    }
    Err(EnsureError {
        code: "git_identity_missing",
        message: "no git identity for the workspace".to_owned(),
    })
}

/// Read the global identity as the distro user would see it.
// Only `ensure` calls this today, and `ensure` itself is not yet called
// from production code (Task 8 wires it in); allow until then.
#[allow(dead_code)]
fn read_identity(home: &Path) -> Option<(String, String)> {
    let name = global_config(home, "user.name")?;
    let email = global_config(home, "user.email")?;
    (!name.is_empty() && !email.is_empty()).then_some((name, email))
}

#[allow(dead_code)]
fn write_identity(
    home: &Path,
    name: &str,
    email: &str,
) -> Result<(), EnsureError> {
    set_global(home, "user.name", name)?;
    set_global(home, "user.email", email)
}

#[allow(dead_code)]
fn global_config(home: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["config", "--global", key])
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[allow(dead_code)]
fn set_global(home: &Path, key: &str, value: &str) -> Result<(), EnsureError> {
    let ok = Command::new("git")
        .args(["config", "--global", key, value])
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(EnsureError {
            code: "git_identity_missing",
            message: format!("could not write git {key}"),
        })
    }
}

#[allow(dead_code)]
fn git_config(repo: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", key])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;
    use willie_proto::session::GitIdentity;

    fn scratch(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join(format!("willie-identity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn gitconfig(home: &Path) -> String {
        home.join(".gitconfig").display().to_string()
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    #[test]
    fn an_existing_gitconfig_identity_is_left_untouched() {
        let home = scratch("existing");
        run_git(
            &home,
            &["config", "--file", &gitconfig(&home), "user.name", "Keep"],
        );
        run_git(
            &home,
            &[
                "config",
                "--file",
                &gitconfig(&home),
                "user.email",
                "keep@x",
            ],
        );
        ensure(&home, None, None).unwrap();
        assert_eq!(
            read_identity(&home),
            Some(("Keep".into(), "keep@x".into()))
        );
    }

    #[test]
    fn the_windows_identity_is_written_when_the_home_has_none() {
        let home = scratch("fromparams");
        ensure(
            &home,
            Some(&GitIdentity {
                name: "A".into(),
                email: "a@x".into(),
            }),
            None,
        )
        .unwrap();
        assert_eq!(read_identity(&home), Some(("A".into(), "a@x".into())));
    }

    #[test]
    fn the_source_identity_is_the_last_resort_then_it_fails_closed() {
        let home = scratch("fromsource");
        let src = home.join("src");
        std::fs::create_dir_all(&src).unwrap();
        run_git(&src, &["init"]);
        run_git(&src, &["config", "user.name", "Src"]);
        run_git(&src, &["config", "user.email", "src@x"]);
        ensure(&home, None, Some(&src)).unwrap();
        assert_eq!(read_identity(&home), Some(("Src".into(), "src@x".into())));

        let bare = scratch("none");
        let err = ensure(&bare, None, None).unwrap_err();
        assert_eq!(err.code(), "git_identity_missing");
    }
}
