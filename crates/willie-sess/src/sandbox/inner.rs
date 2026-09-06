//! The in-namespace stage: reached by re-executing `willie-sess --inner`
//! as bwrap's command. It proves the namespace is real, sets the resource
//! limits from inside it (NPROC is counted per user namespace), applies
//! Landlock, installs the syscall filter, reports over the inherited
//! socket what applied, then execs the harness.

/// Why this process is not inside a fresh session namespace, if it is
/// not. `uid_map` is `/proc/self/uid_map`, `status` is
/// `/proc/self/status`. The helper builds the namespace with a bounded
/// uid mapping — bwrap's default is the single-uid identity map
/// `<uid> <uid> 1` — whereas the initial user namespace maps the whole
/// uid space (length 4294967295); the base also sets no_new_privs. A
/// bounded mapping together with no_new_privs is the helper's
/// fingerprint. Comparing the mapped uid to the caller's own is not it:
/// the mapping is the identity, so inside and outside read the same
/// number. Either signal missing means the helper never built the
/// namespace, and the stage refuses — fail closed, so an unreadable or
/// empty file is "not a namespace" too.
#[must_use]
pub fn not_in_namespace(uid_map: &str, status: &str) -> Option<String> {
    // The range length is the third field. The initial namespace maps the
    // whole uid space (u32::MAX); a namespace the helper built maps a
    // bounded range. A zero-length, whole-space or unreadable map is no
    // fresh namespace at all.
    let mapped = uid_map
        .split_whitespace()
        .nth(2)
        .and_then(|len| len.parse::<u64>().ok())
        .is_some_and(|len| (1..u64::from(u32::MAX)).contains(&len));
    let no_new_privs = status
        .lines()
        .find_map(|l| l.strip_prefix("NoNewPrivs:"))
        .map(|rest| rest.trim() == "1")
        .unwrap_or(false);
    if mapped && no_new_privs {
        None
    } else {
        Some(
            "not inside the sandbox namespace: the helper did not build it"
                .to_owned(),
        )
    }
}

