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

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;
/// Exit code when the session could not be started.
const EXIT_FAILURE: u8 = 1;

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
        pty::SpawnError::Exec { step, .. } if *step == pty::WORKSPACE_STEP => {
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
    use std::{path::Path, sync::Arc};

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
    let prepared = match sandbox::prepare(&spec, &sandbox::helper_path()) {
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
    // never the harness's, which `prepare` already checked.
    let child = match pty::spawn(
        &master,
        slave,
        &prepared.argv,
        &spec.workspace,
        &spec.env,
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
    let pid = u32::try_from(child).unwrap_or(0);
    // Block the shutdown signals now: the fork has happened, so nothing
    // downstream of it inherits the block, and no supervisor thread
    // exists yet, so every one of them inherits the mask and the waiter
    // alone consumes `SIGTERM`/`SIGHUP`. The constraint is *after the
    // fork*, not after the shared state: a mask survives both fork and
    // exec, so blocking any earlier would hand the block to the helper
    // and through it to the harness, and the ladder's `SIGTERM` would
    // sit pending against the harness instead of being delivered.
    // Everything from here to the waiter — the wait for the harness
    // above all — is time in which a signal to the supervisor would
    // otherwise kill it outright, with no `failed` recorded and no
    // client told why; blocked, it merely stays pending.
    signals::block_shutdown_signals();
    // The helper builds the namespace before it forks the harness, so
    // wait for the harness to exist before saying the session started
    // and before answering ready: both promised a running harness before
    // the helper stood between them, and a stop that arrives inside that
    // window must reach the harness, not the group.
    let harness =
        match sandbox::wait_for_harness(child, sandbox::harness_wait()) {
            sandbox::HarnessWait::Running(pid) => Some(pid),
            sandbox::HarnessWait::GaveUp => None,
            // The helper refused while building the namespace. Everything
            // visible before it ran is already a coded refusal, so this is
            // the one class left, and its only channel is the terminal
            // nobody is attached to yet: drain it, or the session becomes a
            // start and an exit with no cause anywhere.
            // Nothing is gone for certain here: a harness that ran and
            // exited inside one poll leaves the same trace. Only the
            // helper's own words tell the two apart, so a session whose
            // harness merely finished first is left to the ordinary
            // path, with the pid the ladder would have used unknown —
            // which costs nothing, since it has already exited.
            sandbox::HarnessWait::HelperGone => {
                let said = pty::drain(&master, sandbox::HELPER_DRAIN);
                if !sandbox::is_helper_refusal(&said) {
                    // Nothing is certain here: a harness that ran and
                    // exited inside one poll leaves the same trace as a
                    // helper that never forked one. Only the helper's
                    // own words tell them apart, so a session whose
                    // harness merely finished first takes the ordinary
                    // path, with the pid the ladder would have used
                    // unknown — which costs nothing, since it is gone.
                    None
                } else {
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
            }
        };
    // Only now is anything applied: a helper that refused built no
    // namespace, and an event saying otherwise would be the record
    // claiming what did not happen.
    events.append(SessionEventKind::SandboxApplied {
        mechanisms: prepared.mechanisms,
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
    let raw = pty::wait(child).unwrap_or(pty::Exit {
        code: None,
        signal: None,
    });
    let (code, signal) = sandbox::helper_exit(raw.code, raw.signal);
    server::finish(&shared, pty::Exit { code, signal });
    ExitCode::SUCCESS
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
        sandbox::parse_state,
        sandbox::MECHANISMS,
        sandbox::helper_path,
        sandbox::apply_failure,
        sandbox::HELPER_DRAIN,
        sandbox::is_helper_refusal,
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
            step: pty::WORKSPACE_STEP,
            error: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        let helper = spawn_failure(&pty::SpawnError::Exec {
            step: pty::EXEC_STEP,
            error: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        });

        assert_eq!(workspace.0, "harness_exec_failed");
        assert!(
            workspace.1.starts_with(pty::WORKSPACE_STEP),
            "{}",
            workspace.1
        );
        assert!(!workspace.1.contains("namespace helper"), "{}", workspace.1);
        assert_eq!(helper.0, "sandbox_apply_failed");
        assert!(helper.1.contains("the namespace helper did not start"));
    }
}
