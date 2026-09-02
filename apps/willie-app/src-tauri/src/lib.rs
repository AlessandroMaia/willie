//! Tauri application shell.
//!
//! Commands are thin: lock the engine, call one method, emit the fresh
//! status. The UI never computes truth; it renders `EngineStatus` for
//! the engine/daemon lifecycle and the `daemon://event` stream (see
//! `events`) for project and job state.

mod events;
mod state;

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, State};
use willie_core::id::{JobId, ProjectId, SessionId};
use willie_core::project::Project;
use willie_core::sandbox::SandboxProfile;
use willie_engine::discover::Candidate;
use willie_engine::error::EngineError;
use willie_engine::{Engine, EngineStatus, Problem, SessionOpened};
use willie_proto::daemon::DoctorReport;
use willie_proto::project::{AddResult, JobRef, ProjectList};
use willie_proto::sandbox::CapabilityInfo;
use willie_proto::state::Snapshot;

use crate::events::EventPump;
use crate::state::{EngineState, image_candidates, workspace_root};

const STATUS_EVENT: &str = "engine://status";
const OUTPUT_EVENT: &str = "session://output";

#[derive(Clone, serde::Serialize)]
struct SessionOutput {
    id: String,
    chunk: Vec<u8>,
}

#[tauri::command]
fn app_version() -> &'static str {
    willie_core::VERSION
}

fn with_engine<T>(
    state: &State<'_, EngineState>,
    f: impl FnOnce(&mut Engine) -> T,
) -> Result<T, Problem> {
    let mut engine = state.0.lock().map_err(|_| Problem {
        code: "engine_poisoned".into(),
        message: "the engine lock was poisoned by a previous panic".into(),
        remediation: "restart Willie".into(),
    })?;
    Ok(f(&mut engine))
}

fn emit_status(app: &AppHandle, status: &EngineStatus) {
    if let Err(e) = app.emit(STATUS_EVENT, status) {
        eprintln!("willie-app: cannot emit status: {e}");
    }
}

// Every command talks to WSL and can block for seconds (`--import`, VM
// boot); `async` keeps them off the main thread so the window stays
// responsive.
#[tauri::command(async)]
fn engine_status(
    state: State<'_, EngineState>,
) -> Result<EngineStatus, Problem> {
    with_engine(&state, Engine::status)
}

fn mutate(
    app: AppHandle,
    state: State<'_, EngineState>,
    op: impl FnOnce(&mut Engine) -> Result<(), EngineError>,
) -> Result<EngineStatus, Problem> {
    let outcome = with_engine(&state, |engine| {
        let result = op(engine).map_err(|e| Problem::from(&e));
        (result, engine.status())
    })?;
    emit_status(&app, &outcome.1);
    outcome.0.map(|()| outcome.1)
}

