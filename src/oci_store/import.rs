//! `hologram oci import`: a Docker Registry volume copied into this layout.
//!
//! The reference's filesystem layout, as `registry/storage/paths.go` at
//! v3.1.1 defines it, under `<source>/docker/registry/v2/`:
//!
//! ```text
//! blobs/<algorithm>/<first two hex>/<hex>/data
//! repositories/<name>/_layers/<algorithm>/<hex>/link
//! repositories/<name>/_manifests/revisions/<algorithm>/<hex>/link
//! repositories/<name>/_manifests/tags/<tag>/current/link
//! ```
//!
//! Per repository, in order: every layer link, streamed through an upload so
//! every byte is hashed on the way in (FR-020); then every manifest revision,
//! through the same checks as a push, children before the indexes that name
//! them; then every tag. What this repository already links is skipped, so a
//! second run adds nothing and an interrupted run finishes when run again.
//! A blob the volume already holds for another repository is linked, not
//! copied again: those links go in one transaction per repository.
//!
//! The source is only ever opened for reading.

use super::{Digest, ManifestPlan, OciStore, OciStoreError, Reference, RepoName, Tag, FRAME};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Reads a manifest body the way a push is read: its media type and what the
/// store must check. The registry module supplies its validator.
pub type Planner<'a> = &'a dyn Fn(&str, &[u8]) -> Result<(String, ManifestPlan), String>;

/// What went wrong with one object of the source. The rest is imported.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ImportProblem {
    pub kind: ProblemKind,
    pub repository: String,
    /// The file in the source it concerns.
    pub path: PathBuf,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProblemKind {
    /// A link names a blob the source no longer holds. The reference cannot
    /// serve it either (its garbage collection leaves layer links behind), so
    /// this alone does not fail the run.
    Missing,
    /// The bytes do not hash to the digest they are stored under.
    Mismatch,
    /// A file could not be read, or a link or name is not in the grammar.
    Unreadable,
    /// A manifest the reference holds that a push here would refuse.
    Refused,
    /// A tag whose manifest was not imported.
    TagDangling,
}

impl ProblemKind {
    /// Whether this problem means something the reference serves did not
    /// come over.
    #[must_use]
    pub fn fails(self) -> bool {
        self != Self::Missing
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Mismatch => "mismatch",
            Self::Unreadable => "unreadable",
            Self::Refused => "refused",
            Self::TagDangling => "tag-dangling",
        }
    }
}

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportReport {
    pub repositories: usize,
    /// Blobs streamed in through an upload.
    pub blobs_copied: usize,
    /// The bytes of those blobs.
    pub bytes_copied: u64,
    /// Blobs the volume already held for another repository, linked here.
    pub blobs_linked: usize,
    pub manifests: usize,
    pub tags: usize,
    pub problems: Vec<ImportProblem>,
}

impl ImportReport {
    /// Whether anything the reference serves did not come over.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.problems.iter().any(|problem| problem.kind.fails())
    }

    /// Objects and tags this run added.
    #[must_use]
    pub fn added(&self) -> usize {
        self.blobs_copied + self.blobs_linked + self.manifests + self.tags
    }
}

/// Progress, as the run goes.
#[derive(Debug)]
pub enum ImportEvent<'a> {
    /// Starting repository `index` (from 0) of `total`.
    Repository {
        name: &'a str,
        index: usize,
        total: usize,
    },
    Problem(&'a ImportProblem),
}

/// The reference's storage root inside its volume.
fn v2(source: &Path) -> PathBuf {
    source.join("docker").join("registry").join("v2")
}

/// Every repository under `repositories/`, sorted. Names contain slashes, so
/// the directory is a tree: a directory is a repository if it has
/// `_manifests` or `_layers`, and it may also hold deeper repositories.
fn repositories(root: &Path) -> Result<Vec<String>, OciStoreError> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, name)) = pending.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| OciStoreError::Io(format!("read {}: {error}", dir.display())))?;
        let mut is_repository = false;
        for entry in entries {
            let entry = entry
                .map_err(|error| OciStoreError::Io(format!("read {}: {error}", dir.display())))?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if file_name == "_manifests" || file_name == "_layers" {
                is_repository = true;
            } else if !file_name.starts_with('_') && entry.path().is_dir() {
                let child = if name.is_empty() {
                    file_name
                } else {
                    format!("{name}/{file_name}")
                };
                pending.push((entry.path(), child));
            }
        }
        if is_repository && !name.is_empty() {
            found.push(name);
        }
    }
    found.sort();
    Ok(found)
}

