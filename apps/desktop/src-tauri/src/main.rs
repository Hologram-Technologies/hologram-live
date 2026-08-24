#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsStr;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn daemon_start(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["start"]).await
}

#[tauri::command]
async fn daemon_stop(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["stop"]).await
}

#[tauri::command]
async fn daemon_restart(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["restart"]).await
}

#[tauri::command]
async fn daemon_status(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["status"]).await
}

#[tauri::command]
async fn modules_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "modules", "list"]).await
}

#[tauri::command]
async fn objects_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "registry", "list"]).await
}

#[tauri::command]
async fn file_put(app: AppHandle, path: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "files", "put", path.as_str()]).await
}

#[tauri::command]
async fn object_get(app: AppHandle, id: String, output: String) -> Result<String, String> {
    run_hologram(
        &app,
        ["registry", "get", id.as_str(), "--output", output.as_str()],
    )
    .await
}

async fn run_hologram<I, S>(app: &AppHandle, arguments: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let command = app
        .shell()
        .sidecar("hologram")
        .map_err(|error| error.to_string())?;
    let output = arguments
        .into_iter()
        .fold(command, |command, argument| command.arg(argument.as_ref()))
        .output()
        .await
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(if stdout.trim().is_empty() { stderr } else { stdout })
    } else {
        Err(if stderr.trim().is_empty() { stdout } else { stderr })
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            daemon_start,
            daemon_stop,
            daemon_restart,
            daemon_status,
            modules_list,
            objects_list,
            file_put,
            object_get
        ])
        .run(tauri::generate_context!())
        .expect("run Hologram desktop application");
}
