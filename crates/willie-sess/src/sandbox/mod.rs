//! Applying the plan: what must be true before the helper is spawned,
//! what this version applies, and how the helper's exit and the stop
//! ladder map back to the harness. The plan itself is data in
//! `willie_linux::sandbox`; this is the I/O around it.

pub mod inner;
#[cfg(target_os = "linux")]
pub mod landlock;
#[cfg(target_os = "linux")]
pub mod seccomp;

#[cfg(target_os = "linux")]
use std::time::Duration;
use std::{fmt, fs, io, path::Path};

use willie_core::session::SessionSpec;
// The re-exec stage and its Landlock applier live in this crate's own
// `inner` and `landlock` submodules, so the request/report types and the
// rule derivation cross-linked from `willie_linux` are pulled in by name
// rather than under a clashing module alias.
use willie_linux::sandbox::inner::{Request, Rlimits};
use willie_linux::sandbox::landlock::rules as landlock_rules;
use willie_linux::sandbox::{self as plan, bwrap};

/// Everything the supervisor needs to spawn the confined session, bar the
/// report descriptor it creates at spawn time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub plan: plan::Plan,
    pub request: Request,
}

#[derive(Debug)]
pub enum PrepareError {
    /// The spec names a harness this build does not know.
    HarnessUnknown(String),
    Plan(plan::PlanError),
    /// The namespace helper is not in the image.
    BackendMissing(String),
    /// A per-project cache directory could not be created.
    CacheDir {
        path: String,
        error: io::Error,
    },
    /// The harness binary or the workspace is not there. Checked here
    /// because inside the helper the same failure is a bare exit 1.
    Harness {
        step: &'static str,
        error: io::Error,
        path: String,
    },
    /// An extra path the policy's guard allowed as written, but which
    /// resolves into a guarded location, or which cannot be resolved at
    /// all. `willie-core` does no I/O and can see neither; here both are
    /// checked before anything is mounted.
    ExtraPath {
        path: String,
        detail: String,
    },
    /// A path the plan binds without tolerance is not on this machine.
    /// Checked here because inside the helper the same failure is a
    /// message on the session's terminal and a bare exit 1, which no
    /// log keeps.
    BindSource {
        path: String,
    },
}

impl PrepareError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::HarnessUnknown(_) | Self::Plan(_) => "spec_invalid",
            Self::BackendMissing(_) => "sandbox_backend_missing",
            Self::CacheDir { .. } | Self::BindSource { .. } => {
                "sandbox_apply_failed"
            }
            Self::Harness { .. } => "harness_exec_failed",
            Self::ExtraPath { .. } => "sandbox_profile_invalid",
        }
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HarnessUnknown(id) => write!(f, "unknown harness `{id}`"),
            Self::Plan(e) => write!(f, "{e}"),
            Self::BackendMissing(path) => {
                write!(f, "the namespace helper is missing: {path}")
            }
            Self::CacheDir { path, error } => {
                write!(f, "cannot create the cache directory {path}: {error}")
            }
            Self::Harness { step, error, path } => {
                write!(f, "{step}: {error} ({path})")
            }
            Self::ExtraPath { path, detail } => write!(f, "`{path}`: {detail}"),
            Self::BindSource { path } => {
                write!(f, "the sandbox needs {path}, which is not there")
            }
        }
    }
}

impl std::error::Error for PrepareError {}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Point one extra path's bind at `resolved`, leaving its destination
/// as configured. The plan appends one bind per extra path after
/// everything else, so the last op with that destination is that bind.
fn rebind_source(ops: &mut [plan::Op], destination: &str, resolved: &str) {
    for op in ops.iter_mut().rev() {
        if let plan::Op::Bind { src, dest, .. } = op
            && dest == destination
        {
            *src = resolved.to_owned();
            return;
        }
    }
}