/// Every `<algorithm>/<hex>/link` under `dir`, as (digest, link file). A
/// directory that does not exist has none.
/// A link file's digest, or why it could not be read, and the file.
type Found = (Result<Digest, String>, PathBuf);

fn links_under(dir: &Path) -> Result<Vec<Found>, OciStoreError> {
    let mut found = Vec::new();
    let Ok(algorithms) = std::fs::read_dir(dir) else {
        return Ok(found);
    };
    for algorithm in algorithms {
        let algorithm = algorithm
            .map_err(|error| OciStoreError::Io(format!("read {}: {error}", dir.display())))?;
        let Ok(hexes) = std::fs::read_dir(algorithm.path()) else {
            continue;
        };
        for hex in hexes {
            let hex =
                hex.map_err(|error| OciStoreError::Io(format!("read {}: {error}", dir.display())))?;
            let link = hex.path().join("link");
            let named = format!(
                "{}:{}",
                algorithm.file_name().to_string_lossy(),
                hex.file_name().to_string_lossy()
            );
            found.push((read_link(&link, &named), link));
        }
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(found)
}

/// A link file holds its digest, and must hold the one its path names.
fn read_link(link: &Path, named: &str) -> Result<Digest, String> {
    let text = std::fs::read_to_string(link).map_err(|error| format!("read the link: {error}"))?;
    let text = text.trim();
    if text != named {
        return Err(format!("the link holds {text:?}, its path names {named}"));
    }
    Digest::parse(text).map_err(|error| error.to_string())
}

/// Where the source keeps a blob's bytes.
fn blob_data(v2: &Path, digest: &Digest) -> PathBuf {
    let (algorithm, hex) = digest.as_str().split_once(':').unwrap_or_default();
    v2.join("blobs")
        .join(algorithm)
        .join(hex.get(..2).unwrap_or_default())
        .join(hex)
        .join("data")
}

/// One repository's run, with the report it adds to.
struct Run<'a> {
    store: &'a OciStore,
    v2: &'a Path,
    repo: RepoName,
    report: &'a mut ImportReport,
    progress: &'a mut dyn FnMut(ImportEvent<'_>),
}

