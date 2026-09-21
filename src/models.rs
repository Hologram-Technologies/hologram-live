use crate::error::{LiveError, Result};
use crate::protocol::ObjectMetadata;
use crate::store::ObjectStore;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use utoipa::ToSchema;

pub const MODEL_KIND: &str = "model";
const MODEL_MEDIA_TYPE: &str = "application/vnd.hologram.model+json";
const GGUF_FILE_NAME: &str = "model.gguf";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub source: String,
    pub size: u64,
    pub created_at_millis: u64,
}

/// Small JSON document stored as the object-store blob for a model. The
/// artifact itself is a directory, so it cannot be a single blob; the manifest
/// is the content-addressed record and the files live under
/// `data_dir/models/<digest>/`.
#[derive(Debug, Serialize, Deserialize)]
struct ModelManifest {
    name: String,
    engine: String,
    source: String,
    size: u64,
    files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_blake3: Option<String>,
}

/// Catalog of imported inference models.
///
/// Registration metadata lives in the shared object store under
/// `kind = "model"`; the copied `.wcpu` artifact directory lives under the
/// catalog root so engines receive a stable local path.
pub struct ModelCatalog {
    store: Arc<ObjectStore>,
    models_dir: PathBuf,
}

impl ModelCatalog {
    pub fn open(store: Arc<ObjectStore>, models_dir: impl Into<PathBuf>) -> Result<Self> {
        let models_dir = models_dir.into();
        std::fs::create_dir_all(&models_dir).map_err(|error| LiveError::io(&models_dir, error))?;
        Ok(Self { store, models_dir })
    }

    /// Import a local `.wcpu` artifact directory produced by `weightc`.
    pub fn import(&self, source: &Path) -> Result<ModelInfo> {
        if !source.is_dir() {
            return Err(LiveError::Protocol(format!(
                "{} is not a .wcpu artifact directory",
                source.display()
            )));
        }
        if !source.join("manifest.json").is_file() {
            return Err(LiveError::Protocol(format!(
                "{} has no manifest.json; not a weightc artifact directory",
                source.display()
            )));
        }
        let files = collect_files(source)?;
        let size = files.iter().map(|(_, bytes)| bytes).sum();
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "model.wcpu".to_owned());
        let name = name.strip_suffix(".wcpu").unwrap_or(&name).to_owned();
        let manifest = ModelManifest {
            name,
            engine: "weightc".to_owned(),
            source: source.display().to_string(),
            size,
            files: files.into_iter().map(|(path, _)| path).collect(),
            content_blake3: None,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        let metadata = self.store.put(
            MODEL_KIND,
            MODEL_MEDIA_TYPE,
            Some(manifest.name.clone()),
            &bytes,
        )?;
        let destination = self.artifact_path(&metadata.id);
        if destination.exists() {
            std::fs::remove_dir_all(&destination)
                .map_err(|error| LiveError::io(&destination, error))?;
        }
        copy_directory(source, &destination)?;
        Ok(model_info(metadata, &manifest))
    }