/// Resolve the plan and check what the helper would otherwise report as
/// a bare exit: the helper exists, the harness binary is executable,
/// the workspace is a directory, and every source the plan binds
/// without tolerance is on this machine. The per-project caches are
/// created first, because the plan both creates and binds those — a
/// session never starts without them. `helper` is injected so the
/// checks are testable without the image.
pub fn prepare(
    spec: &SessionSpec,
    helper: &Path,
    inner_exe: &str,
) -> Result<Prepared, PrepareError> {
    let harness = willie_harness::registry()
        .into_iter()
        .find(|h| h.id() == spec.harness)
        .ok_or_else(|| PrepareError::HarnessUnknown(spec.harness.clone()))?;
    let mut plan = plan::plan(spec, harness.as_ref(), inner_exe)
        .map_err(PrepareError::Plan)?;
    if !helper.is_file() {
        return Err(PrepareError::BackendMissing(
            helper.to_string_lossy().into_owned(),
        ));
    }
    let binary = plan.argv.first().cloned().unwrap_or_default();
    if !is_executable_file(Path::new(&binary)) {
        return Err(PrepareError::Harness {
            step: crate::EXEC_STEP,
            error: not_found(),
            path: binary,
        });
    }
    if !Path::new(&plan.workspace).is_dir() {
        return Err(PrepareError::Harness {
            step: crate::WORKSPACE_STEP,
            error: not_found(),
            path: plan.workspace.clone(),
        });
    }
    // The policy's guard is lexical, because `willie-core` does no I/O:
    // an extra path that is a symbolic link into a guarded location
    // passes it as written. Here the resolved path is available, so the
    // same guard runs again on what will actually be mounted.
    for extra in &spec.capabilities.extra_paths {
        let resolved = fs::canonicalize(&extra.path).map_err(|e| {
            PrepareError::ExtraPath {
                path: extra.path.clone(),
                detail: format!("cannot be resolved: {e}"),
            }
        })?;
        let resolved = resolved.to_string_lossy().into_owned();
        if resolved != extra.path
            && let Some(reason) =
                willie_core::sandbox::guard_extra_path(&resolved, &plan.home)
        {
            return Err(PrepareError::ExtraPath {
                path: extra.path.clone(),
                detail: format!(
                    "resolves to `{resolved}`, which cannot be an extra \
                     path: {reason}"
                ),
            });
        }
        // The helper mounts what this check resolved, not the path as
        // written: a last component that is a symbolic link can be
        // re-pointed between the two, and a link inside the project is
        // writable by every session on it. The destination stays as the
        // policy configured it.
        rebind_source(&mut plan.ops, &extra.path, &resolved);
    }
    // The caches first: the plan both creates and binds those, so they
    // have to exist before the sources are checked.
    for dir in &plan.ensure_dirs {
        fs::create_dir_all(dir).map_err(|error| PrepareError::CacheDir {
            path: dir.clone(),
            error,
        })?;
    }
    // Every remaining source the plan binds without tolerance. A missing
    // one kills the helper with a message that goes to the session's
    // terminal and nowhere else, so the whole class is a coded refusal
    // here instead. A tolerant bind is skipped: that is what tolerant
    // means — the layout is fixed, the machine is not.
    for op in &plan.ops {
        if let plan::Op::Bind {
            src,
            optional: false,
            ..
        } = op
            && !Path::new(src).exists()
        {
            return Err(PrepareError::BindSource { path: src.clone() });
        }
    }
    // The vector is rendered in `session_main`, where the report
    // descriptor exists; here the plan and the request the stage will read
    // are all that is settled. The request carries the harness argv the
    // stage execs, the default limits it sets and the Landlock rules,
    // derived from the plan as finally mounted so the writable set has
    // one source. The helper verified above becomes the vector's
    // `argv[0]` there.
    Ok(Prepared {
        request: Request {
            argv: plan.argv.clone(),
            rlimits: Rlimits::DEFAULT,
            landlock: landlock_rules(&plan),
        },
        plan,
    })
}

/// Render the helper's argument vector for this prepared session, running
/// the inner stage on `inner_fd`. `helper` is the namespace helper the
/// supervisor verified; the vector's `argv[0]` is it.
#[must_use]
pub fn helper_argv(
    prepared: &Prepared,
    helper: &Path,
    inner_fd: i32,
) -> Vec<String> {
    let mut argv = bwrap::argv(&prepared.plan, inner_fd);
    if let Some(first) = argv.first_mut() {
        *first = helper.to_string_lossy().into_owned();
    }
    argv
}

/// Where the re-executed supervisor lives: this binary's own resolved
/// path, so a test drives the staged one and production the installed
/// one. `WILLIE_SESS_INNER_EXE` overrides it, test-only.
#[cfg(target_os = "linux")]
#[must_use]
pub fn inner_exe_path() -> String {
    if let Some(p) = std::env::var_os("WILLIE_SESS_INNER_EXE") {
        return p.to_string_lossy().into_owned();
    }
    fs::read_link("/proc/self/exe")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| willie_linux::paths::SUPERVISOR_BIN.to_owned())
}

/// A socketpair for the report: (supervisor end, stage end).
#[cfg(target_os = "linux")]
pub fn report_socket() -> io::Result<(
    std::os::unix::net::UnixStream,
    std::os::unix::net::UnixStream,
)> {
    std::os::unix::net::UnixStream::pair()
}

/// Hand the stage its request over the supervisor's socket end: one
/// newline-framed JSON line, the frame the stage blocks reading before it
/// reports. Written after the spawn, so the bytes wait in the socket
/// buffer until the re-executed supervisor reads them.
#[cfg(target_os = "linux")]
pub fn write_request(
    sock: &mut std::os::unix::net::UnixStream,
    request: &Request,
) -> io::Result<()> {
    use std::io::Write;
    let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
    line.push(b'\n');
    sock.write_all(&line)?;
    sock.flush()
}

/// Room for one control message carrying one descriptor, by the kernel's
/// own arithmetic. It is more than one int: `CMSG_SPACE` rounds the
/// payload up to a long, so two ints fit without the kernel flagging
/// truncation, and the receiver counts what arrived rather than assume.
#[cfg(target_os = "linux")]
// SAFETY: a pure size computation.
const CONTROL_SPACE: usize =
    unsafe { libc::CMSG_SPACE(size_of::<libc::c_int>() as u32) } as usize;

/// The control buffer for one `SCM_RIGHTS` message. A `cmsghdr` is
/// written and read through a pointer into it, so it is aligned the way
/// the kernel aligns control messages: to a long.
#[cfg(target_os = "linux")]
#[repr(C, align(8))]
struct Control([u8; CONTROL_SPACE]);