/// `min(requested, hard)`, with `u64::MAX` as the hard limit meaning
/// "unlimited", so a host that does not cap a resource takes the request.
#[must_use]
pub fn clamp(requested: u64, hard: u64) -> u64 {
    if hard == u64::MAX {
        requested
    } else {
        requested.min(hard)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::{
        ffi::CString,
        io::{Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::net::UnixStream,
        },
        process::ExitCode,
    };

    use willie_linux::sandbox::inner::{Report, Request};

    use super::{clamp, not_in_namespace};
    use crate::sandbox::landlock::{self, Outcome};

    /// The stage. Reads the request from `fd`, verifies the namespace,
    /// applies the limits, Landlock and the syscall filter in that order,
    /// reports over the same socket, marks the socket close-on-exec, and
    /// execs the harness. A failure before the report
    /// is a `Refused` line the supervisor reads, and a non-zero exit. A
    /// failure at or after the exec is past the report — the supervisor
    /// already read `Applied` and replied ready — so a `Refused` there
    /// would never be read: the cause goes to stderr, which bwrap wired to
    /// the session's PTY, and the stage exits 127 (0017).
    #[must_use]
    pub fn run_inner(fd: i32) -> ExitCode {
        // SAFETY: the supervisor passed us this socketpair end as `fd`,
        // open and ours to own.
        let mut sock = unsafe { UnixStream::from_raw_fd(fd) };

        let request = match read_request(&mut sock) {
            Ok(request) => request,
            Err(msg) => {
                return refuse(&mut sock, "sandbox_apply_failed", &msg);
            }
        };

        let uid_map =
            std::fs::read_to_string("/proc/self/uid_map").unwrap_or_default();
        let status =
            std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        if let Some(reason) = not_in_namespace(&uid_map, &status) {
            return refuse(&mut sock, "sandbox_apply_failed", &reason);
        }

        if let Err(msg) = apply_rlimits(&request.rlimits) {
            return refuse(
                &mut sock,
                "sandbox_apply_failed",
                &format!("cannot set {msg}"),
            );
        }

        let mut mechanisms = vec![
            "namespaces".to_owned(),
            "mounts".to_owned(),
            "rlimits".to_owned(),
        ];
        let mut unavailable = Vec::new();

        // Landlock before the filter, so its own syscalls run before the
        // filter is in force. Optional: a kernel without a usable ABI is
        // reported, not refused. A kernel that offered one and then
        // refused to apply is a refusal, never a silent downgrade.
        match landlock::apply(&request.landlock) {
            Ok(Outcome::Applied { .. }) => {
                mechanisms.push("landlock".to_owned());
            }
            Ok(Outcome::Unavailable) => {
                unavailable.push("landlock".to_owned());
            }
            Err(e) => {
                return refuse(
                    &mut sock,
                    "sandbox_apply_failed",
                    &e.to_string(),
                );
            }
        }

        // The syscall filter, with a user-notification listener. This
        // works unprivileged only because bwrap already set NO_NEW_PRIVS
        // for this namespace; the stage runs inside it, so the bit holds.
        let listener = match crate::sandbox::seccomp::install() {
            Ok(listener) => listener,
            Err(e) => {
                let (code, what) = match e.raw_os_error() {
                    Some(libc::EINVAL) | Some(libc::ENOSYS) => (
                        "sandbox_backend_missing",
                        "this kernel has no seccomp user notification",
                    ),
                    _ => (
                        "sandbox_apply_failed",
                        "cannot install the syscall filter",
                    ),
                };
                return refuse(&mut sock, code, &format!("{what}: {e}"));
            }
        };

        mechanisms.push("seccomp".to_owned());

        // The report carries the listener alongside it: SCM_RIGHTS
        // duplicates the descriptor into the supervisor's process with its
        // own reference, so the supervisor's copy survives the stage
        // closing its own.
        if let Err(msg) = write_applied(
            &mut sock,
            mechanisms,
            unavailable,
            Some(listener.as_raw_fd()),
        ) {
            eprintln!("willie-sess --inner: cannot report: {msg}");
            return ExitCode::FAILURE;
        }
        // Close the stage's own copy now the supervisor holds one: the
        // harness must never inherit the listener. A process holding its
        // own listener could approve its own syscalls with
        // SECCOMP_USER_NOTIF_FLAG_CONTINUE. Dropping closes it here and
        // now, rather than leaving it open until exec as CLOEXEC would.
        drop(listener);

        set_cloexec(fd);
        exec_harness(&request.argv)
    }

    /// The `Applied` report, with the filter's listener riding alongside
    /// as `SCM_RIGHTS` when there is one, so the supervisor reads the
    /// descriptor and the words that describe it together. `None` is the
    /// plain line.
    fn write_applied(
        sock: &mut UnixStream,
        mechanisms: Vec<String>,
        unavailable: Vec<String>,
        listener: Option<i32>,
    ) -> Result<(), String> {
        let report = Report::Applied {
            mechanisms,
            unavailable,
        };
        let line = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
        crate::sandbox::send_report_with_fd(sock, &line, listener)
            .map_err(|e| e.to_string())
    }

    fn read_request(sock: &mut UnixStream) -> Result<Request, String> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match sock.read(&mut byte) {
                Ok(0) => {
                    return Err(
                        "the supervisor closed the request socket".to_owned()
                    );
                }
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) => buf.push(byte[0]),
                Err(e) => return Err(e.to_string()),
            }
        }
        serde_json::from_slice(&buf).map_err(|e| e.to_string())
    }

    fn write_report(
        sock: &mut UnixStream,
        report: &Report,
    ) -> Result<(), String> {
        let mut line = serde_json::to_vec(report).map_err(|e| e.to_string())?;
        line.push(b'\n');
        sock.write_all(&line).map_err(|e| e.to_string())?;
        sock.flush().map_err(|e| e.to_string())
    }

    fn refuse(sock: &mut UnixStream, code: &str, message: &str) -> ExitCode {
        let _ = write_report(
            sock,
            &Report::Refused {
                code: code.to_owned(),
                message: message.to_owned(),
            },
        );
        ExitCode::FAILURE
    }

    /// Ratchet one `RLIMIT_*` down to `min(value, current hard)` on BOTH
    /// the soft and the hard limit. Lowering only the soft limit would
    /// leave the inherited hard limit as a ceiling a confined harness
    /// could raise its soft limit back up to — raising a soft limit up to
    /// the hard one needs no privilege — undoing the bound. Lowering the
    /// hard limit needs no privilege either and cannot be reversed without
    /// `CAP_SYS_RESOURCE`, so the ceiling is real. Clamping to the current
    /// hard limit keeps a stricter host from being loosened: the new value
    /// is `min(value, current hard)`, never above the limit already in
    /// force, so it only ever ratchets down.
    fn set_one(resource: libc::c_int, value: u64) -> Result<(), String> {
        let mut current = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit writes a valid rlimit through the pointer.
        if unsafe { libc::getrlimit(resource, &mut current) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let hard = current.rlim_max;
        let soft = clamp(value, hard);
        let limit = libc::rlimit {
            rlim_cur: soft,
            rlim_max: soft,
        };
        // SAFETY: setrlimit reads a valid rlimit through the pointer.
        if unsafe { libc::setrlimit(resource, &limit) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    fn apply_rlimits(
        r: &willie_linux::sandbox::inner::Rlimits,
    ) -> Result<(), String> {
        set_one(libc::RLIMIT_NPROC, r.nproc)
            .map_err(|e| format!("NPROC: {e}"))?;
        set_one(libc::RLIMIT_NOFILE, r.nofile)
            .map_err(|e| format!("NOFILE: {e}"))?;
        set_one(libc::RLIMIT_CORE, r.core).map_err(|e| format!("CORE: {e}"))?;
        Ok(())
    }

    /// Set `FD_CLOEXEC` so the harness never inherits the report socket: a
    /// process that held it could answer the supervisor in the stage's
    /// place.
    fn set_cloexec(fd: i32) {
        // SAFETY: fcntl on a descriptor we own.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }

    /// Exec the harness command. Returns only on failure, and every
    /// failure here is past the report: `run_inner` sent `Applied` and the
    /// supervisor replied ready before this ran, so a `Refused` would be
    /// written into a socket nothing reads again. The cause goes to stderr
    /// instead — bwrap wired the stage's stderr to the session's PTY, so
    /// an attached user sees it — and the stage exits 127, the documented
    /// "exec failed inside the stage" code (0017).
    fn exec_harness(argv: &[String]) -> ExitCode {
        let Some(program) = argv.first() else {
            eprintln!("willie-sess --inner: no harness command");
            return ExitCode::from(127);
        };
        let c_args: Result<Vec<CString>, _> =
            argv.iter().map(|a| CString::new(a.as_bytes())).collect();
        let (Ok(program_c), Ok(c_args)) =
            (CString::new(program.as_bytes()), c_args)
        else {
            eprintln!("willie-sess --inner: a harness argument holds a NUL");
            return ExitCode::from(127);
        };
        let mut ptrs: Vec<*const libc::c_char> =
            c_args.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: execv with a valid program path and NULL-terminated
        // argv. The environment is already exactly what the vector set
        // (--clearenv + --setenv), so execv, which keeps environ, is right.
        unsafe {
            libc::execv(program_c.as_ptr(), ptrs.as_ptr());
        }
        let errno = std::io::Error::last_os_error();
        eprintln!("willie-sess --inner: cannot exec the harness: {errno}");
        ExitCode::from(127)
    }
}

#[cfg(target_os = "linux")]
pub use imp::run_inner;

#[cfg(test)]
mod tests {
    use super::*;

    /// A real session namespace has a bounded uid mapping — bwrap's
    /// default is the single-uid identity map — and NoNewPrivs is 1. The
    /// initial namespace maps the whole uid space; a missing NoNewPrivs
    /// or an empty map means the namespace was never built.
    #[test]
    fn a_bounded_uid_map_with_no_new_privs_is_a_namespace() {
        // bwrap's identity single-uid map is a real namespace.
        assert_eq!(
            not_in_namespace(
                "      1000       1000          1\n",
                "NoNewPrivs:\t1\n"
            ),
            None
        );
        // A remapped single-uid map is one too.
        assert_eq!(
            not_in_namespace(
                "         0       1000          1\n",
                "NoNewPrivs:\t1\n"
            ),
            None
        );
        // The initial namespace maps the whole range: not a fresh one.
        assert!(
            not_in_namespace(
                "         0          0 4294967295\n",
                "NoNewPrivs:\t1\n"
            )
            .is_some()
        );
        // NoNewPrivs missing, or an empty map, is no namespace.
        assert!(
            not_in_namespace(
                "      1000       1000          1\n",
                "NoNewPrivs:\t0\n"
            )
            .is_some()
        );
        assert!(not_in_namespace("", "").is_some());
    }

    #[test]
    fn clamp_never_raises_a_stricter_host_limit() {
        assert_eq!(clamp(4096, 8192), 4096);
        assert_eq!(clamp(4096, 1000), 1000);
        assert_eq!(clamp(0, 0), 0);
        assert_eq!(clamp(65536, u64::MAX), 65536);
    }
}
