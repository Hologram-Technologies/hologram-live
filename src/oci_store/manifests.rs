//! Manifests, tags and referrers in the store.
//!
//! A manifest is the one thing read and written whole: it is small (the HTTP
//! layer caps it at 4 MiB), and its digest must be checked against every byte
//! before anything is recorded. This is the only file where the store's
//! whole-buffer calls are allowed (`scripts/check-oci-streaming.sh`).
//!
//! Writes happen in this order, each durable before the next
//! (`data-model.md`, "Manifest PUT"):
//!
//! 1. the bytes into the store, verified against the digest;
//! 2. one `links.redb` transaction: link, referrers row, repository row;
//! 3. the tag, when the reference is a tag.
//!
//! No order of kills leaves a tag pointing at a manifest that is not linked,
//! or a link pointing at bytes that are not stored.

use super::links::{Link, LinkKind, ReferrerDescriptor};
use super::{Algorithm, Digest, OciStore, OciStoreError, Reference, RepoName, Tag};
use kappa_core::types::StoreError;
use kappa_core::KappaStore;
use sha2::Digest as _;

/// The store records who owns a namespace; the registry is the only writer.
const NAMESPACE_OWNER: &str = "hologram-registry";

/// The largest manifest the store reads whole. The HTTP layer refuses a
/// larger one on the way in; this holds the line on the way out.
pub const MANIFEST_MAX: u64 = 4 * 1024 * 1024;

/// What a manifest without a recorded media type is served as.
const DEFAULT_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// What the HTTP layer's validator learned from a manifest body. This layer
/// does not parse manifests; it enforces what the plan says.
#[derive(Debug, Clone)]
pub struct ManifestPlan {
    /// `Manifest` or `Index`.
    pub kind: LinkKind,
    /// Every config, layer and child manifest the body names. Each must be
    /// linked in the same repository.
    pub must_exist: Vec<Digest>,
    /// Present when the manifest names a `subject`.
    pub subject: Option<SubjectPlan>,
}

/// The referrers row a manifest with a `subject` creates.
#[derive(Debug, Clone)]
pub struct SubjectPlan {
    /// What the manifest refers to. Never validated to be a manifest: the
    /// hub's sidecar manifests point their subject at a blob.
    pub subject: Digest,
    pub artifact_type: Option<String>,
    pub annotations: Option<serde_json::Map<String, serde_json::Value>>,
}

/// A manifest as it was pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredManifest {
    /// The digest the caller asked by, or the one the tag pointed at.
    pub digest: Digest,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

fn store_io(what: &str, error: &StoreError) -> OciStoreError {
    OciStoreError::Io(format!("{what}: {error}"))
}

/// The digest of `bytes` under `algorithm`.
fn digest_of(algorithm: Algorithm, bytes: &[u8]) -> Result<Digest, OciStoreError> {
    match algorithm {
        Algorithm::Sha256 => Ok(Digest::sha256_of(bytes)),
        Algorithm::Blake3 => Ok(Digest::from_blake3(&blake3::hash(bytes))),
        Algorithm::Sha512 => Digest::parse(&format!(
            "sha512:{}",
            crate::util::hex(&sha2::Sha512::digest(bytes))
        )),
    }
}