#[tauri::command(async)]
fn engine_install_distro(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> Result<EngineStatus, Problem> {
    mutate(app, state, Engine::install_distro)
}

#[tauri::command(async)]
fn engine_start_daemon(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> Result<EngineStatus, Problem> {
    mutate(app, state, Engine::start_daemon)
}

#[tauri::command(async)]
fn engine_stop_daemon(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> Result<EngineStatus, Problem> {
    mutate(app, state, Engine::stop_daemon)
}

/// The commands that clear the one host prerequisite an administrator
/// owns. A constant the engine holds, so no lock is taken and nothing
/// can fail; the UI asks for it only when it sees that problem code.
#[tauri::command]
fn engine_logon_fix_script() -> &'static str {
    willie_engine::error::SERVICE_LOGON_FIX_SCRIPT
}

#[tauri::command(async)]
fn engine_doctor(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> Result<DoctorReport, Problem> {
    let outcome = with_engine(&state, |engine| {
        let report = engine.run_doctor().map_err(|e| Problem::from(&e));
        (report, engine.status())
    })?;
    emit_status(&app, &outcome.1);
    outcome.0
}

/// An engine call that never goes through the daemon: lock, call,
/// flatten the `EngineError`. No daemon RPC means no status to
/// re-emit and no event pump to attach — true of both read-only
/// filesystem queries and the host-side embedded-terminal spawn.
fn query<T>(
    state: &State<'_, EngineState>,
    op: impl FnOnce(&mut Engine) -> Result<T, EngineError>,
) -> Result<T, Problem> {
    with_engine(state, op)?.map_err(|e| Problem::from(&e))
}

/// Every project and job command goes through here: it starts the daemon
/// on demand (`Engine::daemon_call`, inside `op`), so the fresh
/// `engine://status` is always worth re-emitting, and the event pump
/// needs a chance to (re)attach to whatever `RpcClient` the daemon start
/// just created. Project and job truth itself is not returned here — it
/// flows to the webview only through `daemon://event` — so the command's
/// own result is just the raw RPC reply.
fn daemon_command<T>(
    app: &AppHandle,
    state: &State<'_, EngineState>,
    pump: &State<'_, EventPump>,
    op: impl FnOnce(&mut Engine) -> Result<T, EngineError>,
) -> Result<T, Problem> {
    let outcome = with_engine(state, |engine| {
        let result = op(engine).map_err(|e| Problem::from(&e));
        (result, engine.status())
    })?;
    emit_status(app, &outcome.1);
    pump.ensure(app, state);
    outcome.0
}

#[tauri::command(async)]
fn project_list(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
) -> Result<ProjectList, Problem> {
    daemon_command(&app, &state, &pump, Engine::project_list)
}

#[tauri::command(async)]
fn project_add(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    windows_path: String,
    name: Option<String>,
) -> Result<AddResult, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_add(&windows_path, name)
    })
}

#[tauri::command(async)]
fn project_remove(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    delete_workspace: bool,
    force: bool,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_remove(id, delete_workspace, force)
    })
}

#[tauri::command(async)]
fn project_sync_to_windows(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_sync_to_windows(id)
    })
}

#[tauri::command(async)]
fn project_update_from_windows(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_update_from_windows(id)
    })
}

#[tauri::command(async)]
fn project_relocate(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    windows_path: String,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_relocate(id, windows_path)
    })
}

#[tauri::command(async)]
fn project_rename(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    name: String,
) -> Result<Project, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_rename(id, name)
    })
}

#[tauri::command(async)]
fn project_set_sandbox(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    profile: SandboxProfile,
) -> Result<Project, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_set_sandbox(id, profile)
    })
}

/// The capability catalogue: static domain data, so no engine lock and
/// no daemon. The app renders this copy verbatim rather than keeping a
/// second copy of ten user-facing sentences in TypeScript.
#[tauri::command]
fn sandbox_catalogue() -> Vec<CapabilityInfo> {
    willie_core::sandbox::Capability::ALL
        .iter()
        .map(|&c| CapabilityInfo {
            capability: c,
            display_name: c.display_name().to_owned(),
            consequence: c.consequence().to_owned(),
            implemented: c.is_implemented(),
        })
        .collect()
}

#[tauri::command(async)]
fn job_cancel(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: JobId,
) -> Result<(), Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.job_cancel(id))
}

#[tauri::command(async)]
fn state_snapshot(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
) -> Result<Snapshot, Problem> {
    daemon_command(&app, &state, &pump, Engine::state_snapshot)
}

#[tauri::command(async)]
fn projects_roots(
    state: State<'_, EngineState>,
) -> Result<Vec<String>, Problem> {
    with_engine(&state, |engine| engine.projects_roots())
}

#[tauri::command(async)]
fn set_projects_roots(
    state: State<'_, EngineState>,
    roots: Vec<String>,
) -> Result<(), Problem> {
    query(&state, |engine| engine.set_projects_roots(roots))
}

