//! Where the registry keeps its data, and which volumes it refuses to open.
//!
//! Layout version 1 (`data-model.md`, ADR 029):
//!
//! ```text
//! <root>/HOLOGRAM_REGISTRY_LAYOUT   one line: "1"
//! <root>/kappa/blobs/…              the Kappa store's blobs
//! <root>/kappa/staging/…            the Kappa store's staging (blob_root.parent()/staging)
//! <root>/kappa/kappa.redb           the Kappa store's database
//! <root>/oci/links.redb             ours: links, repositories, referrers, aliases, uploads
//! ```
//!
//! The marker is written last, so a crash while creating a new volume leaves a
//! directory the next start treats as new again. Every creation step here is
//! idempotent for that reason.

use super::OciStoreError;
use std::path::{Path, PathBuf};

/// The only layout version this binary reads and writes.
const LAYOUT_VERSION: &str = "1";
const MARKER: &str = "HOLOGRAM_REGISTRY_LAYOUT";

/// Paths of one registry volume.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    #[must_use]
    pub fn resolve(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn marker(&self) -> PathBuf {
        self.root.join(MARKER)
    }

    #[must_use]
    pub fn blob_root(&self) -> PathBuf {
        self.root.join("kappa").join("blobs")
    }

    /// Where the Kappa store stages uploads: beside its blob root.
    #[must_use]
    pub fn staging(&self) -> PathBuf {
        self.root.join("kappa").join("staging")
    }

    #[must_use]
    pub fn kappa_db(&self) -> PathBuf {
        self.root.join("kappa").join("kappa.redb")
    }

    #[must_use]
    pub fn links_db(&self) -> PathBuf {
        self.root.join("oci").join("links.redb")
    }

    pub(crate) fn create_directories(&self) -> Result<(), OciStoreError> {
        for directory in [self.blob_root(), self.root.join("oci")] {
            std::fs::create_dir_all(&directory).map_err(|error| {
                OciStoreError::Io(format!("create {}: {error}", directory.display()))
            })?;
        }
        Ok(())
    }

    /// Staging and blobs must share a file system, or the rename that
    /// publishes a finished upload stops being atomic. A rename between the
    /// two directories decides, which works the same on every system.
    pub(crate) fn require_one_filesystem(&self) -> Result<(), OciStoreError> {
        let kappa = self.root.join("kappa");
        let from = kappa.join(".layout-probe");
        let to = self.blob_root().join(".layout-probe");
        let io = |what: &str, error: std::io::Error| {
            OciStoreError::Io(format!("{what} {}: {error}", from.display()))
        };
        std::fs::write(&from, b"").map_err(|error| io("write", error))?;
        let renamed = std::fs::rename(&from, &to);
        let _ = std::fs::remove_file(&from);
        let _ = std::fs::remove_file(&to);
        renamed.map_err(|error| {
            OciStoreError::Layout(format!(
                "{} and {} are on different file systems ({error}); the registry needs them on one",
                kappa.display(),
                self.blob_root().display()
            ))
        })
    }

    pub(crate) fn write_marker_if_missing(&self) -> Result<(), OciStoreError> {
        let marker = self.marker();
        if marker.exists() {
            return Ok(());
        }
        std::fs::write(&marker, format!("{LAYOUT_VERSION}\n"))
            .map_err(|error| OciStoreError::Io(format!("write {}: {error}", marker.display())))
    }
}

/// Decide whether `root` may be opened, before anything is created in it.
pub(crate) fn check(root: &Path, create: bool) -> Result<(), OciStoreError> {
    let layout = Layout::resolve(root);
    match std::fs::read_to_string(layout.marker()) {
        Ok(text) => match text.trim() {
            LAYOUT_VERSION => Ok(()),
            other => Err(OciStoreError::Layout(format!(
                "this volume has layout version {other}; this binary reads version {LAYOUT_VERSION}"
            ))),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if root.join("docker").join("registry").join("v2").is_dir() {
                return Err(OciStoreError::Layout(format!(
                    "this volume is in the Docker Registry layout. Hologram Registry cannot open \
                     it in place. Run: hologram oci import {} --into <new directory>",
                    root.display()
                )));
            }
            if !create {
                return Err(OciStoreError::Layout(format!(
                    "{} is not a registry volume",
                    root.display()
                )));
            }
            // `open` creates the directories and both databases, then writes
            // the marker last.
            Ok(())
        }
        Err(error) => Err(OciStoreError::Io(format!(
            "read {}: {error}",
            layout.marker().display()
        ))),
    }
}
