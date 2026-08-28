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
mod screen;
#[cfg(target_os = "linux")]
mod server;
mod spec;

use std::process::ExitCode;

/// Exit code for usage errors and unsupported platforms.
const EXIT_USAGE: u8 = 2;
/// Exit code when the session could not be started.
const EXIT_FAILURE: u8 = 1;

fn version_line() -> String {
    format!("willie-sess {}", willie_core::VERSION)
}

/// The detached grandchild: set the session up and answer the launcher.
#[cfg(target_os = "linux")]
fn session_main(spec_path: &str, mut reply: detach::Reply) -> ExitCode {
    use std::path::Path;

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
    let child = match pty::spawn(
        &master,
        slave,
        &spec.argv,
        &spec.workspace,
        &spec.env,
    ) {
        Ok(pid) => pid,
        Err(e) => {
            let (code, text) = match &e {
                pty::SpawnError::Exec { .. } => {
                    ("harness_exec_failed", format!("{e} ({})", spec.workspace))
                }
                pty::SpawnError::Setup(_) => {
                    ("supervisor_spawn_failed", e.to_string())
                }
            };
            events.append(SessionEventKind::Failed {
                code: code.into(),
                message: text.clone(),
            });
            let _ = reply.fail(code, &text);
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let pid = u32::try_from(child).unwrap_or(0);
    let started = events.append(SessionEventKind::Started { pid });
    let shared = server::Shared::new(
        master,
        child,
        started.at.clone(),
        paths.socket.clone(),
        events,
    );
    if let Err(e) = server::start(&shared, listener) {
        let _ = reply.fail("supervisor_spawn_failed", &e.to_string());
        return ExitCode::from(EXIT_FAILURE);
    }
    let _ = reply.ok(pid);
    reply.close();
    server::serve(&shared);
    let exit = pty::wait(child).unwrap_or(pty::Exit {
        code: None,
        signal: None,
    });
    server::finish(&shared, exit);
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
}
