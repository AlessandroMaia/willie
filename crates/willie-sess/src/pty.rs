//! The PTY pair and the child that owns its slave side.
//!
//! Opening `/dev/ptmx` by hand instead of `openpty` keeps the supervisor to
//! plain syscalls, which cross-compile to musl with nothing to link against.
//! The child gets its own session and makes the slave its controlling
//! terminal, so job control, `Ctrl-C` and `SIGWINCH` behave as they do in a
//! real terminal.

use std::{
    ffi::CString,
    io,
    os::fd::{AsRawFd, RawFd},
};

/// `_IOR('T', 0x30, unsigned int)` — number of the slave behind a master.
const TIOCGPTN: libc::c_ulong = 0x8004_5430;
/// `_IOW('T', 0x31, int)` — unlock the slave so it can be opened.
const TIOCSPTLCK: libc::c_ulong = 0x4004_5431;

/// Window size a session starts with, before the first client resizes it.
pub const DEFAULT_ROWS: u16 = 24;
pub const DEFAULT_COLS: u16 = 80;

/// A file descriptor closed when it goes out of scope.
///
/// Shared between the thread that reads the PTY master and the threads that
/// write to it; the kernel serialises those, so `&Fd` is enough.
#[derive(Debug)]
pub struct Fd(RawFd);

impl Fd {
    /// Take ownership of a descriptor obtained from a syscall.
    pub fn new(raw: RawFd) -> Self {
        Self(raw)
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            // SAFETY: we own this descriptor and close it exactly once.
            unsafe { libc::close(self.0) };
        }
    }
}

fn errno() -> io::Error {
    io::Error::last_os_error()
}

/// Open a PTY pair. The master is close-on-exec; the slave is too, and the
/// child duplicates it onto its standard descriptors, which clears the flag.
pub fn open() -> io::Result<(Fd, Fd)> {
    // SAFETY: a constant path and flags; the return value is checked.
    let master = unsafe {
        libc::open(
            c"/dev/ptmx".as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if master < 0 {
        return Err(errno());
    }
    let master = Fd::new(master);
    let mut unlock: libc::c_int = 0;
    // SAFETY: TIOCSPTLCK reads one int through the pointer we pass.
    let rc = unsafe {
        libc::ioctl(master.as_raw_fd(), TIOCSPTLCK as _, &mut unlock)
    };
    if rc != 0 {
        return Err(errno());
    }
    let mut number: libc::c_uint = 0;
    // SAFETY: TIOCGPTN writes one unsigned int through the pointer.
    let rc =
        unsafe { libc::ioctl(master.as_raw_fd(), TIOCGPTN as _, &mut number) };
    if rc != 0 {
        return Err(errno());
    }
    let path = CString::new(format!("/dev/pts/{number}"))
        .map_err(|_| io::Error::other("slave path holds a NUL"))?;
    // SAFETY: `path` stays alive for the call; the result is checked.
    let slave = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if slave < 0 {
        return Err(errno());
    }
    Ok((master, Fd::new(slave)))
}

/// Apply a window size to the master. The kernel forwards `SIGWINCH` to the
/// foreground process group of the slave, which is how a full-screen
/// program learns it must redraw.
pub fn set_window_size(master: &Fd, rows: u16, cols: u16) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads a `winsize` through the pointer we pass.
    let rc =
        unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) };
    if rc == 0 { Ok(()) } else { Err(errno()) }
}

/// Read from the master. `Ok(0)` also stands for `EIO`, which is what a
/// master returns once the last slave descriptor is gone: for the caller
/// that is simply the end of the session's output.
pub fn read(master: &Fd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: the buffer is valid for `buf.len()` bytes.
        let n = unsafe {
            libc::read(master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len())
        };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = errno();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EIO) => return Ok(0),
            _ => return Err(e),
        }
    }
}

/// Write every byte to a descriptor, retrying short writes and `EINTR`.
pub fn write_all(fd: &Fd, buf: &[u8]) -> io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        // SAFETY: the slice is valid for the length we pass.
        let n = unsafe {
            libc::write(
                fd.as_raw_fd(),
                buf[done..].as_ptr().cast(),
                buf.len() - done,
            )
        };
        if n > 0 {
            done += n as usize;
            continue;
        }
        let e = errno();
        if e.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(e);
    }
    Ok(())
}

/// Why the harness could not be started.
#[derive(Debug)]
pub enum SpawnError {
    /// PTY, fork or pipe trouble in the supervisor itself.
    Setup(io::Error),
    /// The child reached `chdir`/`execve` and it failed: the errno, and
    /// what was being attempted.
    Exec {
        step: &'static str,
        error: io::Error,
    },
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Setup(e) => write!(f, "{e}"),
            Self::Exec { step, error } => write!(f, "{step}: {error}"),
        }
    }
}