impl Run<'_> {
    fn problem(&mut self, kind: ProblemKind, path: &Path, detail: String) {
        let problem = ImportProblem {
            kind,
            repository: self.repo.as_str().to_owned(),
            path: path.to_path_buf(),
            detail,
        };
        (self.progress)(ImportEvent::Problem(&problem));
        self.report.problems.push(problem);
    }

    fn layers(&mut self) -> Result<(), OciStoreError> {
        let dir = self
            .v2
            .join("repositories")
            .join(self.repo.as_str())
            .join("_layers");
        let mut shared = Vec::new();
        for (digest, link) in links_under(&dir)? {
            let digest = match digest {
                Ok(digest) => digest,
                Err(detail) => {
                    self.problem(ProblemKind::Unreadable, &link, detail);
                    continue;
                }
            };
            if self.store.require_link(&self.repo, &digest).is_ok() {
                continue;
            }
            let data = blob_data(self.v2, &digest);
            if !data.is_file() {
                self.problem(
                    ProblemKind::Missing,
                    &data,
                    format!("{digest} is linked but not held"),
                );
                continue;
            }
            // Held for another repository, by bytes the registry stored and
            // hashed: a link is enough (the alias row is written only then).
            if self.store.alias_of(&digest)?.is_some()
                && self.store.resolve_stored(&self.repo, &digest).is_ok()
            {
                shared.push(digest);
                continue;
            }
            self.copy(&digest, &data)?;
        }
        if !shared.is_empty() {
            let rows = shared.iter().map(|digest| {
                (
                    &self.repo,
                    digest,
                    super::Link {
                        kind: super::LinkKind::Blob,
                        media_type: None,
                    },
                )
            });
            self.store.links_add_bulk(rows)?;
            self.report.blobs_linked += shared.len();
        }
        Ok(())
    }

    /// Stream one blob in, hashed against its address.
    fn copy(&mut self, digest: &Digest, data: &Path) -> Result<(), OciStoreError> {
        let mut file = match std::fs::File::open(data) {
            Ok(file) => file,
            Err(error) => {
                self.problem(ProblemKind::Unreadable, data, format!("open: {error}"));
                return Ok(());
            }
        };
        let id = self.store.upload_begin(&self.repo)?;
        let mut frame = vec![0_u8; FRAME];
        let mut offset = 0_u64;
        loop {
            let filled = match fill(&mut file, &mut frame) {
                Ok(filled) => filled,
                Err(error) => {
                    self.store.upload_cancel(&id)?;
                    self.problem(ProblemKind::Unreadable, data, format!("read: {error}"));
                    return Ok(());
                }
            };
            if filled == 0 {
                break;
            }
            offset = self.store.upload_append(&id, offset, &frame[..filled])?;
            if filled < frame.len() {
                break;
            }
        }
        match self.store.upload_finish(&id, digest) {
            Ok(_) => {
                self.report.blobs_copied += 1;
                self.report.bytes_copied += offset;
                Ok(())
            }
            Err(OciStoreError::DigestMismatch { .. }) => {
                self.problem(
                    ProblemKind::Mismatch,
                    data,
                    format!("the bytes do not hash to {digest}"),
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Every revision, as many passes as it takes: a manifest is pushed once
    /// everything it names is here, so children go before their indexes.
    fn manifests(&mut self, plan: Planner<'_>) -> Result<(), OciStoreError> {
        let dir = self
            .v2
            .join("repositories")
            .join(self.repo.as_str())
            .join("_manifests")
            .join("revisions");
        let mut waiting = Vec::new();
        for (digest, link) in links_under(&dir)? {
            let digest = match digest {
                Ok(digest) => digest,
                Err(detail) => {
                    self.problem(ProblemKind::Unreadable, &link, detail);
                    continue;
                }
            };
            if self.store.require_link(&self.repo, &digest).is_ok() {
                continue;
            }
            let data = blob_data(self.v2, &digest);
            let bytes = match read_manifest(&data) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    self.problem(
                        ProblemKind::Missing,
                        &data,
                        format!("manifest {digest} is linked but not held"),
                    );
                    continue;
                }
                Err(detail) => {
                    self.problem(ProblemKind::Unreadable, &data, detail);
                    continue;
                }
            };
            let media_type = stored_media_type(&bytes);
            match plan(&media_type, &bytes) {
                Ok((media_type, manifest_plan)) => {
                    waiting.push((digest, data, media_type, bytes, manifest_plan));
                }
                Err(detail) => self.problem(ProblemKind::Refused, &data, detail),
            }
        }
        loop {
            let before = waiting.len();
            let mut still = Vec::new();
            for (digest, data, media_type, bytes, manifest_plan) in waiting {
                let reference = Reference::Digest(digest.clone());
                match self.store.manifest_put(
                    &self.repo,
                    &reference,
                    &media_type,
                    &bytes,
                    &manifest_plan,
                ) {
                    Ok(_) => self.report.manifests += 1,
                    Err(OciStoreError::MissingReferences(_)) => {
                        still.push((digest, data, media_type, bytes, manifest_plan));
                    }
                    Err(OciStoreError::DigestMismatch { .. }) => {
                        self.problem(
                            ProblemKind::Mismatch,
                            &data,
                            format!("the bytes do not hash to {digest}"),
                        );
                    }
                    Err(error) => return Err(error),
                }
            }
            waiting = still;
            if waiting.is_empty() || waiting.len() == before {
                break;
            }
        }
        for (digest, data, _, _, manifest_plan) in waiting {
            let missing: Vec<&str> = manifest_plan
                .must_exist
                .iter()
                .filter(|needed| self.store.require_link(&self.repo, needed).is_err())
                .map(Digest::as_str)
                .collect();
            let detail = format!(
                "manifest {digest} names what this repository does not hold: {}",
                missing.join(", ")
            );
            self.problem(ProblemKind::Refused, &data, detail);
        }
        Ok(())
    }

    fn tags(&mut self) -> Result<(), OciStoreError> {
        let dir = self
            .v2
            .join("repositories")
            .join(self.repo.as_str())
            .join("_manifests")
            .join("tags");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(());
        };
        let mut names: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect();
        names.sort();
        for (name, path) in names {
            let link = path.join("current").join("link");
            let tag = match Tag::parse(&name) {
                Ok(tag) => tag,
                Err(error) => {
                    self.problem(ProblemKind::Unreadable, &path, error.to_string());
                    continue;
                }
            };
            let digest = match std::fs::read_to_string(&link)
                .map_err(|error| format!("read the link: {error}"))
                .and_then(|text| Digest::parse(text.trim()).map_err(|error| error.to_string()))
            {
                Ok(digest) => digest,
                Err(detail) => {
                    self.problem(ProblemKind::Unreadable, &link, detail);
                    continue;
                }
            };
            if self.store.tag_resolve(&self.repo, &tag).ok().as_ref() == Some(&digest) {
                continue;
            }
            match self.store.tag_set(&self.repo, &tag, &digest) {
                Ok(()) => self.report.tags += 1,
                Err(OciStoreError::NotInRepository { .. }) => {
                    let detail = format!("tag {name} points at {digest}, which was not imported");
                    self.problem(ProblemKind::TagDangling, &link, detail);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// Fill `buffer` from `file` as far as the file goes.
fn fill(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

/// A manifest's bytes, or `None` when the source does not hold them.
fn read_manifest(data: &Path) -> Result<Option<Vec<u8>>, String> {
    let size = match std::fs::metadata(data) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("stat: {error}")),
    };
    if size > super::manifests::MANIFEST_MAX {
        return Err(format!(
            "{size} bytes, over the {} byte manifest limit",
            super::manifests::MANIFEST_MAX
        ));
    }
    std::fs::read(data)
        .map(Some)
        .map_err(|error| format!("read: {error}"))
}

/// The media type the reference serves a stored manifest with: the body's
/// own `mediaType`; an OCI body without one is an index if it lists
/// `manifests`, else an image manifest (`registry/storage/manifeststore.go`).
fn stored_media_type(bytes: &[u8]) -> String {
    let body: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    if let Some(declared) = body.get("mediaType").and_then(serde_json::Value::as_str) {
        return declared.to_owned();
    }
    if body
        .get("manifests")
        .is_some_and(serde_json::Value::is_array)
    {
        "application/vnd.oci.image.index.v1+json".to_owned()
    } else {
        "application/vnd.oci.image.manifest.v1+json".to_owned()
    }
}

impl OciStore {
    /// Import the reference's volume at `source` into this store.
    ///
    /// # Errors
    ///
    /// `Layout` when `source` is not a Docker Registry volume; `Io` when this
    /// store refuses. A problem with one object of the source is not an
    /// error: it is in the report, and the rest is imported.
    pub fn import(
        &self,
        source: &Path,
        plan: Planner<'_>,
        progress: &mut dyn FnMut(ImportEvent<'_>),
    ) -> Result<ImportReport, OciStoreError> {
        let v2 = v2(source);
        let root = v2.join("repositories");
        if !v2.join("blobs").is_dir() && !root.is_dir() {
            return Err(OciStoreError::Layout(format!(
                "{} is not a Docker Registry volume: it has no docker/registry/v2",
                source.display()
            )));
        }
        let names = if root.is_dir() {
            repositories(&root)?
        } else {
            Vec::new()
        };
        let mut report = ImportReport::default();
        for (index, name) in names.iter().enumerate() {
            progress(ImportEvent::Repository {
                name,
                index,
                total: names.len(),
            });
            let repo = match RepoName::parse(name) {
                Ok(repo) => repo,
                Err(error) => {
                    let problem = ImportProblem {
                        kind: ProblemKind::Unreadable,
                        repository: name.clone(),
                        path: root.join(name),
                        detail: error.to_string(),
                    };
                    progress(ImportEvent::Problem(&problem));
                    report.problems.push(problem);
                    continue;
                }
            };
            report.repositories += 1;
            let mut run = Run {
                store: self,
                v2: &v2,
                repo,
                report: &mut report,
                progress: &mut *progress,
            };
            run.layers()?;
            run.manifests(plan)?;
            run.tags()?;
        }
        Ok(report)
    }
}