/// `sendmsg` on `sock`: `bytes` as the one buffer and, when `carry` is
/// `Some`, that descriptor as the one `SCM_RIGHTS` control message.
/// Returns how many bytes the socket took; the descriptor travels with
/// the first of them.
#[cfg(target_os = "linux")]
fn sendmsg_with_fd(
    sock: libc::c_int,
    bytes: &[u8],
    carry: Option<libc::c_int>,
) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    let mut control = Control([0; CONTROL_SPACE]);
    // SAFETY: all-zero is a valid msghdr — no name, no data, no control —
    // and every pointer set below outlives the call.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = carry {
        msg.msg_control = control.0.as_mut_ptr().cast();
        msg.msg_controllen = CONTROL_SPACE as _;
        // SAFETY: `msg_control` is `control`: CMSG_SPACE(int) bytes,
        // long-aligned, so the first header lies inside it, aligned, and
        // its data slot has room for one int.
        unsafe {
            let hdr = libc::CMSG_FIRSTHDR(&raw const msg);
            if hdr.is_null() {
                return Err(io::Error::other(
                    "no room for the descriptor's control message",
                ));
            }
            (*hdr).cmsg_level = libc::SOL_SOCKET;
            (*hdr).cmsg_type = libc::SCM_RIGHTS;
            (*hdr).cmsg_len =
                libc::CMSG_LEN(size_of::<libc::c_int>() as u32) as _;
            std::ptr::write_unaligned(
                libc::CMSG_DATA(hdr).cast::<libc::c_int>(),
                fd,
            );
        }
    }
    loop {
        // SAFETY: msg is fully initialised and the buffers it points at
        // live for the call. A peer that went away is an error to report,
        // never a signal.
        let n =
            unsafe { libc::sendmsg(sock, &raw const msg, libc::MSG_NOSIGNAL) };
        if let Ok(n) = usize::try_from(n) {
            return Ok(n);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Write one report line over the stage's socket end, framed by its
/// newline, with at most one descriptor riding alongside as `SCM_RIGHTS`.
/// The stage reports `Applied` this way so the filter's listener reaches
/// the supervisor together with the words that describe it; `None` is the
/// plain line, and nothing else changes on the wire.
#[cfg(target_os = "linux")]
pub fn send_report_with_fd(
    sock: &mut std::os::unix::net::UnixStream,
    line: &[u8],
    fd: Option<i32>,
) -> io::Result<()> {
    use std::{io::Write, os::fd::AsRawFd};
    let mut framed = Vec::with_capacity(line.len() + 1);
    framed.extend_from_slice(line);
    framed.push(b'\n');
    let sent = sendmsg_with_fd(sock.as_raw_fd(), &framed, fd)?;
    // A stream socket may take fewer bytes than offered. The descriptor
    // went with the first of them, so the rest is a plain write.
    sock.write_all(framed.get(sent..).unwrap_or_default())?;
    sock.flush()
}

/// What the first `recvmsg` on the report socket delivered.
#[cfg(target_os = "linux")]
struct FirstRead {
    /// How much of the buffer was filled; zero is EOF.
    len: usize,
    /// Every descriptor the control message carried, each owned so none
    /// leaks whatever the caller decides about them.
    fds: Vec<std::os::fd::OwnedFd>,
    /// The kernel had more ancillary data than the buffer holds.
    truncated: bool,
}

/// One `recvmsg` into `buf`, with room for a control message carrying
/// one descriptor. Over a stream socket the ancillary data comes with the
/// first bytes of the message it was sent with, so this is the read that
/// captures it; the bytes after those carry none.
#[cfg(target_os = "linux")]
fn recvmsg_with_fd(sock: libc::c_int, buf: &mut [u8]) -> io::Result<FirstRead> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut control = Control([0; CONTROL_SPACE]);
    // SAFETY: all-zero is a valid msghdr; every pointer set below outlives
    // the call.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.0.as_mut_ptr().cast();
    msg.msg_controllen = CONTROL_SPACE as _;
    let len = loop {
        // SAFETY: msg is fully initialised and its buffers live for the
        // call. A received descriptor is close-on-exec from the start, so
        // it never leaks into anything this process executes.
        let n = unsafe {
            libc::recvmsg(sock, &raw mut msg, libc::MSG_CMSG_CLOEXEC)
        };
        if let Ok(n) = usize::try_from(n) {
            break n;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let truncated = msg.msg_flags & libc::MSG_CTRUNC != 0;
    let mut fds = Vec::new();
    // SAFETY: the kernel wrote `msg_controllen` bytes of control data into
    // `control`, no more than it holds; CMSG_FIRSTHDR is null unless a
    // whole header is among them, and `cmsg_len` is clamped to what was
    // written before the data it describes is read. Every int read is a
    // descriptor the kernel installed in this process for us to own.
    unsafe {
        let hdr = libc::CMSG_FIRSTHDR(&raw const msg);
        if !hdr.is_null()
            && (*hdr).cmsg_level == libc::SOL_SOCKET
            && (*hdr).cmsg_type == libc::SCM_RIGHTS
        {
            let header = libc::CMSG_LEN(0) as usize;
            let written = (msg.msg_controllen as usize).min(CONTROL_SPACE);
            let total = ((*hdr).cmsg_len as usize).min(written);
            let count = total.saturating_sub(header) / size_of::<libc::c_int>();
            let data = libc::CMSG_DATA(hdr).cast::<libc::c_int>();
            for i in 0..count {
                fds.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(
                    data.add(i),
                )));
            }
        }
    }
    Ok(FirstRead {
        len,
        fds,
        truncated,
    })
}

/// How long the supervisor waits for the stage's report before refusing.
/// `WILLIE_SESS_HARNESS_WAIT_MS` shortens it for the tests; default two
/// seconds, far inside the daemon's ten-second readiness budget. An
/// overriding value is floored to one millisecond: `set_read_timeout`
/// rejects `Duration::ZERO` with `InvalidInput` and leaves the socket
/// blocking, so a `0` override still means "time out almost at once" —
/// its intent — rather than block until the daemon's own budget runs out.
#[cfg(target_os = "linux")]
#[must_use]
pub fn report_wait() -> Duration {
    std::env::var("WILLIE_SESS_HARNESS_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(Duration::from_secs(2), |v| {
            Duration::from_millis(v).max(Duration::from_millis(1))
        })
}

/// What reading the stage's report resolved to.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub enum ReportOutcome {
    Applied {
        mechanisms: Vec<String>,
        unavailable: Vec<String>,
        /// The syscall filter's notification listener, when the stage
        /// sent one alongside the line.
        listener: Option<std::os::fd::OwnedFd>,
    },
    Refused {
        code: String,
        message: String,
    },
    /// EOF before any report: the helper died building the namespace.
    HelperGone,
    /// The ceiling passed with nothing read.
    TimedOut,
}

