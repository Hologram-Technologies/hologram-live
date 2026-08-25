#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod holo_watch;

use serde::Serialize;
use std::ffi::OsStr;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_shell::ShellExt;

// Rootfs-backed applications include a compressed OCI image. The desktop owns
// its local service lifecycle, so give both its CLI clients and spawned service
// enough headroom to import the documented Python examples.
const DESKTOP_MAX_RPC_BYTES: &str = "268435456";

struct MenuBarItems {
    status: MenuItem<Wry>,
    start: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    stop: MenuItem<Wry>,
}

#[tauri::command]
async fn service_start(app: AppHandle) -> Result<String, String> {
    run_service_action(&app, &["start"], true).await
}

#[tauri::command]
async fn service_stop(app: AppHandle) -> Result<String, String> {
    run_service_action(&app, &["stop"], false).await
}

#[tauri::command]
async fn service_restart(app: AppHandle) -> Result<String, String> {
    run_service_action(&app, &["restart"], true).await
}

#[tauri::command]
async fn service_status(app: AppHandle) -> Result<String, String> {
    let result = run_hologram(&app, &["--json", "status"]).await;
    update_menu_bar(&app, result.is_ok());
    result
}

#[tauri::command]
async fn modules_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "modules", "list"]).await
}

#[tauri::command]
async fn history_list(app: AppHandle) -> Result<String, String> {
    // The desktop groups archived threads itself, so it asks for the full set.
    run_hologram(&app, &["--json", "history", "list", "--all"]).await
}

#[tauri::command]
async fn history_archive(app: AppHandle, id: String, archived: bool) -> Result<String, String> {
    let action = if archived { "archive" } else { "unarchive" };
    run_hologram(&app, ["--json", "history", action, id.as_str()]).await
}

#[tauri::command]
async fn history_create(app: AppHandle, title: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "history", "new", title.as_str()]).await
}

#[tauri::command]
async fn history_get(app: AppHandle, id: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "history", "show", id.as_str()]).await
}

#[tauri::command]
async fn chat_send(app: AppHandle, id: String, content: String) -> Result<String, String> {
    run_hologram(
        &app,
        ["--json", "chat", "send", id.as_str(), content.as_str()],
    )
    .await
}

#[tauri::command]
async fn objects_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "files", "list"]).await
}

#[tauri::command]
async fn file_put(app: AppHandle, path: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "files", "put", path.as_str()]).await
}

#[tauri::command]
async fn file_rename(app: AppHandle, id: String, filename: String) -> Result<String, String> {
    run_hologram(
        &app,
        ["--json", "files", "rename", id.as_str(), filename.as_str()],
    )
    .await
}

#[tauri::command]
async fn object_get(app: AppHandle, id: String, output: String) -> Result<String, String> {
    run_hologram(
        &app,
        ["registry", "get", id.as_str(), "--output", output.as_str()],
    )
    .await
}

#[tauri::command]
async fn config_show(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["config", "show"]).await
}

#[derive(Serialize)]
struct SystemInfo {
    host: String,
    cores: usize,
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    disk_used_bytes: u64,
    disk_total_bytes: u64,
}

#[tauri::command]
fn system_info() -> SystemInfo {
    let mut system = sysinfo::System::new();
    system.refresh_memory();

    // Report the disk backing the home directory, falling back to the largest volume.
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let disk = home
        .and_then(|home| {
            disks
                .list()
                .iter()
                .filter(|disk| home.starts_with(disk.mount_point()))
                .max_by_key(|disk| disk.mount_point().as_os_str().len())
        })
        .or_else(|| disks.list().iter().max_by_key(|disk| disk.total_space()));
    let (disk_total_bytes, disk_available_bytes) = disk
        .map(|disk| (disk.total_space(), disk.available_space()))
        .unwrap_or((0, 0));

    SystemInfo {
        host: sysinfo::System::host_name().unwrap_or_else(|| "this device".to_owned()),
        cores: system.physical_core_count().unwrap_or(0),
        memory_used_bytes: system.used_memory(),
        memory_total_bytes: system.total_memory(),
        disk_used_bytes: disk_total_bytes.saturating_sub(disk_available_bytes),
        disk_total_bytes,
    }
}

