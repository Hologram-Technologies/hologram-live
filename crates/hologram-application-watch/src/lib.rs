//! Tauri-independent watched application project orchestration.
//!
//! This crate owns filesystem observation, persistence, debounce behavior, and
//! watched-project state. A host supplies the actual compile/import operation,
//! output/configuration paths, and a change notification callback. Keeping
//! those concerns injected makes the engine reusable by the desktop adapter or
//! a future hosted development control plane without depending on Tauri.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MANIFEST_NAME: &str = "hologram.json";
const DEBOUNCE: Duration = Duration::from_millis(600);

/// User-selected source project tracked by the watch engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Complete input supplied to the host's compile/import boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildRequest {
    pub id: String,
    pub manifest: PathBuf,
    pub project_name: String,
    pub output: PathBuf,
    pub previous_kappa: Option<String>,
}

/// Immutable catalog identity returned after a successful compile/import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    pub archive_kappa: String,
    pub archive_name: String,
}

type BuildProject = dyn Fn(BuildRequest) -> Result<BuildResult, String> + Send + Sync;
type NotifyChange = dyn Fn() + Send + Sync;

struct WatchHandle {
    record: WatchedHoloProject,
    _watcher: RecommendedWatcher,
    trigger: mpsc::Sender<()>,
}

/// Persistent registry and worker owner for watched source projects.
pub struct ApplicationWatchRegistry {
    watches: Mutex<BTreeMap<String, WatchHandle>>,
    persistence: Mutex<()>,
    persistence_file: PathBuf,
    build_root: PathBuf,
    build_project: Arc<BuildProject>,
    notify_change: Arc<NotifyChange>,
}

