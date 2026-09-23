//! Blob uploads: streamed in, hashed on write, linked only when the bytes match.
//!
//! A layer is never held in memory. The HTTP side re-cuts the request body
//! into exact [`FRAME`]-sized slices with [`Framer`] and appends them one at a
//! time; the store writes each to its staging file. Each store call opens the
//! file and keeps a record, so frames must be large: 20 GB in 4 MiB frames is
//! 5,120 calls, in hyper's 16 KiB pieces it would be 1.3 million.
//!
//! Hash on write (FR-020): `upload_finish` asks the store to hash the staged
//! bytes against the digest the client gave. Nothing becomes visible before
//! that succeeds, and a mismatch leaves nothing reachable. blake3 is computed
//! in the same pass as the bytes arrive, so a blob pushed by sha256 can also
//! be read by its blake3 address without a second read (ADR 030).

use super::links::UploadRow;
use super::{now_ms, Algorithm, Digest, OciStore, OciStoreError, RepoName, UploadId};
use kappa_core::types::StoreError;
use kappa_core::KappaStore;
use std::io::Read;
use std::sync::{Arc, Mutex, PoisonError};

/// The size of every slice handed to the store, except the last.
pub const FRAME: usize = 4 * 1024 * 1024;

/// The `uploads` row is rewritten at most this often while bytes arrive, so a
/// 20 GB push costs about a hundred row writes, not one per frame.
const ROW_WRITE_INTERVAL_MS: u64 = 5_000;

/// The store records who owns a namespace; the registry is the only writer.
const NAMESPACE_OWNER: &str = "hologram-registry";

/// Turns input slices of any size into exact [`FRAME`] slices.
#[derive(Default)]
pub struct Framer {
    buffer: bytes::BytesMut,
}

impl Framer {
    /// Feed bytes as they arrive; get back zero or more full frames.
    pub fn push(&mut self, input: &[u8]) -> impl Iterator<Item = bytes::Bytes> + '_ {
        self.buffer.extend_from_slice(input);
        std::iter::from_fn(move || {
            (self.buffer.len() >= FRAME).then(|| self.buffer.split_to(FRAME).freeze())
        })
    }

    /// The short last frame, if any.
    #[must_use]
    pub fn finish(self) -> Option<bytes::Bytes> {
        (!self.buffer.is_empty()).then(|| self.buffer.freeze())
    }
}

/// What `resume_uploads` found at start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeReport {
    /// Sessions re-attached to their staging file.
    pub resumed: usize,
    /// Rows whose staging file was gone.
    pub dropped_rows: usize,
    /// Staging files no row knew about.
    pub dropped_files: usize,
}

/// What a status request reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadStatus {
    pub repo: RepoName,
    pub received: u64,
}

pub(crate) struct SessionState {
    repo: RepoName,
    kappa_id: String,
    received: u64,
    /// `None` after a restart: the running hash cannot be saved, so the alias
    /// is computed from the finished blob instead.
    blake3: Option<blake3::Hasher>,
    created_ms: u64,
    /// Last append, for expiry by inactivity.
    touched_ms: u64,
    row_written_ms: u64,
}

pub(crate) type Sessions = Mutex<std::collections::HashMap<String, Arc<Mutex<SessionState>>>>;

fn store_io(what: &str, error: &StoreError) -> OciStoreError {
    OciStoreError::Io(format!("{what}: {error}"))
}

