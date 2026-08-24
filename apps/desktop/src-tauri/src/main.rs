#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn daemon_start(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, "start").await
}

#[tauri::command]
async fn daemon_stop(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, "stop").await
}

#[tauri::command]
async fn daemon_restart(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, "restart").await
}

#[tauri::command]
async fn daemon_status(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, "status").await
}

async fn run_hologram(app: &AppHandle, command: &'static str) -> Result<String, String> {
    let output = app
        .shell()
        .sidecar("hologram")
        .map_err(|error| error.to_string())?
        .arg(command)
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
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            daemon_start,
            daemon_stop,
            daemon_restart,
            daemon_status
        ])
        .run(tauri::generate_context!())
        .expect("run Hologram desktop application");
}