impl OciStore {
    /// Store a manifest under `reference`. Returns its digest.
    ///
    /// # Errors
    ///
    /// `DigestMismatch` when `reference` is a digest the bytes do not hash to;
    /// `MissingReferences` when the plan names content this repository does
    /// not link; `Io`.
    pub fn manifest_put(
        &self,
        repo: &RepoName,
        reference: &Reference,
        media_type: &str,
        bytes: &[u8],
        plan: &ManifestPlan,
    ) -> Result<Digest, OciStoreError> {
        // The address: what the client named, or sha256 for a push by tag.
        let digest = match reference {
            Reference::Digest(claimed) => {
                if digest_of(claimed.algorithm(), bytes)? != *claimed {
                    return Err(OciStoreError::DigestMismatch {
                        claimed: claimed.as_str().to_owned(),
                    });
                }
                claimed.clone()
            }
            Reference::Tag(_) => Digest::sha256_of(bytes),
        };

        let mut missing = Vec::new();
        for needed in &plan.must_exist {
            if self.require_link(repo, needed).is_err() {
                missing.push(needed.as_str().to_owned());
            }
        }
        if !missing.is_empty() {
            return Err(OciStoreError::MissingReferences(missing));
        }

        // 1. The bytes. The store hashes them again against the address.
        let newly_stored = match self.kappa().ingest_verified(digest.as_str(), bytes) {
            Ok(result) => result.newly_stored,
            Err(StoreError::Rejected(_)) => {
                return Err(OciStoreError::DigestMismatch {
                    claimed: digest.as_str().to_owned(),
                });
            }
            Err(error) => return Err(store_io("store the manifest", &error)),
        };
        // The same bytes under their other name, so a manifest pushed by
        // blake3 is also served by sha256 and back.
        let other = if digest.algorithm() == Algorithm::Blake3 {
            Digest::sha256_of(bytes)
        } else {
            Digest::from_blake3(&blake3::hash(bytes))
        };
        self.alias_put(&digest, &other)?;

        // 2. Link, referrers row and repository row, in one transaction.
        let link = Link {
            kind: plan.kind,
            media_type: Some(media_type.to_owned()),
        };
        let referrer = plan.subject.as_ref().map(|subject| {
            (
                &subject.subject,
                ReferrerDescriptor {
                    digest: digest.as_str().to_owned(),
                    media_type: media_type.to_owned(),
                    size: bytes.len() as u64,
                    artifact_type: subject.artifact_type.clone(),
                    annotations: subject.annotations.clone(),
                },
            )
        });
        self.manifest_commit(
            repo,
            &digest,
            &link,
            referrer
                .as_ref()
                .map(|(subject, descriptor)| (*subject, descriptor)),
        )?;
        // Only bytes this push stored are the registry's to sweep later.
        if newly_stored {
            self.object_note(&digest)?;
        }

        // 3. The tag moves last, and only now.
        if let Reference::Tag(tag) = reference {
            let namespace = self.namespace(repo)?;
            self.kappa()
                .tag_set(&namespace, tag.as_str(), digest.as_str())
                .map_err(|error| store_io("set the tag", &error))?;
        }
        Ok(digest)
    }

    /// Read a manifest by tag or digest. A tag is resolved to a digest once,
    /// and that digest is what is read: a tag moved during the call cannot
    /// produce a body that does not match the reported digest.
    ///
    /// # Errors
    ///
    /// `UnknownRepository`; `NotInRepository` when the tag does not exist,
    /// the digest is not linked here, or it is linked as a blob: a layer is
    /// never read whole through the manifest route; `Io`.
    pub fn manifest_get(
        &self,
        repo: &RepoName,
        reference: &Reference,
    ) -> Result<StoredManifest, OciStoreError> {
        if !self.repo_exists(repo)? {
            return Err(OciStoreError::UnknownRepository(repo.as_str().to_owned()));
        }
        let digest = match reference {
            Reference::Digest(digest) => digest.clone(),
            Reference::Tag(tag) => self.tag_resolve(repo, tag)?,
        };
        self.require_link(repo, &digest)?;
        let link = match self.link_get(repo, &digest)? {
            Some(link) => Some(link),
            None => match self.alias_of(&digest)? {
                Some(alias) => self.link_get(repo, &alias)?,
                None => None,
            },
        };
        let not_a_manifest = || OciStoreError::NotInRepository {
            repo: repo.as_str().to_owned(),
            digest: digest.as_str().to_owned(),
        };
        let link = match link {
            Some(link) if link.kind != LinkKind::Blob => Some(link),
            _ => return Err(not_a_manifest()),
        };
        let stored_as = self.resolve_stored(repo, &digest)?;
        let size = self
            .kappa()
            .blob_size(stored_as.as_str())
            .map_err(|error| store_io("size the manifest", &error))?;
        if size > MANIFEST_MAX {
            return Err(OciStoreError::Io(format!(
                "{stored_as} is linked as a manifest but holds {size} bytes"
            )));
        }
        let bytes = self
            .kappa()
            .blob_get(stored_as.as_str())
            .map_err(|error| store_io("read the manifest", &error))?;
        Ok(StoredManifest {
            digest,
            media_type: link
                .and_then(|link| link.media_type)
                .unwrap_or_else(|| DEFAULT_MEDIA_TYPE.to_owned()),
            bytes,
        })
    }