    /// Import a single-file GGUF model for the in-process `llamacpp` engine.
    pub fn import_gguf(&self, source: &Path) -> Result<ModelInfo> {
        if !source.is_file()
            || source
                .extension()
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("gguf"))
        {
            return Err(LiveError::Protocol(format!(
                "{} is not a .gguf file",
                source.display()
            )));
        }
        let mut input =
            std::fs::File::open(source).map_err(|error| LiveError::io(source, error))?;
        let mut staged = tempfile::NamedTempFile::new_in(&self.models_dir)
            .map_err(|error| LiveError::io(&self.models_dir, error))?;
        let mut hasher = blake3::Hasher::new();
        let mut magic = [0_u8; 4];
        if let Err(error) = input.read_exact(&mut magic) {
            if error.kind() != std::io::ErrorKind::UnexpectedEof {
                return Err(LiveError::io(source, error));
            }
        }
        if &magic != b"GGUF" {
            return Err(LiveError::Protocol(format!(
                "{} does not have GGUF magic bytes",
                source.display()
            )));
        }
        hasher.update(&magic);
        staged
            .write_all(&magic)
            .map_err(|error| LiveError::io(staged.path(), error))?;
        let mut size = 4_u64;
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| LiveError::io(source, error))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            staged
                .write_all(&buffer[..read])
                .map_err(|error| LiveError::io(staged.path(), error))?;
            size = size.saturating_add(read.try_into().unwrap_or(u64::MAX));
        }
        let name = source
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "model".to_owned());
        let manifest = ModelManifest {
            name,
            engine: "llamacpp".to_owned(),
            source: source.display().to_string(),
            size,
            files: vec![GGUF_FILE_NAME.to_owned()],
            content_blake3: Some(format!("blake3:{}", hasher.finalize().to_hex())),
        };
        let encoded = serde_json::to_vec_pretty(&manifest)?;
        let metadata = self.store.put(
            MODEL_KIND,
            MODEL_MEDIA_TYPE,
            Some(manifest.name.clone()),
            &encoded,
        )?;
        let destination = self.artifact_path(&metadata.id);
        std::fs::create_dir_all(&destination)
            .map_err(|error| LiveError::io(&destination, error))?;
        let file = destination.join(GGUF_FILE_NAME);
        if !file.exists() {
            staged
                .persist_noclobber(&file)
                .map_err(|error| LiveError::io(&file, error.error))?;
        }
        Ok(model_info(metadata, &manifest))
    }

    pub fn list(&self) -> Result<Vec<ModelInfo>> {
        self.store
            .list(Some(MODEL_KIND))?
            .into_iter()
            .map(|metadata| {
                let manifest = self.manifest(&metadata.id)?;
                Ok(model_info(metadata, &manifest))
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<ModelInfo> {
        let metadata = self.metadata(id)?;
        let manifest = self.manifest(id)?;
        Ok(model_info(metadata, &manifest))
    }

    /// Look up a model by blake3 id first, then by name.
    pub fn resolve(&self, id_or_name: &str) -> Result<ModelInfo> {
        if let Ok(info) = self.get(id_or_name) {
            return Ok(info);
        }
        self.list()?
            .into_iter()
            .find(|info| info.name == id_or_name)
            .ok_or_else(|| LiveError::NotFound(format!("unknown model {id_or_name}")))
    }

    /// Local path of the copied artifact directory for an imported model.
    pub fn artifact_dir(&self, id: &str) -> Result<PathBuf> {
        let _ = self.get(id)?;
        let path = self.artifact_path(id);
        if !path.is_dir() {
            return Err(LiveError::NotFound(format!(
                "artifact directory for model {id} is missing"
            )));
        }
        Ok(path)
    }

    /// Local path of an imported single-file GGUF artifact.
    pub fn artifact_file(&self, id: &str) -> Result<PathBuf> {
        let file = self.artifact_dir(id)?.join(GGUF_FILE_NAME);
        if !file.is_file() {
            return Err(LiveError::NotFound(format!(
                "model {id} has no {GGUF_FILE_NAME}; it may be a .wcpu artifact"
            )));
        }
        Ok(file)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let _ = self.get(id)?;
        self.store.remove_metadata(id)?;
        let path = self.artifact_path(id);
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|error| LiveError::io(&path, error))?;
        }
        Ok(())
    }

    fn metadata(&self, id: &str) -> Result<ObjectMetadata> {
        let metadata = self.store.metadata(id)?;
        if metadata.kind != MODEL_KIND {
            return Err(LiveError::NotFound(format!(
                "object {id} is not cataloged as a model"
            )));
        }
        Ok(metadata)
    }

    fn manifest(&self, id: &str) -> Result<ModelManifest> {
        let bytes = self.store.get(id)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn artifact_path(&self, id: &str) -> PathBuf {
        let digest = id.strip_prefix("blake3:").unwrap_or(id);
        self.models_dir.join(digest)
    }
}

fn model_info(metadata: ObjectMetadata, manifest: &ModelManifest) -> ModelInfo {
    ModelInfo {
        id: metadata.id,
        name: manifest.name.clone(),
        engine: manifest.engine.clone(),
        source: manifest.source.clone(),
        size: manifest.size,
        created_at_millis: metadata.created_at_millis,
    }
}

fn collect_files(root: &Path) -> Result<Vec<(String, u64)>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries =
            std::fs::read_dir(&directory).map_err(|error| LiveError::io(&directory, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| LiveError::io(&directory, error))?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let size = entry
                    .metadata()
                    .map_err(|error| LiveError::io(&path, error))?
                    .len();
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| {
                        LiveError::Protocol(format!(
                            "{} escapes the artifact directory",
                            path.display()
                        ))
                    })?
                    .to_string_lossy()
                    .into_owned();
                files.push((relative, size));
            }
        }
    }
    files.sort();
    Ok(files)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination).map_err(|error| LiveError::io(destination, error))?;
    let entries = std::fs::read_dir(source).map_err(|error| LiveError::io(source, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| LiveError::io(source, error))?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if path.is_dir() {
            copy_directory(&path, &target)?;
        } else {
            std::fs::copy(&path, &target).map_err(|error| LiveError::io(&target, error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        temporary: tempfile::TempDir,
        store_root: PathBuf,
        models_dir: PathBuf,
        artifact: PathBuf,
    }

    fn fixture() -> Fixture {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path();
        let artifact = root.join("tiny.wcpu");
        std::fs::create_dir_all(artifact.join("weights")).expect("artifact dirs");
        std::fs::write(artifact.join("manifest.json"), br#"{"format":"wcpu"}"#)
            .expect("write manifest");
        std::fs::write(artifact.join("weights/layer0.bin"), b"weights").expect("write weights");
        Fixture {
            store_root: root.join("store"),
            models_dir: root.join("models"),
            artifact,
            temporary,
        }
    }

    #[test]
    fn import_list_and_remove_round_trip() {
        let fixture = fixture();
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");

        let imported = catalog.import(&fixture.artifact).expect("import");
        assert_eq!(imported.name, "tiny");
        assert_eq!(imported.engine, "weightc");
        assert!(imported.size > 0);

        let listed = catalog.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, imported.id);

        let artifact = catalog.artifact_dir(&imported.id).expect("artifact dir");
        assert!(artifact.join("manifest.json").is_file());
        assert!(artifact.join("weights/layer0.bin").is_file());

        catalog.remove(&imported.id).expect("remove");
        assert!(catalog.list().expect("list after remove").is_empty());
        assert!(!artifact.exists());
        assert!(catalog.get(&imported.id).is_err());
    }

    #[test]
    fn gguf_import_records_a_single_file_and_the_llamacpp_engine() {
        let fixture = fixture();
        let source = fixture.temporary.path().join("tiny.gguf");
        std::fs::write(&source, b"GGUF\x03\x00\x00\x00rest-of-file").expect("write gguf");
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");

        let info = catalog.import_gguf(&source).expect("import");

        assert_eq!(info.engine, "llamacpp");
        assert_eq!(info.name, "tiny");
        let stored = catalog.artifact_file(&info.id).expect("artifact file");
        assert_eq!(
            std::fs::read(stored).expect("read stored"),
            b"GGUF\x03\x00\x00\x00rest-of-file"
        );
    }

    #[test]
    fn gguf_import_rejects_a_directory_and_bad_magic() {
        let fixture = fixture();
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");
        let directory = fixture.temporary.path().join("not-a-file.gguf");
        std::fs::create_dir_all(&directory).expect("directory");
        assert!(catalog.import_gguf(&directory).is_err());

        let bad = fixture.temporary.path().join("bad.gguf");
        std::fs::write(&bad, b"not gguf").expect("bad fixture");
        let error = catalog.import_gguf(&bad).expect_err("bad magic");
        assert!(error.to_string().contains("magic"), "{error}");
    }

    #[test]
    fn gguf_identity_covers_model_bytes() {
        let fixture = fixture();
        let source = fixture.temporary.path().join("tiny.gguf");
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");
        std::fs::write(&source, b"GGUFfirst").expect("first model");
        let first = catalog.import_gguf(&source).expect("first import");
        std::fs::write(&source, b"GGUFother").expect("second model");
        let second = catalog.import_gguf(&source).expect("second import");

        assert_ne!(first.id, second.id);
        assert_eq!(
            std::fs::read(catalog.artifact_file(&first.id).expect("first file"))
                .expect("first bytes"),
            b"GGUFfirst"
        );
    }

    #[test]
    fn import_rejects_non_artifact_paths() {
        let fixture = fixture();
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");

        let file = fixture.artifact.join("manifest.json");
        assert!(catalog.import(&file).is_err());

        let empty = fixture.models_dir.with_file_name("empty.wcpu");
        std::fs::create_dir_all(&empty).expect("empty dir");
        assert!(catalog.import(&empty).is_err());
    }

    #[test]
    fn resolve_matches_by_id_or_name() {
        let fixture = fixture();
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");
        let imported = catalog.import(&fixture.artifact).expect("import");

        assert_eq!(
            catalog.resolve(&imported.id).expect("by id").id,
            imported.id
        );
        assert_eq!(catalog.resolve("tiny").expect("by name").id, imported.id);
        assert!(catalog.resolve("ghost").is_err());
    }

    #[test]
    fn reimporting_the_same_artifact_keeps_a_stable_id() {
        let fixture = fixture();
        let store = Arc::new(ObjectStore::open(&fixture.store_root).expect("store"));
        let catalog = ModelCatalog::open(store, &fixture.models_dir).expect("catalog");

        let first = catalog.import(&fixture.artifact).expect("first import");
        let second = catalog.import(&fixture.artifact).expect("second import");
        assert_eq!(first.id, second.id);
        assert_eq!(catalog.list().expect("list").len(), 1);
        assert!(catalog.artifact_dir(&first.id).expect("artifact").is_dir());
    }
}
