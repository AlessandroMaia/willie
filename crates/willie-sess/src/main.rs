//! Per-session supervisor.
//!
//! One detached process per session: owns the PTY, runs the harness,
//! serves attach clients on a Unix socket and appends to the session's
//! event log. It deliberately outlives the daemon so open terminals keep
//! working while the app is closed. Linux only.

#![deny(clippy::unwrap_used, clippy::expect_used)]

mod cli;
#[cfg(target_os = "linux")]
mod detach;
mod events;
#[cfg(target_os = "linux")]
mod pty;
mod sandbox;
mod screen;
#[cfg(target_os = "linux")]
mod server;
#[cfg(target_os = "linux")]
mod signals;
mod spec;
mod tally;
mod vt_filter;

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;
/// Exit code when the session could not be started.
const EXIT_FAILURE: u8 = 1;

/// Label for a sandbox-prepare or spawn failure that could not enter the
/// session's working directory.
pub(crate) const WORKSPACE_STEP: &str = "cannot enter the workspace";
/// Label for a sandbox-prepare or spawn failure that could not execute
/// the harness.
pub(crate) const EXEC_STEP: &str = "cannot execute the harness";

fn version_line() -> String {
    format!("willie-sess {}", willie_core::VERSION)
}

/// How a spawn failure is recorded. `prepare` checked both the
/// workspace and the helper, so either failure here is a path that went
/// away in between — but only the exec one is the helper failing to
/// start. A working directory that cannot be entered is the same
/// condition `prepare` refuses as `harness_exec_failed`, and saying the
/// helper did not start would name the wrong step.
#[cfg(target_os = "linux")]
fn spawn_failure(error: &pty::SpawnError) -> (&'static str, String) {
    match error {
        pty::SpawnError::Exec { step, .. } if *step == WORKSPACE_STEP => {
            ("harness_exec_failed", error.to_string())
        }
        pty::SpawnError::Exec { .. } => (
            "sandbox_apply_failed",
            format!("the namespace helper did not start: {error}"),
        ),
        pty::SpawnError::Setup(_) => {
            ("supervisor_spawn_failed", error.to_string())
        }
    }
}

