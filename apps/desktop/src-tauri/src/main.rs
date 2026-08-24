#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsStr;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_shell::ShellExt;

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
    let result = run_hologram(&app, &["status"]).await;
    update_menu_bar(&app, result.is_ok());
    result
}

#[tauri::command]
async fn modules_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "modules", "list"]).await
}

#[tauri::command]
async fn history_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, &["--json", "history", "list"]).await
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
        [
            "--json",
            "files",
            "rename",
            id.as_str(),
            filename.as_str(),
        ],
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

async fn run_service_action(
    app: &AppHandle,
    arguments: &[&str],
    running_after: bool,
) -> Result<String, String> {
    let result = run_hologram(app, arguments).await;
    if result.is_ok() {
        publish_service_state(app, running_after);
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
            object_get
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