impl ApplicationWatchRegistry {
    /// Creates a registry whose host callback performs compilation and import.
    pub fn new(
        persistence_file: PathBuf,
        build_root: PathBuf,
        build_project: impl Fn(BuildRequest) -> Result<BuildResult, String> + Send + Sync + 'static,
        notify_change: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            watches: Mutex::new(BTreeMap::new()),
            persistence: Mutex::new(()),
            persistence_file,
            build_root,
            build_project: Arc::new(build_project),
            notify_change: Arc::new(notify_change),
        }
    }

    /// Restores every persisted registration, returning recoverable failures.
    pub fn restore(self: &Arc<Self>) -> Vec<String> {
        match self.load() {
            Ok(saved) => saved
                .into_iter()
                .filter_map(|mut record| {
                    "watching".clone_into(&mut record.status);
                    record.error = None;
                    self.register(record.directory.clone(), Some(record)).err()
                })
                .collect(),
            Err(error) => vec![error],
        }
    }

    /// Returns the current records in stable identifier order.
    pub fn list(&self) -> Result<Vec<WatchedHoloProject>, String> {
        let records = self
            .watches
            .lock()
            .map_err(|_| "watched project registry lock poisoned".to_owned())?
            .values()
            .map(|watched| watched.record.clone())
            .collect();
        Ok(records)
    }

    /// Registers and immediately builds a source directory.
    pub fn add(
        self: &Arc<Self>,
        directory: impl Into<PathBuf>,
    ) -> Result<WatchedHoloProject, String> {
        self.register(directory.into(), None)
    }

    /// Stops observing a project without deleting its last immutable archive.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let removed = self
            .watches
            .lock()
            .map_err(|_| "watched project registry lock poisoned".to_owned())?
            .remove(id);
        if removed.is_none() {
            return Err(format!("watched project {id} was not found"));
        }
        self.persist()?;
        self.emit_change();
        Ok(())
    }

    /// Schedules every project for a rebuild, for example after service start.
    pub fn schedule_all(&self) {
        if let Ok(watches) = self.watches.lock() {
            for watched in watches.values() {
                let _ = watched.trigger.send(());
            }
        }
    }

    fn register(
        self: &Arc<Self>,
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
        if let Some(existing) = self
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
        "watching".clone_into(&mut record.status);
        record.error = None;

        let (sender, receiver) = mpsc::channel();
        let callback_sender = sender.clone();
        let weak_registry = Arc::downgrade(self);
        let callback_id = id.clone();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                Ok(event) if is_relevant_event(&event) => {
                    let _ = callback_sender.send(());
                }
                Ok(_) => {}
                Err(error) => {
                    if let Some(registry) = weak_registry.upgrade() {
                        registry.mark_failed(&callback_id, format!("watch directory: {error}"));
                    }
                }
            })
            .map_err(|error| format!("create directory watcher: {error}"))?;
        watcher
            .watch(&directory, RecursiveMode::Recursive)
            .map_err(|error| format!("watch {}: {error}", directory.display()))?;

        self.watches
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
        if let Err(error) = self.persist() {
            if let Ok(mut watches) = self.watches.lock() {
                watches.remove(&id);
            }
            return Err(error);
        }
        self.emit_change();

        let worker_registry = Arc::clone(self);
        if let Err(error) = std::thread::Builder::new()
            .name(format!("holo-watch-{id}"))
            .spawn(move || worker_registry.watch_worker(id, receiver))
        {
            if let Ok(mut watches) = self.watches.lock() {
                watches.remove(&record.id);
            }
            let _ = self.persist();
            self.emit_change();
            return Err(format!("start directory watch worker: {error}"));
        }
        let _ = sender.send(());
        Ok(record)
    }

    fn watch_worker(self: Arc<Self>, id: String, receiver: mpsc::Receiver<()>) {
        while receiver.recv().is_ok() {
            loop {
                match receiver.recv_timeout(DEBOUNCE) {
                    Ok(()) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
            self.build_and_import(&id);
        }
    }

    fn build_and_import(&self, id: &str) {
        let Some(request) = self.project_build_input(id) else {
            return;
        };
        self.update_record(id, |record| {
            "compiling".clone_into(&mut record.status);
            record.error = None;
        });

        let result = request.output.parent().map_or_else(
            || Err("watched project output has no parent directory".to_owned()),
            |parent| {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!("create build directory {}: {error}", parent.display())
                })
            },
        );
        let result = result.and_then(|()| (self.build_project)(request));

        match result {
            Ok(build) => {
                self.update_record(id, |record| {
                    "ready".clone_into(&mut record.status);
                    record.archive_kappa = Some(build.archive_kappa);
                    record.archive_name = Some(build.archive_name);
                    record.last_compiled_at_millis = Some(now_millis());
                    record.error = None;
                });
                let _ = self.persist();
            }
            Err(error) => self.mark_failed(id, error),
        }
    }

    fn project_build_input(&self, id: &str) -> Option<BuildRequest> {
        let watch_map = self.watches.lock().ok()?;
        let watched = watch_map.get(id)?;
        Some(BuildRequest {
            id: id.to_owned(),
            manifest: watched.record.manifest.clone(),
            project_name: watched.record.name.clone(),
            output: self
                .build_root
                .join(id)
                .join(format!("{}.holo", safe_name(&watched.record.name))),
            previous_kappa: watched.record.archive_kappa.clone(),
        })
    }

    fn update_record(&self, id: &str, update: impl FnOnce(&mut WatchedHoloProject)) {
        if let Ok(mut watches) = self.watches.lock() {
            if let Some(watched) = watches.get_mut(id) {
                update(&mut watched.record);
            }
        }
        self.emit_change();
    }

    fn mark_failed(&self, id: &str, error: String) {
        self.update_record(id, |record| {
            "failed".clone_into(&mut record.status);
            record.error = Some(error);
        });
        let _ = self.persist();
    }

    fn persist(&self) -> Result<(), String> {
        let _persistence_guard = self
            .persistence
            .lock()
            .map_err(|_| "watched project persistence lock poisoned".to_owned())?;
        if let Some(parent) = self.persistence_file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create watched project directory: {error}"))?;
        }
        let bytes = serde_json::to_vec_pretty(&self.list()?).map_err(|error| error.to_string())?;
        let temporary = self.persistence_file.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &self.persistence_file)
            .map_err(|error| format!("replace {}: {error}", self.persistence_file.display()))
    }

    fn load(&self) -> Result<Vec<WatchedHoloProject>, String> {
        match std::fs::read(&self.persistence_file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse {}: {error}", self.persistence_file.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("read {}: {error}", self.persistence_file.display())),
        }
    }

    fn emit_change(&self) {
        (self.notify_change)();
    }
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
            attrs: notify::event::EventAttributes::default(),
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

    #[test]
    fn empty_registry_does_not_require_existing_storage_directories() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let registry = ApplicationWatchRegistry::new(
            temporary.path().join("config/watches.json"),
            temporary.path().join("cache"),
            |_| Err("build callback should not run".to_owned()),
            || {},
        );
        assert!(registry.list().expect("list registry").is_empty());
        assert!(Arc::new(registry).restore().is_empty());
    }

    #[test]
    fn registry_builds_persists_and_removes_a_project() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let project = temporary.path().join("demo app");
        std::fs::create_dir(&project).expect("create project");
        std::fs::write(project.join(MANIFEST_NAME), b"{}\n").expect("write manifest");
        let persistence_file = temporary.path().join("config/watches.json");
        let build_root = temporary.path().join("cache");
        let (built_sender, built_receiver) = mpsc::channel();
        let registry = Arc::new(ApplicationWatchRegistry::new(
            persistence_file.clone(),
            build_root.clone(),
            move |request| {
                built_sender.send(request).expect("record build request");
                Ok(BuildResult {
                    archive_kappa: "blake3:archive".to_owned(),
                    archive_name: "demo-app.holo".to_owned(),
                })
            },
            || {},
        ));

        let added = registry.add(&project).expect("add project");
        let request = built_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("debounced build request");
        let canonical_project = project.canonicalize().expect("canonical project");
        assert_eq!(request.id, added.id);
        assert_eq!(request.manifest, canonical_project.join(MANIFEST_NAME));
        assert_eq!(
            request.output,
            build_root.join(&added.id).join("demo-app.holo")
        );

        let ready = (0..100).find_map(|_| {
            let current = registry
                .list()
                .expect("list projects")
                .into_iter()
                .next()
                .expect("watched project");
            if current.status == "ready" {
                Some(current)
            } else {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        });
        let ready = ready.expect("build reaches ready state");
        assert_eq!(ready.archive_kappa.as_deref(), Some("blake3:archive"));
        assert!(persistence_file.is_file());

        registry.remove(&added.id).expect("remove watch");
        assert!(registry.list().expect("list after removal").is_empty());
        let saved: Vec<WatchedHoloProject> = serde_json::from_slice(
            &std::fs::read(persistence_file).expect("read persisted registry"),
        )
        .expect("decode persisted registry");
        assert!(saved.is_empty());
    }
}