/// The detached grandchild: set the session up and answer the launcher.
#[cfg(target_os = "linux")]
fn session_main(spec_path: &str, mut reply: detach::Reply) -> ExitCode {
    use std::{
        os::fd::AsRawFd,
        path::Path,
        sync::Arc,
        time::{Duration, Instant},
    };

    use willie_core::session::SessionEventKind;

    let (spec, paths) = match spec::load(Path::new(spec_path)) {
        Ok(loaded) => loaded,
        Err(e) => {
            let _ = reply.fail("spec_invalid", &e.to_string());
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let log = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
    {
        Ok(file) => file,
        Err(e) => {
            let _ = reply.fail("supervisor_spawn_failed", &e.to_string());
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    if let Err(e) = detach::redirect_std(&log) {
        let _ = reply.fail("supervisor_spawn_failed", &e.to_string());
        return ExitCode::from(EXIT_FAILURE);
    }
    drop(log);
    pty::ignore_sigpipe();
    let events = match events::EventLog::open(&paths.events, events::epoch_secs)
    {
        Ok(events) => events,
        Err(e) => {
            let _ = reply.fail("supervisor_spawn_failed", &e.to_string());
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    events.append(SessionEventKind::Created);
    let listener = match server::bind(&paths.socket) {
        Ok(listener) => listener,
        Err(e) => {
            let text = format!("cannot bind {}: {e}", paths.socket.display());
            events.append(SessionEventKind::Failed {
                code: "supervisor_spawn_failed".into(),
                message: text.clone(),
            });
            let _ = reply.fail("supervisor_spawn_failed", &text);
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    // Fail closed before a PTY exists: an unknown harness, a spec with no
    // home, a missing helper, a missing binary or workspace, a cache
    // directory that cannot be made — each refuses with its own code.
    let inner_exe = sandbox::inner_exe_path();
    let prepared =
        match sandbox::prepare(&spec, &sandbox::helper_path(), &inner_exe) {
            Ok(prepared) => prepared,
            Err(e) => {
                let text = e.to_string();
                events.append(SessionEventKind::Failed {
                    code: e.code().into(),
                    message: text.clone(),
                });
                let _ = reply.fail(e.code(), &text);
                return ExitCode::from(EXIT_FAILURE);
            }
        };
    // The report socket: the stage's end is inherited by the helper (and
    // through it by the in-namespace stage) and named in the vector; the
    // supervisor keeps its own end to read the one-line report.
    let (mut ours, theirs) = match sandbox::report_socket() {
        Ok(pair) => pair,
        Err(e) => {
            let text = format!("cannot make the report socket: {e}");
            events.append(SessionEventKind::Failed {
                code: "supervisor_spawn_failed".into(),
                message: text.clone(),
            });
            let _ = reply.fail("supervisor_spawn_failed", &text);
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let child_fd = theirs.as_raw_fd();
    let helper = sandbox::helper_path();
    let argv = sandbox::helper_argv(&prepared, &helper, child_fd);
    let (master, slave) = match pty::open() {
        Ok(pair) => pair,
        Err(e) => {
            let text = format!("cannot open a pty: {e}");
            events.append(SessionEventKind::Failed {
                code: "supervisor_spawn_failed".into(),
                message: text.clone(),
            });
            let _ = reply.fail("supervisor_spawn_failed", &text);
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    // argv[0] is now the helper: an exec failure here is the helper's,
    // never the harness's, which `prepare` already checked. The stage's
    // end of the report socket is kept open across the exec so bwrap
    // inherits it and passes it to the inner stage at the same number.
    let child = match pty::spawn(
        &master,
        slave,
        &argv,
        &spec.workspace,
        &spec.env,
        Some(child_fd),
    ) {
        Ok(pid) => pid,
        Err(e) => {
            let (code, text) = spawn_failure(&e);
            events.append(SessionEventKind::Failed {
                code: code.into(),
                message: text.clone(),
            });
            let _ = reply.fail(code, &text);
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    // The helper inherited `theirs`; drop the supervisor's copy so the
    // socket sees EOF when the stage or the helper closes it.
    drop(theirs);
    // Hand the stage its request — the harness argv it execs and the
    // limits it sets — the frame it blocks reading before it reports. The
    // bytes wait in the socket buffer until it reads them. A write that
    // fails means the stage is already unreachable: a helper that died
    // before reading closes the socket, and `SIGPIPE` is ignored, so the
    // write returns a broken pipe rather than killing the supervisor.
    // That is not handled here — `read_report` resolves it, as `HelperGone`
    // when the peer is gone (draining the helper's own words) or as
    // `TimedOut` if the stage is somehow alive but never reads.
    let _ = sandbox::write_request(&mut ours, &prepared.request);
    let pid = u32::try_from(child).unwrap_or(0);
    // Block the shutdown signals now: the fork has happened, so nothing
    // downstream of it inherits the block, and no supervisor thread
    // exists yet, so every one of them inherits the mask and the waiter
    // alone consumes `SIGTERM`/`SIGHUP`. The constraint is *after the
    // fork*, not after the shared state: a mask survives both fork and
    // exec, so blocking any earlier would hand the block to the helper
    // and through it to the harness, and the ladder's `SIGTERM` would
    // sit pending against the harness instead of being delivered.
    signals::block_shutdown_signals();
    // Read the stage's report with the ceiling. Four outcomes; three of
    // them refuse the session and end it, one starts it and carries the
    // filter's listener out to be served.
    let (mechanisms, unavailable, filter_listener) =
        match sandbox::read_report(&mut ours, sandbox::report_wait()) {
            sandbox::ReportOutcome::Applied {
                mechanisms,
                unavailable,
                listener: filter_listener,
            } => {
                // A report that names the filter but carries no listener is
                // a broken stage: refuse rather than run a session whose
                // denied syscalls nothing would ever answer.
                if mechanisms.iter().any(|m| m == "seccomp")
                    && filter_listener.is_none()
                {
                    let text = "the sandbox reported seccomp but sent no \
                                listener"
                        .to_owned();
                    events.append(SessionEventKind::Failed {
                        code: "sandbox_apply_failed".into(),
                        message: text.clone(),
                    });
                    // SAFETY: signalling our own child's process group.
                    unsafe { libc::kill(-child, libc::SIGKILL) };
                    let _ = reply.fail("sandbox_apply_failed", &text);
                    return ExitCode::from(EXIT_FAILURE);
                }
                if let Some(missing) =
                    willie_linux::sandbox::inner::required_missing(&mechanisms)
                {
                    let text = format!("the sandbox did not apply {missing}");
                    events.append(SessionEventKind::Failed {
                        code: "sandbox_backend_missing".into(),
                        message: text.clone(),
                    });
                    // SAFETY: signalling our own child's process group.
                    unsafe { libc::kill(-child, libc::SIGKILL) };
                    let _ = reply.fail("sandbox_backend_missing", &text);
                    return ExitCode::from(EXIT_FAILURE);
                }
                (mechanisms, unavailable, filter_listener)
            }
            sandbox::ReportOutcome::Refused { code, message } => {
                events.append(SessionEventKind::Failed {
                    code: code.clone(),
                    message: message.clone(),
                });
                // SAFETY: signalling our own child's process group.
                unsafe { libc::kill(-child, libc::SIGKILL) };
                let _ = reply.fail(&code, &message);
                return ExitCode::from(EXIT_FAILURE);
            }
            sandbox::ReportOutcome::HelperGone => {
                // The helper died before the stage reported; its own words
                // are on the terminal nobody is attached to. Drain and
                // carry them — part 1's path.
                let said = pty::drain(&master, sandbox::HELPER_DRAIN);
                let exit = pty::wait(child).unwrap_or(pty::Exit {
                    code: None,
                    signal: None,
                });
                let text =
                    sandbox::apply_failure(&said, exit.code, exit.signal);
                events.append(SessionEventKind::Failed {
                    code: "sandbox_apply_failed".into(),
                    message: text.clone(),
                });
                let _ = reply.fail("sandbox_apply_failed", &text);
                return ExitCode::from(EXIT_FAILURE);
            }
            sandbox::ReportOutcome::TimedOut => {
                let text = "the sandbox reported nothing within the deadline"
                    .to_owned();
                events.append(SessionEventKind::Failed {
                    code: "sandbox_apply_failed".into(),
                    message: text.clone(),
                });
                // SAFETY: signalling our own child's process group.
                unsafe { libc::kill(-child, libc::SIGKILL) };
                let _ = reply.fail("sandbox_apply_failed", &text);
                return ExitCode::from(EXIT_FAILURE);
            }
        };
    // The stage reported success, so the harness is running behind it.
    let harness = sandbox::harness_pid(child);
    events.append(SessionEventKind::SandboxApplied {
        mechanisms,
        unavailable,
    });
    // The pid a session records is the helper's monitor, the supervisor's
    // own child (decision 0016); the harness pid is the ladder's business.
    let started = events.append(SessionEventKind::Started { pid });
    let shared = server::Shared::new(
        master,
        child,
        harness,
        started.at.clone(),
        paths.socket.clone(),
        events,
        server::stop_grace(),
    );
    // Serve the filter from before the session is announced ready, so a
    // denial from the very first syscall is answered with EPERM and
    // recorded. A harness whose first call is intercepted before the
    // thread is up only waits for the answer; nothing is lost.
    let notifications =
        filter_listener.and_then(|listener| serve_filter(listener, &shared));
    if let Err(e) = server::start(&shared, listener) {
        let _ = reply.fail("supervisor_spawn_failed", &e.to_string());
        return ExitCode::from(EXIT_FAILURE);
    }
    if let Err(e) = signals::spawn_waiter(Arc::clone(&shared)) {
        eprintln!("willie-sess: signals are not handled: {e}");
    }
    let _ = reply.ok(pid);
    reply.close();
    server::serve(&shared);
    shared.begin_teardown();
    let raw = pty::wait(child).unwrap_or(pty::Exit {
        code: None,
        signal: None,
    });
    // The filter's server ends on its own once no process is left under
    // the filter, which happens before the monitor's exit reaches `wait`;
    // its last record may still be in flight, so give it a moment to
    // land before the flush below. Bounded: a server that has not ended
    // is not something the exit waits on.
    if let Some(handle) = &notifications {
        let until = Instant::now() + FILTER_SETTLE;
        while !handle.is_finished() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let (code, signal) = sandbox::helper_exit(raw.code, raw.signal);
    server::finish(&shared, pty::Exit { code, signal });
    ExitCode::SUCCESS
}

/// How long the exit waits for the filter's server to finish its last
/// record. It has normally ended already; this is a bound, not a delay.
#[cfg(target_os = "linux")]
const FILTER_SETTLE: std::time::Duration =
    std::time::Duration::from_millis(250);

/// Start the thread that answers and records the filter's denials, for
/// the session's life. The handle is held so the thread stays owned.
///
/// The loop ends orderly when no process is left under the filter — the
/// session ending — and that is never degraded. It ends any other way
/// only by failing; if that happens while the session is not tearing
/// down, the server died under a live session, and the session records
/// that it runs degraded from then on. It does: the listener died with
/// the thread, so the kernel answers every later intercepted call with
/// `ENOSYS` — closed, not open. A thread that cannot even start is the
/// same condition from the first call.
#[cfg(target_os = "linux")]
fn serve_filter(
    listener: std::os::fd::OwnedFd,
    shared: &std::sync::Arc<server::Shared>,
) -> Option<std::thread::JoinHandle<()>> {
    use willie_core::session::SessionEventKind;

    let owned = std::sync::Arc::clone(shared);
    let spawned = std::thread::Builder::new()
        .name("seccomp".to_owned())
        .spawn(move || {
            let end = sandbox::seccomp::serve_notifications(listener, |nr| {
                server::record_denial(
                    &owned,
                    "syscall",
                    &sandbox::seccomp::syscall_name(nr),
                );
            });
            match end {
                Ok(()) => eprintln!(
                    "willie-sess: no process is left under the syscall \
                     filter; its server ends"
                ),
                Err(e) if owned.tearing_down() => eprintln!(
                    "willie-sess: the syscall filter's server ended during \
                     teardown: {e}"
                ),
                Err(e) => {
                    let message = format!(
                        "the syscall filter is no longer served ({e}); \
                         intercepted calls now fail with ENOSYS"
                    );
                    eprintln!("willie-sess: {message}");
                    server::log_event(
                        &owned,
                        SessionEventKind::SandboxDegraded {
                            mechanism: "seccomp".to_owned(),
                            message,
                        },
                    );
                }
            }
        });
    match spawned {
        Ok(handle) => Some(handle),
        Err(e) => {
            let message = format!(
                "the syscall filter's server did not start ({e}); \
                 intercepted calls fail with ENOSYS"
            );
            eprintln!("willie-sess: {message}");
            server::log_event(
                shared,
                SessionEventKind::SandboxDegraded {
                    mechanism: "seccomp".to_owned(),
                    message,
                },
            );
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn run(spec_path: &str) -> ExitCode {
    match detach::detach() {
        Err(e) => {
            println!("fail supervisor_spawn_failed: cannot detach: {e}");
            ExitCode::from(EXIT_FAILURE)
        }
        Ok(detach::Side::Launcher(handshake)) => {
            let log = std::path::Path::new(spec_path)
                .with_file_name("supervisor.log");
            match handshake.wait(&log) {
                detach::Ready::Ok(pid) => {
                    println!("ok {pid}");
                    ExitCode::SUCCESS
                }
                detach::Ready::Fail { code, text } => {
                    println!("fail {code}: {text}");
                    ExitCode::from(EXIT_FAILURE)
                }
            }
        }
        Ok(detach::Side::Session(reply)) => session_main(spec_path, reply),
    }
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&args) {
        cli::Command::Version => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        cli::Command::Run { spec } => run(&spec),
        cli::Command::Inner { fd } => sandbox::inner::run_inner(fd),
        cli::Command::Usage(reason) => {
            eprintln!("willie-sess: {reason}");
            eprintln!("{}", cli::USAGE);
            ExitCode::from(EXIT_USAGE)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    // Off Linux the supervisor is a stub, so the items the detached path
    // alone consumes have no caller here; naming them keeps the portable
    // modules honestly dead-code checked without a blanket allow.
    let _ = (
        cli::parse,
        spec::load,
        events::EventLog::open,
        events::EventLog::append,
        events::epoch_secs,
        EXIT_FAILURE,
        sandbox::prepare,
        sandbox::PrepareError::code,
        sandbox::helper_exit,
        sandbox::parse_children,
        sandbox::parse_child_pids,
        sandbox::children_include,
        sandbox::helper_argv,
        sandbox::helper_path,
        sandbox::apply_failure,
        sandbox::HELPER_DRAIN,
        sandbox::inner::not_in_namespace,
        sandbox::inner::clamp,
    );
    // `screen` is pure and compiled on every target, yet only the Linux
    // socket code drives it; name its items so the host build checks them.
    let _ = (
        screen::Ring::new,
        screen::Ring::push,
        screen::Ring::snapshot,
        screen::AltScreen::default,
        screen::AltScreen::feed,
        screen::AltScreen::active,
    );
    // `tally` is pure as well, driven only by the Linux server thread.
    let _ = (tally::Tally::new, tally::Tally::record, tally::Tally::flush);
    // `vt_filter` is pure and portable, driven only by the Linux `serve`.
    let _ = (vt_filter::VtFilter::new, vt_filter::VtFilter::feed);
    eprintln!(
        "{} runs only inside the Willie Linux distribution",
        version_line()
    );
    eprintln!("{}", cli::USAGE);
    ExitCode::from(EXIT_USAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_binary_and_version() {
        assert_eq!(
            version_line(),
            format!("willie-sess {}", willie_core::VERSION)
        );
    }

    /// The helper is `argv[0]` now, so an exec failure is the helper's.
    /// A working directory that cannot be entered is not: the child
    /// never reached the helper, and the message must say so.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_working_directory_failure_is_not_the_helper_failing_to_start() {
        let workspace = spawn_failure(&pty::SpawnError::Exec {
            step: WORKSPACE_STEP,
            error: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        let helper = spawn_failure(&pty::SpawnError::Exec {
            step: EXEC_STEP,
            error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        });

        assert_eq!(workspace.0, "harness_exec_failed");
        assert!(workspace.1.starts_with(WORKSPACE_STEP), "{}", workspace.1);
        assert!(!workspace.1.contains("namespace helper"), "{}", workspace.1);
        assert_eq!(helper.0, "sandbox_apply_failed");
        assert!(helper.1.contains("the namespace helper did not start"));
    }
}
