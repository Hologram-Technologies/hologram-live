//! Fetching a named artifact into the local content store.
//!
//! Every step is content-addressed, which makes the pull idempotent and
//! resumable for free: a re-run transfers only what is still missing, and an
//! interrupted pull leaves verified blobs the next run reuses.
//!
//! Two properties of the upstream registry shape this code. It does not verify
//! that a manifest's layers exist, so a dangling manifest is publishable and
//! the client must confirm presence itself. And tags are mutable, so a name is
//! never a cache key — every pull re-resolves and records the manifest digest.

use crate::artifact_manifest::ArtifactManifest;
use crate::artifact_ref::ArtifactRef;
use crate::error::{LiveError, Result};
use crate::store::ObjectStore;

/// Where a layer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerSource {
    AlreadyPresent,
    Fetched,
}

/// One layer resolved. Emitted as it happens so a caller can render progress
/// without the pull knowing anything about presentation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PullProgress {
    pub kappa: String,
    pub index: usize,
    pub total: usize,
    pub source: LayerSource,
    pub bytes: u64,
}

/// The machine-readable result of a completed pull.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PullReport {
    pub reference: String,
    /// The resolved manifest digest. Recording it is what makes a pull
    /// reproducible when the tag later moves.
    pub manifest_digest: Option<String>,
    pub archive_kappa: String,
    pub layers_fetched: usize,
    pub layers_present: usize,
    pub bytes_transferred: u64,
}

/// The registry operations a pull needs.
///
/// Narrower than the full client so the pull can be tested against a fake
/// without a running registry.
pub trait LayerFetch {
    /// The manifest body and its digest, or `None` when the tag is unknown.
    fn manifest(&self, repository: &str, tag: &str) -> Result<Option<(Vec<u8>, Option<String>)>>;
    /// Layer bytes from the *same* repository the manifest came from.
    ///
    /// The repository is a parameter rather than client state because a
    /// reference names its own namespace: resolving a manifest from
    /// `models/demo` and then fetching its layers from the client's configured
    /// namespace would look up blobs that are not there.
    fn blob(&self, repository: &str, kappa: &str) -> Result<Vec<u8>>;
}