/// Read one newline-framed report from the supervisor's socket end, with a
/// deadline, together with any descriptor the stage sent alongside it. EOF
/// with no line is `HelperGone`; the deadline is `TimedOut`; an
/// unparseable line is a refusal, and so is ancillary data that is not
/// exactly what one listener looks like — a descriptor that cannot be
/// trusted is closed, never kept.
#[cfg(target_os = "linux")]
#[must_use]
pub fn read_report(
    sock: &mut std::os::unix::net::UnixStream,
    within: Duration,
) -> ReportOutcome {
    use std::{io::Read, os::fd::AsRawFd};

    use willie_linux::sandbox::inner::Report;
    let _ = sock.set_read_timeout(Some(within));
    // The first read is the one that carries the descriptor, so it is a
    // recvmsg with room for one, into a buffer most reports fit whole.
    let mut buf = [0u8; 512];
    let FirstRead {
        len,
        mut fds,
        truncated,
    } = match recvmsg_with_fd(sock.as_raw_fd(), &mut buf) {
        Ok(first) if first.len == 0 => return ReportOutcome::HelperGone,
        Ok(first) => first,
        Err(e)
            if e.kind() == io::ErrorKind::WouldBlock
                || e.kind() == io::ErrorKind::TimedOut =>
        {
            return ReportOutcome::TimedOut;
        }
        Err(_) => return ReportOutcome::HelperGone,
    };
    if truncated {
        return ReportOutcome::Refused {
            code: "sandbox_apply_failed".into(),
            message: "the sandbox report carried a truncated control \
                      message"
                .into(),
        };
    }
    if fds.len() > 1 {
        return ReportOutcome::Refused {
            code: "sandbox_apply_failed".into(),
            message: format!(
                "the sandbox report carried {} descriptors, where at most \
                 one belongs",
                fds.len()
            ),
        };
    }
    let listener = fds.pop();
    // The line runs to its newline. Whatever the first read returned past
    // it is nothing the stage sends — the report is its last word — and
    // whatever is still to come carries no ancillary data, so the rest is
    // plain reads.
    let head = buf.get(..len).unwrap_or_default();
    let mut line = head
        .split(|&b| b == b'\n')
        .next()
        .unwrap_or_default()
        .to_vec();
    if !head.contains(&b'\n') {
        let mut byte = [0u8; 1];
        loop {
            match sock.read(&mut byte) {
                Ok(0) => return ReportOutcome::HelperGone,
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) => line.push(byte[0]),
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    return ReportOutcome::TimedOut;
                }
                Err(_) => return ReportOutcome::HelperGone,
            }
        }
    }
    match serde_json::from_slice::<Report>(&line) {
        Ok(Report::Applied {
            mechanisms,
            unavailable,
        }) => ReportOutcome::Applied {
            mechanisms,
            unavailable,
            listener,
        },
        Ok(Report::Refused { code, message }) => {
            ReportOutcome::Refused { code, message }
        }
        Err(e) => ReportOutcome::Refused {
            code: "sandbox_apply_failed".into(),
            message: format!("unreadable sandbox report: {e}"),
        },
    }
}

