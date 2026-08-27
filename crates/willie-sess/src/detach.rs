//! Becoming a process that nothing can take down by accident.
//!
//! A session must outlive the daemon that started it, and on this host the
//! daemon itself is a child of `wsl.exe`: when the Windows side goes away
//! the whole `--exec` process tree is killed. The escape is a double fork
//! plus `setsid`, so the supervisor ends up reparented to the init of the
//! distribution, in its own session, holding none of the launcher's
//! descriptors.
//!
//! Detaching is only useful if the launcher can tell whether it worked, so
//! the two halves are joined by a pipe: the launcher blocks until the
//! grandchild has bound its socket, or until the pipe closes because the
//! grandchild died.

use std::{io, os::fd::AsRawFd, path::Path};

use crate::pty::{self, Fd};

/// Which side of the double fork the caller is on.
#[derive(Debug)]
pub enum Side {
    /// The process the user invoked: waits for the handshake and reports.
    Launcher(Handshake),
    /// The detached grandchild: sets the session up, then answers.
    Session(Reply),
}

/// The launcher's end of the handshake pipe.
#[derive(Debug)]
pub struct Handshake(Fd);

/// The grandchild's end of the handshake pipe.
#[derive(Debug)]
pub struct Reply(Fd);

/// Fork twice and `setsid`, leaving the caller as one of the two ends.
pub fn detach() -> io::Result<Side> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-element out parameter. `O_CLOEXEC` keeps
    // the handshake out of the harness process image.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    let read = Fd::new(fds[0]);
    let write = Fd::new(fds[1]);
    // SAFETY: no threads have been spawned yet, so the child inherits a
    // consistent process image.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            drop(read);
            let mut reply = Reply(write);
            // SAFETY: leaving the launcher's session and controlling
            // terminal; this is the point of the exercise.
            if unsafe { libc::setsid() } == -1 {
                let _ = reply.fail("supervisor_spawn_failed", "setsid failed");
                // SAFETY: the intermediate must run no destructors that
                // belong to the launcher's process image.
                unsafe { libc::_exit(1) }
            }
            // SAFETY: as the first fork.
            match unsafe { libc::fork() } {
                -1 => {
                    let _ = reply
                        .fail("supervisor_spawn_failed", "second fork failed");
                    // SAFETY: as above.
                    unsafe { libc::_exit(1) }
                }
                0 => Ok(Side::Session(reply)),
                // SAFETY: the intermediate's only job is to die, so the
                // grandchild is reparented to init.
                _ => unsafe { libc::_exit(0) },
            }
        }
        intermediate => {
            drop(write);
            let mut status: libc::c_int = 0;
            // SAFETY: reaping the intermediate, which exits immediately.
            unsafe { libc::waitpid(intermediate, &mut status, 0) };
            Ok(Side::Launcher(Handshake(read)))
        }
    }
}

/// What the grandchild reported.
#[derive(Debug, PartialEq, Eq)]
pub enum Ready {
    /// The harness started; its pid.
    Ok(u32),
    /// `code` is a stable snake_case code, `text` the detail.
    Fail { code: String, text: String },
}

impl Handshake {
    /// Block until the grandchild answers or the pipe closes.
    pub fn wait(self, log: &Path) -> Ready {
        let mut text = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match pty::read(&self.0, &mut buf) {
                Ok(0) => break,
                Ok(n) => text.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&text);
        let line = text.lines().next().unwrap_or("").trim();
        if let Some(pid) = line.strip_prefix("ok ") {
            if let Ok(pid) = pid.trim().parse::<u32>() {
                return Ready::Ok(pid);
            }
        } else if let Some(rest) = line.strip_prefix("fail ")
            && let Some((code, detail)) = rest.split_once(": ")
        {
            return Ready::Fail {
                code: code.to_owned(),
                text: detail.to_owned(),
            };
        }
        Ready::Fail {
            code: "supervisor_spawn_failed".to_owned(),
            text: format!(
                "the supervisor exited before reporting; its log is {}",
                log.display()
            ),
        }
    }
}

impl Reply {
    /// Report the harness pid. The launcher prints `ok <pid>` and exits 0.
    pub fn ok(&mut self, pid: u32) -> io::Result<()> {
        self.send(&format!("ok {pid}\n"))
    }

    /// Report a failure. The launcher prints `fail <code>: <text>`.
    pub fn fail(&mut self, code: &str, text: &str) -> io::Result<()> {
        let text = text.replace('\n', " ");
        self.send(&format!("fail {code}: {text}\n"))
    }

    fn send(&mut self, line: &str) -> io::Result<()> {
        pty::write_all(&self.0, line.as_bytes())
    }

    /// Close the pipe so the launcher stops waiting.
    pub fn close(self) {
        drop(self.0);
    }
}

/// Point the standard descriptors away from the launcher: stdin at
/// `/dev/null`, stdout and stderr at the log. Nothing the supervisor writes
/// may reach the terminal or the pipes that started it.
pub fn redirect_std(log: &std::fs::File) -> io::Result<()> {
    // SAFETY: a constant path, with the result checked.
    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };
    if null < 0 {
        return Err(io::Error::last_os_error());
    }
    let null = Fd::new(null);
    // SAFETY: both descriptors are open; `dup2` onto 0/1/2 replaces
    // whatever the launcher handed us.
    unsafe {
        libc::dup2(null.as_raw_fd(), 0);
        libc::dup2(log.as_raw_fd(), 1);
        libc::dup2(log.as_raw_fd(), 2);
    }
    Ok(())
}
