use hologram_application_watch::{
    ApplicationWatchRegistry, BuildRequest, BuildResult, WatchedHoloProject,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use crate::run_hologram;
use crate::view_surface::{DesktopHoloRuntime, DesktopSessionReport};

const CHANGE_EVENT: &str = "holo-watch-changed";
const SESSION_EVENT: &str = "holo-session-changed";

#[derive(Clone, Serialize)]
struct SessionEvent<'a> {
    #[serde(flatten)]
    session: &'a DesktopSessionReport,
    lifecycle: &'static str,
}

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
    let bytes = cached_holo_bytes(&app, &kappa).await?;
    let result = app
        .state::<DesktopHoloRuntime>()
        .execute(&bytes, vec![input.into_bytes()])
        .await?;
    let outputs = result
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            std::str::from_utf8(output)
                .map(str::to_owned)
                .map_err(|error| format!("application output {index} is not UTF-8: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&outputs).map_err(|error| format!("encode application output: {error}"))
}

#[tauri::command]
pub async fn holo_catalog_session_list(app: AppHandle) -> Vec<DesktopSessionReport> {
    app.state::<DesktopHoloRuntime>().sessions().await
}

#[tauri::command]
pub async fn holo_catalog_session_start(
    app: AppHandle,
    kappa: String,
) -> Result<DesktopSessionReport, String> {
    let bytes = cached_holo_bytes(&app, &kappa).await?;
    let report = app
        .state::<DesktopHoloRuntime>()
        .start_session(&kappa, &bytes)
        .await?;
    emit_session_event(&app, &report, "running");
    Ok(report)
}

#[tauri::command]
pub async fn holo_catalog_session_stop(
    app: AppHandle,
    session_id: String,
) -> Result<DesktopSessionReport, String> {
    let report = app
        .state::<DesktopHoloRuntime>()
        .stop_session(&session_id)
        .await?;
    emit_session_event(&app, &report, "stopped");
    Ok(report)
}

pub fn emit_session_event(
    app: &AppHandle,
    report: &DesktopSessionReport,
    lifecycle: &'static str,
) {
    let _ = app.emit(
        SESSION_EVENT,
        SessionEvent {
            session: report,
            lifecycle,
        },
    );
}

async fn cached_holo_bytes(app: &AppHandle, kappa: &str) -> Result<Vec<u8>, String> {
    let cache_directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("resolve application cache: {error}"))?
        .join("holo-run");
    std::fs::create_dir_all(&cache_directory)
        .map_err(|error| format!("create application run cache: {error}"))?;
    let archive = cache_directory.join(catalog_cache_name(kappa)?);

    run_hologram(app, ["--json", "holo", "verify", kappa]).await?;
    run_hologram(
        app,
        vec![
            OsString::from("--json"),
            OsString::from("files"),
            OsString::from("get"),
            OsString::from(&kappa),
            OsString::from("--output"),
            archive.as_os_str().to_owned(),
        ],
    )
    .await?;
    tokio::fs::read(&archive)
        .await
        .map_err(|error| format!("read cached .holo application: {error}"))
}

fn catalog_cache_name(kappa: &str) -> Result<String, String> {
    let digest = kappa
        .strip_prefix("blake3:")
        .filter(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| format!("invalid .holo catalog kappa {kappa:?}"))?;
    Ok(format!("{digest}.holo"))
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
        let imported = match run_hologram(app, import_arguments.clone()).await {
            Ok(imported) => imported,
            Err(error) if import_needs_larger_transport(&error) => {
                run_hologram(app, ["--json", "restart"])
                    .await
                    .map_err(|restart| {
                        format!(
                            "{error}\nrestart the local service for a larger .holo import: {restart}"
                        )
                    })?;
                run_hologram(app, import_arguments).await?
            }
            Err(error) => return Err(error),
        };
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

fn import_needs_larger_transport(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("h2 protocol error")
        || error.contains("message length too large")
        || error.contains("resource exhausted")
}

#[cfg(test)]
mod tests {
    use super::{catalog_cache_name, import_needs_larger_transport};

    #[test]
    fn catalog_cache_names_accept_only_a_blake3_digest() {
        let digest = "a".repeat(64);
        assert_eq!(
            catalog_cache_name(&format!("blake3:{digest}")).expect("valid kappa"),
            format!("{digest}.holo")
        );
        assert!(catalog_cache_name("../../application.holo").is_err());
        assert!(catalog_cache_name("blake3:not-a-digest").is_err());
    }

    #[test]
    fn only_transport_size_failures_restart_an_existing_local_service() {
        assert!(import_needs_larger_transport(
            "Internal error: h2 protocol error: stream reset"
        ));
        assert!(import_needs_larger_transport(
            "Resource exhausted: message length too large"
        ));
        assert!(!import_needs_larger_transport(
            "LIVE_INVALID_HOLO: malformed archive"
        ));
    }
}
