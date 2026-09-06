//! Applying the Landlock rules the request carries.
//!
//! The rules are data from `willie_linux::sandbox::landlock`: read and
//! execute beneath `/`, every handled right beneath each path the plan
//! mounted read-write. This is the I/O around them — the three syscalls,
//! in order, against the kernel the stage runs on. Landlock is optional:
//! a kernel without it, or with an ABI below 2, leaves the session on the
//! mounts alone and is reported unavailable. A kernel that answered an
//! ABI and then refused to apply is a refusal, never a silent downgrade.

use std::{
    ffi::CString,
    fmt, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr,
};

use willie_linux::sandbox::landlock::{
    ACCESS_FS_EXECUTE, ACCESS_FS_READ_FILE, ACCESS_FS_TRUNCATE,
    ACCESS_FS_WRITE_FILE, Apply, LANDLOCK_CREATE_RULESET_VERSION,
    LANDLOCK_RULE_PATH_BENEATH, PathBeneathAttr, READ_EXEC, Rules, RulesetAttr,
    applicability,
};

/// What applying the rules came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The ruleset is in force for this process and everything it execs.
    Applied { abi: i32 },
    /// This kernel offers no Landlock the session can use: no syscall,
    /// disabled at boot, or ABI 1, which lacks `REFER`.
    Unavailable,
}

/// Why the rules could not be applied on a kernel that offers Landlock.
/// Every variant is a `sandbox_apply_failed` refusal: the kernel said it
/// could, and then did not. `errno` is the kernel's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A rule's path is not there to open, or to describe, inside the
    /// namespace.
    PathWontOpen { path: String, errno: i32 },
    /// `landlock_create_ruleset` refused the handled set.
    Ruleset(i32),
    /// `landlock_add_rule` refused the rule for this path.
    AddRule { path: String, errno: i32 },
    /// `landlock_restrict_self` refused; nothing took effect.
    RestrictSelf(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let os = |errno: i32| io::Error::from_raw_os_error(errno);
        match self {
            Self::PathWontOpen { path, errno } => write!(
                f,
                "cannot open `{path}` for a Landlock rule: {}",
                os(*errno)
            ),
            Self::Ruleset(errno) => write!(
                f,
                "the kernel offers Landlock but refused the ruleset: {}",
                os(*errno)
            ),
            Self::AddRule { path, errno } => write!(
                f,
                "cannot add the Landlock rule for `{path}`: {}",
                os(*errno)
            ),
            Self::RestrictSelf(errno) => write!(
                f,
                "cannot restrict the stage with Landlock: {}",
                os(*errno)
            ),
        }
    }
}

impl std::error::Error for Error {}

/// The rights the kernel accepts in a rule whose path is a file rather
/// than a directory; a file rule carrying any other right is `EINVAL`.
pub const FILE_RIGHTS: u64 = ACCESS_FS_EXECUTE
    | ACCESS_FS_WRITE_FILE
    | ACCESS_FS_READ_FILE
    | ACCESS_FS_TRUNCATE;

/// The rights a rule grants: `granted`, never beyond what the ruleset
/// handles, and for a file — a read-write extra path can be one — never
/// beyond what a file can have, or the kernel refuses the rule and with
/// it a session whose configuration was fine.
#[must_use]
pub fn rule_rights(granted: u64, handled: u64, is_dir: bool) -> u64 {
    let rights = granted & handled;
    if is_dir { rights } else { rights & FILE_RIGHTS }
}

/// Apply `rules` to this process. Probe the ABI, decide by it, create the
/// ruleset, add the `/` rule and one rule per writable path, then
/// restrict. `restrict_self` needs `no_new_privs`, which the helper set
/// for the namespace and the stage verified before getting here.
pub fn apply(rules: &Rules) -> Result<Outcome, Error> {
    let abi = probe_abi();
    let handled = match applicability(abi) {
        Apply::Unavailable => return Ok(Outcome::Unavailable),
        Apply::Ruleset { handled } => handled,
    };

    let attr = RulesetAttr {
        handled_access_fs: handled,
    };
    // SAFETY: `attr` outlives the call and the size is its own; the kernel
    // copies it and answers a fresh descriptor, or -1 with errno set.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &raw const attr,
            size_of::<RulesetAttr>(),
            0u32,
        )
    };
    let ruleset = owned_fd(ret).ok_or_else(|| Error::Ruleset(errno()))?;

    add_rule(&ruleset, "/", READ_EXEC, handled)?;
    for path in &rules.write {
        add_rule(&ruleset, path, handled, handled)?;
    }

    // SAFETY: the ruleset descriptor is open and ours; flags are zero.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset.as_raw_fd(),
            0u32,
        )
    };
    if ret < 0 {
        return Err(Error::RestrictSelf(errno()));
    }
    Ok(Outcome::Applied { abi })
}

/// The ABI the kernel offers, or a negative errno, which `applicability`
/// reads as "no Landlock". With no attribute and no size there is nothing
/// for the kernel to reject but the flag itself, which only a kernel
/// without Landlock (`ENOSYS`) or with it disabled at boot (`EOPNOTSUPP`)
/// does.
fn probe_abi() -> i32 {
    // SAFETY: a null attribute with a zero size under the VERSION flag
    // reads nothing; the call answers the ABI or -1 with errno set.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            ptr::null::<RulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if ret < 0 {
        -errno()
    } else {
        i32::try_from(ret).unwrap_or(i32::MAX)
    }
}

