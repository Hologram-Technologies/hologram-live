//! `hologram oci garbage-collect` (plan P8 T1, FR-010, SC-008), as the
//! reference's `registry garbage-collect` marks and sweeps
//! (`registry/storage/garbagecollect.go`), with two rules of this registry:
//!
//! - it sweeps only objects the registry itself put in the store (`OBJECTS`):
//!   the Kappa store keeps records of its own beside the blobs (errata E12);
//! - a referrer (a signature, an SBOM) of a kept manifest is kept too, so
//!   `--delete-untagged` never strips an image of its signatures.
//!
//! Mark is a worklist over every repository: a manifest is walked in each
//! repository that links it once it is a root there (every manifest, or every
//! tagged one with `--delete-untagged`) or is marked anywhere. Walking marks
//! its config, layers and an index's children, and its referrers. Every digest
//! is marked under both of its names (sha256 and blake3), so a descriptor may
//! name bytes by either. A manifest that cannot be read or parsed stops the
//! run before anything is swept. Then, and only then, the sweep: untagged
//! manifests no kept manifest reaches, every recorded object that is not
//! marked (by its recorded name only), and each repository's links to what
//! is not marked. An untagged manifest that another repository keeps is kept
//! in every repository that links it, where the reference would delete it per
//! repository. Offline: the volume must not be open in a server.

use super::{Digest, LinkKind, OciStore, OciStoreError, Reference, RepoName, Tag};
use kappa_core::store::KappaStore as _;
use std::collections::BTreeSet;

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
    /// A manifest that cannot be read or parsed, or a store that refuses;
    /// nothing is swept after an error in mark.
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

        // Mark: a worklist. Every manifest link, under both of its names, so a
        // name marked anywhere finds each repository that links it.
        let mut linked_as_manifest: std::collections::BTreeMap<String, Vec<(usize, Digest)>> =
            std::collections::BTreeMap::new();
        let mut pending: Vec<(usize, Digest)> = Vec::new();
        for (index, repo) in repos.iter().enumerate() {
            emit(repo.name.as_str().to_owned());
            for (digest, kind) in &repo.links {
                if *kind == LinkKind::Blob {
                    continue;
                }
                let all = names(digest)?;
                for name in &all {
                    linked_as_manifest
                        .entry(name.clone())
                        .or_default()
                        .push((index, digest.clone()));
                }
                if !options.delete_untagged || all.iter().any(|name| repo.tagged.contains(name)) {
                    pending.push((index, digest.clone()));
                }
            }
        }
        while let Some((index, digest)) = pending.pop() {
            let repo = &repos[index];
            if !walked.insert((repo.name.as_str().to_owned(), digest.as_str().to_owned())) {
                continue;
            }
            emit(format!(
                "{}: marking manifest {} ",
                repo.name.as_str(),
                digest.as_str()
            ));
            let mut found = vec![digest.clone()];
            found.extend(self.gc_references(&repo.name, &digest)?);
            // Signatures and SBOMs of what is kept are kept.
            for subject in names(&digest)? {
                for referrer in self.referrers_of(&repo.name, &Digest::parse(&subject)?)? {
                    found.push(Digest::parse(&referrer.digest)?);
                }
            }
            for (position, reference) in found.into_iter().enumerate() {
                let newly = mark(&reference, &mut marked)?;
                if newly && position > 0 {
                    emit(format!(
                        "{}: marking blob {}",
                        repo.name.as_str(),
                        reference.as_str()
                    ));
                }
                // A manifest marked here is walked in every repository that links it.
                for name in names(&reference)? {
                    if let Some(links) = linked_as_manifest.get(&name) {
                        pending.extend(links.iter().cloned());
                    }
                }
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
            // The recorded name only. Its other name's file is not always the
            // registry's (the store may have had one of its own records at that
            // path, and a blake3 push then links nothing over it), so it stays:
            // a swept blob pushed by blake3 keeps the store's sha256 link.
            self.kappa().blob_delete(digest.as_str()).map_err(|error| {
                OciStoreError::Io(format!("delete {}: {error}", digest.as_str()))
            })?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci_store::OpenOptions;

    /// A dry run on a volume from before `OBJECTS` sees what the backfill
    /// would record, and writes nothing: the table stays empty and the flag
    /// unset. The real run then backfills and sweeps.
    #[test]
    fn a_dry_run_on_an_older_volume_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = OciStore::open(
            dir.path(),
            OpenOptions {
                create: true,
                upload_max_age: std::time::Duration::from_hours(1),
            },
        )
        .expect("open");
        let repo = RepoName::parse("old/volume").expect("repo");
        let id = store.upload_begin(&repo).expect("begin");
        store
            .upload_append(&id, 0, b"an orphan from before")
            .expect("append");
        let orphan = store
            .upload_finish(&id, &Digest::sha256_of(b"an orphan from before"))
            .expect("finish");
        store.forget_the_object_table();

        let mut lines = Vec::new();
        store
            .collect(
                GcOptions {
                    dry_run: true,
                    delete_untagged: false,
                },
                &mut |line| lines.push(line),
            )
            .expect("dry run");
        assert!(
            lines.contains(&format!("blob eligible for deletion: {orphan}")),
            "{lines:#?}"
        );
        assert!(
            store.objects().expect("objects").is_empty(),
            "nothing recorded"
        );
        assert!(!store.objects_backfilled(), "the flag is not set");
        assert!(store.blob_stat(&repo, &orphan).is_ok(), "nothing swept");

        store
            .collect(GcOptions::default(), &mut |_| {})
            .expect("collect");
        assert!(store.objects_backfilled());
        assert!(
            store.blob_stat(&repo, &orphan).is_err(),
            "swept after the backfill"
        );
    }
}
