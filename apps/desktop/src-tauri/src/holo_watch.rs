use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

use crate::run_hologram;

const MANIFEST_NAME: &str = "hologram.json";
const CHANGE_EVENT: &str = "holo-watch-changed";
const DEBOUNCE: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedHoloProject {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub manifest: PathBuf,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_kappa: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compiled_at_millis: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct WatchHandle {
    record: WatchedHoloProject,
    _watcher: RecommendedWatcher,
    trigger: mpsc::Sender<()>,
}

#[derive(Default)]
pub struct HoloWatchState {
    watches: Mutex<BTreeMap<String, WatchHandle>>,
}

#[derive(Deserialize)]
struct ImportedHolo {
    kappa: String,
    name: String,
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
pub fn holo_watch_list(app: AppHandle) -> Result<Vec<WatchedHoloProject>, String> {
    records(&app)
}

#[tauri::command]
pub fn holo_watch_add(app: AppHandle, path: String) -> Result<WatchedHoloProject, String> {
    register(&app, PathBuf::from(path), None)
}

#[tauri::command]
pub fn holo_watch_remove(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<HoloWatchState>();
    let removed = state
        .watches
        .lock()
        .map_err(|_| "watched project registry lock poisoned".to_owned())?
        .remove(&id);
    if removed.is_none() {
        return Err(format!("watched project {id} was not found"));
    }
    persist(&app)?;
    emit_change(&app);
    Ok(())
}

pub fn restore(app: &AppHandle) {
    let saved = load(app).unwrap_or_default();
    for mut record in saved {
        record.status = "watching".to_owned();
        record.error = None;
        if let Err(error) = register(app, record.directory.clone(), Some(record)) {
            eprintln!("restore watched .holo project: {error}");
        }
    }
}

pub fn schedule_all(app: &AppHandle) {
    let state = app.state::<HoloWatchState>();
    if let Ok(watches) = state.watches.lock() {
        for watched in watches.values() {
            let _ = watched.trigger.send(());
        }
    };
}

fn register(
    app: &AppHandle,
    directory: PathBuf,
    previous: Option<WatchedHoloProject>,
) -> Result<WatchedHoloProject, String> {
    let directory = directory
        .canonicalize()
        .map_err(|error| format!("open watched directory {}: {error}", directory.display()))?;
    if !directory.is_dir() {
        return Err(format!("{} is not a directory", directory.display()));
    }
    let manifest = directory.join(MANIFEST_NAME);
    if !manifest.is_file() {
        return Err(format!(
            "{} does not contain {MANIFEST_NAME}",
            directory.display()
        ));
    }
    let id = watch_id(&directory);
    let state = app.state::<HoloWatchState>();
    if let Some(existing) = state
        .watches
        .lock()
        .map_err(|_| "watched project registry lock poisoned".to_owned())?
        .get(&id)
    {
        return Ok(existing.record.clone());
    }

    let mut record = previous.unwrap_or_else(|| WatchedHoloProject {
        id: id.clone(),
        name: project_name(&directory),
        directory: directory.clone(),
        manifest: manifest.clone(),
        status: "watching".to_owned(),
        archive_kappa: None,
        archive_name: None,
        last_compiled_at_millis: None,
        error: None,
    });
    record.id.clone_from(&id);
    record.name = project_name(&directory);
    record.directory.clone_from(&directory);
    record.manifest = manifest;
    record.status = "watching".to_owned();
    record.error = None;

    let (sender, receiver) = mpsc::channel();
    let callback_sender = sender.clone();
    let callback_app = app.clone();
    let callback_id = id.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) if is_relevant_event(&event) => {
                let _ = callback_sender.send(());
            }
            Ok(_) => {}
            Err(error) => mark_failed(
                &callback_app,
                &callback_id,
                format!("watch directory: {error}"),
            ),
        })
        .map_err(|error| format!("create directory watcher: {error}"))?;
    watcher
        .watch(&directory, RecursiveMode::Recursive)
        .map_err(|error| format!("watch {}: {error}", directory.display()))?;

    state
        .watches
        .lock()
        .map_err(|_| "watched project registry lock poisoned".to_owned())?
        .insert(
            id.clone(),
            WatchHandle {
                record: record.clone(),
                _watcher: watcher,
                trigger: sender.clone(),
            },
        );
    if let Err(error) = persist(app) {
        if let Ok(mut watches) = state.watches.lock() {
            watches.remove(&id);
        }
        return Err(error);
    }
    emit_change(app);

    let worker_app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(format!("holo-watch-{id}"))
        .spawn(move || watch_worker(worker_app, id, receiver))
    {
        if let Ok(mut watches) = state.watches.lock() {
            watches.remove(&record.id);
        }
        let _ = persist(app);
        emit_change(app);
        return Err(format!("start directory watch worker: {error}"));
    }
    let _ = sender.send(());
    Ok(record)
}