    /// Remove a manifest from `repo`, and every tag in `repo` that points at
    /// it. The bytes stay until garbage collection.
    ///
    /// # Errors
    ///
    /// `NotInRepository` when the manifest is not linked here, or the digest
    /// is linked as a blob; `Io`.
    ///
    /// Tags go first and the link last. A kill in between leaves a manifest
    /// reachable by digest with fewer tags, never a tag that points at nothing.
    pub fn manifest_delete(&self, repo: &RepoName, digest: &Digest) -> Result<(), OciStoreError> {
        let alias = self.alias_of(digest)?;
        let link = match self.link_get(repo, digest)? {
            Some(link) => Some(link),
            None => match &alias {
                Some(alias) => self.link_get(repo, alias)?,
                None => None,
            },
        };
        if !link.is_some_and(|link| link.kind != LinkKind::Blob) {
            return Err(OciStoreError::NotInRepository {
                repo: repo.as_str().to_owned(),
                digest: digest.as_str().to_owned(),
            });
        }
        let namespace = self.namespace(repo)?;
        let tags = self
            .kappa()
            .tag_list(&namespace)
            .map_err(|error| store_io("list tags", &error))?;
        for entry in tags {
            let points_here = entry.kappa == digest.as_str()
                || alias
                    .as_ref()
                    .is_some_and(|alias| entry.kappa == alias.as_str());
            if points_here {
                self.kappa()
                    .tag_delete(&namespace, &entry.name)
                    .map_err(|error| store_io("delete a tag", &error))?;
            }
        }
        self.link_remove(repo, digest)?;
        if let Some(alias) = &alias {
            self.link_remove(repo, alias)?;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// `NotInRepository` when the tag does not exist; `Io`.
    pub fn tag_delete(&self, repo: &RepoName, tag: &Tag) -> Result<(), OciStoreError> {
        self.tag_resolve(repo, tag)?;
        let namespace = self.namespace(repo)?;
        self.kappa()
            .tag_delete(&namespace, tag.as_str())
            .map_err(|error| store_io("delete the tag", &error))
    }

    /// One page of tag names in lexical order, after `after`.
    ///
    /// The store has no ordered range over tags, so this lists them all,
    /// sorts, and slices: a repository with 100,000 tags pays one full list
    /// per page. If the tags route measures over 50 ms there, mirror tag
    /// names into a `tags` table in `links.redb` within `manifest_commit`.
    ///
    /// # Errors
    ///
    /// `UnknownRepository`; `Io`.
    pub fn tags_page(
        &self,
        repo: &RepoName,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Tag>, OciStoreError> {
        if !self.repo_exists(repo)? {
            return Err(OciStoreError::UnknownRepository(repo.as_str().to_owned()));
        }
        let namespace = self.namespace(repo)?;
        let mut names: Vec<String> = self
            .kappa()
            .tag_list(&namespace)
            .map_err(|error| store_io("list tags", &error))?
            .into_iter()
            .map(|entry| entry.name)
            .filter(|name| after.is_none_or(|after| name.as_str() > after))
            .collect();
        names.sort_unstable();
        names
            .into_iter()
            .take(limit)
            .map(|name| Tag::parse(&name))
            .collect()
    }

    pub(crate) fn tag_resolve(&self, repo: &RepoName, tag: &Tag) -> Result<Digest, OciStoreError> {
        let namespace = self.namespace(repo)?;
        match self.kappa().tag_get(&namespace, tag.as_str()) {
            Ok(entry) => Digest::parse(&entry.kappa),
            Err(StoreError::NotFound(_)) => Err(OciStoreError::NotInRepository {
                repo: repo.as_str().to_owned(),
                digest: tag.as_str().to_owned(),
            }),
            Err(error) => Err(store_io("read the tag", &error)),
        }
    }

    fn namespace(&self, repo: &RepoName) -> Result<kappa_core::types::NamespaceRef, OciStoreError> {
        self.kappa()
            .namespace_resolve_or_create(repo.as_str(), NAMESPACE_OWNER, Some("oci"))
            .map_err(|error| store_io("resolve the repository namespace", &error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci_store::OpenOptions;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
    const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";

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

    fn tag(name: &str) -> Reference {
        Reference::Tag(Tag::parse(name).expect("tag"))
    }

    fn plain() -> ManifestPlan {
        ManifestPlan {
            kind: LinkKind::Manifest,
            must_exist: Vec::new(),
            subject: None,
        }
    }

    fn push_blob(store: &OciStore, name: &str, bytes: &[u8]) -> Digest {
        let id = store.upload_begin(&repo(name)).expect("begin");
        store.upload_append(&id, 0, bytes).expect("append");
        store
            .upload_finish(&id, &Digest::sha256_of(bytes))
            .expect("finish")
    }

    #[test]
    fn a_manifest_put_by_tag_reads_back_by_tag_and_by_digest() {
        let (store, _dir) = test_store();
        let body = br#"{"schemaVersion":2}"#;
        let digest = store
            .manifest_put(
                &repo("a/x"),
                &tag("latest"),
                DOCKER_MANIFEST,
                body,
                &plain(),
            )
            .expect("put");
        assert_eq!(digest, Digest::sha256_of(body));
        let by_tag = store
            .manifest_get(&repo("a/x"), &tag("latest"))
            .expect("get");
        let by_digest = store
            .manifest_get(&repo("a/x"), &Reference::Digest(digest.clone()))
            .expect("get");
        assert_eq!(by_tag, by_digest);
        assert_eq!(by_tag.bytes, body);
        assert_eq!(by_tag.media_type, DOCKER_MANIFEST);
        assert_eq!(by_tag.digest, digest);
    }

    #[test]
    fn a_put_by_a_digest_the_body_does_not_hash_to_is_refused() {
        let (store, _dir) = test_store();
        let wrong = Reference::Digest(Digest::sha256_of(b"something else"));
        assert!(matches!(
            store.manifest_put(&repo("a/x"), &wrong, OCI_MANIFEST, b"{}", &plain()),
            Err(OciStoreError::DigestMismatch { .. })
        ));
        assert!(!store.repo_exists(&repo("a/x")).expect("exists"));
    }

    #[test]
    fn a_layer_linked_only_in_another_repository_is_a_missing_reference() {
        let (store, _dir) = test_store();
        let layer = push_blob(&store, "b/y", b"layer bytes");
        let plan = ManifestPlan {
            must_exist: vec![layer.clone()],
            ..plain()
        };
        let outcome = store.manifest_put(&repo("a/x"), &tag("v1"), OCI_MANIFEST, b"{}", &plan);
        assert!(matches!(
            outcome,
            Err(OciStoreError::MissingReferences(missing)) if missing == [layer.as_str()]
        ));
        // Linked where it was pushed, the same plan is accepted.
        store
            .manifest_put(&repo("b/y"), &tag("v1"), OCI_MANIFEST, b"{}", &plan)
            .expect("put where the layer lives");
    }

    #[test]
    fn moving_a_tag_keeps_the_old_manifest_reachable_by_digest() {
        let (store, _dir) = test_store();
        let name = repo("a/x");
        let old = store
            .manifest_put(&name, &tag("latest"), OCI_MANIFEST, b"{\"v\":1}", &plain())
            .expect("put");
        let new = store
            .manifest_put(&name, &tag("latest"), OCI_MANIFEST, b"{\"v\":2}", &plain())
            .expect("put");
        assert_ne!(old, new);
        assert_eq!(
            store
                .manifest_get(&name, &tag("latest"))
                .expect("get")
                .digest,
            new
        );
        let by_old = store
            .manifest_get(&name, &Reference::Digest(old))
            .expect("the old manifest is still here");
        assert_eq!(by_old.bytes, b"{\"v\":1}");
    }

    #[test]
    fn tags_page_in_lexical_order_and_resume_after_a_name() {
        let (store, _dir) = test_store();
        let name = repo("a/x");
        for label in ["b", "a10", "a", "c", "a2"] {
            store
                .manifest_put(&name, &tag(label), OCI_MANIFEST, b"{}", &plain())
                .expect("put");
        }
        let names = |page: Vec<Tag>| -> Vec<String> {
            page.iter().map(|tag| tag.as_str().to_owned()).collect()
        };
        assert_eq!(
            names(store.tags_page(&name, None, 3).expect("page")),
            ["a", "a10", "a2"]
        );
        assert_eq!(
            names(store.tags_page(&name, Some("a2"), 3).expect("page")),
            ["b", "c"]
        );
        assert!(matches!(
            store.tags_page(&repo("no/such"), None, 3),
            Err(OciStoreError::UnknownRepository(_))
        ));
    }

    #[test]
    fn deleting_a_manifest_removes_every_tag_that_points_at_it() {
        let (store, _dir) = test_store();
        let name = repo("a/x");
        let digest = store
            .manifest_put(&name, &tag("v1"), OCI_MANIFEST, b"{}", &plain())
            .expect("put");
        store
            .manifest_put(&name, &tag("stable"), OCI_MANIFEST, b"{}", &plain())
            .expect("same manifest, second tag");
        store
            .manifest_put(&name, &tag("other"), OCI_MANIFEST, b"{\"o\":1}", &plain())
            .expect("put");

        store.manifest_delete(&name, &digest).expect("delete");

        let left: Vec<String> = store
            .tags_page(&name, None, 10)
            .expect("page")
            .iter()
            .map(|tag| tag.as_str().to_owned())
            .collect();
        assert_eq!(left, ["other"]);
        assert!(matches!(
            store.manifest_get(&name, &Reference::Digest(digest.clone())),
            Err(OciStoreError::NotInRepository { .. })
        ));
        assert!(matches!(
            store.manifest_delete(&name, &digest),
            Err(OciStoreError::NotInRepository { .. })
        ));
    }

    #[test]
    fn a_manifest_with_a_subject_is_listed_as_its_referrer() {
        let (store, _dir) = test_store();
        let name = repo("a/x");
        let image = store
            .manifest_put(&name, &tag("v1"), OCI_MANIFEST, b"{\"image\":1}", &plain())
            .expect("put");
        let plan = ManifestPlan {
            subject: Some(SubjectPlan {
                subject: image.clone(),
                artifact_type: Some("application/vnd.example.signature".to_owned()),
                annotations: None,
            }),
            ..plain()
        };
        let body = b"{\"signature\":1}";
        let signature = store
            .manifest_put(
                &name,
                &Reference::Digest(Digest::sha256_of(body)),
                OCI_MANIFEST,
                body,
                &plan,
            )
            .expect("put");
        let referrers = store.referrers_of(&name, &image).expect("referrers");
        assert_eq!(referrers.len(), 1);
        assert_eq!(referrers[0].digest, signature.as_str());
        assert_eq!(referrers[0].size, body.len() as u64);
    }

    #[test]
    fn a_manifest_pushed_by_blake3_is_served_by_both_names() {
        let (store, _dir) = test_store();
        let body = b"{\"hub\":true}";
        let blake = Digest::from_blake3(&blake3::hash(body));
        store
            .manifest_put(
                &repo("model-hub"),
                &Reference::Digest(blake.clone()),
                OCI_MANIFEST,
                body,
                &plain(),
            )
            .expect("FR-022: the hub pushes by blake3");
        for digest in [blake, Digest::sha256_of(body)] {
            let got = store
                .manifest_get(&repo("model-hub"), &Reference::Digest(digest.clone()))
                .expect("get");
            assert_eq!(got.bytes, body);
            assert_eq!(
                got.digest, digest,
                "the digest asked by is the digest reported"
            );
        }
    }

    #[test]
    fn a_layer_is_never_read_or_deleted_through_the_manifest_route() {
        let (store, _dir) = test_store();
        let layer = push_blob(&store, "a/x", &[7_u8; 4096]);
        let asked = Reference::Digest(layer.clone());
        assert!(matches!(
            store.manifest_get(&repo("a/x"), &asked),
            Err(OciStoreError::NotInRepository { .. })
        ));
        assert!(matches!(
            store.manifest_delete(&repo("a/x"), &layer),
            Err(OciStoreError::NotInRepository { .. })
        ));
        assert!(
            store.blob_stat(&repo("a/x"), &layer).is_ok(),
            "the layer is still linked"
        );
    }

    #[test]
    fn a_read_by_tag_during_a_tag_move_returns_one_whole_manifest() {
        let (store, _dir) = test_store();
        let store = Arc::new(store);
        let name = repo("a/x");
        let bodies: [&[u8]; 2] = [b"{\"v\":\"old\"}", b"{\"v\":\"new, and longer\"}"];
        store
            .manifest_put(&name, &tag("latest"), OCI_MANIFEST, bodies[0], &plain())
            .expect("put");
        let stop = Arc::new(AtomicBool::new(false));
        let mover = {
            let (store, stop, name) = (store.clone(), stop.clone(), name.clone());
            std::thread::spawn(move || {
                let mut turn = 0_usize;
                while !stop.load(Ordering::Relaxed) {
                    turn += 1;
                    store
                        .manifest_put(
                            &name,
                            &tag("latest"),
                            OCI_MANIFEST,
                            bodies[turn % 2],
                            &plain(),
                        )
                        .expect("move the tag");
                }
            })
        };
        for _ in 0..1000 {
            let got = store.manifest_get(&name, &tag("latest")).expect("get");
            assert_eq!(
                Digest::sha256_of(&got.bytes),
                got.digest,
                "the body must hash to the digest reported with it"
            );
        }
        stop.store(true, Ordering::Relaxed);
        mover.join().expect("mover");
    }
}
