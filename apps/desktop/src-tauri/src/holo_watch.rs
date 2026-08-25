use hologram_application_watch::{
    ApplicationWatchRegistry, BuildRequest, BuildResult, WatchedHoloProject,
};
use serde::Deserialize;
use std::ffi::OsString;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::run_hologram;

const CHANGE_EVENT: &str = "holo-watch-changed";

#[derive(Deserialize)]
struct ImportedHolo {
    kappa: String,
    name: String,
}

pub fn initialize(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let persistence_file = app
        .path()
        .app_config_dir()?
        .join("watched-holo-projects.json");
    let build_root = app.path().app_cache_dir()?.join("holo-watch");

    let build_app = app.handle().clone();
    let change_app = app.handle().clone();
    let registry = Arc::new(ApplicationWatchRegistry::new(
        persistence_file,
        build_root,
        move |request| build_and_import(&build_app, request),
        move || {
            let _ = change_app.emit(CHANGE_EVENT, ());
        },
    ));
    for error in registry.restore() {
        eprintln!("restore watched .holo project: {error}");
    }
    app.manage(registry);
    Ok(())
}

#[tauri::command]
pub async fn holo_catalog_list(app: AppHandle) -> Result<String, String> {
    run_hologram(&app, ["--json", "holo", "list"]).await
}

#[tauri::command]
pub async fn holo_catalog_inspect(app: AppHandle, kappa: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "holo", "inspect", kappa.as_str()]).await
}

#[tauri::command]
pub async fn holo_catalog_import(app: AppHandle, path: String) -> Result<String, String> {
    run_hologram(&app, ["--json", "holo", "import", path.as_str()]).await
}

#[tauri::command]
pub async fn holo_catalog_run(
    app: AppHandle,
    kappa: String,
    input: String,
) -> Result<String, String> {
    run_hologram(&app, ["--json", "holo", "load", kappa.as_str()]).await?;
    run_hologram(
        &app,
        [
            "--json",
            "run",
            kappa.as_str(),
            "--input-text",
            input.as_str(),
            "--output-format",
            "text",
        ],
    )
    .await
}

#[tauri::command]
pub fn holo_watch_list(app: AppHandle) -> Result<Vec<WatchedHoloProject>, String> {
    app.state::<Arc<ApplicationWatchRegistry>>().list()
}

#[tauri::command]
pub fn holo_watch_add(app: AppHandle, path: String) -> Result<WatchedHoloProject, String> {
    app.state::<Arc<ApplicationWatchRegistry>>().add(path)
}

#[tauri::command]
pub fn holo_watch_remove(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<Arc<ApplicationWatchRegistry>>().remove(&id)
}

pub fn schedule_all(app: &AppHandle) {
    app.state::<Arc<ApplicationWatchRegistry>>().schedule_all();
}

fn build_and_import(app: &AppHandle, request: BuildRequest) -> Result<BuildResult, String> {
    tauri::async_runtime::block_on(async {
        let compile_arguments = vec![
            OsString::from("--json"),
            OsString::from("compile"),
            request.manifest.as_os_str().to_owned(),
            OsString::from("--output"),
            request.output.as_os_str().to_owned(),
        ];
        run_hologram(app, compile_arguments).await?;
        let import_arguments = vec![
            OsString::from("--json"),
            OsString::from("holo"),
            OsString::from("import"),
            request.output.as_os_str().to_owned(),
        ];
        let imported = run_hologram(app, import_arguments).await?;
        let inspection: ImportedHolo = serde_json::from_str(&imported)
            .map_err(|error| format!("decode imported .holo inspection: {error}"))?;

        if let Some(previous) = request
            .previous_kappa
            .filter(|value| value != &inspection.kappa)
        {
            let _ = run_hologram(app, ["--json", "holo", "remove", previous.as_str()]).await;
        }
        Ok(BuildResult {
            archive_kappa: inspection.kappa,
            archive_name: inspection.name,
        })
    })
}
