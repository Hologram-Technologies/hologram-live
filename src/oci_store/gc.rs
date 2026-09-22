//! `hologram oci garbage-collect` (plan P8 T1, FR-010, SC-008), as the
//! reference's `registry garbage-collect` marks and sweeps
//! (`registry/storage/garbagecollect.go`), with two rules of this registry:
//!
//! - it sweeps only objects the registry itself put in the store (`OBJECTS`):
//!   the Kappa store keeps records of its own beside the blobs (errata E12);
//! - a referrer (a signature, an SBOM) of a kept manifest is kept too, so
//!   `--delete-untagged` never strips an image of its signatures.
//!
//! Mark is a fixpoint over every repository: a manifest is walked in each
//! repository that links it once it is a root there (every manifest, or every
//! tagged one with `--delete-untagged`) or is marked anywhere. Walking marks
//! its config, layers and an index's children, and its referrers. Every digest
//! is marked under both of its names (sha256 and blake3), so a descriptor may
//! name bytes by either. A manifest that cannot be read or parsed stops the
//! run before anything is swept. Then, and only then, the sweep: untagged
//! manifests no kept manifest reaches, every recorded object that is not
//! marked (and its hard-linked other name), and each repository's links to
//! what is not marked. Offline: the volume must not be open in a server.

use super::{Digest, LinkKind, OciStore, OciStoreError, Reference, RepoName, Tag};
use kappa_core::store::KappaStore as _;
use std::collections::BTreeSet;

/// A chain of indexes, subjects and referrers longer than this is taken as a
/// loop or a fault: the run stops before sweeping.
const MAX_ROUNDS: usize = 64;
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

/// One repository as mark sees it.
struct Repo {
    name: RepoName,
    links: Vec<(Digest, LinkKind)>,
    tags: Vec<String>,
    /// Manifests a tag points at (by the digest the tag names).
    tagged: BTreeSet<String>,
}

