//! `hologram oci garbage-collect` (plan P8 T1, FR-010, SC-008), as the
//! reference's `registry garbage-collect` marks and sweeps
//! (`registry/storage/garbagecollect.go`), with two rules of this registry:
//!
//! - it sweeps only objects the registry itself put in the store (`OBJECTS`):
//!   the Kappa store keeps records of its own beside the blobs (errata E12);
//! - a referrer (a signature, an SBOM) of a marked manifest is marked too, so
//!   `--delete-untagged` never strips an image of its signatures.
//!
//! Mark, per repository: every manifest (every tagged one with
//! `--delete-untagged`), then everything it references (config, layers, an
//! index's children, recursively, depth 8), then the referrers of what is
//! marked; both names of an aliased pair. A manifest that cannot be read or
//! parsed stops the run before anything is swept. Sweep: untagged manifests
//! (with `--delete-untagged`), then every recorded object that is not
//! marked, then each repository's links to what is not marked. Offline: the
//! volume must not be open in a server.

use super::{Digest, LinkKind, OciStore, OciStoreError, Reference, RepoName, Tag};
use kappa_core::store::KappaStore as _;
use std::collections::{BTreeMap, BTreeSet};

/// How deep an index may nest before it is taken as a loop.
const MAX_DEPTH: usize = 8;
const PAGE: usize = 1000;

/// The reference's flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcOptions {
    /// Report what would be removed, and remove nothing.
    pub dry_run: bool,
    /// Also remove manifests no tag points at.
    pub delete_untagged: bool,
}

/// What a run marked and removed (or, dry, would remove).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct GcReport {
    pub marked: usize,
    pub swept_blobs: usize,
    pub swept_manifests: usize,
    pub removed_links: usize,
}

impl OciStore {
    /// Mark and sweep. `emit` gets each line the reference prints, in its
    /// words, so scripts that read the reference's output read ours.
    ///
    /// # Errors
    ///
    /// A manifest that cannot be read or parsed, an index nested deeper than
    /// 8, or a store that refuses; nothing is swept after an error in mark.
    pub fn collect(
        &self,
        options: GcOptions,
        emit: &mut dyn FnMut(String),
    ) -> Result<GcReport, OciStoreError> {
        self.objects_backfill()?;
        let mut marked: BTreeSet<String> = BTreeSet::new();
        let mut untagged: Vec<(RepoName, Digest, Vec<String>)> = Vec::new();
        let mut stale_links: BTreeMap<String, Vec<Digest>> = BTreeMap::new();
        let repos = self.gc_repos()?;

        // Mark.
        for repo in &repos {
            emit(repo.as_str().to_owned());
            let links = self.gc_links(repo)?;
            let tags = self.gc_tags(repo)?;
            let mut tagged: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for tag in &tags {
                let digest = self.tag_resolve(repo, tag)?;
                tagged
                    .entry(digest.as_str().to_owned())
                    .or_default()
                    .push(tag.as_str().to_owned());
            }
            let manifests: Vec<&Digest> = links
                .iter()
                .filter(|(_, kind)| *kind != LinkKind::Blob)
                .map(|(digest, _)| digest)
                .collect();
            let mut roots = Vec::new();
            for digest in manifests {
                if options.delete_untagged && !tagged.contains_key(digest.as_str()) {
                    let all: Vec<String> = tags.iter().map(|tag| tag.as_str().to_owned()).collect();
                    untagged.push((repo.clone(), digest.clone(), all));
                    continue;
                }
                roots.push(digest.clone());
            }
            let mut queue: Vec<(Digest, usize)> =
                roots.into_iter().map(|digest| (digest, 0)).collect();
            while let Some((digest, depth)) = queue.pop() {
                if depth == 0 {
                    emit(format!(
                        "{}: marking manifest {} ",
                        repo.as_str(),
                        digest.as_str()
                    ));
                }
                marked.insert(digest.as_str().to_owned());
                if depth > MAX_DEPTH {
                    return Err(OciStoreError::Io(format!(
                        "{}: {} is nested deeper than {MAX_DEPTH} indexes; nothing was swept",
                        repo.as_str(),
                        digest.as_str()
                    )));
                }
                for reference in self.gc_references(repo, &digest)? {
                    let newly = marked.insert(reference.as_str().to_owned());
                    if newly {
                        emit(format!(
                            "{}: marking blob {}",
                            repo.as_str(),
                            reference.as_str()
                        ));
                    }
                    // A manifest in this repository is walked too.
                    let is_manifest = links
                        .iter()
                        .any(|(linked, kind)| linked == &reference && *kind != LinkKind::Blob);
                    if newly && is_manifest {
                        queue.push((reference, depth + 1));
                    }
                }
                // Signatures and SBOMs of what is kept are kept.
                for referrer in self.referrers_of(repo, &digest)? {
                    let referrer = Digest::parse(&referrer.digest)?;
                    if !marked.contains(referrer.as_str()) {
                        queue.push((referrer, 0));
                    }
                }
            }
            let stale: Vec<Digest> = links
                .iter()
                .filter(|(digest, kind)| {
                    *kind == LinkKind::Blob && !marked.contains(digest.as_str())
                })
                .map(|(digest, _)| digest.clone())
                .collect();
            if !stale.is_empty() {
                stale_links.insert(repo.as_str().to_owned(), stale);
            }
        }
        // Both names of a pair stand for the same bytes.
        for digest in marked.clone() {
            if let Some(alias) = self.alias_of(&Digest::parse(&digest)?)? {
                marked.insert(alias.as_str().to_owned());
            }
        }
        // An untagged manifest that another kept manifest references stays.
        untagged.retain(|(repo, digest, tags)| {
            let delete = !marked.contains(digest.as_str());
            if delete {
                emit(format!(
                    "manifest eligible for deletion: {{{} {} [{}]}}",
                    repo.as_str(),
                    digest.as_str(),
                    tags.join(" ")
                ));
            }
            delete
        });

        // Sweep.
        let mut report = GcReport {
            marked: marked.len(),
            ..GcReport::default()
        };
        if !options.dry_run {
            for (repo, digest, _) in &untagged {
                self.manifest_delete(repo, digest)?;
            }
        }
        report.swept_manifests = untagged.len();
        let objects = self.objects()?;
        let doomed: Vec<Digest> = objects
            .into_iter()
            .map(|(digest, _)| digest)
            .filter(|digest| !marked.contains(digest.as_str()))
            .collect();
        emit(format!(
            "\n{} blobs marked, {} blobs and {} manifests eligible for deletion",
            marked.len(),
            doomed.len(),
            untagged.len()
        ));
        for digest in &doomed {
            emit(format!("blob eligible for deletion: {}", digest.as_str()));
            if options.dry_run {
                continue;
            }
            self.kappa().blob_delete(digest.as_str()).map_err(|error| {
                OciStoreError::Io(format!("delete {}: {error}", digest.as_str()))
            })?;
            report.removed_links += self.object_forget(digest)?;
        }
        report.swept_blobs = doomed.len();
        for (repo, digests) in &stale_links {
            let repo = RepoName::parse(repo)?;
            for digest in digests {
                emit(format!(
                    "{}: layer link eligible for deletion: {}",
                    repo.as_str(),
                    digest.as_str()
                ));
                if !options.dry_run && self.link_remove(&repo, digest)? {
                    report.removed_links += 1;
                }
            }
        }
        Ok(report)
    }

