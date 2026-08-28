//! Locating and describing the harness inside the distribution. The
//! daemon detects it before creating a session and offers it to the
//! doctor; installing it is a job (see `tools`).

use std::path::{Path, PathBuf};

use willie_harness::{ClaudeCode, Harness, Installed, locate, session_path};
use willie_proto::daemon::{CheckStatus, DoctorCheck};

/// The distro user's home; overridable so tests plant a fake binary.
#[must_use]
pub fn home() -> PathBuf {
    std::env::var_os("WILLIE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/willie"))
}

/// The Claude Code harness descriptor.
// Read by session creation once it lands (Task 9); allow until then so
// the plain (non-test) binary still builds clean.
#[allow(dead_code)]
#[must_use]
pub fn claude() -> ClaudeCode {
    ClaudeCode
}

/// Detect Claude Code on the session `PATH` under `home`, honouring
/// `WILLIE_HARNESS_BIN` when it points at an explicit binary.
#[must_use]
pub fn detect_claude(home: &Path) -> Option<Installed> {
    let c = ClaudeCode;
    if let Some(explicit) = std::env::var_os("WILLIE_HARNESS_BIN") {
        return c.detect(Path::new(&explicit));
    }
    let binary = locate(&c, &session_path(home))?;
    c.detect(&binary)
}

/// The doctor line for the harness: ok with the version, or a fail whose
/// remediation points at the Dashboard's Install button.
#[must_use]
pub fn doctor_check(home: &Path) -> DoctorCheck {
    match detect_claude(home) {
        Some(found) => DoctorCheck {
            name: "Claude Code".into(),
            status: CheckStatus::Ok,
            detail: found.version,
            remediation: None,
            required: false,
        },
        None => DoctorCheck {
            name: "Claude Code".into(),
            status: CheckStatus::Fail,
            detail: "not installed".into(),
            remediation: Some(
                "install it from the Dashboard, or run the official \
                 installer inside the distribution"
                    .into(),
            ),
            required: false,
        },
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;

    #[test]
    fn detect_finds_a_binary_planted_on_the_session_path() {
        let home = std::env::temp_dir()
            .join(format!("willie-harness-mod-{}", std::process::id()));
        let bin_dir = home.join(".local/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\necho '3.2.1 (fake)'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let found = detect_claude(&home).unwrap();
        assert_eq!(found.version, "3.2.1");
        assert!(detect_claude(&PathBuf::from("/no/such/home")).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_doctor_line_is_ok_with_a_version_or_a_fail_with_a_remediation() {
        let missing = PathBuf::from("/no/such/home");
        let check = doctor_check(&missing);
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.remediation.is_some());
        assert!(!check.required);
    }
}
