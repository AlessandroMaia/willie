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
use willie_core::session::{Session, SessionKind};
use willie_engine::config::Ui;
use willie_engine::discover::Candidate;
use willie_engine::error::EngineError;
use willie_engine::{Engine, EngineStatus, Problem, SessionOpened};
use willie_harness::{ClaudeCode, Harness};
use willie_proto::daemon::DoctorReport;
use willie_proto::plugin::PluginStatus;
use willie_proto::project::{
    AddResult, JobRef, ProjectList, ReadFileResult, TreeResult,
};
use willie_proto::sandbox::CapabilityInfo;
use willie_proto::state::Snapshot;
use willie_proto::tool::ToolList;
use willie_proto::usage::UsageSnapshot;

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

#[tauri::command(async)]
fn project_tree(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    path: Option<String>,
) -> Result<TreeResult, Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.project_tree(id, path))
}

#[tauri::command(async)]
fn project_read_file(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: ProjectId,
    path: String,
) -> Result<ReadFileResult, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.project_read_file(id, path)
    })
}

/// The capability catalogue: static domain data, so no engine lock and
/// no daemon. The app renders this copy verbatim rather than keeping a
/// second copy of ten user-facing sentences in TypeScript — and, since
/// each row carries layer 1's answer, without restating the harness
/// defaults either: a row the project's profile says nothing about
/// shows what the harness decided, whichever harness that is.
#[tauri::command]
fn sandbox_catalogue() -> Vec<CapabilityInfo> {
    let defaults = ClaudeCode.default_capabilities();
    willie_core::sandbox::Capability::ALL
        .iter()
        .map(|&c| CapabilityInfo {
            capability: c,
            display_name: c.display_name().to_owned(),
            consequence: c.consequence().to_owned(),
            implemented: c.is_implemented(),
            default_enabled: defaults.enabled(c),
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
fn ui_prefs(state: State<'_, EngineState>) -> Result<Ui, Problem> {
    with_engine(&state, |engine| engine.ui_prefs())
}

#[tauri::command(async)]
fn set_ui_prefs(
    state: State<'_, EngineState>,
    prefs: Ui,
) -> Result<(), Problem> {
    query(&state, |engine| engine.set_ui_prefs(prefs))
}

#[tauri::command(async)]
fn session_open(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    project_id: ProjectId,
    kind: Option<SessionKind>,
) -> Result<SessionOpened, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_open(project_id, kind.unwrap_or_default())
    })
}

#[tauri::command(async)]
fn session_resume(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    project_id: ProjectId,
    resume_from: Option<SessionId>,
) -> Result<SessionOpened, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_resume(project_id, resume_from)
    })
}

#[tauri::command(async)]
fn session_rename(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: SessionId,
    label: Option<String>,
) -> Result<Session, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.session_rename(id, label)
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
fn tool_list(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
) -> Result<ToolList, Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.tool_list())
}

#[tauri::command(async)]
fn tool_update(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    tool: String,
) -> Result<JobRef, Problem> {
    daemon_command(&app, &state, &pump, |engine| engine.tool_update(&tool))
}

#[tauri::command(async)]
fn plugin_list(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
) -> Result<Vec<PluginStatus>, Problem> {
    daemon_command(&app, &state, &pump, Engine::plugin_list)
}

#[tauri::command(async)]
fn usage_snapshot(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
) -> Result<UsageSnapshot, Problem> {
    daemon_command(&app, &state, &pump, Engine::usage_snapshot)
}

#[tauri::command(async)]
fn plugin_enable(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: String,
    project_id: Option<ProjectId>,
) -> Result<PluginStatus, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.plugin_enable(id, project_id)
    })
}

#[tauri::command(async)]
fn plugin_disable(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    id: String,
    project_id: Option<ProjectId>,
) -> Result<PluginStatus, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.plugin_disable(id, project_id)
    })
}

