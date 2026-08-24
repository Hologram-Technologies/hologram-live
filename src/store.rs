use crate::error::{LiveError, Result};
use crate::protocol::ObjectMetadata;
use crate::util::{atomic_write, hex, now_millis};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ObjectStore {
    root: PathBuf,
    write_lock: Mutex<()>,
}

impl ObjectStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("blobs/blake3"))
            .map_err(|error| LiveError::io(&root, error))?;
        std::fs::create_dir_all(root.join("metadata"))
            .map_err(|error| LiveError::io(&root, error))?;
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    pub fn put(
        &self,
        kind: impl Into<String>,
        media_type: impl Into<String>,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("object store lock poisoned".to_owned()))?;
        let digest = blake3::hash(bytes);
        let digest_hex = digest.to_hex().to_string();
        let id = format!("blake3:{digest_hex}");
        let blob = self.blob_path(&digest_hex);
        if !blob.exists() {
            atomic_write(&blob, bytes)?;
        }
        let metadata = ObjectMetadata {
            id,
            kind: kind.into(),
            media_type: media_type.into(),
            filename,
            size: bytes.len().try_into().unwrap_or(u64::MAX),
            created_at_millis: now_millis(),
        };
        let encoded = serde_json::to_vec_pretty(&metadata)?;
        atomic_write(&self.metadata_path(&digest_hex), &encoded)?;
        Ok(metadata)
    }

    pub fn get(&self, id: &str) -> Result<Vec<u8>> {
        let digest = validate_id(id)?;
        let path = self.blob_path(digest);
        std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::NotFound(format!("object {id} not found"))
            } else {
                LiveError::io(&path, error)
            }
        })
    }

    pub fn metadata(&self, id: &str) -> Result<ObjectMetadata> {
        let digest = validate_id(id)?;
        let path = self.metadata_path(digest);
        let bytes = std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::NotFound(format!("metadata for {id} not found"))
            } else {
                LiveError::io(&path, error)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(Into::into)
    }

    pub fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        let filename = validate_filename(filename)?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("object store lock poisoned".to_owned()))?;
        let digest = validate_id(id)?;
        let path = self.metadata_path(digest);
        let bytes = std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                LiveError::NotFound(format!("metadata for {id} not found"))
            } else {
                LiveError::io(&path, error)
            }
        })?;
        let mut metadata: ObjectMetadata = serde_json::from_slice(&bytes)?;
        if metadata.kind != "file" {
            return Err(LiveError::NotFound(format!("file {id} not found")));
        }
        metadata.filename = Some(filename);
        let encoded = serde_json::to_vec_pretty(&metadata)?;
        atomic_write(&path, &encoded)?;
        Ok(metadata)
    }

    pub fn list(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        let directory = self.root.join("metadata");
        let mut output = Vec::new();
        for entry in
            std::fs::read_dir(&directory).map_err(|error| LiveError::io(&directory, error))?
        {
            let entry = entry.map_err(|error| LiveError::io(&directory, error))?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes =
                std::fs::read(entry.path()).map_err(|error| LiveError::io(&entry.path(), error))?;
            let metadata: ObjectMetadata = serde_json::from_slice(&bytes)?;
            if kind.is_none_or(|expected| metadata.kind == expected) {
                output.push(metadata);
            }
        }
        output.sort_by(|left, right| {
            right
                .created_at_millis
                .cmp(&left.created_at_millis)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(output)
    }

    pub fn remove_metadata(&self, id: &str) -> Result<()> {
        let digest = validate_id(id)?;
        let path = self.metadata_path(digest);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| LiveError::io(&path, error))?;
        }
        Ok(())
    }

    pub fn verify(&self, id: &str) -> Result<bool> {
        let bytes = self.get(id)?;
        let digest = blake3::hash(&bytes);
        Ok(format!("blake3:{}", hex(digest.as_bytes())) == id)
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        self.root.join("blobs/blake3").join(digest)
    }

    fn metadata_path(&self, digest: &str) -> PathBuf {
        self.root.join("metadata").join(format!("{digest}.json"))
    }
}

fn validate_id(id: &str) -> Result<&str> {
    let digest = id
        .strip_prefix("blake3:")
        .ok_or_else(|| LiveError::NotFound(format!("unsupported object id {id:?}")))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LiveError::NotFound(format!("malformed object id {id:?}")));
    }
    Ok(digest)
}

fn validate_filename(filename: String) -> Result<String> {
    let filename = filename.trim();
    if filename.is_empty() {
        return Err(LiveError::Protocol("filename cannot be empty".to_owned()));
    }
    if filename.len() > 255 {
        return Err(LiveError::Protocol(
            "filename cannot be longer than 255 characters".to_owned(),
        ));
    }
    if filename == "."
        || filename == ".."
        || filename.contains(['/', '\\'])
        || filename.chars().any(char::is_control)
    {
        return Err(LiveError::Protocol(
            "filename cannot contain path separators, control characters, or reserved names"
                .to_owned(),
        ));
    }
    Ok(filename.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_is_addressed_and_retrievable() {
        let root = std::env::temp_dir().join(format!("hologram-store-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let metadata = store
            .put("test", "application/octet-stream", None, b"hello")
            .expect("put");
        assert_eq!(store.get(&metadata.id).expect("get"), b"hello");
        assert!(store.verify(&metadata.id).expect("verify"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn list_can_filter_files_without_hiding_other_objects() {
        let root = std::env::temp_dir().join(format!("hologram-store-list-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let file = store
            .put("file", "text/plain", Some("hello.txt".to_owned()), b"hello")
            .expect("put file");
        store
            .put("holo", "application/vnd.hologram.holo", None, b"archive")
            .expect("put holo");

        let files = store.list(Some("file")).expect("list files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, file.id);
        assert_eq!(store.list(None).expect("list objects").len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rename_updates_metadata_without_changing_content_identity() {
        let root = std::env::temp_dir().join(format!("hologram-store-rename-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let original = store
            .put("file", "text/plain", None, b"hello")
            .expect("put");

        let renamed = store
            .rename_file(&original.id, "notes.txt".to_owned())
            .expect("rename");

        assert_eq!(renamed.id, original.id);
        assert_eq!(renamed.filename.as_deref(), Some("notes.txt"));
        assert_eq!(store.get(&original.id).expect("get"), b"hello");
        assert_eq!(
            store
                .metadata(&original.id)
                .expect("metadata")
                .filename
                .as_deref(),
            Some("notes.txt")
        );
        assert!(store.rename_file(&original.id, "  ".to_owned()).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
