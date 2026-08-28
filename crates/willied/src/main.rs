//! Willie daemon.
//!
//! Runs inside the Willie WSL distribution as an unprivileged user and owns
//! projects, sessions, plugins and the SQLite index. Speaks the control
//! protocol over stdio (to the engine). Only builds its real entry point on
//! Linux; elsewhere it is a stub so the workspace still type-checks.

#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod git;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod handlers;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod harness;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod identity;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod jobs;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod outbound;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod projects;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod server;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod state;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod store;

use std::process::ExitCode;

const EXIT_USAGE: u8 = 2;

/// The ext4 directory the daemon owns, overridable so integration tests
/// can run against a private, hermetic state directory.
#[cfg(target_os = "linux")]
const STATE_DIR_ENV: &str = "WILLIE_STATE_DIR";
/// Where session sockets and other runtime state live, overridable for
/// the same reason. Consumed by the sessions RPC task.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
const RUN_DIR_ENV: &str = "WILLIE_RUN_DIR";
/// Where workspaces are cloned, overridable for the same reason.
#[cfg(target_os = "linux")]
const PROJECTS_DIR_ENV: &str = "WILLIE_PROJECTS_DIR";
/// Default workspaces root inside the distribution (docs/ARCHITECTURE.md).
#[cfg(target_os = "linux")]
const DEFAULT_PROJECTS_DIR: &str = "/home/willie/projects";

fn version_line() -> String {
    format!("willied {}", willie_core::VERSION)
}

fn usage() -> ExitCode {
    eprintln!("usage: willied --stdio | --version");
    ExitCode::from(EXIT_USAGE)
}

/// Seconds since the Unix epoch, formatted as a string. The event and
/// project types carry timestamps as opaque strings; a std-only epoch
/// count keeps them monotonic-ish without pulling in a date-time crate.
#[cfg(target_os = "linux")]
fn real_clock() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// A daemon that died mid-add leaves a project stuck `Preparing`. On the
/// next start, turn any such project into `Failed { code: "interrupted" }`
/// and persist it, so the app never shows a clone that will never finish.
/// No event is emitted: no client is attached this early in startup.
#[cfg(target_os = "linux")]
fn interrupt_preparing(
    state: &std::sync::Arc<std::sync::Mutex<state::State>>,
    state_dir: &std::path::Path,
) {
    use std::sync::PoisonError;

    use willie_core::project::ProjectState;

    let mut guard = state.lock().unwrap_or_else(PoisonError::into_inner);
    let stuck: Vec<willie_core::id::ProjectId> = guard
        .projects
        .iter()
        .filter(|(_, p)| matches!(p.state, ProjectState::Preparing))
        .map(|(id, _)| *id)
        .collect();
    for id in stuck {
        if let Some(p) = guard.projects.get_mut(&id) {
            p.state = ProjectState::Failed {
                code: "interrupted".to_owned(),
                message: "the daemon stopped before the operation finished"
                    .to_owned(),
                remediation: "remove the project and add it again".to_owned(),
            };
            store::save_or_log(state_dir, p);
        }
    }
}

#[cfg(target_os = "linux")]
fn run_stdio() -> ExitCode {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    let state_dir = PathBuf::from(
        std::env::var(STATE_DIR_ENV)
            .unwrap_or_else(|_| willie_linux::paths::STATE_DIR.to_owned()),
    );
    let workspaces_dir = PathBuf::from(
        std::env::var(PROJECTS_DIR_ENV)
            .unwrap_or_else(|_| DEFAULT_PROJECTS_DIR.to_owned()),
    );

    let (out, writer_handle) = outbound::Outbound::spawn(std::io::stdout());
    let state = Arc::new(Mutex::new(state::State::load(&state_dir)));
    interrupt_preparing(&state, &state_dir);
    let runner = jobs::Runner::new(Arc::clone(&state), out.clone(), real_clock);
    let ops = projects::Ops::new(
        Arc::clone(&state),
        runner,
        state_dir,
        workspaces_dir,
        real_clock,
        out.clone(),
    );
    let mut server = server::Server::new(
        willie_linux::doctor::run_all,
        Arc::clone(&state),
        ops,
        out.clone(),
    );

    let code = match server.serve(std::io::stdin().lock()) {
        Ok(reason) => {
            eprintln!("willied: exiting ({reason:?})");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("willied: transport error: {e}");
            ExitCode::FAILURE
        }
    };

    // Trip in-flight jobs, then drop every writer sender so the writer
    // thread drains its queue and exits; join it so buffered replies flush.
    server.shutdown();
    drop(server);
    drop(out);
    let _ = writer_handle.join();
    code
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Some("--stdio") => run_stdio(),
        _ => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    // The server is still compiled and unit-tested here; only the entry
    // point is Linux-specific.
    let _ = server::Server::new;
    eprintln!(
        "{} runs only inside the Willie Linux distribution",
        version_line()
    );
    usage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_binary_and_version() {
        assert_eq!(version_line(), format!("willied {}", willie_core::VERSION));
    }
}
