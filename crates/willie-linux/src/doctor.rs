//! `doctor`: can this distribution run Willie sessions?
//!
//! Each check is pure given its input text or a command result, so the
//! decisions are unit-tested on any host; only `run_all` touches the system.

use std::{fs, path::Path, process::Command, time::Duration};

use willie_proto::daemon::{CheckStatus, DoctorCheck, DoctorReport};

use crate::paths::{RUN_DIR, STATE_DIR};

fn check(
    name: &str,
    status: CheckStatus,
    detail: impl Into<String>,
    remediation: Option<&str>,
    required: bool,
) -> DoctorCheck {
    DoctorCheck {
        name: name.to_owned(),
        status,
        detail: detail.into(),
        remediation: remediation.map(str::to_owned),
        required,
    }
}

/// Willie must not run as root inside the distribution. `proc_status` is
/// the content of `/proc/self/status` or the reason it could not be read;
/// either way this check is required, so an undecidable answer is a
/// failure and never a skip.
#[must_use]
pub fn check_uid_text(proc_status: Result<&str, &str>) -> DoctorCheck {
    let unreadable = |reason: &str| {
        check(
            "unprivileged user",
            CheckStatus::Fail,
            format!("cannot read /proc/self/status: {reason}"),
            Some("click Install distribution to reinstall the image"),
            true,
        )
    };
    let text = match proc_status {
        Ok(text) => text,
        Err(reason) => return unreadable(reason),
    };
    let uid = text
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse::<u32>().ok());
    match uid {
        Some(0) => check(
            "unprivileged user",
            CheckStatus::Fail,
            "running as root",
            Some("start the daemon with `wsl.exe --user willie`"),
            true,
        ),
        Some(uid) => check(
            "unprivileged user",
            CheckStatus::Ok,
            format!("uid {uid}"),
            None,
            true,
        ),
        None => unreadable("no Uid line"),
    }
}

/// Landlock is needed by the sandbox (later slice); informational here.
#[must_use]
pub fn check_lsm_text(lsm_list: &str) -> DoctorCheck {
    let present = lsm_list.split(',').any(|m| m.trim() == "landlock");
    if present {
        check(
            "landlock LSM",
            CheckStatus::Ok,
            lsm_list.trim(),
            None,
            false,
        )
    } else {
        check(
            "landlock LSM",
            CheckStatus::Skip,
            lsm_list.trim(),
            Some("kernel without Landlock: sandboxing will be reduced"),
            false,
        )
    }
}

#[must_use]
pub fn command_check(
    name: &str,
    argv: &[&str],
    required: bool,
    remediation: &str,
) -> DoctorCheck {
    let Some((program, args)) = argv.split_first() else {
        return check(name, CheckStatus::Skip, "empty command", None, required);
    };
    match Command::new(program).args(args).output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("ok");
            check(name, CheckStatus::Ok, line, None, required)
        }
        Ok(out) => check(
            name,
            CheckStatus::Fail,
            format!("exit {}", out.status),
            Some(remediation),
            required,
        ),
        Err(e) => check(
            name,
            CheckStatus::Fail,
            e.to_string(),
            Some(remediation),
            required,
        ),
    }
}

#[must_use]
pub fn writable_dir_check(
    name: &str,
    dir: &Path,
    required: bool,
) -> DoctorCheck {
    let probe = dir.join(".willie-doctor-probe");
    let result = fs::create_dir_all(dir)
        .and_then(|()| fs::write(&probe, b"probe"))
        .and_then(|()| fs::remove_file(&probe));
    match result {
        Ok(()) => check(
            name,
            CheckStatus::Ok,
            dir.display().to_string(),
            None,
            required,
        ),
        Err(e) => check(
            name,
            CheckStatus::Fail,
            format!("{}: {e}", dir.display()),
            Some("run `sudo chown willie:willie /var/lib/willie /run/willie`"),
            required,
        ),
    }
}

/// Every check, in display order.
#[must_use]
pub fn run_all() -> DoctorReport {
    let proc_status =
        fs::read_to_string("/proc/self/status").map_err(|e| e.to_string());
    let lsm =
        fs::read_to_string("/sys/kernel/security/lsm").unwrap_or_default();
    let checks = vec![
        check_uid_text(proc_status.as_deref().map_err(String::as_str)),
        writable_dir_check("state dir", Path::new(STATE_DIR), true),
        writable_dir_check("run dir", Path::new(RUN_DIR), true),
        command_check(
            "bubblewrap",
            &["bwrap", "--version"],
            true,
            "apt-get install bubblewrap (image bug: rebuild the distro)",
        ),
        command_check(
            "git",
            &["git", "--version"],
            true,
            "image bug: rebuild the distro",
        ),
        command_check(
            "curl",
            &["curl", "--version"],
            true,
            "image bug: rebuild the distro",
        ),
        command_check(
            "user namespaces",
            &["unshare", "-U", "-r", "true"],
            false,
            "unprivileged user namespaces disabled; sandbox unavailable",
        ),
        check_lsm_text(&lsm),
        network_check(),
    ];
    DoctorReport { checks }
}

fn network_check() -> DoctorCheck {
    let timeout = Duration::from_secs(8).as_secs().to_string();
    command_check(
        "network (api.anthropic.com)",
        &[
            "curl",
            "-sSI",
            "-m",
            &timeout,
            "-o",
            "/dev/null",
            "https://api.anthropic.com",
        ],
        false,
        "no route to the API: proxy/CA propagation (engine → machine.env)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landlock_in_the_lsm_list_is_ok() {
        let c = check_lsm_text("lockdown,capability,landlock,yama\n");
        assert_eq!(c.status, CheckStatus::Ok);
        assert!(!c.required);
    }

    #[test]
    fn missing_landlock_is_a_skip_not_a_failure() {
        assert_eq!(check_lsm_text("capability,yama").status, CheckStatus::Skip);
    }

    #[test]
    fn running_as_root_fails_the_uid_check() {
        let status = "Name:\twillied\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n";
        let c = check_uid_text(Ok(status));
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.required);
    }

    #[test]
    fn an_unprivileged_uid_passes() {
        assert_eq!(
            check_uid_text(Ok("Uid:\t1000\t1000\t1000\t1000\n")).status,
            CheckStatus::Ok
        );
    }

    /// A required check that cannot decide must fail: "unknown" is not
    /// "unprivileged".
    #[test]
    fn an_unreadable_proc_status_fails_the_user_check() {
        let c = check_uid_text(Err("permission denied"));
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.required);
        assert_eq!(
            c.detail,
            "cannot read /proc/self/status: permission denied"
        );
        assert!(c.remediation.is_some());
    }

    #[test]
    fn a_proc_status_without_a_uid_line_fails_too() {
        let c = check_uid_text(Ok("Name:\twillied\n"));
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(c.detail.starts_with("cannot read /proc/self/status"));
    }

    #[test]
    fn a_missing_tool_fails_with_its_remediation() {
        let c = command_check(
            "frobnicate",
            &["willie-surely-missing-tool", "--version"],
            true,
            "install frobnicate",
        );
        assert_eq!(c.status, CheckStatus::Fail);
        assert_eq!(c.remediation.as_deref(), Some("install frobnicate"));
    }

    #[test]
    fn a_temp_dir_is_writable() {
        let dir = std::env::temp_dir();
        assert_eq!(
            writable_dir_check("tmp", &dir, true).status,
            CheckStatus::Ok
        );
    }
}