/// How the harness ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// Start `argv` on the slave side of `master`, in its own session with the
/// slave as controlling terminal, `cwd` as working directory and exactly
/// `env` as environment. Returns once `execve` has succeeded: the child
/// holds a close-on-exec pipe and writes the errno there if `chdir` or
/// `execve` fails, so an unstartable harness is an error here, never a
/// "started" followed by exit 127.
pub fn spawn(
    master: &Fd,
    slave: Fd,
    argv: &[String],
    cwd: &str,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<libc::pid_t, SpawnError> {
    let Some(program) = argv.first() else {
        return Err(SpawnError::Setup(io::Error::other("no program to run")));
    };
    let c = |s: &str| {
        CString::new(s).map_err(|_| {
            SpawnError::Setup(io::Error::other("an argument holds a NUL"))
        })
    };
    let program = c(program)?;
    let owned: Vec<CString> =
        argv.iter().map(|a| c(a)).collect::<Result<_, _>>()?;
    let mut argv_ptrs: Vec<*const libc::c_char> =
        owned.iter().map(|a| a.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());
    let env_owned: Vec<CString> = env
        .iter()
        .map(|(k, v)| c(&format!("{k}={v}")))
        .collect::<Result<_, _>>()?;
    let mut env_ptrs: Vec<*const libc::c_char> =
        env_owned.iter().map(|e| e.as_ptr()).collect();
    env_ptrs.push(std::ptr::null());
    let dir = c(cwd)?;

    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-element out parameter.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(SpawnError::Setup(errno()));
    }
    let (err_read, err_write) = (Fd::new(fds[0]), Fd::new(fds[1]));
    set_window_size(master, DEFAULT_ROWS, DEFAULT_COLS)
        .map_err(SpawnError::Setup)?;

    // SAFETY: no thread exists yet; the child only makes syscalls before
    // `execve` and reports failures through the pipe.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => Err(SpawnError::Setup(errno())),
        0 => {
            // SAFETY: every call takes descriptors and pointers valid in
            // this process image; `report` writes 5 bytes and exits.
            unsafe {
                drop(err_read);
                libc::setsid();
                libc::ioctl(slave.as_raw_fd(), libc::TIOCSCTTY as _, 0);
                libc::dup2(slave.as_raw_fd(), 0);
                libc::dup2(slave.as_raw_fd(), 1);
                libc::dup2(slave.as_raw_fd(), 2);
                if slave.as_raw_fd() > 2 {
                    libc::close(slave.as_raw_fd());
                }
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                if libc::chdir(dir.as_ptr()) != 0 {
                    report_and_exit(&err_write, b'c');
                }
                libc::execve(
                    program.as_ptr(),
                    argv_ptrs.as_ptr(),
                    env_ptrs.as_ptr(),
                );
                report_and_exit(&err_write, b'e');
            }
        }
        pid => {
            drop(slave);
            drop(err_write);
            let mut buf = [0u8; 5];
            let n = read(&err_read, &mut buf).map_err(SpawnError::Setup)?;
            if n == 0 {
                return Ok(pid);
            }
            // The child failed before exec: reap it and report why.
            let _ = wait(pid);
            let code = i32::from_ne_bytes([buf[1], buf[2], buf[3], buf[4]]);
            let step = if buf[0] == b'c' {
                "cannot enter the workspace"
            } else {
                "cannot execute the harness"
            };
            Err(SpawnError::Exec {
                step,
                error: io::Error::from_raw_os_error(code),
            })
        }
    }
}

/// Child-only: write `step` + errno to the pipe and leave without running
/// any destructor of the parent's image.
///
/// # Safety
/// Must only be called in the forked child, before `execve`.
unsafe fn report_and_exit(pipe: &Fd, step: u8) -> ! {
    let err = errno().raw_os_error().unwrap_or(0);
    let mut msg = [0u8; 5];
    msg[0] = step;
    msg[1..].copy_from_slice(&err.to_ne_bytes());
    // SAFETY: the pipe is open and the buffer is 5 valid bytes.
    unsafe {
        libc::write(pipe.as_raw_fd(), msg.as_ptr().cast(), msg.len());
        libc::_exit(127)
    }
}

/// Wait for `pid` and say how it ended.
pub fn wait(pid: libc::pid_t) -> io::Result<Exit> {
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: `status` is a valid out parameter.
        let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
        if rc == pid {
            return Ok(exit_of(status));
        }
        if rc < 0 {
            let e = errno();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(e);
        }
    }
}

fn exit_of(status: libc::c_int) -> Exit {
    if libc::WIFEXITED(status) {
        Exit {
            code: Some(libc::WEXITSTATUS(status)),
            signal: None,
        }
    } else if libc::WIFSIGNALED(status) {
        Exit {
            code: None,
            signal: Some(libc::WTERMSIG(status)),
        }
    } else {
        Exit {
            code: None,
            signal: None,
        }
    }
}

/// Ignore `SIGPIPE`: an attach client that vanishes mid-write must cost the
/// supervisor an error return, never the session.
pub fn ignore_sigpipe() {
    // SAFETY: setting a disposition on a valid signal number.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_exit_keeps_its_code() {
        assert_eq!(
            exit_of(7 << 8),
            Exit {
                code: Some(7),
                signal: None
            }
        );
    }

    #[test]
    fn a_signalled_child_reports_the_signal() {
        assert_eq!(
            exit_of(libc::SIGKILL),
            Exit {
                code: None,
                signal: Some(libc::SIGKILL)
            }
        );
    }
}
