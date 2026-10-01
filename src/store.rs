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
        let blob = self.blob_path(&digest_hex);
        if !blob.exists() {
            atomic_write(&blob, bytes)?;
        }
        self.finish_put(
            &digest_hex,
            bytes.len().try_into().unwrap_or(u64::MAX),
            kind,
            media_type,
            filename,
        )
    }

    /// The streaming half of [`ObjectStore::put`]: hashes while writing, so an
    /// object of any size lands without ever being held whole in memory. Phase
    /// 2b's cluster replication imports fetched blobs through it; everything
    /// else keeps the whole-bytes entry point.
    ///
    /// The id is only known at EOF, so the stream first lands in a randomly
    /// named temporary beside the blobs and is then renamed to the digest
    /// path. That is the durability discipline of `util::atomic_write` — the
    /// destination is never unlinked and is never observed partially written —
    /// with the temporary named per call rather than from the destination,
    /// which does not exist yet. Like `atomic_write` it does not fsync: the
    /// blob path has never needed crash durability (a missing blob is a cache
    /// miss, not corruption), and the reasoning is recorded on that function.
    pub fn put_reader(
        &self,
        kind: impl Into<String>,
        media_type: impl Into<String>,
        filename: Option<String>,
        mut reader: impl std::io::Read,
    ) -> Result<ObjectMetadata> {
        use std::io::Write as _;

        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("object store lock poisoned".to_owned()))?;
        let temporary = self.temporary_blob_path();
        let streamed = (|| -> Result<(String, u64)> {
            let mut file = std::fs::File::create(&temporary)
                .map_err(|error| LiveError::io(&temporary, error))?;
            let mut hasher = blake3::Hasher::new();
            let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
            let mut size = 0_u64;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        file.write_all(&buffer[..read])
                            .map_err(|error| LiveError::io(&temporary, error))?;
                        hasher.update(&buffer[..read]);
                        size = size.saturating_add(read as u64);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(LiveError::io(&temporary, error)),
                }
            }
            Ok((hasher.finalize().to_hex().to_string(), size))
        })();
        let (digest_hex, size) = match streamed {
            Ok(result) => result,
            // Leave nothing behind beside real state, as `atomic_write` does.
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        let blob = self.blob_path(&digest_hex);
        if blob.exists() {
            let _ = std::fs::remove_file(&temporary);
        } else if let Err(error) = std::fs::rename(&temporary, &blob) {
            let _ = std::fs::remove_file(&temporary);
            return Err(LiveError::io(&blob, error));
        }
        self.finish_put(&digest_hex, size, kind, media_type, filename)
    }

    /// A unique temporary name inside the blob directory, so the rename to the
    /// digest path stays on one filesystem. Mirrors `atomic_write`'s
    /// process-id-plus-sequence convention: two threads streaming at once must
    /// not rename each other's file away.
    fn temporary_blob_path(&self) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.root.join("blobs/blake3").join(format!(
            "tmp.{}.{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    /// The metadata half every put shares. Content addressing makes an object
    /// immutable, so its creation time is a property of the content's first
    /// appearance. Re-putting the same bytes must not move it: duplicate
    /// metadata records are resolved by greatest creation time, so a drifting
    /// timestamp would silently change which record wins.
    fn finish_put(
        &self,
        digest_hex: &str,
        size: u64,
        kind: impl Into<String>,
        media_type: impl Into<String>,
        filename: Option<String>,
    ) -> Result<ObjectMetadata> {
        let metadata = ObjectMetadata {
            id: format!("blake3:{digest_hex}"),
            kind: kind.into(),
            media_type: media_type.into(),
            filename,
            size,
            created_at_millis: match self.read_metadata(digest_hex) {
                Some(existing) => existing.created_at_millis,
                None => now_millis(),
            },
        };
        let encoded = serde_json::to_vec_pretty(&metadata)?;
        atomic_write(&self.metadata_path(digest_hex), &encoded)?;
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

    /// Cache bytes under an already-declared BLAKE3 content address without
    /// creating user-facing metadata. This is used for payloads unpacked from
    /// fat `.holo` archives so thin archives can resolve the same kappa later.
    pub fn cache_addressed(&self, id: &str, bytes: &[u8]) -> Result<()> {
        self.cache_addressed_bounded(id, bytes, u64::MAX)?;
        Ok(())
    }

    /// Cache verified bytes when their newly materialized size fits `max_new_bytes`.
    ///
    /// Returns `true` only when this call created the blob. The address check,
    /// existence check, byte ceiling, and atomic write share the store lock so
    /// capability mediators can account persistent storage without a race.
    pub fn cache_addressed_bounded(
        &self,
        id: &str,
        bytes: &[u8],
        max_new_bytes: u64,
    ) -> Result<bool> {
        let digest = validate_id(id)?;
        let actual = blake3::hash(bytes).to_hex().to_string();
        if actual != digest {
            return Err(LiveError::InvalidHolo(format!(
                "content blob {id} does not match its bytes"
            )));
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LiveError::Conflict("object store lock poisoned".to_owned()))?;
        let path = self.blob_path(digest);
        if path.exists() {
            return Ok(false);
        }
        let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if byte_count > max_new_bytes {
            return Err(LiveError::Capability(
                "addressed object exceeds the permitted new-byte ceiling".to_owned(),
            ));
        }
        atomic_write(&path, bytes)?;
        Ok(true)
    }

    pub fn get_cached(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let digest = validate_id(id)?;
        let path = self.blob_path(digest);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(LiveError::io(&path, error)),
        }
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

    /// The content-addressed file for a digest: `blobs/blake3/<digest>`.
    /// `pub(crate)` for Phase 2b's blob mirror, which imports these files into
    /// the iroh-blobs store by reference rather than copying them.
    pub(crate) fn blob_path(&self, digest: &str) -> PathBuf {
        self.root.join("blobs/blake3").join(digest)
    }

    fn metadata_path(&self, digest: &str) -> PathBuf {
        self.root.join("metadata").join(format!("{digest}.json"))
    }

    /// Best-effort read of an existing record. A missing or unreadable file is
    /// treated as absent: this only chooses a creation timestamp, and failing
    /// the whole write because a stale record will not parse would be worse.
    fn read_metadata(&self, digest: &str) -> Option<ObjectMetadata> {
        let bytes = std::fs::read(self.metadata_path(digest)).ok()?;
        serde_json::from_slice(&bytes).ok()
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

    #[test]
    fn reputting_identical_content_preserves_the_original_creation_time() {
        let root = std::env::temp_dir().join(format!("hologram-store-created-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let first = store
            .put("file", "text/plain", Some("a.txt".to_owned()), b"stable")
            .expect("first put");

        // now_millis() has millisecond resolution, so without a deliberate gap
        // a regression could pass by coincidence.
        std::thread::sleep(std::time::Duration::from_millis(5));

        let second = store
            .put("file", "text/plain", Some("a.txt".to_owned()), b"stable")
            .expect("second put");

        assert_eq!(first.id, second.id, "content addressing must be stable");
        assert_eq!(
            first.created_at_millis, second.created_at_millis,
            "creation time of immutable content must not move on re-put"
        );
        assert_eq!(
            store
                .metadata(&first.id)
                .expect("metadata")
                .created_at_millis,
            first.created_at_millis,
            "the persisted record must agree with the returned one"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn addressed_cache_does_not_create_or_replace_metadata() {
        let root = std::env::temp_dir().join(format!("hologram-store-cache-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let id = format!("blake3:{}", blake3::hash(b"layer"));

        store.cache_addressed(&id, b"layer").expect("cache");
        assert_eq!(store.get_cached(&id).expect("get"), Some(b"layer".to_vec()));
        assert!(store.metadata(&id).is_err());
        assert!(store.cache_addressed(&id, b"forged").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    // Phase 2b: the streaming put must be indistinguishable from the
    // whole-bytes put on the id it assigns — the cluster's replication path
    // stores through it while every other caller uses `put`, and both must
    // agree on what an object is.
    #[test]
    fn a_streamed_put_addresses_content_identically_to_a_whole_bytes_put() {
        let root = std::env::temp_dir().join(format!("hologram-store-stream-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let bytes: Vec<u8> = (0..100_000_u32).flat_map(u32::to_le_bytes).collect();

        let whole = store
            .put("file", "application/octet-stream", None, &bytes)
            .expect("whole-bytes put");
        let streamed = store
            .put_reader(
                "file",
                "application/octet-stream",
                None,
                std::io::Cursor::new(&bytes),
            )
            .expect("streamed put");

        assert_eq!(whole.id, streamed.id);
        assert_eq!(streamed.size, bytes.len() as u64);
        assert_eq!(store.get(&streamed.id).expect("get"), bytes);
        // The temporary file was renamed to the digest: nothing is left beside it.
        let temporaries = std::fs::read_dir(root.join("blobs/blake3"))
            .expect("read blob directory")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("tmp."))
            .count();
        assert_eq!(temporaries, 0, "a streamed put must clean up its temporary");
        let _ = std::fs::remove_dir_all(root);
    }

    // The empty input is the edge a streaming writer most easily gets wrong:
    // zero read calls with content still has to produce the empty hash's file.
    #[test]
    fn a_streamed_put_of_nothing_lands_under_the_empty_hash() {
        let root = std::env::temp_dir().join(format!("hologram-store-empty-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");

        let streamed = store
            .put_reader("file", "application/octet-stream", None, std::io::empty())
            .expect("streamed put of empty input");

        assert_eq!(
            streamed.id,
            format!("blake3:{}", blake3::hash(b"").to_hex())
        );
        assert_eq!(streamed.size, 0);
        assert_eq!(store.get(&streamed.id).expect("get"), Vec::<u8>::new());
        let _ = std::fs::remove_dir_all(root);
    }

    // A reader is free to yield content in arbitrary chunks; the digest must
    // not depend on where the chunk boundaries fell.
    #[test]
    fn a_streamed_put_digests_bytes_not_chunk_boundaries() {
        struct Chunked {
            remaining: std::collections::VecDeque<u8>,
            chunk: usize,
        }
        impl std::io::Read for Chunked {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let take = self.chunk.min(buffer.len()).min(self.remaining.len());
                for slot in buffer.iter_mut().take(take) {
                    *slot = self.remaining.pop_front().expect("within length");
                }
                Ok(take)
            }
        }

        let root = std::env::temp_dir().join(format!("hologram-store-chunks-{}", now_millis()));
        let store = ObjectStore::open(&root).expect("open");
        let bytes: Vec<u8> = (0..255_u8).cycle().take(10_000).collect();

        let streamed = store
            .put_reader(
                "file",
                "application/octet-stream",
                None,
                Chunked {
                    remaining: bytes.iter().copied().collect(),
                    chunk: 3,
                },
            )
            .expect("streamed put over odd-sized chunks");

        assert_eq!(
            streamed.id,
            format!("blake3:{}", blake3::hash(&bytes).to_hex())
        );
        assert_eq!(store.get(&streamed.id).expect("get"), bytes);
        let _ = std::fs::remove_dir_all(root);
    }
}
