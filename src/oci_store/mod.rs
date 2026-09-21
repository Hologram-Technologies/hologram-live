//! Everything the registry needs from storage, behind one type.
//!
//! `OciStore` owns the Kappa store (blob bytes and tags) and a database of its
//! own, `links.redb` (repository links, referrers, hash aliases, upload
//! records). No Kappa type leaves this directory: the registry module sees
//! `OciStore`, the validated types in [`types`], and [`OciStoreError`]. A later
//! store swap touches this directory only (ADR 025, ADR 027).
//!
//! Every method is synchronous. Callers on the async side wrap them in
//! `spawn_blocking`.

pub mod layout;
pub mod links;
pub mod types;

pub use layout::Layout;
pub use links::{Link, LinkKind, ReferrerDescriptor};
pub use types::{Algorithm, Digest, Reference, RepoName, Tag, UploadId};

use kappa_core::clock::Clock;
use kappa_store_redb::{PersistentStore, PersistentStoreConfig};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Why a store operation did not happen. The registry module maps each variant
/// to one registry error code; that mapping lives with the HTTP layer, so this
/// layer knows nothing about HTTP.
#[derive(Debug)]
pub enum OciStoreError {
    /// The digest, name, tag or upload id is not in the grammar.
    Invalid { what: &'static str, value: String },
    /// Not linked in this repository (it may exist in another).
    NotInRepository { repo: String, digest: String },
    /// The repository has never been written to.
    UnknownRepository(String),
    /// No such upload session.
    UnknownUpload(String),
    /// The bytes do not hash to the digest the client gave.
    DigestMismatch { claimed: String },
    /// A chunk does not start where the last one ended.
    OffsetMismatch { expected: u64, got: u64 },
    /// A manifest names content this repository does not hold.
    MissingReferences(Vec<String>),
    /// The volume is in another layout, or a newer one.
    Layout(String),
    /// Another process holds the store.
    Locked,
    /// Anything the disk or the store refused.
    Io(String),
}

impl std::fmt::Display for OciStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { what, value } => write!(f, "invalid {what}: {value:?}"),
            Self::NotInRepository { repo, digest } => {
                write!(f, "{digest} is not linked in repository {repo}")
            }
            Self::UnknownRepository(repo) => write!(f, "unknown repository {repo}"),
            Self::UnknownUpload(id) => write!(f, "unknown upload {id}"),
            Self::DigestMismatch { claimed } => {
                write!(f, "the bytes do not hash to {claimed}")
            }
            Self::OffsetMismatch { expected, got } => {
                write!(f, "chunk starts at {got}, expected {expected}")
            }
            Self::MissingReferences(digests) => {
                write!(f, "missing references: {}", digests.join(", "))
            }
            Self::Layout(message) => f.write_str(message),
            Self::Locked => f.write_str("another process holds the registry store"),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for OciStoreError {}

/// How to open a volume.
#[derive(Debug, Clone, Copy)]
pub struct OpenOptions {
    /// Create the layout when the directory is new. The server passes `true`;
    /// offline commands that must not invent a volume pass `false`.
    pub create: bool,
    /// Upload sessions untouched for longer than this are aborted.
    pub upload_max_age: Duration,
}

/// The registry's storage.
pub struct OciStore {
    kappa: Arc<PersistentStore>,
    links: redb::Database,
    layout: Layout,
    upload_max_age: Duration,
}

impl std::fmt::Debug for OciStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OciStore")
            .field("root", &self.layout.root())
            .finish_non_exhaustive()
    }
}

impl OciStore {
    /// Open, or with `create` make, a registry volume at `root`.
    ///
    /// # Errors
    ///
    /// `Layout` when the volume is in the Docker Registry layout, in a newer
    /// layout, or split across file systems; `Locked` when another process
    /// holds it; `Io` for anything the disk refused.
    pub fn open(root: &Path, options: OpenOptions) -> Result<Self, OciStoreError> {
        layout::check(root, options.create)?;
        let layout = Layout::resolve(root);
        layout.create_directories()?;
        layout.require_one_filesystem()?;
        // Ours first: its lock error is typed, and a second opener always
        // meets it before it meets the Kappa database.
        let links = redb::Database::create(layout.links_db()).map_err(|error| match error {
            redb::DatabaseError::DatabaseAlreadyOpen => OciStoreError::Locked,
            other => OciStoreError::Io(format!("open {}: {other}", layout.links_db().display())),
        })?;
        let mut config = PersistentStoreConfig::new(layout.blob_root(), layout.kappa_db());
        // Expiry of upload sessions is ours: it goes by last activity.
        config.upload_timeout_secs = None;
        let kappa = PersistentStore::new(config, Arc::new(WallClock)).map_err(kappa_open_error)?;
        links::create_tables(&links)?;
        layout.write_marker_if_missing()?;
        Ok(Self {
            kappa: Arc::new(kappa),
            links,
            layout,
            upload_max_age: options.upload_max_age,
        })
    }

    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    #[must_use]
    pub fn upload_max_age(&self) -> Duration {
        self.upload_max_age
    }

    #[expect(dead_code, reason = "first used by blob reads in P1 T4")]
    pub(crate) fn kappa(&self) -> &Arc<PersistentStore> {
        &self.kappa
    }
}

/// The Kappa store reports a redb failure as text, so the lock case is
/// recognised by its wording. It is a fallback: `links.redb` is opened first
/// and its lock error is typed.
fn kappa_open_error(error: kappa_core::types::StoreError) -> OciStoreError {
    let text = error.to_string();
    if text.contains("already open") || text.contains("locked") {
        OciStoreError::Locked
    } else {
        OciStoreError::Io(format!("open the kappa store: {text}"))
    }
}

/// The store stamps records with wall-clock milliseconds.
struct WallClock;

impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}
