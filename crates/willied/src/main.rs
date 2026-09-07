//! Willie daemon.
//!
//! Runs inside the Willie WSL distribution as an unprivileged user and owns
//! projects, sessions, plugins and the SQLite index. Speaks the control
//! protocol over stdio (to the engine). Only builds its real entry point on
//! Linux; elsewhere it is a stub so the workspace still type-checks.

#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod control;
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
mod manifest;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod outbound;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod projects;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod server;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod session_store;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod sessions;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod state;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod store;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod tools;

pub(crate) use state::lock;

use std::process::ExitCode;

const EXIT_USAGE: u8 = 2;

/// The ext4 directory the daemon owns, overridable so integration tests
/// can run against a private, hermetic state directory.
#[cfg(target_os = "linux")]
const STATE_DIR_ENV: &str = "WILLIE_STATE_DIR";
/// Where session sockets and other runtime state live, overridable for
/// the same reason. Consumed by the sessions RPC task.
#[cfg(target_os = "linux")]
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

/// A clock for timestamps produced outside the request path (finalising a
/// lost session on a background thread), where threading the injected
/// clock through is not worth it. The real epoch clock on Linux; a fixed
/// stamp on other targets, which never reach this code at runtime.
#[cfg(target_os = "linux")]
pub(crate) fn real_clock_or_zero() -> String {
    real_clock()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn real_clock_or_zero() -> String {
    "0".to_owned()
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

/// Refuse to steal a live socket, remove a dead one, bind, `0600`.
/// Mirrors the supervisor's `willie-sess` `server::bind`: a socket a client
/// can still connect to means a second daemon, which must not exist.
#[cfg(target_os = "linux")]
fn bind(
    socket: &std::path::Path,
) -> std::io::Result<std::os::unix::net::UnixListener> {
    use std::os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    };

    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("another daemon is listening on {}", socket.display()),
            ));
        }
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// One socket connection: read a line, hand it to the dispatcher with a
/// private reply channel, write the response back, repeat. One request is
/// in flight per connection. EOF from the client, a write failure, or the
/// dispatcher going away ends the connection and only the connection — its
/// dispatched effect stands, as a broken stdio pipe's does today.
#[cfg(target_os = "linux")]
fn serve_connection(
    stream: std::os::unix::net::UnixStream,
    tx: &std::sync::mpsc::Sender<server::Inbound>,
) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Write};

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        if tx
            .send(server::Inbound::Line {
                text: std::mem::take(&mut line),
                origin: server::Origin::Socket,
                reply: Some(reply_tx),
            })
            .is_err()
        {
            return Ok(());
        }
        let Ok(response) = reply_rx.recv() else {
            return Ok(());
        };
        match serde_json::to_string(&response) {
            Ok(text) => {
                writeln!(writer, "{text}")?;
                writer.flush()?;
            }
            Err(e) => {
                eprintln!("willied: cannot encode a socket response: {e}");
                return Ok(());
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn run_stdio() -> ExitCode {
    use std::{
        io::BufRead,
        path::PathBuf,
        sync::{Arc, Mutex, mpsc},
        thread,
    };

    let state_dir = PathBuf::from(
        std::env::var(STATE_DIR_ENV)
            .unwrap_or_else(|_| willie_linux::paths::STATE_DIR.to_owned()),
    );
    let workspaces_dir = PathBuf::from(
        std::env::var(PROJECTS_DIR_ENV)
            .unwrap_or_else(|_| DEFAULT_PROJECTS_DIR.to_owned()),
    );
    let run_dir = PathBuf::from(
        std::env::var(RUN_DIR_ENV)
            .unwrap_or_else(|_| willie_linux::paths::RUN_DIR.to_owned()),
    );
    // The distro user's home (session PATH, git identity, harness lookup),
    // overridable via `WILLIE_HOME` for hermetic tests.
    let home = harness::home();

    let (out, writer_handle) = outbound::Outbound::spawn(std::io::stdout());
    let state = Arc::new(Mutex::new(state::State::load(&state_dir)));
    interrupt_preparing(&state, &state_dir);
    let runner = jobs::Runner::new(Arc::clone(&state), out.clone(), real_clock);
    let ops = projects::Ops::new(
        Arc::clone(&state),
        runner,
        state_dir.clone(),
        workspaces_dir,
        real_clock,
        out.clone(),
    );
    // The run dir feeds both the socket path written into each spec and
    // the daemon's own connect/scan path, so both sides agree on where a
    // session's socket lives. The daemon's own socket (bound below) sits
    // directly in it too, so it is cloned rather than moved.
    let session_ops = sessions::SessionOps::new(
        Arc::clone(&state),
        out.clone(),
        state_dir,
        run_dir.clone(),
        home,
        real_clock,
        ops.runner_handle(),
    );
    // Re-adopt live supervisors (and finalise dead ones) before serving.
    session_ops.scan();

    // The daemon's own socket for local CLI clients. Bound after
    // re-adoption and before serving, and fail closed if it cannot be
    // bound: a live socket means a second daemon, which must not exist.
    // The run dir is the daemon's own runtime directory (provisioned
    // `0750 willie:willie`); ensure it exists so a hermetic test's private
    // run dir binds the same way the provisioned one does.
    //
    // Failing closed here must actually exit: `ops` (which owns the
    // `Runner`) and `session_ops` each hold an `Outbound` sender clone, so
    // the writer thread only ends once every clone is dropped. Releasing
    // them before joining is what keeps a bind failure — the exact second-
    // daemon case — from hanging on the join instead of returning FAILURE.
    let socket = willie_linux::paths::daemon_socket(&run_dir);
    if let Err(e) = std::fs::create_dir_all(&run_dir) {
        eprintln!(
            "willied: cannot create the run dir {}: {e}",
            run_dir.display()
        );
        drop(session_ops);
        drop(ops);
        drop(out);
        let _ = writer_handle.join();
        return ExitCode::FAILURE;
    }
    let listener = match bind(&socket) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("willied: cannot bind {}: {e}", socket.display());
            drop(session_ops);
            drop(ops);
            drop(out);
            let _ = writer_handle.join();
            return ExitCode::FAILURE;
        }
    };

    let mut server = server::Server::new(
        willie_linux::doctor::run_all,
        Arc::clone(&state),
        ops,
        session_ops,
        out.clone(),
    );

    // One channel, two origins feed the single dispatcher: the engine's
    // stdin reader, and one thread per socket connection.
    let (tx, rx) = mpsc::channel::<server::Inbound>();

    // The engine's stdin: each line is a request; EOF ends the daemon.
    let stdin_tx = tx.clone();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(text) => {
                    if stdin_tx
                        .send(server::Inbound::Line {
                            text,
                            origin: server::Origin::Stdio,
                            reply: None,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) => {
                    eprintln!("willied: stdin read failed: {e}");
                    break;
                }
            }
        }
        let _ = stdin_tx.send(server::Inbound::StdinClosed);
    });

    // Local CLI clients: the accept thread owns the listener and spawns one
    // thread per connection. Neither is joined — the process exit ends them.
    thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let conn_tx = tx.clone();
                    let spawned = thread::Builder::new()
                        .name("willied-conn".to_owned())
                        .spawn(move || {
                            if let Err(e) = serve_connection(stream, &conn_tx) {
                                eprintln!(
                                    "willied: socket connection ended: {e}"
                                );
                            }
                        });
                    if let Err(e) = spawned {
                        eprintln!(
                            "willied: cannot start a socket connection: {e}"
                        );
                    }
                }
                Err(e) => eprintln!("willied: socket accept failed: {e}"),
            }
        }
    });

    let reason = server.serve_inbound(rx);
    eprintln!("willied: exiting ({reason:?})");

    // Trip in-flight jobs, then drop every writer sender so the writer
    // thread drains its queue and exits; join it so buffered replies flush.
    // Remove the socket file last. The accept and connection threads are
    // not joined (accept would block); the process exit ends them and their
    // clients read EOF.
    server.shutdown();
    drop(server);
    drop(out);
    let _ = writer_handle.join();
    let _ = std::fs::remove_file(&socket);
    ExitCode::SUCCESS
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

    /// Every workspace this daemon creates lives directly under the
    /// root below, and an extra path renders after the workspace's own
    /// bind, so granting that root would mount the whole tree over the
    /// session's project and hand it every other project at once. The
    /// guard that refuses it lives in `willie-core`, which cannot see
    /// this constant, so the two are held together here: moving the
    /// root without teaching the guard fails this test rather than
    /// quietly reopening what the base closed.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_workspaces_root_is_refused_as_an_extra_path() {
        let home = harness::home();

        assert!(
            willie_core::sandbox::guard_extra_path(
                DEFAULT_PROJECTS_DIR,
                &home.to_string_lossy(),
            )
            .is_some(),
            "{DEFAULT_PROJECTS_DIR} is grantable under {}",
            home.display()
        );
    }
}