impl OciStore {
    /// Start an upload into `repo`. The repository is listed from this moment,
    /// as in the reference.
    ///
    /// # Errors
    ///
    /// `Io` when the store or the database refuses.
    pub fn upload_begin(&self, repo: &RepoName) -> Result<UploadId, OciStoreError> {
        let namespace = self
            .kappa()
            .namespace_resolve_or_create(repo.as_str(), NAMESPACE_OWNER, Some("oci"))
            .map_err(|error| store_io("resolve the repository namespace", &error))?;
        let kappa_id = self
            .kappa()
            .upload_begin(&namespace, 0)
            .map_err(|error| store_io("begin upload", &error))?;
        // The store's id is a hyphenated UUID, which is our public id too.
        let id = UploadId::parse(&kappa_id)?;
        let now = now_ms();
        self.upload_row_put(
            &id,
            &UploadRow {
                repo: repo.as_str().to_owned(),
                kappa_id: kappa_id.clone(),
                created_ms: now,
                touched_ms: now,
            },
        )?;
        self.repo_touch(repo)?;
        let state = SessionState {
            repo: repo.clone(),
            kappa_id,
            received: 0,
            blake3: Some(blake3::Hasher::new()),
            created_ms: now,
            touched_ms: now,
            row_written_ms: now,
        };
        self.sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.as_str().to_owned(), Arc::new(Mutex::new(state)));
        Ok(id)
    }

    /// Append `frame` at `offset`. Returns the new total.
    ///
    /// The session's lock is held for the whole call, so two appends on one
    /// session are ordered and the loser sees `OffsetMismatch`.
    ///
    /// # Errors
    ///
    /// `UnknownUpload`; `OffsetMismatch` when `offset` is not the current
    /// total; `Io` when the store refuses.
    pub fn upload_append(
        &self,
        id: &UploadId,
        offset: u64,
        frame: &[u8],
    ) -> Result<u64, OciStoreError> {
        let session = self.session(id)?;
        let mut state = session.lock().unwrap_or_else(PoisonError::into_inner);
        if offset != state.received {
            return Err(OciStoreError::OffsetMismatch {
                expected: state.received,
                got: offset,
            });
        }
        let total = self
            .kappa()
            .upload_put_part(&state.kappa_id, offset, frame)
            .map_err(|error| store_io("append to upload", &error))?;
        if let Some(hasher) = state.blake3.as_mut() {
            hasher.update(frame);
        }
        state.received = total;
        let now = now_ms();
        state.touched_ms = now;
        if now.saturating_sub(state.row_written_ms) >= ROW_WRITE_INTERVAL_MS {
            self.upload_row_put(
                id,
                &UploadRow {
                    repo: state.repo.as_str().to_owned(),
                    kappa_id: state.kappa_id.clone(),
                    created_ms: state.created_ms,
                    touched_ms: now,
                },
            )?;
            state.row_written_ms = now;
        }
        Ok(total)
    }

    /// # Errors
    ///
    /// `UnknownUpload` when there is no such session.
    pub fn upload_status(&self, id: &UploadId) -> Result<UploadStatus, OciStoreError> {
        let session = self.session(id)?;
        let state = session.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(UploadStatus {
            repo: state.repo.clone(),
            received: state.received,
        })
    }

    /// Verify the staged bytes against `claimed`, publish the blob, link it
    /// into the session's repository, record its alias, and drop the session.
    ///
    /// The session is removed first, so a second finish is `UnknownUpload`.
    ///
    /// # Errors
    ///
    /// `UnknownUpload`; `DigestMismatch` when the bytes do not hash to
    /// `claimed` (the session is gone and nothing is reachable); `Io`.
    pub fn upload_finish(&self, id: &UploadId, claimed: &Digest) -> Result<Digest, OciStoreError> {
        let session = self
            .sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id.as_str())
            .ok_or_else(|| OciStoreError::UnknownUpload(id.as_str().to_owned()))?;
        let state = session.lock().unwrap_or_else(PoisonError::into_inner);

        // 1. The store re-reads the staging file, hashes it, and refuses a
        //    mismatch. Nothing is visible before this returns Ok.
        let result = match self
            .kappa()
            .upload_complete(&state.kappa_id, Some(claimed.as_str()))
        {
            Ok(result) => result,
            Err(StoreError::Rejected(_)) => {
                self.upload_row_delete(id)?;
                return Err(OciStoreError::DigestMismatch {
                    claimed: claimed.as_str().to_owned(),
                });
            }
            Err(error) => return Err(store_io("complete upload", &error)),
        };
        let stored = Digest::parse(&result.kappa)?;

        // 2. The other name for the same bytes: our running blake3 for a
        //    sha256 or sha512 push; the store's mandatory sha256 for a blake3
        //    push. After a restart the running hash is gone: read the blob once.
        let alias = if stored.algorithm() == Algorithm::Blake3 {
            result
                .additional_kappas
                .iter()
                .find(|kappa| kappa.starts_with("sha256:"))
                .map(|kappa| Digest::parse(kappa))
                .transpose()?
        } else if let Some(hasher) = state.blake3.as_ref() {
            Some(Digest::from_blake3(&hasher.finalize()))
        } else {
            Some(self.blake3_of_stored(&stored)?)
        };

        // 3. Link, alias and the session row, in one transaction.
        self.commit_finished_upload(
            id,
            &state.repo,
            &stored,
            alias.as_ref(),
            result.newly_stored,
        )?;
        Ok(stored)
    }

    /// Abort a session and delete its staged bytes. Idempotent.
    ///
    /// # Errors
    ///
    /// `Io` when the store or the database refuses.
    pub fn upload_cancel(&self, id: &UploadId) -> Result<(), OciStoreError> {
        let session = self
            .sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id.as_str());
        if let Some(session) = session {
            let state = session.lock().unwrap_or_else(PoisonError::into_inner);
            self.kappa()
                .upload_abort(&state.kappa_id)
                .map_err(|error| store_io("abort upload", &error))?;
        }
        self.upload_row_delete(id)
    }

    /// Re-attach to the uploads a restart interrupted. Called once, by `open`.
    ///
    /// The offset always comes from the staging file's length, never from the
    /// row: the row is written at most every few seconds, the file is the
    /// truth. A torn last frame is fine, since finish verifies the whole.
    ///
    /// # Errors
    ///
    /// `Io` when the database or the staging directory cannot be read.
    pub fn resume_uploads(&self) -> Result<ResumeReport, OciStoreError> {
        let mut report = ResumeReport::default();
        let mut known = std::collections::HashSet::new();
        for (id, row) in self.upload_rows()? {
            let resumed = RepoName::parse(&row.repo).ok().and_then(|repo| {
                let namespace = self
                    .kappa()
                    .namespace_resolve_or_create(repo.as_str(), NAMESPACE_OWNER, Some("oci"))
                    .ok()?;
                let received = self
                    .kappa()
                    .upload_resume(&row.kappa_id, &namespace, 0)
                    .ok()?;
                Some((repo, received))
            });
            let Some((repo, received)) = resumed else {
                self.upload_row_delete(&id)?;
                report.dropped_rows += 1;
                continue;
            };
            known.insert(row.kappa_id.clone());
            let state = SessionState {
                repo,
                kappa_id: row.kappa_id,
                received,
                blake3: None,
                created_ms: row.created_ms,
                touched_ms: row.touched_ms,
                row_written_ms: row.touched_ms,
            };
            self.sessions()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.as_str().to_owned(), Arc::new(Mutex::new(state)));
            report.resumed += 1;
        }
        // Staging files no row knows: a crash between the store's begin and
        // our row, or leftovers of a finished upload.
        if let Ok(entries) = std::fs::read_dir(self.layout().staging()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
                if is_file && !known.contains(&name) && std::fs::remove_file(entry.path()).is_ok() {
                    report.dropped_files += 1;
                }
            }
        }
        Ok(report)
    }

    /// Abort every session untouched for longer than the purge age. The
    /// server calls this on a timer; `now_ms` is a parameter so a test can
    /// move the clock. Returns how many were aborted.
    ///
    /// # Errors
    ///
    /// `Io` when the store or the database refuses.
    pub fn purge_expired_uploads(&self, now_ms: u64) -> Result<usize, OciStoreError> {
        let expired = self.expired_uploads(now_ms);
        for id in &expired {
            self.upload_cancel(&UploadId::parse(id)?)?;
        }
        Ok(expired.len())
    }

    /// The sessions a purge at `now_ms` would abort, aborting none: the
    /// reference's `dryrun`.
    #[must_use]
    pub fn expired_uploads(&self, now_ms: u64) -> Vec<String> {
        let max_age_ms = u64::try_from(self.upload_max_age().as_millis()).unwrap_or(u64::MAX);
        self.sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, session)| {
                let state = session.lock().unwrap_or_else(PoisonError::into_inner);
                now_ms.saturating_sub(state.touched_ms) > max_age_ms
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn session(&self, id: &UploadId) -> Result<Arc<Mutex<SessionState>>, OciStoreError> {
        self.sessions()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id.as_str())
            .cloned()
            .ok_or_else(|| OciStoreError::UnknownUpload(id.as_str().to_owned()))
    }

    /// One streamed read of a stored blob, to find its blake3 address.
    fn blake3_of_stored(&self, stored: &Digest) -> Result<Digest, OciStoreError> {
        let mut reader = self
            .kappa()
            .blob_open(stored.as_str())
            .map_err(|error| store_io("open the finished blob", &error))?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| OciStoreError::Io(format!("read {stored}: {error}")))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(Digest::from_blake3(&hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci_store::OpenOptions;
    use std::time::Duration;

    fn test_store() -> (OciStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let options = OpenOptions {
            create: true,
            upload_max_age: Duration::from_hours(1),
        };
        (OciStore::open(dir.path(), options).expect("open"), dir)
    }

    fn repo(name: &str) -> RepoName {
        RepoName::parse(name).expect("repository name")
    }

    fn sha(seed: &str) -> Digest {
        Digest::parse(&format!("sha256:{seed:0>64}")).expect("digest")
    }

    #[test]
    fn a_wrong_digest_leaves_nothing() {
        let (store, _dir) = test_store();
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        store.upload_append(&id, 0, b"hello").expect("append");
        let wrong = sha("0");
        assert!(matches!(
            store.upload_finish(&id, &wrong),
            Err(OciStoreError::DigestMismatch { .. })
        ));
        assert!(matches!(
            store.blob_stat(&repo("a/x"), &wrong),
            Err(OciStoreError::NotInRepository { .. })
        ));
        let real = Digest::sha256_of(b"hello");
        assert!(
            matches!(
                store.blob_stat(&repo("a/x"), &real),
                Err(OciStoreError::NotInRepository { .. })
            ),
            "FR-020: nothing reachable is left"
        );
        assert!(
            matches!(
                store.upload_status(&id),
                Err(OciStoreError::UnknownUpload(_))
            ),
            "the session is gone"
        );
    }

    #[test]
    fn a_gap_or_an_overlap_is_refused_with_the_expected_offset() {
        let (store, _dir) = test_store();
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        assert_eq!(store.upload_append(&id, 0, b"abc").expect("append"), 3);
        assert!(matches!(
            store.upload_append(&id, 5, b"x"),
            Err(OciStoreError::OffsetMismatch {
                expected: 3,
                got: 5
            })
        ));
        assert!(matches!(
            store.upload_append(&id, 1, b"x"),
            Err(OciStoreError::OffsetMismatch {
                expected: 3,
                got: 1
            })
        ));
        assert_eq!(store.upload_status(&id).expect("status").received, 3);
    }

    #[test]
    fn a_zero_length_blob_is_a_blob() {
        let (store, _dir) = test_store();
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        let empty = Digest::sha256_of(b"");
        assert_eq!(store.upload_finish(&id, &empty).expect("finish"), empty);
        assert_eq!(store.blob_stat(&repo("a/x"), &empty).expect("stat").size, 0);
    }

    #[test]
    fn a_finished_blob_is_linked_and_a_second_finish_is_unknown() {
        let (store, _dir) = test_store();
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        store.upload_append(&id, 0, b"hello").expect("append");
        let digest = Digest::sha256_of(b"hello");
        assert_eq!(store.upload_finish(&id, &digest).expect("finish"), digest);
        assert_eq!(
            store.blob_stat(&repo("a/x"), &digest).expect("stat").size,
            5
        );
        assert!(matches!(
            store.upload_finish(&id, &digest),
            Err(OciStoreError::UnknownUpload(_))
        ));
    }

    #[test]
    fn a_cancelled_upload_is_gone_and_cancel_is_idempotent() {
        let (store, _dir) = test_store();
        let id = store.upload_begin(&repo("a/x")).expect("begin");
        store.upload_append(&id, 0, b"abc").expect("append");
        store.upload_cancel(&id).expect("cancel");
        assert!(matches!(
            store.upload_status(&id),
            Err(OciStoreError::UnknownUpload(_))
        ));
        store.upload_cancel(&id).expect("cancel again");
        assert!(
            store.repo_exists(&repo("a/x")).expect("exists"),
            "the repository is listed from the first upload POST"
        );
    }

    #[test]
    fn the_framer_emits_exact_frames_whatever_the_input_sizes() {
        let mut framer = Framer::default();
        let mut emitted = Vec::new();
        let inputs = [1, 16 * 1024, FRAME - 1, FRAME, FRAME + 1, 3];
        for len in inputs {
            emitted.extend(framer.push(&vec![7_u8; len]).map(|frame| frame.len()));
        }
        let tail = framer.finish().map(|frame| frame.len());
        assert!(emitted.iter().all(|len| *len == FRAME));
        let total: usize = emitted.iter().sum::<usize>() + tail.unwrap_or(0);
        assert_eq!(total, inputs.iter().sum::<usize>());
    }
}