pub fn pull(
    fetch: &dyn LayerFetch,
    store: &ObjectStore,
    reference: &ArtifactRef,
    on_progress: &mut dyn FnMut(PullProgress),
) -> Result<PullReport> {
    let (body, manifest_digest) = fetch
        .manifest(&reference.repository(), &reference.tag)?
        .ok_or_else(|| {
            LiveError::NotFound(format!("artifact {} not found", reference.display()))
        })?;
    let manifest = ArtifactManifest::decode(&body)?;
    let archive_kappa = manifest.archive()?.kappa.clone();

    let total = manifest.layers.len();
    let mut layers_fetched = 0;
    let mut layers_present = 0;
    let mut bytes_transferred = 0_u64;

    for (index, layer) in manifest.layers.iter().enumerate() {
        // Consulting the local store first is what makes the transfer
        // deduplicating: two artifacts sharing weights fetch them once.
        if store.get_cached(&layer.kappa)?.is_some() {
            layers_present += 1;
            on_progress(PullProgress {
                kappa: layer.kappa.clone(),
                index,
                total,
                source: LayerSource::AlreadyPresent,
                bytes: 0,
            });
            continue;
        }

        let bytes =
            fetch
                .blob(&reference.repository(), &layer.kappa)
                .map_err(|error| match error {
                    // Name the layer: the registry does not validate layer presence,
                    // so a dangling manifest is the likely cause and the operator
                    // needs to know which blob is missing.
                    LiveError::NotFound(_) => LiveError::NotFound(format!(
                        "artifact {} references layer {} which the registry does not hold",
                        reference.display(),
                        layer.kappa
                    )),
                    other => other,
                })?;

        // cache_addressed re-hashes the bytes and refuses to store them under
        // an address they do not produce, so a corrupt or substituted layer
        // cannot enter the store.
        store.cache_addressed(&layer.kappa, &bytes)?;

        let transferred = bytes.len().try_into().unwrap_or(u64::MAX);
        bytes_transferred = bytes_transferred.saturating_add(transferred);
        layers_fetched += 1;
        on_progress(PullProgress {
            kappa: layer.kappa.clone(),
            index,
            total,
            source: LayerSource::Fetched,
            bytes: transferred,
        });
    }

    // The registry accepts manifests whose layers were never uploaded, so
    // confirm the whole set is present before reporting success. Without this,
    // a dangling manifest would produce a "successful" pull of an artifact
    // that cannot run.
    for layer in &manifest.layers {
        if store.get_cached(&layer.kappa)?.is_none() {
            return Err(LiveError::NotFound(format!(
                "artifact {} is incomplete: layer {} is missing after pull",
                reference.display(),
                layer.kappa
            )));
        }
    }

    Ok(PullReport {
        reference: reference.display(),
        manifest_digest,
        archive_kappa,
        layers_fetched,
        layers_present,
        bytes_transferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ARCHIVE_BYTES: &[u8] = b"THIN-HOLO-ARCHIVE";
    const WEIGHT_BYTES: &[u8] = b"SHARED-WEIGHTS";

    fn kappa(bytes: &[u8]) -> String {
        format!("blake3:{}", blake3::hash(bytes).to_hex())
    }

    /// Records every blob request so a test can prove a cached layer was never
    /// fetched, which is the whole point of deduplication.
    struct FakeRegistry {
        blobs: HashMap<String, Vec<u8>>,
        manifest: Option<Vec<u8>>,
        requested: RefCell<Vec<String>>,
    }

    impl FakeRegistry {
        fn new() -> Self {
            let archive = kappa(ARCHIVE_BYTES);
            let weights = kappa(WEIGHT_BYTES);
            let manifest = format!(
                r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                     "layers":[
                       {{"mediaType":"application/vnd.hologram.holo","digest":"{archive}","size":17,
                         "annotations":{{"dev.hologram.role":"archive"}}}},
                       {{"mediaType":"application/octet-stream","digest":"{weights}","size":14,
                         "annotations":{{"dev.hologram.role":"layer"}}}}]}}"#
            );
            let mut blobs = HashMap::new();
            blobs.insert(archive, ARCHIVE_BYTES.to_vec());
            blobs.insert(weights, WEIGHT_BYTES.to_vec());
            Self {
                blobs,
                manifest: Some(manifest.into_bytes()),
                requested: RefCell::new(Vec::new()),
            }
        }
    }

    impl LayerFetch for FakeRegistry {
        fn manifest(
            &self,
            _repository: &str,
            _tag: &str,
        ) -> Result<Option<(Vec<u8>, Option<String>)>> {
            Ok(self
                .manifest
                .clone()
                .map(|body| (body, Some("sha256:deadbeef".to_owned()))))
        }

        fn blob(&self, _repository: &str, kappa: &str) -> Result<Vec<u8>> {
            self.requested.borrow_mut().push(kappa.to_owned());
            self.blobs
                .get(kappa)
                .cloned()
                .ok_or_else(|| LiveError::NotFound(format!("blob {kappa} not found")))
        }
    }

    fn store(label: &str) -> (ObjectStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hologram-pull-{label}-{}-{}",
            std::process::id(),
            crate::util::now_millis()
        ));
        (ObjectStore::open(&root).expect("open"), root)
    }

    fn reference() -> ArtifactRef {
        ArtifactRef::parse("qwen3.5:4b", &RegistryConfig::default()).expect("parse")
    }

    #[test]
    fn a_first_pull_fetches_every_layer() {
        let (store, root) = store("first");
        let registry = FakeRegistry::new();
        let mut events = Vec::new();

        let report = pull(&registry, &store, &reference(), &mut |p| events.push(p)).expect("pull");

        assert_eq!(report.layers_fetched, 2);
        assert_eq!(report.layers_present, 0);
        assert_eq!(report.bytes_transferred, 31);
        assert_eq!(report.archive_kappa, kappa(ARCHIVE_BYTES));
        assert_eq!(report.manifest_digest.as_deref(), Some("sha256:deadbeef"));
        assert_eq!(events.len(), 2, "one progress event per layer");
        assert_eq!(
            store.get_cached(&kappa(WEIGHT_BYTES)).expect("cached"),
            Some(WEIGHT_BYTES.to_vec())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_shared_layer_already_present_is_not_fetched_again() {
        let (store, root) = store("dedup");
        store
            .cache_addressed(&kappa(WEIGHT_BYTES), WEIGHT_BYTES)
            .expect("seed the shared layer");
        let registry = FakeRegistry::new();

        let report = pull(&registry, &store, &reference(), &mut |_| {}).expect("pull");

        assert_eq!(report.layers_present, 1, "the seeded layer was reused");
        assert_eq!(report.layers_fetched, 1, "only the archive was transferred");
        assert!(
            !registry.requested.borrow().contains(&kappa(WEIGHT_BYTES)),
            "a cached layer must never be requested"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pulling_twice_transfers_nothing_the_second_time() {
        let (store, root) = store("resume");
        let registry = FakeRegistry::new();

        pull(&registry, &store, &reference(), &mut |_| {}).expect("first pull");
        let second = pull(&registry, &store, &reference(), &mut |_| {}).expect("second pull");

        assert_eq!(second.layers_fetched, 0, "pull is idempotent");
        assert_eq!(second.bytes_transferred, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_layer_whose_bytes_do_not_match_its_kappa_fails_the_pull() {
        let (store, root) = store("corrupt");
        let mut registry = FakeRegistry::new();
        // Serve the right address with the wrong bytes.
        registry
            .blobs
            .insert(kappa(WEIGHT_BYTES), b"tampered".to_vec());

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("corrupt content must not be accepted");
        assert!(
            matches!(error, LiveError::InvalidHolo(_) | LiveError::Protocol(_)),
            "expected a content-integrity error, got {error:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_manifest_referencing_an_absent_layer_fails_naming_it() {
        // The registry accepts manifests whose layers were never uploaded, so
        // the client is the only thing that can catch a dangling reference.
        let (store, root) = store("dangling");
        let mut registry = FakeRegistry::new();
        registry.blobs.remove(&kappa(WEIGHT_BYTES));

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("a dangling layer must fail the pull");
        let message = format!("{error}");
        assert!(
            message.contains(&kappa(WEIGHT_BYTES)),
            "the error must name the missing layer, got {message}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_reference_is_not_found() {
        let (store, root) = store("missing");
        let mut registry = FakeRegistry::new();
        registry.manifest = None;

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("an unresolvable reference must fail");
        assert!(matches!(error, LiveError::NotFound(_)), "got {error:?}");
        let _ = std::fs::remove_dir_all(root);
    }
}