#[tauri::command(async)]
fn discover_projects(
    state: State<'_, EngineState>,
) -> Result<Vec<Candidate>, Problem> {
    query(&state, |engine| engine.discover_projects())
}

#[tauri::command(async)]
fn session_open(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    project_id: ProjectId,
) -> Result<SessionOpened, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_open(project_id)
    })
}

#[tauri::command(async)]
fn session_resume(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    project_id: ProjectId,
) -> Result<SessionOpened, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_resume(project_id)
    })
}

#[tauri::command(async)]
fn session_attach(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: SessionId,
    title: String,
) -> Result<(), Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_attach(id, title)
    })
}

#[tauri::command(async)]
fn session_stop(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: SessionId,
) -> Result<(), Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.session_stop(id))
}

#[tauri::command(async)]
fn tool_install(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    harness: String,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.tool_install(&harness))
}

#[tauri::command(async)]
fn session_terminal_open(
    app: AppHandle,
    state: State<'_, EngineState>,
    id: SessionId,
) -> Result<(), Problem> {
    let sink_app = app.clone();
    let sink_id = id.to_string();
    let sink = move |chunk: Vec<u8>| {
        if let Err(e) = sink_app.emit(
            OUTPUT_EVENT,
            SessionOutput {
                id: sink_id.clone(),
                chunk,
            },
        ) {
            eprintln!("willie-app: cannot emit session output: {e}");
        }
    };
    query(&state, |engine| engine.session_terminal_open(id, sink))
}

#[tauri::command(async)]
fn session_terminal_input(
    state: State<'_, EngineState>,
    id: SessionId,
    data: String,
) -> Result<(), Problem> {
    with_engine(&state, |engine| {
        engine.session_terminal_input(id, data.as_bytes())
    })
}

#[tauri::command(async)]
fn session_terminal_resize(
    state: State<'_, EngineState>,
    id: SessionId,
    rows: u16,
    cols: u16,
) -> Result<(), Problem> {
    with_engine(&state, |engine| {
        engine.session_terminal_resize(id, rows, cols)
    })
}

#[tauri::command(async)]
fn session_terminal_close(
    state: State<'_, EngineState>,
    id: SessionId,
) -> Result<(), Problem> {
    with_engine(&state, |engine| engine.session_terminal_close(id))
}

#[tauri::command(async)]
fn open_in_explorer(path: String) -> Result<(), Problem> {
    std::process::Command::new("explorer.exe")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| Problem {
            code: "explorer_failed".into(),
            message: e.to_string(),
            remediation: "open the path manually".into(),
        })
}

/// Starts the desktop application. Exits the process on a startup failure
/// because there is no UI yet to report it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let resource_dir = app.path().resource_dir().ok();
            let env_override =
                std::env::var_os("WILLIE_ROOTFS").map(PathBuf::from);
            let candidates =
                image_candidates(env_override, resource_dir, &workspace_root());
            let engine = Engine::new(candidates);
            app.manage(EngineState(std::sync::Mutex::new(engine)));
            // No daemon runs yet at startup, so there is nothing to
            // subscribe to; the pump attaches lazily the first time a
            // command starts the daemon. See `events`.
            app.manage(EventPump::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            engine_status,
            engine_install_distro,
            engine_start_daemon,
            engine_stop_daemon,
            engine_doctor,
            engine_logon_fix_script,
            project_list,
            project_add,
            project_remove,
            project_sync_to_windows,
            project_update_from_windows,
            project_relocate,
            project_rename,
            project_set_sandbox,
            sandbox_catalogue,
            job_cancel,
            state_snapshot,
            projects_roots,
            set_projects_roots,
            discover_projects,
            open_in_explorer,
            session_open,
            session_resume,
            session_attach,
            session_stop,
            tool_install,
            session_terminal_open,
            session_terminal_input,
            session_terminal_resize,
            session_terminal_close
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("willie-app: failed to start: {error}");
        std::process::exit(1);
    }
}
