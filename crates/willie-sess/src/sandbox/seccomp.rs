//! Installing the syscall filter and answering the calls it intercepts.
//!
//! The in-namespace stage installs the classic-BPF program from
//! `willie_linux::sandbox::seccomp` with a user-notification listener and
//! hands that descriptor to the supervisor with its report. The
//! supervisor answers every intercepted syscall with `EPERM` from a
//! thread it starts before the session is announced ready, so a denial
//! from the very first syscall is handled, and records each one by name.
//! The program is portable data; the kernel's own `sock_filter` is built
//! from it field by field.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
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

/// The name a denied call is recorded under: the filter's own name for
/// it, or `syscall_<nr>` for a number the filter does not name — which
/// cannot happen with this filter, but a record must never be lost over
/// a missing label.
#[must_use]
pub fn syscall_name(nr: u32) -> String {
    seccomp::name(nr).map_or_else(|| format!("syscall_{nr}"), str::to_owned)
}

/// Answer every syscall the filter intercepts with `EPERM` until no
/// process is left under the filter, calling `on_deny` with each denied
/// call's number after the answer is sent.
///
/// The loop waits in `poll`, not in `NOTIF_RECV`: a `RECV` with nothing
/// pending sleeps on the filter's semaphore and is not woken when the
/// last process under the filter exits, whereas `poll` reports that as
/// `POLLHUP` — the one orderly way for the loop to end, `Ok(())`. A
/// `POLLIN` is a pending call: received, answered, recorded. `EINTR`
/// anywhere is retried, and a `RECV` that finds the caller already gone
/// (`ENOENT`) has nothing to answer and goes back to waiting. Any other
/// failure is `Err`; the caller decides whether that is a session ending
/// or the filter's server dying under a live session. Either way the
/// listener is dropped on return, and from then on the kernel answers
/// every intercepted call with `ENOSYS`: closed, not open.
pub fn serve_notifications(
    listener: OwnedFd,
    mut on_deny: impl FnMut(u32),
) -> io::Result<()> {
    let fd = listener.as_raw_fd();

    loop {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one `pollfd`, and the count says one; no timeout, so
        // the call returns only for an event, an error or a signal.
        let ready = unsafe { libc::poll(&raw mut pfd, 1, -1) };
        if ready < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if pfd.revents & libc::POLLIN != 0 {
            match answer_one(fd) {
                Ok(nr) => on_deny(nr),
                Err(e)
                    if e.kind() == io::ErrorKind::Interrupted
                        || e.raw_os_error() == Some(libc::ENOENT) => {}
                Err(e) => return Err(e),
            }
            continue;
        }
        if pfd.revents & libc::POLLHUP != 0 {
            return Ok(());
        }
        if pfd.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::other(format!(
                "the listener reported poll events {:#x}",
                pfd.revents
            )));
        }
    }
}

/// Receive one intercepted call, answer it `EPERM` and return its
/// number. A `SEND` that finds the caller already gone (`ENOENT`) has
/// nothing left to answer; the call was still refused, so it is still
/// recorded. Any other `SEND` failure is said on stderr and the loop
/// goes on: the caller stays blocked, which is closed, not open.
fn answer_one(fd: libc::c_int) -> io::Result<u32> {
    // The kernel requires the notification zeroed before RECV.
    // SAFETY: an all-zero `seccomp_notif` is a valid empty request.
    let mut req: libc::seccomp_notif = unsafe { std::mem::zeroed() };
    // SAFETY: RECV fills `req` through the pointer; `fd` is the listener
    // we own, and `poll` said a call is pending.
    let received = unsafe {
        libc::ioctl(fd, libc::SECCOMP_IOCTL_NOTIF_RECV, &raw mut req)
    };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    let nr = req.data.nr as u32;

    let mut resp = libc::seccomp_notif_resp {
        id: req.id,
        val: 0,
        error: -libc::EPERM,
        flags: 0,
    };
    // SAFETY: SEND reads `resp` through the pointer; `resp.id` is the id
    // of the request just received.
    let sent = unsafe {
        libc::ioctl(fd, libc::SECCOMP_IOCTL_NOTIF_SEND, &raw mut resp)
    };
    if sent < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ENOENT) {
            eprintln!(
                "willie-sess: cannot answer denied syscall {}: {e}",
                syscall_name(nr)
            );
        }
    }
    Ok(nr)
}