/// One `PATH_BENEATH` rule granting `granted` (within `handled`, and
/// within a file's rights when `path` is one) beneath `path`. The
/// directory descriptor lives exactly as long as this call: opened here,
/// closed when it returns, after the kernel has read the rule.
fn add_rule(
    ruleset: &OwnedFd,
    path: &str,
    granted: u64,
    handled: u64,
) -> Result<(), Error> {
    let wont_open = |errno: i32| Error::PathWontOpen {
        path: path.to_owned(),
        errno,
    };
    let c_path =
        CString::new(path.as_bytes()).map_err(|_| wont_open(libc::EINVAL))?;
    // SAFETY: `c_path` is NUL-terminated; O_PATH opens the location
    // without reading it, which is all a rule needs.
    let ret =
        unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    let parent = owned_fd(ret.into()).ok_or_else(|| wont_open(errno()))?;
    let is_dir = is_directory(&parent)
        .map_err(|e| wont_open(e.raw_os_error().unwrap_or(libc::EIO)))?;

    let attr = PathBeneathAttr {
        allowed_access: rule_rights(granted, handled, is_dir),
        parent_fd: parent.as_raw_fd(),
    };
    // SAFETY: `attr` is the kernel's own twelve-byte layout, outlives the
    // call, and names a descriptor `parent` holds open until this function
    // returns; flags are zero.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            LANDLOCK_RULE_PATH_BENEATH,
            &raw const attr,
            0u32,
        )
    };
    if ret < 0 {
        return Err(Error::AddRule {
            path: path.to_owned(),
            errno: errno(),
        });
    }
    Ok(())
}

/// Whether the open location is a directory. `fstat` works on an `O_PATH`
/// descriptor.
fn is_directory(fd: &OwnedFd) -> io::Result<bool> {
    // SAFETY: an all-zero `stat` is a valid buffer for fstat to fill.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fstat writes a `stat` through the pointer; `fd` is open.
    if unsafe { libc::fstat(fd.as_raw_fd(), &raw mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st.st_mode & libc::S_IFMT == libc::S_IFDIR)
}

/// A descriptor the kernel just returned, owned from here on; `None` for
/// a failed call (-1) or a value no descriptor can have.
fn owned_fd(ret: libc::c_long) -> Option<OwnedFd> {
    if ret < 0 {
        return None;
    }
    let fd = libc::c_int::try_from(ret).ok()?;
    // SAFETY: the kernel returned a fresh descriptor this process owns and
    // nothing else has wrapped.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use willie_linux::sandbox::landlock::{
        ACCESS_FS_MAKE_DIR, ACCESS_FS_REFER, ALL_ABI2,
    };

    use super::*;

    /// A directory rule grants what it is given, within the handled set;
    /// a file rule keeps only the rights a file can carry, because the
    /// kernel refuses a file rule with a directory right and a read-write
    /// file grant must not refuse the session.
    #[test]
    fn a_file_rule_keeps_only_the_rights_a_file_can_have() {
        let handled = ALL_ABI2 | ACCESS_FS_TRUNCATE;

        assert_eq!(rule_rights(handled, handled, true), handled);
        assert_eq!(rule_rights(READ_EXEC, handled, true), READ_EXEC);
        assert_eq!(rule_rights(handled, ALL_ABI2, true), ALL_ABI2);

        let file = rule_rights(handled, handled, false);
        assert_eq!(file, FILE_RIGHTS);
        assert_eq!(file & ACCESS_FS_MAKE_DIR, 0);
        assert_eq!(file & ACCESS_FS_REFER, 0);
        assert_ne!(file & ACCESS_FS_TRUNCATE, 0);
        // A file under an ABI 2 ruleset: no TRUNCATE, because it is not
        // handled there.
        assert_eq!(
            rule_rights(handled, ALL_ABI2, false) & ACCESS_FS_TRUNCATE,
            0
        );
    }

    /// Every refusal names the syscall that refused and the kernel's
    /// words for the errno, and the path where there is one.
    #[test]
    fn errors_name_the_path_and_the_errno() {
        let e = Error::PathWontOpen {
            path: "/leak".into(),
            errno: libc::ENOENT,
        };
        let text = e.to_string();
        assert!(text.contains("`/leak`"), "{text}");
        assert!(text.contains("os error 2"), "{text}");

        let text = Error::AddRule {
            path: "/tmp".into(),
            errno: libc::EINVAL,
        }
        .to_string();
        assert!(text.contains("`/tmp`") && text.contains("22"), "{text}");
        assert!(Error::Ruleset(libc::EINVAL).to_string().contains("ruleset"));
        assert!(
            Error::RestrictSelf(libc::EPERM)
                .to_string()
                .contains("restrict")
        );
    }

    /// The probe answers this kernel's ABI (1 or more), or a negative
    /// errno; never zero, which `applicability` would read as "no
    /// Landlock" on a kernel that has it. Nothing is restricted here: a
    /// test must not restrict the test runner.
    #[test]
    fn the_probe_answers_an_abi_or_a_negative_errno() {
        let abi = probe_abi();

        assert_ne!(abi, 0);
        if abi < 0 {
            assert!([-libc::ENOSYS, -libc::EOPNOTSUPP].contains(&abi), "{abi}");
        }
    }
}
