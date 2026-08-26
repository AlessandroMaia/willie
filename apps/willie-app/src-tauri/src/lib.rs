//! Tauri application shell.
//!
//! Hosts the React UI, the tray icon and the engine. Commands exposed to the
//! webview are thin: they translate UI actions into engine calls and never
//! hold state of their own.

/// Version of the running app, shown in the UI shell.
#[tauri::command]
fn app_version() -> &'static str {
    willie_core::VERSION
}

/// Starts the desktop application. Exits the process on a startup failure
/// because there is no UI yet to report it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("willie-app: failed to start: {error}");
        std::process::exit(1);
    }
}
