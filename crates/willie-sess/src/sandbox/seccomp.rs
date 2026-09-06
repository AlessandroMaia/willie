//! Installing the syscall filter and answering the calls it intercepts.
//!
//! The in-namespace stage installs the classic-BPF program from
//! `willie_linux::sandbox::seccomp` with a user-notification listener and
//! hands that descriptor to the supervisor with its report. The
//! supervisor answers every intercepted syscall with `EPERM` from a
//! thread it starts before the session is announced ready, so a denial
//! from the very first syscall is handled. The program is portable data;
//! the kernel's own `sock_filter` is built from it field by field.

use std::{
    io,
    os::fd::{FromRawFd, OwnedFd},
};

use willie_linux::sandbox::seccomp;

/// Install the filter and return the notification listener descriptor.
///
/// The classic-BPF program is copied onto the kernel's `sock_filter` one
/// field at a time — `Insn` mirrors that layout, but a foreign type is
/// mapped, never transmuted — and installed with
/// `SECCOMP_FILTER_FLAG_NEW_LISTENER`, whose return value is the listener
/// fd. This succeeds unprivileged only because bubblewrap already set
/// `NO_NEW_PRIVS` for the namespace (without it the call is `EACCES`); the
/// stage runs inside that namespace, so the bit holds. `EINVAL`/`ENOSYS`
/// is a kernel without user notification — the caller maps it to
/// `sandbox_backend_missing`; any other errno is `sandbox_apply_failed`.
pub fn install() -> io::Result<OwnedFd> {
    let program = seccomp::program();
    let filter: Vec<libc::sock_filter> = program
        .iter()
        .map(|insn| libc::sock_filter {
            code: insn.code,
            jt: insn.jt,
            jf: insn.jf,
            k: insn.k,
        })
        .collect();
    let len = u16::try_from(filter.len()).map_err(|_| {
        io::Error::other("the syscall filter is too long to install")
    })?;
    let prog = libc::sock_fprog {
        len,
        filter: filter.as_ptr().cast_mut(),
    };
    // SAFETY: `prog` points at `filter`, which outlives the call, and its
    // `len` is the instruction count. SET_MODE_FILTER with the
    // new-listener flag installs the program and returns the listener fd,
    // or -1 with errno set.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_NEW_LISTENER,
            &raw const prog,
        )
    };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = libc::c_int::try_from(ret).map_err(|_| {
        io::Error::other("seccomp returned an out-of-range descriptor")
    })?;
    // SAFETY: the syscall returned a fresh descriptor this process owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Answer every syscall the filter intercepts with `EPERM`, for the life
/// of the session, calling `on_deny` with each denied call's number after
/// the answer is sent. This task hands it a no-op — the calls are denied
/// but not yet recorded; Task 5 passes a recorder and adds the degraded
/// path when the loop ends unexpectedly.
///
/// The loop blocks in `NOTIF_RECV`. A `NOTIF_SEND` that finds the caller
/// already gone (`ENOENT`) has nothing to answer and is ignored. A
/// `NOTIF_RECV` that fails for any reason but `EINTR` ends the loop: the
/// last process under the filter is gone and the session is tearing down.
pub fn serve_notifications(
    listener: OwnedFd,
    mut on_deny: impl FnMut(u32) + Send + 'static,
) {
    use std::os::fd::AsRawFd;

    let fd = listener.as_raw_fd();

    loop {
        // The kernel requires the notification zeroed before RECV.
        // SAFETY: an all-zero `seccomp_notif` is a valid empty request.
        let mut req: libc::seccomp_notif = unsafe { std::mem::zeroed() };

        // SAFETY: RECV fills `req` through the pointer; `fd` is the
        // listener we own. It blocks until a call is intercepted.
        let received = unsafe {
            libc::ioctl(fd, libc::SECCOMP_IOCTL_NOTIF_RECV, &raw mut req)
        };
        if received < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }

        let mut resp = libc::seccomp_notif_resp {
            id: req.id,
            val: 0,
            error: -libc::EPERM,
            flags: 0,
        };
        // SAFETY: SEND reads `resp` through the pointer; `resp.id` is the
        // id of the request just received. An `ENOENT` (the caller died
        // before the answer) is nothing to act on; the next RECV decides
        // whether the loop goes on.
        let _ = unsafe {
            libc::ioctl(fd, libc::SECCOMP_IOCTL_NOTIF_SEND, &raw mut resp)
        };

        on_deny(req.data.nr as u32);
    }
}
