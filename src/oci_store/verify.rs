//! `hologram oci verify` (plan P8 T3, FR-014): every blob read again and
//! hashed with the algorithm of its own address.
//!
//! Verify reports and changes nothing: it never deletes, never quarantines,
//! and a server beside it keeps serving. A damaged blob is named with the
//! repositories that link it and the tags that reach it, so the repair is
//! plain: push those tags again, or restore those files.

use super::{Algorithm, Digest, OciStore, OciStoreError, Reference, RepoName};
use kappa_core::store::KappaStore as _;
use sha2::Digest as _;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

/// Read size while hashing.
const CHUNK: usize = 1 << 20;
/// How deep an index may nest before it is taken as a loop.
const MAX_DEPTH: usize = 8;
/// Page size when walking repositories, links and tags.
const PAGE: usize = 1000;

/// What a run found.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VerifyReport {
    pub checked: u64,
    pub bytes: u64,
    pub damaged: Vec<Damaged>,
}

/// One blob whose bytes are not the bytes its address names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Damaged {
    pub digest: String,
    /// Bytes read before the fault was found; the whole blob for a mismatch.
    pub size: u64,
    /// `hash mismatch: <what the bytes hash to>`, or why it could not be read.
    pub reason: String,
    /// Each repository that links the blob, with the tags that reach it there.
    pub repositories: Vec<Reach>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Reach {
    pub name: String,
    pub tags: Vec<String>,
}

impl OciStore {
    /// Hash every blob the store holds. `progress` sees (checked, total) after
    /// each blob. A blob that appears during the run is not counted; one that
    /// is removed during it is skipped, never reported damaged.
    ///
    /// # Errors
    ///
    /// The blob directory or the link tables cannot be read.
    pub fn verify(
        &self,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<VerifyReport, OciStoreError> {
        let blobs = list_blobs(&self.layout().blob_root())?;
        let total = blobs.len() as u64;
        let mut report = VerifyReport::default();
        for digest in blobs {
            match self.check_blob(&digest) {
                Check::Whole(size) => report.bytes += size,
                Check::Gone => {}
                Check::Damaged { size, reason } => {
                    report.bytes += size;
                    report.damaged.push(Damaged {
                        digest: digest.as_str().to_owned(),
                        size,
                        reason,
                        repositories: Vec::new(),
                    });
                }
            }
            report.checked += 1;
            progress(report.checked, total);
        }
        if !report.damaged.is_empty() {
            let reach = self.reach_of(&report.damaged)?;
            for damaged in &mut report.damaged {
                damaged.repositories = reach.get(&damaged.digest).cloned().unwrap_or_default();
            }
        }
        Ok(report)
    }

    fn check_blob(&self, digest: &Digest) -> Check {
        let mut reader = match self.kappa().blob_open(digest.as_str()) {
            Ok(reader) => reader,
            // Removed since it was listed: not damage.
            Err(kappa_core::types::StoreError::NotFound(_)) => return Check::Gone,
            Err(error) => {
                return Check::Damaged {
                    size: 0,
                    reason: format!("cannot open: {error}"),
                }
            }
        };
        let mut hasher = Hasher::new(digest.algorithm());
        let mut buffer = vec![0_u8; CHUNK];
        let mut size = 0_u64;
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buffer[..n]);
                    size += n as u64;
                }
                Err(error) => {
                    return Check::Damaged {
                        size,
                        reason: format!("cannot read: {error}"),
                    }
                }
            }
        }
        let found = hasher.finish();
        let expected = digest.as_str().split_once(':').map_or("", |(_, hex)| hex);
        if found == expected {
            Check::Whole(size)
        } else {
            let algorithm = digest.as_str().split_once(':').map_or("", |(name, _)| name);
            Check::Damaged {
                size,
                reason: format!("hash mismatch: the bytes are {algorithm}:{found}"),
            }
        }
    }

    /// For each damaged digest: the repositories that link it or its alias,
    /// and in each the tags whose manifest tree reaches it.
    fn reach_of(&self, damaged: &[Damaged]) -> Result<BTreeMap<String, Vec<Reach>>, OciStoreError> {
        // Either address of a blob stands for it.
        let mut wanted: BTreeMap<String, String> = BTreeMap::new();
        for entry in damaged {
            wanted.insert(entry.digest.clone(), entry.digest.clone());
            if let Ok(digest) = Digest::parse(&entry.digest) {
                if let Some(alias) = self.alias_of(&digest)? {
                    wanted.insert(alias.as_str().to_owned(), entry.digest.clone());
                }
            }
        }
        let mut out: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
        for repo in self.all_repos()? {
            for (digest, _) in self.all_links(&repo)? {
                if let Some(damaged) = wanted.get(digest.as_str()) {
                    out.entry(damaged.clone())
                        .or_default()
                        .entry(repo.as_str().to_owned())
                        .or_default();
                }
            }
            for tag in self.all_tags(&repo)? {
                let mut reached = BTreeSet::new();
                self.walk(&repo, &Reference::Tag(tag.clone()), 0, &mut reached);
                for digest in reached {
                    if let Some(damaged) = wanted.get(&digest) {
                        out.entry(damaged.clone())
                            .or_default()
                            .entry(repo.as_str().to_owned())
                            .or_default()
                            .insert(tag.as_str().to_owned());
                    }
                }
            }
        }
        Ok(out
            .into_iter()
            .map(|(digest, repos)| {
                let reach = repos
                    .into_iter()
                    .map(|(name, tags)| Reach {
                        name,
                        tags: tags.into_iter().collect(),
                    })
                    .collect();
                (digest, reach)
            })
            .collect())
    }

    /// Every digest a manifest reaches, itself included. A manifest that
    /// cannot be read or parsed reaches only itself: it may be the damage.
    fn walk(
        &self,
        repo: &RepoName,
        reference: &Reference,
        depth: usize,
        reached: &mut BTreeSet<String>,
    ) {
        if let Reference::Digest(digest) = reference {
            reached.insert(digest.as_str().to_owned());
        }
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(manifest) = self.manifest_get(repo, reference) else {
            return;
        };
        reached.insert(manifest.digest.as_str().to_owned());
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&manifest.bytes) else {
            return;
        };
        let digest_of = |node: &serde_json::Value| {
            node.get("digest")
                .and_then(|d| d.as_str())
                .map(str::to_owned)
        };
        if let Some(config) = value.get("config").and_then(digest_of) {
            reached.insert(config);
        }
        for layer in value
            .get("layers")
            .and_then(|l| l.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(digest) = digest_of(layer) {
                reached.insert(digest);
            }
        }
        for child in value
            .get("manifests")
            .and_then(|m| m.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(digest) = digest_of(child).and_then(|d| Digest::parse(&d).ok()) {
                // Each child once: an index that lists itself does not loop.
                if reached.insert(digest.as_str().to_owned()) {
                    self.walk(repo, &Reference::Digest(digest), depth + 1, reached);
                }
            }
        }
    }

    fn all_repos(&self) -> Result<Vec<RepoName>, OciStoreError> {
        let mut all = Vec::new();
        loop {
            let after = all.last().map(|repo: &RepoName| repo.as_str().to_owned());
            let page = self.repos_page(after.as_deref(), PAGE)?;
            let done = page.len() < PAGE;
            all.extend(page);
            if done {
                return Ok(all);
            }
        }
    }

    fn all_links(&self, repo: &RepoName) -> Result<Vec<(Digest, super::Link)>, OciStoreError> {
        let mut all: Vec<(Digest, super::Link)> = Vec::new();
        loop {
            let after = all.last().map(|(digest, _)| digest.clone());
            let page = self.links_page(repo, after.as_ref(), PAGE)?;
            let done = page.len() < PAGE;
            all.extend(page);
            if done {
                return Ok(all);
            }
        }
    }

    fn all_tags(&self, repo: &RepoName) -> Result<Vec<super::Tag>, OciStoreError> {
        let mut all: Vec<super::Tag> = Vec::new();
        loop {
            let after = all.last().map(|tag| tag.as_str().to_owned());
            let page = self.tags_page(repo, after.as_deref(), PAGE)?;
            let done = page.len() < PAGE;
            all.extend(page);
            if done {
                return Ok(all);
            }
        }
    }
}