fn watch_worker(app: AppHandle, id: String, receiver: mpsc::Receiver<()>) {
    while receiver.recv().is_ok() {
        loop {
            match receiver.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        tauri::async_runtime::block_on(build_and_import(&app, &id));
    }
}

async fn build_and_import(app: &AppHandle, id: &str) {
    let Some((manifest, project_name, previous_kappa)) = project_build_input(app, id) else {
        return;
    };
    update_record(app, id, |record| {
        record.status = "compiling".to_owned();
        record.error = None;
    });

    let result = async {
        let output = build_output(app, id, &project_name)?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create build directory {}: {error}", parent.display()))?;
        }
        let compile_arguments = vec![
            OsString::from("--json"),
            OsString::from("compile"),
            manifest.as_os_str().to_owned(),
            OsString::from("--output"),
            output.as_os_str().to_owned(),
        ];
        run_hologram(app, compile_arguments).await?;
        let import_arguments = vec![
            OsString::from("--json"),
            OsString::from("holo"),
            OsString::from("import"),
            output.as_os_str().to_owned(),
        ];
        let imported = run_hologram(app, import_arguments).await?;
        let inspection: ImportedHolo = serde_json::from_str(&imported)
            .map_err(|error| format!("decode imported .holo inspection: {error}"))?;
        Ok::<_, String>(inspection)
    }
    .await;

    match result {
        Ok(inspection) => {
            if let Some(previous) = previous_kappa.filter(|value| value != &inspection.kappa) {
                let _ = run_hologram(app, ["--json", "holo", "remove", previous.as_str()]).await;
            }
            update_record(app, id, |record| {
                record.status = "ready".to_owned();
                record.archive_kappa = Some(inspection.kappa);
                record.archive_name = Some(inspection.name);
                record.last_compiled_at_millis = Some(now_millis());
                record.error = None;
            });
            let _ = persist(app);
        }
        Err(error) => mark_failed(app, id, error),
    }
}

fn project_build_input(app: &AppHandle, id: &str) -> Option<(PathBuf, String, Option<String>)> {
    let state = app.state::<HoloWatchState>();
    let watches = state.watches.lock().ok()?;
    let watched = watches.get(id)?;
    Some((
        watched.record.manifest.clone(),
        watched.record.name.clone(),
        watched.record.archive_kappa.clone(),
    ))
}

fn update_record(app: &AppHandle, id: &str, update: impl FnOnce(&mut WatchedHoloProject)) {
    let state = app.state::<HoloWatchState>();
    if let Ok(mut watches) = state.watches.lock() {
        if let Some(watched) = watches.get_mut(id) {
            update(&mut watched.record);
        }
    }
    emit_change(app);
}

fn mark_failed(app: &AppHandle, id: &str, error: String) {
    update_record(app, id, |record| {
        record.status = "failed".to_owned();
        record.error = Some(error);
    });
    let _ = persist(app);
}

fn records(app: &AppHandle) -> Result<Vec<WatchedHoloProject>, String> {
    let state = app.state::<HoloWatchState>();
    let records = state
        .watches
        .lock()
        .map_err(|_| "watched project registry lock poisoned".to_owned())?
        .values()
        .map(|watched| watched.record.clone())
        .collect();
    Ok(records)
}

fn persist(app: &AppHandle) -> Result<(), String> {
    let path = persistence_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create watched project directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&records(app)?).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("replace {}: {error}", path.display()))
}

fn load(app: &AppHandle) -> Result<Vec<WatchedHoloProject>, String> {
    let path = persistence_path(app)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}

fn persistence_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("watched-holo-projects.json"))
        .map_err(|error| error.to_string())
}

fn build_output(app: &AppHandle, id: &str, project_name: &str) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|directory| {
            directory
                .join("holo-watch")
                .join(id)
                .join(format!("{}.holo", safe_name(project_name)))
        })
        .map_err(|error| error.to_string())
}

fn safe_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('-');
    if value.is_empty() {
        "application".to_owned()
    } else {
        value.to_owned()
    }
}

fn watch_id(directory: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    directory.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn project_name(directory: &Path) -> String {
    directory.file_name().map_or_else(
        || "Hologram application".to_owned(),
        |name| name.to_string_lossy().into(),
    )
}

fn is_relevant_event(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event.paths.iter().any(|path| {
            !path.components().any(|component| {
                let Component::Normal(name) = component else {
                    return false;
                };
                matches!(
                    name.to_str(),
                    Some(".git" | ".venv" | "node_modules" | "target" | "__pycache__")
                )
            }) && path.file_name().and_then(|name| name.to_str()) != Some(".DS_Store")
        })
}

fn emit_change(app: &AppHandle) {
    let _ = app.emit(CHANGE_EVENT, ());
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_identity_is_stable_for_a_directory() {
        let path = Path::new("/tmp/example-holo");
        assert_eq!(watch_id(path), watch_id(path));
        assert_ne!(watch_id(path), watch_id(Path::new("/tmp/other-holo")));
    }

    #[test]
    fn watcher_ignores_dependency_and_build_trees() {
        let event = |path: &str| Event {
            kind: EventKind::Any,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        };
        assert!(!is_relevant_event(&event("project/.git/index")));
        assert!(!is_relevant_event(&event(
            "project/node_modules/pkg/index.js"
        )));
        assert!(!is_relevant_event(&event("project/target/app.wasm")));
        assert!(is_relevant_event(&event("project/dist/app.wasm")));
        assert!(is_relevant_event(&event("project/hologram.json")));
    }

    #[test]
    fn project_names_are_safe_archive_filenames() {
        assert_eq!(safe_name("My demo app"), "My-demo-app");
        assert_eq!(safe_name("***"), "application");
    }
}