/// Where the namespace helper lives. `WILLIE_SESS_HELPER_BIN` points
/// the supervisor at another one so a test can drive the refusal path
/// without a kernel that refuses; test-only, like the stop grace and
/// the report wait.
#[must_use]
pub fn helper_path() -> std::path::PathBuf {
    std::env::var_os("WILLIE_SESS_HELPER_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(bwrap::BWRAP))
}

/// The helper reports a harness killed by signal `n` as exit `128 + n`
/// (decision 0016). Put the signal back so the record says what
/// happened; a harness that itself exits in that range is recorded as
/// a signal death, the shell's own convention.
#[must_use]
pub fn helper_exit(
    code: Option<i32>,
    signal: Option<i32>,
) -> (Option<i32>, Option<i32>) {
    match (code, signal) {
        (Some(c), None) if (129..=192).contains(&c) => (None, Some(c - 128)),
        other => other,
    }
}

/// Every pid a `/proc/<pid>/task/<pid>/children` file names, or none if
/// it holds anything that is not a pid.
#[must_use]
pub fn parse_child_pids(text: &str) -> Option<Vec<i32>> {
    text.split_whitespace()
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

/// The one pid such a file names, or none: the helper's monitor has one
/// child (the reaper) and the reaper one child (the harness); any other
/// shape is not what we launched.
#[must_use]
pub fn parse_children(text: &str) -> Option<i32> {
    match parse_child_pids(text)?.as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// Whether such a file still names `harness`. This is what tells a
/// *noisy* shape from a *gone* one: the reaper is pid 1 inside the
/// session's namespace, so an orphaned grandchild is reparented onto it
/// and hides which of its children is the harness, while the harness is
/// still one of them. A file that no longer names it means the harness
/// has exited, and its number is then free for any process in the
/// distribution to take.
#[must_use]
pub fn children_include(text: &str, harness: i32) -> bool {
    parse_child_pids(text).is_some_and(|ids| ids.contains(&harness))
}

/// The harness process behind the helper's monitor, resolved when a
/// signal has to reach the harness and not the monitor (decision
/// 0016: the monitor dies on the polite signals and takes the session
/// with it). `None` when the shape is gone: the caller then signals
/// the group, which ends the session — the closed direction.
#[cfg(target_os = "linux")]
#[must_use]
pub fn harness_pid(monitor: libc::pid_t) -> Option<libc::pid_t> {
    let reaper = only_child(monitor)?;
    only_child(reaper)
}

/// Whether the harness a session recorded at start-up is still one of
/// the reaper's children, which is the only case where a pid resolved
/// once may be signalled later: the shape is noisy rather than gone
/// (see [`children_include`]). False whenever the reaper cannot be
/// reached or no longer names it — the harness has exited, the number
/// is stale, and signalling it would reach whatever holds it now.
#[cfg(target_os = "linux")]
#[must_use]
pub fn harness_still_behind(
    monitor: libc::pid_t,
    harness: libc::pid_t,
) -> bool {
    let Some(reaper) = only_child(monitor) else {
        return false;
    };
    fs::read_to_string(format!("/proc/{reaper}/task/{reaper}/children"))
        .is_ok_and(|text| children_include(&text, harness))
}

/// At most this much of what the helper said reaches the record. The
/// log is append-only and read by people; one runaway helper must not
/// fill a screen of it.
const HELPER_WORDS_LIMIT: usize = 400;

/// How many bytes of the terminal are read back when the helper
/// refused. Generous against the limit above, because the helper may
/// have written control sequences the words are buried in.
pub const HELPER_DRAIN: usize = 16 * 1024;

/// What the helper said, as the one line an event can carry. A terminal
/// turns each newline into a carriage return and a newline, and a helper
/// may complain more than once, so the lines are joined and the empty
/// ones dropped.
fn helper_words(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let joined = text
        .lines()
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() > HELPER_WORDS_LIMIT {
        let cut: String = joined.chars().take(HELPER_WORDS_LIMIT).collect();
        return Some(format!("{cut}…"));
    }
    Some(joined)
}

/// The message for a helper that refused after it was executed.
///
/// Everything the supervisor can see before the helper runs is already
/// a coded refusal (`prepare`). What is left is the helper refusing
/// while it builds the namespace, and its only channel is the session's
/// terminal, which nothing is attached to yet. Carrying its words here
/// is the difference between a session that says why it could not start
/// and one that merely appears and disappears.
#[must_use]
pub fn apply_failure(
    raw: &[u8],
    code: Option<i32>,
    signal: Option<i32>,
) -> String {
    match (helper_words(raw), code, signal) {
        (Some(words), _, _) => {
            format!("the namespace helper refused: {words}")
        }
        (None, Some(code), _) => format!(
            "the namespace helper exited with {code} before the harness \
             started, and said nothing"
        ),
        (None, None, Some(signal)) => format!(
            "the namespace helper was killed by signal {signal} before the \
             harness started"
        ),
        (None, None, None) => "the namespace helper ended before the harness \
             started, and said nothing"
            .to_owned(),
    }
}

#[cfg(target_os = "linux")]
fn only_child(pid: libc::pid_t) -> Option<libc::pid_t> {
    let text =
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
    parse_children(&text)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf};

    use willie_core::{
        id::{ProjectId, SessionId},
        sandbox::CapabilitySet,
        session::{SessionKind, SessionSpec},
    };

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-sandbox-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::write(path, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }

    /// A touched file standing in for the re-executed supervisor. Its
    /// read-only bind is non-tolerant, so `prepare` checks it exists like
    /// any other source; the tests point it at a real file.
    fn inner_bin(root: &Path) -> PathBuf {
        let p = root.join("willie-sess");
        touch(&p);
        p
    }

    /// A spec whose paths all live under `root`, with the per-project
    /// caches on so `prepare` has directories to create.
    fn spec_under(root: &Path, binary: &str, workspace: &str) -> SessionSpec {
        let mut env = BTreeMap::new();
        env.insert("HOME".to_owned(), root.to_string_lossy().into_owned());
        SessionSpec {
            id: SessionId::new(),
            project_id: ProjectId::new(),
            harness: "claude-code".into(),
            workspace: workspace.to_owned(),
            socket: root.join("s.sock").to_string_lossy().into_owned(),
            argv: vec![binary.to_owned()],
            env,
            created_at: "1".into(),
            willie_version: "0".into(),
            resumed_from: None,
            kind: SessionKind::Agent,
            capabilities: CapabilitySet {
                caches_rw: true,
                ..CapabilitySet::default()
            },
        }
    }

    #[test]
    fn prepare_creates_the_per_project_caches_and_carries_the_request() {
        let root = scratch("ok");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let inner = inner_bin(&root);
        let ws = root.join("ws");
        fs::create_dir_all(&ws).unwrap();
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &ws.to_string_lossy());

        let prepared = prepare(&spec, &helper, &inner.to_string_lossy())
            .expect("prepared");

        // The stage execs the harness argv and sets the default limits.
        assert_eq!(
            prepared.request.argv.last().unwrap(),
            &bin.to_string_lossy()
        );
        assert_eq!(prepared.request.rlimits, Rlimits::DEFAULT);
        // The stage grants writing where the plan mounted read-write —
        // the workspace, at its own path — and under the base's `/tmp`;
        // the set is the plan's, not a second derivation.
        let write = &prepared.request.landlock.write;
        assert!(
            write.iter().any(|p| p == &ws.to_string_lossy()),
            "{write:?}"
        );
        assert!(write.iter().any(|p| p == "/tmp"), "{write:?}");
        assert_eq!(prepared.request.landlock, landlock_rules(&prepared.plan));
        let caches = root
            .join(".willie")
            .join("caches")
            .join(spec.project_id.to_string());
        for name in ["npm", "nuget", "cache"] {
            assert!(caches.join(name).is_dir(), "{name}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_helper_is_sandbox_backend_missing_before_anything_is_created()
    {
        let root = scratch("nohelper");
        let bin = root.join("claude");
        touch(&bin);
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());

        let err = prepare(&spec, &root.join("absent"), &bin.to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "sandbox_backend_missing");
        assert!(!root.join(".willie").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_harness_binary_is_harness_exec_failed_naming_it() {
        let root = scratch("nobin");
        let helper = root.join("bwrap");
        touch(&helper);
        let spec =
            spec_under(&root, "/nonexistent/claude", &root.to_string_lossy());

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "harness_exec_failed");
        assert!(
            err.to_string().starts_with("cannot execute the harness"),
            "{err}"
        );
        assert!(err.to_string().contains("/nonexistent/claude"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_workspace_is_harness_exec_failed_naming_the_directory() {
        let root = scratch("nows");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let gone = root.join("gone");
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &gone.to_string_lossy());

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "harness_exec_failed");
        assert!(
            err.to_string().starts_with("cannot enter the workspace"),
            "{err}"
        );
        assert!(err.to_string().contains("gone"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// The policy's guard is lexical, because `willie-core` does no I/O,
    /// so a symbolic link into a guarded location passes it as written.
    /// The supervisor sees where it goes and refuses what would actually
    /// be mounted.
    #[cfg(unix)]
    #[test]
    fn an_extra_path_that_resolves_into_a_guarded_location_is_refused() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("symlink");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let link = root.join("shared");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: link.to_string_lossy().into_owned(),
            mode: PathMode::Ro,
        }];

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.to_string().contains("/etc"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// The guard resolves the path, so the mount must use what it
    /// resolved: a last component that is a symbolic link can be
    /// re-pointed between the check and the mount, and a link inside
    /// the project is writable by every session on it.
    #[cfg(unix)]
    #[test]
    fn an_extra_path_is_bound_from_the_source_the_check_resolved() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("resolvedsrc");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = root.join("shared");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let configured = link.to_string_lossy().into_owned();
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: configured.clone(),
            mode: PathMode::Ro,
        }];

        let prepared =
            prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
                .expect("prepared");

        let argv = helper_argv(&prepared, &helper, 4);
        let bind = argv
            .windows(3)
            .find(|w| w[0] == "--ro-bind" && w[2] == configured)
            .expect("the extra path's bind");
        assert_eq!(bind[1], fs::canonicalize(&real).unwrap().to_string_lossy());
        assert_ne!(bind[1], configured);
        let _ = fs::remove_dir_all(&root);
    }

    /// An extra path that is not there refuses the session here, with a
    /// code, rather than letting the helper fail with its own message
    /// from inside a namespace nobody is watching.
    #[test]
    fn an_extra_path_that_does_not_exist_is_refused() {
        use willie_core::sandbox::{ExtraPath, PathMode};

        let root = scratch("noextra");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.extra_paths = vec![ExtraPath {
            path: root.join("absent").to_string_lossy().into_owned(),
            mode: PathMode::Ro,
        }];

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "sandbox_profile_invalid");
        assert!(err.to_string().contains("cannot be resolved"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_harness_id_is_spec_invalid() {
        let root = scratch("unknown");
        let helper = root.join("bwrap");
        touch(&helper);
        let mut spec = spec_under(&root, "/x", &root.to_string_lossy());
        spec.harness = "not-a-harness".into();

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "spec_invalid");
        assert!(err.to_string().contains("not-a-harness"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_spec_without_a_home_is_spec_invalid() {
        let root = scratch("nohome");
        let helper = root.join("bwrap");
        touch(&helper);
        let mut spec = spec_under(&root, "/x", &root.to_string_lossy());
        spec.env.remove("HOME");

        assert_eq!(
            prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
                .unwrap_err()
                .code(),
            "spec_invalid"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The helper reports a harness killed by signal n as exit 128 + n;
    /// the record should say the signal, as it did before the helper.
    #[test]
    fn a_helper_exit_above_128_is_the_harness_signal() {
        assert_eq!(helper_exit(Some(143), None), (None, Some(15)));
        assert_eq!(helper_exit(Some(130), None), (None, Some(2)));
        assert_eq!(helper_exit(Some(137), None), (None, Some(9)));
    }

    #[test]
    fn an_ordinary_exit_or_a_direct_signal_passes_through() {
        assert_eq!(helper_exit(Some(0), None), (Some(0), None));
        assert_eq!(helper_exit(Some(7), None), (Some(7), None));
        assert_eq!(helper_exit(Some(128), None), (Some(128), None));
        assert_eq!(helper_exit(Some(193), None), (Some(193), None));
        assert_eq!(helper_exit(None, Some(9)), (None, Some(9)));
    }

    /// The helper dies with a message on the session's terminal and a
    /// bare exit when a source it must bind is not there, so the whole
    /// class is refused here, by name, before any process exists.
    #[test]
    fn a_missing_non_tolerant_bind_source_is_sandbox_apply_failed_naming_it() {
        let root = scratch("nobind");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.git_identity = true;

        let err = prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
            .unwrap_err();

        assert_eq!(err.code(), "sandbox_apply_failed");
        assert!(err.to_string().contains(".gitconfig"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    /// A tool root the machine has not installed is bound tolerantly, so
    /// its absence is not a refusal: the layout is fixed, the machine is
    /// not.
    #[test]
    fn a_missing_tolerant_bind_source_is_not_a_refusal() {
        let root = scratch("tolerant");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let mut spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());
        spec.capabilities.tools_ro = true;

        assert!(
            prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
                .is_ok()
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The helper's own complaint goes to the terminal, which is the
    /// session's only output channel and which nothing is attached to
    /// when it refuses while building the namespace. Carrying it into
    /// the record is the difference between a session that says why it
    /// could not start and one that merely disappears.
    #[test]
    fn a_refusal_carries_the_helper_s_own_words() {
        let raw = b"bwrap: Can't mount on symlink destination /home/w/.local/bin/claude\r\n";

        let text = apply_failure(raw, Some(1), None);

        assert!(
            text.contains("Can't mount on symlink destination"),
            "{text}"
        );
        assert!(text.contains("/home/w/.local/bin/claude"), "{text}");
        assert!(!text.contains('\r'), "{text}");
        assert!(!text.contains('\n'), "{text}");
    }

    /// A terminal turns every newline into a carriage return and a
    /// newline, and a helper may say several things. The record takes
    /// one line, so the lines are joined and the blank ones dropped.
    #[test]
    fn several_lines_become_one_and_blank_ones_are_dropped() {
        let raw = b"bwrap: first\r\n\r\nbwrap: second\r\n";

        let text = apply_failure(raw, Some(1), None);

        assert!(text.contains("bwrap: first; bwrap: second"), "{text}");
    }

    /// A helper that says nothing at all still has to produce a record
    /// someone can act on, so the exit stands in for the words.
    #[test]
    fn a_silent_refusal_is_reported_by_its_exit() {
        let text = apply_failure(b"", Some(1), None);

        assert!(text.contains('1'), "{text}");
        assert!(text.contains("before the harness"), "{text}");

        let killed = apply_failure(b"   \r\n", None, Some(9));

        assert!(killed.contains('9'), "{killed}");
    }

    /// An event log is append-only and read by people, so one runaway
    /// helper must not put a screenful into it.
    #[test]
    fn a_helper_that_will_not_stop_talking_is_cut_short() {
        let raw = "bwrap: ".repeat(400);

        let text = apply_failure(raw.as_bytes(), Some(1), None);

        assert!(text.len() < 600, "{}", text.len());
        assert!(text.ends_with('…'), "{text}");
    }

    /// The path that was checked is the path that runs: `prepare`
    /// verifies a helper and the vector must then execute that one, or
    /// the check answers for a different binary than the launch.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_vector_runs_the_helper_that_was_verified() {
        let root = scratch("helper-argv");
        let helper = root.join("bwrap");
        touch(&helper);
        let bin = root.join("claude");
        touch(&bin);
        let spec =
            spec_under(&root, &bin.to_string_lossy(), &root.to_string_lossy());

        let prepared =
            prepare(&spec, &helper, &inner_bin(&root).to_string_lossy())
                .expect("prepared");

        let argv = helper_argv(&prepared, &helper, 4);
        assert_eq!(argv[0], helper.to_string_lossy());
        let _ = fs::remove_dir_all(&root);
    }

    /// The helper's monitor has one child, the reaper; the reaper has one
    /// child, the harness. Anything else is not the shape we launched.
    #[test]
    fn the_children_file_yields_exactly_one_pid_or_nothing() {
        assert_eq!(parse_children("4242 \n"), Some(4242));
        assert_eq!(parse_children("4242"), Some(4242));
        assert_eq!(parse_children(""), None);
        assert_eq!(parse_children("1 2 "), None);
        assert_eq!(parse_children("x"), None);
    }

    /// The stop ladder may fall back on the pid it resolved at start-up
    /// only while the shape is noisy — the reaper collected an orphan
    /// and has more than one child — never once the harness is gone,
    /// when the number is free for anything in the distribution to
    /// take.
    #[test]
    fn a_cached_harness_counts_only_while_the_children_file_still_names_it() {
        assert!(children_include("41 42 43\n", 42));
        assert!(children_include("42\n", 42));
        assert!(!children_include("41 43\n", 42));
        assert!(!children_include("", 42));
        assert!(!children_include("42 x", 42));
        assert_eq!(parse_child_pids("41 42"), Some(vec![41, 42]));
        assert_eq!(parse_child_pids("41 x"), None);
    }

    /// A pipe as two owned ends, so a test that fails midway closes both.
    #[cfg(target_os = "linux")]
    fn pipe() -> (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
        use std::os::fd::{FromRawFd, OwnedFd};
        let mut ends = [0 as libc::c_int; 2];
        // SAFETY: pipe writes two open descriptors into the array.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        // SAFETY: both ends are open, ours, and each is owned exactly once.
        unsafe {
            (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1]))
        }
    }

    /// Send `line` (framed) with every one of `fds` in a single
    /// `SCM_RIGHTS` message: the sender the production side never is,
    /// so the receiver's fail-closed branches can be reached.
    #[cfg(target_os = "linux")]
    fn send_with_descriptors(
        sock: &std::os::unix::net::UnixStream,
        line: &[u8],
        fds: &[libc::c_int],
    ) {
        use std::os::fd::AsRawFd;
        let mut framed = line.to_vec();
        framed.push(b'\n');
        let payload = size_of_val(fds);
        // SAFETY: pure size computations.
        let space = unsafe { libc::CMSG_SPACE(payload as u32) } as usize;
        // u64 cells: a control buffer is aligned to a long.
        let mut control = vec![0u64; space.div_ceil(size_of::<u64>())];
        let mut iov = libc::iovec {
            iov_base: framed.as_mut_ptr().cast(),
            iov_len: framed.len(),
        };
        // SAFETY: all-zero is a valid empty msghdr; the pointers set below
        // outlive the call.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        // SAFETY: `control` has CMSG_SPACE(payload) bytes, so the first
        // header and `fds.len()` ints of data lie inside it.
        unsafe {
            let hdr = libc::CMSG_FIRSTHDR(&msg);
            assert!(!hdr.is_null());
            (*hdr).cmsg_level = libc::SOL_SOCKET;
            (*hdr).cmsg_type = libc::SCM_RIGHTS;
            (*hdr).cmsg_len = libc::CMSG_LEN(payload as u32) as _;
            std::ptr::copy_nonoverlapping(
                fds.as_ptr(),
                libc::CMSG_DATA(hdr).cast::<libc::c_int>(),
                fds.len(),
            );
        }
        // SAFETY: msg is fully initialised.
        let sent = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, 0) };
        assert_eq!(usize::try_from(sent).ok(), Some(framed.len()));
    }

    /// The report channel can carry a descriptor: one end of a pipe sent
    /// as `SCM_RIGHTS` with a report line arrives on the other socket
    /// together with the line, and is the live pipe end — a byte written
    /// into the pipe comes out of the received descriptor.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_report_channel_round_trips_a_descriptor() {
        use std::{
            fs::File,
            io::{Read, Write},
            os::fd::AsRawFd,
        };

        let (mut a, mut b) = report_socket().unwrap();
        let (read_end, write_end) = pipe();

        send_report_with_fd(
            &mut a,
            br#"{"result":"applied","mechanisms":["x"]}"#,
            Some(read_end.as_raw_fd()),
        )
        .unwrap();
        // The sender's copy goes away: the received one must stand alone.
        drop(read_end);

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Applied {
            mechanisms,
            listener,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(mechanisms, vec!["x".to_owned()]);
        let listener = listener.expect("a descriptor with the report");
        File::from(write_end).write_all(b"z").unwrap();
        let mut got = [0u8; 1];
        File::from(listener).read_exact(&mut got).unwrap();
        assert_eq!(got[0], b'z');
    }

    /// The socket is a stream, so the descriptor arrives with the first
    /// bytes, not with the newline: a line longer than the first read
    /// still comes through whole, and the descriptor with it.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_report_longer_than_the_first_read_arrives_whole_with_its_descriptor() {
        use std::os::fd::AsRawFd;

        let (mut a, mut b) = report_socket().unwrap();
        let (read_end, _write_end) = pipe();
        let mechanisms: Vec<String> =
            (0..200).map(|i| format!("mechanism-{i:04}")).collect();
        let line = serde_json::to_vec(
            &willie_linux::sandbox::inner::Report::Applied {
                mechanisms: mechanisms.clone(),
                unavailable: vec![],
            },
        )
        .unwrap();
        assert!(line.len() > 2048, "{}", line.len());

        send_report_with_fd(&mut a, &line, Some(read_end.as_raw_fd())).unwrap();

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Applied {
            mechanisms: got,
            listener,
            ..
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(got, mechanisms);
        assert!(listener.is_some());
    }

    /// Without a descriptor the channel is what it was: the line alone,
    /// and no listener.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_report_without_a_descriptor_has_no_listener() {
        let (mut a, mut b) = report_socket().unwrap();

        send_report_with_fd(
            &mut a,
            br#"{"result":"applied","mechanisms":["x"],"unavailable":["y"]}"#,
            None,
        )
        .unwrap();

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Applied {
            mechanisms,
            unavailable,
            listener,
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(mechanisms, vec!["x".to_owned()]);
        assert_eq!(unavailable, vec!["y".to_owned()]);
        assert!(listener.is_none());
    }

    /// A refusal carries no descriptor; one attached anyway is closed,
    /// not kept, and the refusal is what the stage said.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_refused_line_is_a_refusal_whatever_rides_with_it() {
        use std::os::fd::AsRawFd;

        let (mut a, mut b) = report_socket().unwrap();
        let (read_end, _write_end) = pipe();

        send_report_with_fd(
            &mut a,
            br#"{"result":"refused","code":"c","message":"m"}"#,
            Some(read_end.as_raw_fd()),
        )
        .unwrap();

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Refused { code, message } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(code, "c");
        assert_eq!(message, "m");
    }

    /// `CMSG_SPACE` rounds one int up to a long, so a buffer sized for one
    /// descriptor has room for two, and the kernel fills it without
    /// flagging truncation. The count is checked, not assumed: two
    /// descriptors are refused, fail-closed, and both are closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn two_descriptors_on_the_report_are_refused() {
        use std::os::fd::AsRawFd;

        let (a, mut b) = report_socket().unwrap();
        let (r1, _w1) = pipe();
        let (r2, _w2) = pipe();

        send_with_descriptors(
            &a,
            br#"{"result":"applied","mechanisms":["x"]}"#,
            &[r1.as_raw_fd(), r2.as_raw_fd()],
        );

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Refused { code, message } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(code, "sandbox_apply_failed");
        assert!(message.contains("2 descriptors"), "{message}");
    }

    /// More descriptors than the buffer holds come back flagged
    /// `MSG_CTRUNC`: the kernel closed what did not fit, and what did fit
    /// cannot be trusted to be the listener. Refused, fail-closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_truncated_control_message_is_refused() {
        use std::os::fd::AsRawFd;

        let (a, mut b) = report_socket().unwrap();
        let (r1, _w1) = pipe();
        let (r2, _w2) = pipe();
        let (r3, _w3) = pipe();

        send_with_descriptors(
            &a,
            br#"{"result":"applied","mechanisms":["x"]}"#,
            &[r1.as_raw_fd(), r2.as_raw_fd(), r3.as_raw_fd()],
        );

        let outcome = read_report(&mut b, Duration::from_secs(2));
        let ReportOutcome::Refused { code, message } = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(code, "sandbox_apply_failed");
        assert!(message.contains("truncated"), "{message}");
    }

    /// The deadline still holds with the new read: a peer that never
    /// writes is `TimedOut`, and one that closes is `HelperGone`.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_deadline_and_the_peer_s_exit_are_still_told_apart() {
        let (a, mut b) = report_socket().unwrap();

        assert!(matches!(
            read_report(&mut b, Duration::from_millis(50)),
            ReportOutcome::TimedOut
        ));
        drop(a);
        assert!(matches!(
            read_report(&mut b, Duration::from_millis(50)),
            ReportOutcome::HelperGone
        ));
    }
}
