//! Tauri application shell.
//!
//! Commands are thin: lock the engine, call one method, emit the fresh
//! status. The UI never computes truth; it renders `EngineStatus`.

mod state;

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, State};
use willie_engine::error::EngineError;
use willie_engine::{Engine, EngineStatus, Problem};
use willie_proto::daemon::DoctorReport;

use crate::state::{EngineState, image_candidates, workspace_root};

const STATUS_EVENT: &str = "engine://status";

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

/// Starts the desktop application. Exits the process on a startup failure
/// because there is no UI yet to report it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let resource_dir = app.path().resource_dir().ok();
            let env_override =
                std::env::var_os("WILLIE_ROOTFS").map(PathBuf::from);
            let candidates =
                image_candidates(env_override, resource_dir, &workspace_root());
            let engine = Engine::new(candidates);
            app.manage(EngineState(std::sync::Mutex::new(engine)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            engine_status,
            engine_install_distro,
            engine_start_daemon,
            engine_stop_daemon,
            engine_doctor
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("willie-app: failed to start: {error}");
        std::process::exit(1);
    }
}
