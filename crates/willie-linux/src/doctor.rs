//! `doctor`: can this distribution run Willie sessions?
//!
//! Each check is pure given its input text, a probe's answer or a command
//! result, so the decisions are unit-tested on any host; only `run_all`
//! and the probes touch the system.

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

/// Landlock is optional (0016): its absence reduces the sandbox but never
/// fails the doctor. `probe` is the ABI the kernel answers, or the errno
/// when it does not. The reason for a skip goes in the detail, the only
/// text a non-failing check shows.
#[must_use]
pub fn check_landlock(probe: Result<i32, i32>) -> DoctorCheck {
    // The two answers of a kernel that has no Landlock to offer: built
    // without it, or with it disabled at boot. Literal values keep the
    // library libc-free; they are the same on every Linux architecture.
    const ENOSYS: i32 = 38;
    const EOPNOTSUPP: i32 = 95;
    let (status, detail) = match probe {
        // ABI 2 brought REFER; without it a rename across directories is
        // refused, and git stops working, so the sandbox applies 2+ only.
        Ok(abi) if abi >= 2 => (CheckStatus::Ok, format!("ABI {abi}")),
        Ok(abi) => (
            CheckStatus::Skip,
            format!(
                "Landlock ABI {abi} has no rename support; \
                 sandboxing will be reduced"
            ),
        ),
        Err(ENOSYS | EOPNOTSUPP) => (
            CheckStatus::Skip,
            "kernel without Landlock: sandboxing will be reduced".to_owned(),
        ),
        Err(errno) => (
            CheckStatus::Skip,
            format!(
                "landlock_create_ruleset failed with errno {errno}; \
                 sandboxing will be reduced"
            ),
        ),
    };
    check("landlock", status, detail, None, false)
}

/// Asks the kernel for the Landlock ABI with the version query of
/// `landlock_create_ruleset`, which creates nothing and reads no memory.
/// It goes through the C library's `syscall(2)`, a symbol std links
/// already, so the library stays libc-crate-free while still asking the
/// kernel rather than a file securityfs may not provide (0016).
#[cfg(target_os = "linux")]
fn probe_landlock_abi() -> Result<i32, i32> {
    use std::ffi::c_long;

    use crate::sandbox::landlock::LANDLOCK_CREATE_RULESET_VERSION;

    // One number on every architecture: the syscall table has been
    // unified since before Landlock was added.
    const SYS_LANDLOCK_CREATE_RULESET: c_long = 444;

    unsafe extern "C" {
        fn syscall(num: c_long, ...) -> c_long;
    }

    // SAFETY: a null attribute with a zero size under the VERSION flag
    // makes the kernel answer the ABI without touching memory; a negative
    // return sets errno, read right after.
    let ret = unsafe {
        syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<u8>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if ret >= 0 {
        Ok(i32::try_from(ret).unwrap_or(i32::MAX))
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

/// Off Linux there is no kernel to ask: the answer is ENOSYS, which the
/// check reads as "no Landlock".
#[cfg(not(target_os = "linux"))]
fn probe_landlock_abi() -> Result<i32, i32> {
    Err(38)
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
        check_landlock(probe_landlock_abi()),
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
    fn landlock_abi_3_is_ok_and_names_the_abi() {
        let c = check_landlock(Ok(3));
        assert_eq!(c.status, CheckStatus::Ok);
        assert_eq!(c.name, "landlock");
        assert_eq!(c.detail, "ABI 3");
        assert!(!c.required);
    }

    /// ABI 2 is the first with REFER, the right the sandbox needs.
    #[test]
    fn landlock_abi_2_is_ok() {
        let c = check_landlock(Ok(2));
        assert_eq!(c.status, CheckStatus::Ok);
        assert_eq!(c.detail, "ABI 2");
        assert!(!c.required);
    }

    #[test]
    fn landlock_abi_1_is_a_skip_that_says_why() {
        let c = check_landlock(Ok(1));
        assert_eq!(c.status, CheckStatus::Skip);
        assert!(c.detail.contains("rename"), "{}", c.detail);
        assert!(!c.required);
    }

    /// ENOSYS (no syscall) and EOPNOTSUPP (disabled at boot) are both
    /// "no Landlock here": a skip, since the check is optional.
    #[test]
    fn a_kernel_without_landlock_is_a_skip_not_a_failure() {
        for errno in [38, 95] {
            let c = check_landlock(Err(errno));
            assert_eq!(c.status, CheckStatus::Skip, "errno {errno}");
            assert!(c.detail.contains("without Landlock"), "{}", c.detail);
            assert!(!c.required);
        }
    }

    /// An optional check that cannot decide skips; the errno stays
    /// visible so the reason can be looked up.
    #[test]
    fn an_unexpected_errno_is_a_skip_naming_it() {
        let c = check_landlock(Err(13));
        assert_eq!(c.status, CheckStatus::Skip);
        assert!(c.detail.contains("13"), "{}", c.detail);
        assert!(!c.required);
    }

    /// The real probe, where there is a kernel to ask: the `syscall`
    /// declaration links and the call answers an ABI, or one of the two
    /// errnos of a kernel without Landlock.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_probe_answers_an_abi_or_a_kernel_without_landlock() {
        let answer = probe_landlock_abi();
        assert!(matches!(answer, Ok(1..) | Err(38 | 95)), "{answer:?}");
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
