//! Blob reads, scoped to a repository, streamed, with hash alias resolution.
//!
//! A blob is readable through a repository only if that repository links it:
//! the link check comes first and beats everything, even if the bytes exist
//! for another repository. The store is asked by the digest the client used;
//! on a miss the alias table supplies the other name for the same bytes
//! (sha256 for a blake3 request and back, ADR 030).
//!
//! Nothing here reads a whole blob. `blob_get` and `blob_get_range` allocate
//! the full result and are banned from this directory by
//! `scripts/check-oci-streaming.sh`.

use super::{Digest, OciStore, OciStoreError, RepoName};
use kappa_core::types::StoreError;
use kappa_core::KappaStore;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::ReadBuf;

/// A synchronous, seekable blob. Our own trait, so the store's reader type
/// does not leak out of this directory.
pub trait BlobRead: Read + Seek + Send {}

impl<T: Read + Seek + Send> BlobRead for T {}

/// What is known about a blob before its bytes are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStat {
    pub size: u64,
    /// The digest the store holds the bytes under. It differs from the
    /// requested digest when the request was answered through an alias.
    pub stored_as: Digest,
}

impl OciStore {
    /// # Errors
    ///
    /// `NotInRepository` when `repo` holds no link to `digest`, even if the
    /// bytes exist; `Io` when the store cannot be read.
    pub fn blob_stat(&self, repo: &RepoName, digest: &Digest) -> Result<BlobStat, OciStoreError> {
        self.require_link(repo, digest)?;
        let stored_as = self.resolve_stored(repo, digest)?;
        let size = self
            .kappa()
            .blob_size(stored_as.as_str())
            .map_err(|error| OciStoreError::Io(format!("size of {stored_as}: {error}")))?;
        Ok(BlobStat { size, stored_as })
    }

    /// Open a blob for reading. The reader is positioned at the start.
    ///
    /// # Errors
    ///
    /// As [`OciStore::blob_stat`].
    pub fn blob_open(
        &self,
        repo: &RepoName,
        digest: &Digest,
    ) -> Result<(BlobStat, Box<dyn BlobRead>), OciStoreError> {
        let stat = self.blob_stat(repo, digest)?;
        let reader = self
            .kappa()
            .blob_open(stat.stored_as.as_str())
            .map_err(|error| OciStoreError::Io(format!("open {}: {error}", stat.stored_as)))?;
        Ok((stat, Box::new(StoreReader(reader))))
    }

    fn require_link(&self, repo: &RepoName, digest: &Digest) -> Result<(), OciStoreError> {
        if self.link_get(repo, digest)?.is_some() {
            return Ok(());
        }
        // A blob linked under its sha256 is also reachable by its blake3 alias.
        if let Some(alias) = self.alias_of(digest)? {
            if self.link_get(repo, &alias)?.is_some() {
                return Ok(());
            }
        }
        Err(OciStoreError::NotInRepository {
            repo: repo.as_str().to_owned(),
            digest: digest.as_str().to_owned(),
        })
    }

    /// The digest the store holds these bytes under: the requested one, or
    /// its alias when the store does not know the requested one.
    fn resolve_stored(&self, repo: &RepoName, digest: &Digest) -> Result<Digest, OciStoreError> {
        match self.kappa().blob_exists(digest.as_str()) {
            Ok(true) => return Ok(digest.clone()),
            Ok(false) | Err(StoreError::NotFound(_)) => {}
            Err(error) => return Err(OciStoreError::Io(format!("look up {digest}: {error}"))),
        }
        if let Some(alias) = self.alias_of(digest)? {
            if matches!(self.kappa().blob_exists(alias.as_str()), Ok(true)) {
                return Ok(alias);
            }
        }
        // Linked, but the bytes are gone: damage, not a missing link.
        Err(OciStoreError::Io(format!(
            "{digest} is linked in {repo} but the store does not hold it"
        )))
    }
}