    /// What a manifest references: its config and layers, or an index's
    /// children, and its subject.
    fn gc_references(
        &self,
        repo: &RepoName,
        digest: &Digest,
    ) -> Result<Vec<Digest>, OciStoreError> {
        let manifest = self.manifest_get(repo, &Reference::Digest(digest.clone()))?;
        let value: serde_json::Value =
            serde_json::from_slice(&manifest.bytes).map_err(|error| {
                OciStoreError::Io(format!(
                    "{}: manifest {} cannot be parsed ({error}); nothing was swept",
                    repo.as_str(),
                    digest.as_str()
                ))
            })?;
        let mut out = Vec::new();
        let mut take = |node: &serde_json::Value| -> Result<(), OciStoreError> {
            if let Some(text) = node.get("digest").and_then(serde_json::Value::as_str) {
                out.push(Digest::parse(text)?);
            }
            Ok(())
        };
        if let Some(config) = value.get("config") {
            take(config)?;
        }
        for key in ["layers", "manifests", "fsLayers"] {
            for node in value
                .get(key)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                take(node)?;
            }
        }
        Ok(out)
    }

    fn gc_repos(&self) -> Result<Vec<RepoName>, OciStoreError> {
        let mut all: Vec<RepoName> = Vec::new();
        loop {
            let after = all.last().map(|repo| repo.as_str().to_owned());
            let page = self.repos_page(after.as_deref(), PAGE)?;
            let done = page.len() < PAGE;
            all.extend(page);
            if done {
                return Ok(all);
            }
        }
    }

    fn gc_links(&self, repo: &RepoName) -> Result<Vec<(Digest, LinkKind)>, OciStoreError> {
        let mut all: Vec<(Digest, LinkKind)> = Vec::new();
        loop {
            let after = all.last().map(|(digest, _)| digest.clone());
            let page = self.links_page(repo, after.as_ref(), PAGE)?;
            let done = page.len() < PAGE;
            all.extend(page.into_iter().map(|(digest, link)| (digest, link.kind)));
            if done {
                return Ok(all);
            }
        }
    }

    fn gc_tags(&self, repo: &RepoName) -> Result<Vec<Tag>, OciStoreError> {
        let mut all: Vec<Tag> = Vec::new();
        loop {
            let after = all.last().map(|tag| tag.as_str().to_owned());
            let page = match self.tags_page(repo, after.as_deref(), PAGE) {
                Ok(page) => page,
                // A repository with blobs and no manifest has no tags.
                Err(OciStoreError::UnknownRepository(_)) => return Ok(all),
                Err(error) => return Err(error),
            };
            let done = page.len() < PAGE;
            all.extend(page);
            if done {
                return Ok(all);
            }
        }
    }
}
