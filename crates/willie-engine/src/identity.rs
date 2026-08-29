//! The Windows-side git identity a new session's commits use. Read from
//! the user's global git config with `git.exe`; the daemon ensures it in
//! the distribution (decision 0015) — the engine only forwards it.

use willie_proto::session::GitIdentity;

/// Both fields, trimmed and non-empty, or `None`.
#[must_use]
pub fn parse(name: Option<&str>, email: Option<&str>) -> Option<GitIdentity> {
    let name = name.map(str::trim).filter(|s| !s.is_empty())?;
    let email = email.map(str::trim).filter(|s| !s.is_empty())?;
    Some(GitIdentity {
        name: name.to_owned(),
        email: email.to_owned(),
    })
}

/// The user's global git identity, or `None` when `git.exe` is absent or
/// either field is unset. Never fails the caller.
#[must_use]
pub fn windows_git_identity() -> Option<GitIdentity> {
    parse(
        git_config("user.name").as_deref(),
        git_config("user.email").as_deref(),
    )
}

#[cfg(windows)]
fn git_config(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("git.exe")
        .args(["config", "--global", key])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(not(windows))]
fn git_config(_key: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_needs_both_a_name_and_an_email() {
        assert_eq!(
            parse(Some("A"), Some("a@x")),
            Some(GitIdentity {
                name: "A".into(),
                email: "a@x".into()
            })
        );
        assert_eq!(parse(Some("A"), None), None);
        assert_eq!(parse(None, Some("a@x")), None);
        assert_eq!(parse(Some(" "), Some("a@x")), None);
        assert_eq!(parse(Some("A"), Some("")), None);
    }
}