impl OciStore {
    /// Mark and sweep. `emit` gets each line the reference prints, in its
    /// words, so scripts that read the reference's output read ours.
    ///
    /// # Errors
    ///
    /// A manifest that cannot be read or parsed, a chain of references
    /// longer than 64, or a store that refuses; nothing is swept after an
    /// error in mark.
    pub fn collect(
        &self,
        options: GcOptions,
        emit: &mut dyn FnMut(String),
    ) -> Result<GcReport, OciStoreError> {
        // A dry run writes nothing, the backfill included.
        let mut objects = if options.dry_run {
            let mut all = self.objects()?;
            all.extend(self.objects_backfill_plan()?);
            all
        } else {
            self.objects_backfill()?;
            self.objects()?
        };
        objects.sort();
        objects.dedup();

        let repos = self.gc_repos_with_links()?;
        let names = |digest: &Digest| -> Result<Vec<String>, OciStoreError> {
            let mut out = vec![digest.as_str().to_owned()];
            if let Some(alias) = self.alias_of(digest)? {
                out.push(alias.as_str().to_owned());
            }
            Ok(out)
        };
        // Every name of every marked digest; `counted` as the reference counts them.
        let mut marked: BTreeSet<String> = BTreeSet::new();
        let mut counted: BTreeSet<String> = BTreeSet::new();
        let mut walked: BTreeSet<(String, String)> = BTreeSet::new();
        let mut mark =
            |digest: &Digest, marked: &mut BTreeSet<String>| -> Result<bool, OciStoreError> {
                counted.insert(digest.as_str().to_owned());
                let mut newly = false;
                for name in names(digest)? {
                    newly |= marked.insert(name);
                }
                Ok(newly)
            };

        // Mark: rounds until nothing new is walked.
        for round in 0.. {
            if round >= MAX_ROUNDS {
                return Err(OciStoreError::Io(format!(
                    "references nest deeper than {MAX_ROUNDS} levels; nothing was swept"
                )));
            }
            let mut progress = false;
            for repo in &repos {
                if round == 0 {
                    emit(repo.name.as_str().to_owned());
                }
                for (digest, kind) in &repo.links {
                    if *kind == LinkKind::Blob {
                        continue;
                    }
                    let key = (repo.name.as_str().to_owned(), digest.as_str().to_owned());
                    if walked.contains(&key) {
                        continue;
                    }
                    let all = names(digest)?;
                    let root = !options.delete_untagged
                        || all.iter().any(|name| repo.tagged.contains(name));
                    let kept = all.iter().any(|name| marked.contains(name));
                    if !(root || kept) {
                        continue;
                    }
                    walked.insert(key);
                    progress = true;
                    emit(format!(
                        "{}: marking manifest {} ",
                        repo.name.as_str(),
                        digest.as_str()
                    ));
                    mark(digest, &mut marked)?;
                    for reference in self.gc_references(&repo.name, digest)? {
                        if mark(&reference, &mut marked)? {
                            emit(format!(
                                "{}: marking blob {}",
                                repo.name.as_str(),
                                reference.as_str()
                            ));
                        }
                    }
                    // Signatures and SBOMs of what is kept are kept.
                    for subject in names(digest)? {
                        for referrer in self.referrers_of(&repo.name, &Digest::parse(&subject)?)? {
                            mark(&Digest::parse(&referrer.digest)?, &mut marked)?;
                        }
                    }
                }
            }
            if !progress {
                break;
            }
        }
        let is_marked = |digest: &Digest| -> Result<bool, OciStoreError> {
            Ok(names(digest)?.iter().any(|name| marked.contains(name)))
        };

        // What goes, decided only now that mark is complete.
        let mut untagged: Vec<(RepoName, Digest)> = Vec::new();
        let mut stale_links: Vec<(RepoName, Digest)> = Vec::new();
        for repo in &repos {
            for (digest, kind) in &repo.links {
                if is_marked(digest)? {
                    continue;
                }
                if *kind == LinkKind::Blob {
                    stale_links.push((repo.name.clone(), digest.clone()));
                } else {
                    emit(format!(
                        "manifest eligible for deletion: {{{} {} [{}]}}",
                        repo.name.as_str(),
                        digest.as_str(),
                        repo.tags.join(" ")
                    ));
                    untagged.push((repo.name.clone(), digest.clone()));
                }
            }
        }
        let doomed: Vec<Digest> = objects
            .into_iter()
            .map(|(digest, _)| digest)
            .filter(|digest| !marked.contains(digest.as_str()))
            .collect();

        // Sweep.
        let mut report = GcReport {
            marked: counted.len(),
            swept_manifests: untagged.len(),
            swept_blobs: doomed.len(),
            ..GcReport::default()
        };
        if !options.dry_run {
            for (repo, digest) in &untagged {
                self.manifest_delete(repo, digest)?;
            }
        }
        emit(format!(
            "\n{} blobs marked, {} blobs and {} manifests eligible for deletion",
            counted.len(),
            doomed.len(),
            untagged.len()
        ));
        for digest in &doomed {
            emit(format!("blob eligible for deletion: {}", digest.as_str()));
            if options.dry_run {
                continue;
            }
            // The other name is a hard link of the same file: unmarked too,
            // or the digest would be marked. Both go, or no space is freed.
            for name in names(digest)? {
                self.kappa()
                    .blob_delete(&name)
                    .map_err(|error| OciStoreError::Io(format!("delete {name}: {error}")))?;
            }
            report.removed_links += self.object_forget(digest)?;
        }
        for (repo, digest) in &stale_links {
            emit(format!(
                "{}: layer link eligible for deletion: {}",
                repo.as_str(),
                digest.as_str()
            ));
            if !options.dry_run && self.link_remove(repo, digest)? {
                report.removed_links += 1;
            }
        }
        Ok(report)
    }

    /// What a manifest references: its config and layers, or an index's
    /// children.
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

    /// Every repository with its links and tags, read once before mark.
    fn gc_repos_with_links(&self) -> Result<Vec<Repo>, OciStoreError> {
        let mut out = Vec::new();
        for name in self.gc_repos()? {
            let links = self.gc_links(&name)?;
            let tags = self.gc_tags(&name)?;
            let mut tagged = BTreeSet::new();
            for tag in &tags {
                tagged.insert(self.tag_resolve(&name, tag)?.as_str().to_owned());
            }
            out.push(Repo {
                tags: tags.iter().map(|tag| tag.as_str().to_owned()).collect(),
                name,
                links,
                tagged,
            });
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