/// The webview's only path to a plugin's own methods (`profile.*`, …):
/// one generic command mirroring `Engine::plugin_call`, which is itself
/// the only place that decides what `method` may forward — never here.
#[tauri::command(async)]
fn plugin_call(
    app: AppHandle,
    state: State<'_, EngineState>,
    pump: State<'_, EventPump>,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, Problem> {
    daemon_command(&app, &state, &pump, |engine| {
        engine.plugin_call(method, params)
    })
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
    query(&state, |engine| {
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
    query(&state, |engine| {
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

/// The Remote-WSL arguments that open `workspace` on the willie distro:
/// VS Code's documented form for a folder in a named WSL distribution.
/// `file`, when given, opens with that file active in the folder window.
/// The tree and the preview hand it over workspace-relative, and VS
/// Code resolves a relative argument against the *launching* process's
/// directory — a Windows path — so it is joined onto the workspace here
/// with the distro's own separator and passed absolute.
fn editor_argv(workspace: &str, file: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        "--remote".to_owned(),
        format!("wsl+{}", willie_engine::wsl::DISTRO_NAME),
        workspace.to_owned(),
    ];
    if let Some(file) = file {
        argv.push(if file.starts_with('/') {
            file.to_owned()
        } else {
            format!("{}/{}", workspace.trim_end_matches('/'), file)
        });
    }
    argv
}

/// The candidate walk behind `locate_code`, pure so it is deterministic
/// under test regardless of whether this machine has VS Code installed:
/// `path` first (PATH-style, `;`-joined dirs), then the per-user
/// (`local`, i.e. `%LOCALAPPDATA%`) and per-machine (`pf`, i.e.
/// `%ProgramFiles%`) install locations. Detected by presence with
/// `symlink_metadata` — never executed — so probing cannot open a window.
fn code_in(
    path: Option<&std::ffi::OsStr>,
    local: Option<&std::ffi::OsStr>,
    pf: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    fn present(p: &std::path::Path) -> bool {
        std::fs::symlink_metadata(p).is_ok()
    }

    if let Some(path) = path
        && let Some(found) = std::env::split_paths(path)
            .map(|dir| dir.join("code.cmd"))
            .find(|p| present(p))
    {
        return Some(found);
    }

    if let Some(local) = local {
        let p = std::path::Path::new(local)
            .join("Programs")
            .join("Microsoft VS Code")
            .join("bin")
            .join("code.cmd");
        if present(&p) {
            return Some(p);
        }
    }

    if let Some(pf) = pf {
        let p = std::path::Path::new(pf)
            .join("Microsoft VS Code")
            .join("bin")
            .join("code.cmd");
        if present(&p) {
            return Some(p);
        }
    }

    None
}

/// VS Code's `code.cmd` launcher, if installed: the PATH first, then the
/// per-user and per-machine install locations. `None` when VS Code is
/// absent.
fn locate_code() -> Option<PathBuf> {
    code_in(
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("LOCALAPPDATA").as_deref(),
        std::env::var_os("ProgramFiles").as_deref(),
    )
}

#[tauri::command(async)]
fn open_in_editor(
    workspace: String,
    file: Option<String>,
) -> Result<(), Problem> {
    let code = locate_code().ok_or_else(|| Problem {
        code: "editor_not_found".into(),
        message: "VS Code was not found on this machine".into(),
        remediation: "install VS Code and its `code` command, or reopen \
                      Willie so it detects a new install"
            .into(),
    })?;
    let mut command = std::process::Command::new(&code);
    command.args(editor_argv(&workspace, file.as_deref()));
    // GUI-subsystem release builds have no console; std runs code.cmd via
    // cmd.exe (a console app), which would otherwise flash a fresh console
    // window. CREATE_NO_WINDOW suppresses it.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map(|_| ()).map_err(|e| Problem {
        code: "editor_launch_failed".into(),
        message: e.to_string(),
        remediation: "try opening the workspace from VS Code directly".into(),
    })
}

#[tauri::command]
fn editor_available() -> bool {
    locate_code().is_some()
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
            ui_prefs,
            set_ui_prefs,
            open_in_explorer,
            open_in_editor,
            editor_available,
            session_open,
            session_resume,
            session_rename,
            session_attach,
            session_stop,
            project_tree,
            project_read_file,
            tool_install,
            tool_list,
            tool_update,
            plugin_list,
            usage_snapshot,
            plugin_enable,
            plugin_disable,
            plugin_call,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_argv_opens_the_workspace_on_the_willie_distro() {
        assert_eq!(
            editor_argv("/home/willie/projects/x", None),
            vec![
                "--remote".to_owned(),
                "wsl+willie".to_owned(),
                "/home/willie/projects/x".to_owned(),
            ]
        );
    }

    /// A file argument opens the folder window with that file active,
    /// for the workspace tree's "Open in VS Code" on a single file. It
    /// must be absolute in the distro: a relative one would be resolved
    /// against the Windows-side working directory and name nothing.
    #[test]
    fn editor_argv_appends_the_file_after_the_workspace() {
        assert_eq!(
            editor_argv("/home/willie/projects/x", Some("src/main.rs"),),
            vec![
                "--remote".to_owned(),
                "wsl+willie".to_owned(),
                "/home/willie/projects/x".to_owned(),
                "/home/willie/projects/x/src/main.rs".to_owned(),
            ]
        );
    }

    /// A workspace with a trailing separator, and a file already
    /// absolute, both still produce exactly one separator and no
    /// duplicated prefix.
    #[test]
    fn editor_argv_joins_the_file_without_doubling_the_separator() {
        assert_eq!(
            editor_argv("/home/willie/projects/x/", Some("src/main.rs"))[3],
            "/home/willie/projects/x/src/main.rs"
        );
        assert_eq!(
            editor_argv("/home/willie/projects/x", Some("/etc/hosts"))[3],
            "/etc/hosts"
        );
    }

    /// A scratch directory this test owns exclusively (named with the
    /// process id so parallel test binaries never collide), holding a
    /// planted `code.cmd` under `bin/` the way a real install would.
    fn plant_launcher(label: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-code-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn code_in_finds_the_launcher_on_path() {
        let dir = plant_launcher("path");
        let launcher = dir.join("code.cmd");
        std::fs::write(&launcher, "@echo off\n").unwrap();

        let path = std::ffi::OsString::from(&dir);
        let found = code_in(Some(&path), None, None);

        assert_eq!(found, Some(launcher));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_in_finds_the_per_user_install_when_path_has_none() {
        let dir = plant_launcher("local");
        let bin = dir.join("Programs").join("Microsoft VS Code").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let launcher = bin.join("code.cmd");
        std::fs::write(&launcher, "@echo off\n").unwrap();

        let local = std::ffi::OsString::from(&dir);
        let found = code_in(None, Some(&local), None);

        assert_eq!(found, Some(launcher));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_in_is_none_when_no_candidate_exists() {
        let dir = plant_launcher("none");
        // The directory exists but holds no code.cmd anywhere.
        let path = std::ffi::OsString::from(&dir);
        let local = std::ffi::OsString::from(&dir);
        let pf = std::ffi::OsString::from(&dir);

        let found = code_in(Some(&path), Some(&local), Some(&pf));

        assert_eq!(found, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