pub(crate) async fn run_hologram<I, S>(app: &AppHandle, arguments: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let command = app
        .shell()
        .sidecar("hologram")
        .map_err(|error| error.to_string())?
        .env("HOLOGRAM_MAX_RPC_BYTES", DESKTOP_MAX_RPC_BYTES);
    let output = arguments
        .into_iter()
        .fold(command, |command, argument| command.arg(argument.as_ref()))
        .output()
        .await
        .map_err(|error| error.to_string())?;
    let stdout = strip_ansi(&String::from_utf8_lossy(&output.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        })
    } else {
        Err(if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        })
    }
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            output.push(character);
            continue;
        }
        if characters.next_if_eq(&'[').is_some() {
            for control in characters.by_ref() {
                if ('@'..='~').contains(&control) {
                    break;
                }
            }
        } else {
            // Non-CSI escape sequences have a one-character final byte.
            let _ = characters.next();
        }
    }
    output
}

async fn run_service_action(
    app: &AppHandle,
    arguments: &[&str],
    running_after: bool,
) -> Result<String, String> {
    let result = run_hologram(app, arguments).await;
    if result.is_ok() {
        publish_service_state(app, running_after);
        if running_after {
            holo_watch::schedule_all(app);
        }
    }
    result
}

fn publish_service_state(app: &AppHandle, running: bool) {
    update_menu_bar(app, running);
    let _ = app.emit(
        "service-state-changed",
        if running { "ready" } else { "stopped" },
    );
}

fn update_menu_bar(app: &AppHandle, running: bool) {
    let items = app.state::<MenuBarItems>();
    let _ = items.status.set_text(if running {
        "Hologram is ready"
    } else {
        "Hologram is stopped"
    });
    let _ = items.start.set_enabled(!running);
    let _ = items.restart.set_enabled(running);
    let _ = items.stop.set_enabled(running);
    if let Some(tray) = app.tray_by_id("hologram") {
        let _ = tray.set_tooltip(Some(if running {
            "Hologram — Ready"
        } else {
            "Hologram — Stopped"
        }));
    }
}

fn run_menu_action(app: &AppHandle, arguments: &'static [&'static str], running_after: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_service_action(&app, arguments, running_after).await;
    });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            holo_watch::initialize(app)?;
            let open = MenuItem::with_id(app, "open", "Open Hologram", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let status = MenuItem::with_id(
                app,
                "service-status",
                "Checking Hologram…",
                false,
                None::<&str>,
            )?;
            let start = MenuItem::with_id(app, "start", "Start Hologram", false, None::<&str>)?;
            let restart =
                MenuItem::with_id(app, "restart", "Restart Hologram", false, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop Hologram", false, None::<&str>)?;
            let actions_separator = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Hologram", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open,
                    &separator,
                    &status,
                    &start,
                    &restart,
                    &stop,
                    &actions_separator,
                    &quit,
                ],
            )?;
            app.manage(MenuBarItems {
                status,
                start,
                restart,
                stop,
            });
            let mut tray = TrayIconBuilder::with_id("hologram")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .tooltip("Hologram")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "start" => run_menu_action(app, &["start"], true),
                    "restart" => run_menu_action(app, &["restart"], true),
                    "stop" => run_menu_action(app, &["stop"], false),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let running = run_hologram(&handle, &["status"]).await.is_ok();
                update_menu_bar(&handle, running);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            service_start,
            service_stop,
            service_restart,
            service_status,
            modules_list,
            history_list,
            history_create,
            history_get,
            chat_send,
            objects_list,
            file_put,
            file_rename,
            object_get,
            holo_watch::holo_catalog_list,
            holo_watch::holo_catalog_inspect,
            holo_watch::holo_catalog_import,
            holo_watch::holo_catalog_run,
            holo_watch::holo_watch_list,
            holo_watch::holo_watch_add,
            holo_watch::holo_watch_remove,
            history_archive,
            config_show,
            system_info
        ])
        .run(tauri::generate_context!())
        .expect("run Hologram desktop application");
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn sidecar_diagnostics_are_plain_text() {
        assert_eq!(
            strip_ansi("\u{1b}[2mstamp\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m: failed"),
            "stamp ERROR: failed"
        );
        assert_eq!(strip_ansi("already plain"), "already plain");
    }
}
