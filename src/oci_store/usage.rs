//! What the volume holds, for the gauges on `/metrics` (`operations.md`
//! section 3): refreshed once a minute, off the request path.

use super::{OciStore, OciStoreError};
use std::sync::PoisonError;

/// One reading of the volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Objects the registry itself stored: blobs and manifests, each once,
    /// under the name it recorded. The Kappa store's own records are not
    /// counted (errata E12).
    pub objects: u64,
    /// Every byte under the blob root, the store's own records included:
    /// what the volume spends on content.
    pub bytes: u64,
    pub repositories: u64,
}

impl OciStore {
    /// Count what the volume holds. Walks the blob tree, so it is for a
    /// background task, not a request.
    ///
    /// # Errors
    ///
    /// `Io` when the database or the blob tree cannot be read.
    pub fn usage(&self) -> Result<Usage, OciStoreError> {
        let mut bytes = 0_u64;
        for (_, leaf) in super::verify::leaf_directories(&self.layout().blob_root())? {
            let Ok(entries) = std::fs::read_dir(&leaf) else {
                // Swept between the listing and now.
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        bytes += metadata.len();
                    }
                }
            }
        }
        Ok(Usage {
            objects: self.object_count()?,
            bytes,
            repositories: self.repo_count()?,
        })
    }

    /// Room left on the volume for this process, in bytes.
    ///
    /// # Errors
    ///
    /// `Io` when the filesystem cannot be asked.
    pub fn free_space(&self) -> Result<u64, OciStoreError> {
        let root = self.layout().root();
        fs4::available_space(root).map_err(|error| {
            OciStoreError::Io(format!("free space of {}: {error}", root.display()))
        })
    }

    /// Upload sessions open now.
    #[must_use]
    pub fn uploads_in_progress(&self) -> usize {
        self.sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci_store::{Digest, OpenOptions, RepoName};

    #[test]
    fn usage_counts_the_registrys_objects_and_every_byte() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = OciStore::open(
            dir.path(),
            OpenOptions {
                create: true,
                upload_max_age: std::time::Duration::from_hours(1),
            },
        )
        .expect("open");
        assert_eq!(store.usage().expect("usage"), Usage::default());
        let bytes = b"one blob, two repositories";
        for name in ["a/one", "a/two"] {
            let repo = RepoName::parse(name).expect("repo");
            let id = store.upload_begin(&repo).expect("begin");
            store.upload_append(&id, 0, bytes).expect("append");
            assert_eq!(store.uploads_in_progress(), 1);
            store
                .upload_finish(&id, &Digest::sha256_of(bytes))
                .expect("finish");
        }
        let usage = store.usage().expect("usage");
        assert_eq!(usage.objects, 1, "stored once");
        assert_eq!(usage.repositories, 2);
        // The blob, and the store's records beside it.
        assert!(usage.bytes > bytes.len() as u64, "{usage:?}");
        assert_eq!(store.uploads_in_progress(), 0);
    }
}