enum Check {
    Whole(u64),
    Gone,
    Damaged { size: u64, reason: String },
}

enum Hasher {
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Blake3(Box<blake3::Hasher>),
}

impl Hasher {
    fn new(algorithm: Algorithm) -> Self {
        match algorithm {
            Algorithm::Sha256 => Self::Sha256(sha2::Sha256::new()),
            Algorithm::Sha512 => Self::Sha512(sha2::Sha512::new()),
            Algorithm::Blake3 => Self::Blake3(Box::new(blake3::Hasher::new())),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(bytes),
            Self::Sha512(hasher) => hasher.update(bytes),
            Self::Blake3(hasher) => {
                hasher.update(bytes);
            }
        }
    }

    fn finish(self) -> String {
        match self {
            Self::Sha256(hasher) => crate::util::hex(&hasher.finalize()),
            Self::Sha512(hasher) => crate::util::hex(&hasher.finalize()),
            Self::Blake3(hasher) => hasher.finalize().to_hex().to_string(),
        }
    }
}

/// Every blob address under the blob root: `<algorithm>/<hh>/<hh>/<hex>`
/// (kappa-core's `blob_path_for`). Anything else there is not a blob: an
/// upload's staging file lives outside it.
fn list_blobs(root: &Path) -> Result<Vec<Digest>, OciStoreError> {
    let io = |path: &Path, error: std::io::Error| {
        OciStoreError::Io(format!("read {}: {error}", path.display()))
    };
    let entries = |path: &Path| -> Result<Vec<std::fs::DirEntry>, OciStoreError> {
        match std::fs::read_dir(path) {
            Ok(entries) => entries
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| io(path, error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(io(path, error)),
        }
    };
    let mut out = Vec::new();
    for algorithm in entries(root)? {
        let name = algorithm.file_name().to_string_lossy().into_owned();
        if !matches!(name.as_str(), "sha256" | "sha512" | "blake3") {
            continue;
        }
        for first in entries(&algorithm.path())? {
            for second in entries(&first.path())? {
                for file in entries(&second.path())? {
                    if !file
                        .file_type()
                        .map_err(|error| io(&file.path(), error))?
                        .is_file()
                    {
                        continue;
                    }
                    let hex = file.file_name().to_string_lossy().into_owned();
                    if let Ok(digest) = Digest::parse(&format!("{name}:{hex}")) {
                        out.push(digest);
                    }
                }
            }
        }
    }
    out.sort();
    Ok(out)
}