/// Hides the store's reader type behind [`BlobRead`].
struct StoreReader(Box<dyn kappa_core::store::BlobReader>);

impl Read for StoreReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Seek for StoreReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(position)
    }
}

/// How much one trip to the blocking pool reads.
const CHUNK: usize = 1024 * 1024;

type ReadOutcome = (Box<dyn BlobRead>, std::io::Result<Vec<u8>>);

/// Reads a synchronous blob from async code, for response bodies.
///
/// Each poll hands the reader to the blocking pool for one buffer, then takes
/// it back, so the runtime is never blocked on disk and at most one chunk is
/// in memory. `Box<dyn BlobRead>` is `Unpin`, so this needs no `unsafe` and no
/// pin projection.
pub struct AsyncBlob {
    state: State,
    /// Where the first read must seek to. Taken by that read.
    seek_to: Option<u64>,
    remaining: u64,
}

enum State {
    Idle(Option<Box<dyn BlobRead>>),
    Reading(tokio::task::JoinHandle<ReadOutcome>),
    Buffered {
        reader: Box<dyn BlobRead>,
        chunk: Vec<u8>,
        at: usize,
    },
}

impl AsyncBlob {
    /// Serve `len` bytes of `reader` starting at `start`.
    #[must_use]
    pub fn new(reader: Box<dyn BlobRead>, start: u64, len: u64) -> Self {
        Self {
            state: State::Idle(Some(reader)),
            seek_to: Some(start),
            remaining: len,
        }
    }
}

impl tokio::io::AsyncRead for AsyncBlob {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        loop {
            match std::mem::replace(&mut this.state, State::Idle(None)) {
                State::Buffered { reader, chunk, at } if at < chunk.len() => {
                    let count = out.remaining().min(chunk.len() - at);
                    out.put_slice(&chunk[at..at + count]);
                    this.state = State::Buffered {
                        reader,
                        chunk,
                        at: at + count,
                    };
                    return Poll::Ready(Ok(()));
                }
                State::Buffered { reader, .. } => this.state = State::Idle(Some(reader)),
                State::Idle(None) => {
                    return Poll::Ready(Err(std::io::Error::other(
                        "blob reader was lost by an earlier failed read",
                    )));
                }
                State::Idle(Some(mut reader)) => {
                    if this.remaining == 0 {
                        this.state = State::Idle(Some(reader));
                        return Poll::Ready(Ok(()));
                    }
                    let want = usize::try_from(this.remaining.min(CHUNK as u64)).unwrap_or(CHUNK);
                    let seek_to = this.seek_to.take();
                    this.state = State::Reading(tokio::task::spawn_blocking(move || {
                        if let Some(position) = seek_to {
                            if let Err(error) = reader.seek(SeekFrom::Start(position)) {
                                return (reader, Err(error));
                            }
                        }
                        let mut chunk = vec![0_u8; want];
                        let result = reader.read(&mut chunk).map(|count| {
                            chunk.truncate(count);
                            chunk
                        });
                        (reader, result)
                    }));
                }
                State::Reading(mut handle) => match Pin::new(&mut handle).poll(cx) {
                    Poll::Pending => {
                        this.state = State::Reading(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(join)) => return Poll::Ready(Err(std::io::Error::other(join))),
                    Poll::Ready(Ok((reader, Err(error)))) => {
                        this.state = State::Idle(Some(reader));
                        return Poll::Ready(Err(error));
                    }
                    Poll::Ready(Ok((reader, Ok(chunk)))) => {
                        if chunk.is_empty() {
                            // The file ended early. Report end of stream; the
                            // HTTP layer sees a short body against its length.
                            this.remaining = 0;
                            this.state = State::Idle(Some(reader));
                            return Poll::Ready(Ok(()));
                        }
                        this.remaining = this.remaining.saturating_sub(chunk.len() as u64);
                        this.state = State::Buffered {
                            reader,
                            chunk,
                            at: 0,
                        };
                    }
                },
            }
        }
    }
}
