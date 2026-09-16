//! Publishing a local `.holo` archive to a registry under a name.
//!
//! The inverse of [`crate::artifact_pull`]. The archive becomes the manifest's
//! `archive` layer, and every payload the archive references without embedding
//! becomes a `layer` — which is what lets a later pull deduplicate against
//! blobs the registry already holds.
//!
//! Tags upstream are mutable: a re-`PUT` silently moves a tag to different
//! content. Push therefore refuses to move an existing tag unless told to,
//! because silently replacing what a name points at is not a thing a publish
//! command should do by default.

use crate::artifact_manifest::{ArtifactLayer, ArtifactManifest, LayerRole};
use crate::artifact_ref::ArtifactRef;
use crate::error::{LiveError, Result};
use crate::protocol::HoloInspection;
use crate::store::ObjectStore;
use std::collections::BTreeSet;

const HOLO_MEDIA_TYPE: &str = "application/vnd.hologram.holo";
const LAYER_MEDIA_TYPE: &str = "application/octet-stream";

/// The machine-readable result of a completed push.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PushReport {
    pub reference: String,
    pub archive_kappa: String,
    pub layers_published: usize,
    pub bytes_transferred: u64,
    /// True when an existing tag was moved to this artifact.
    pub replaced_existing_tag: bool,
}

/// One layer as it is published.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PushProgress {
    pub kappa: String,
    pub index: usize,
    pub total: usize,
    pub bytes: u64,
}

/// The registry operations a push needs.
///
/// Narrower than the full client so the push can be tested against a fake
/// without a running registry.
pub trait LayerPublish {
    fn put_blob(&self, repository: &str, kappa: &str, media_type: &str, bytes: &[u8])
        -> Result<()>;
    fn put_manifest(&self, repository: &str, tag: &str, body: &[u8]) -> Result<()>;
    /// Whether the tag already resolves, so an overwrite can be refused.
    fn tag_exists(&self, repository: &str, tag: &str) -> Result<bool>;
}

/// Payload kappas the archive references but does not embed.
///
/// A fat archive embeds everything, so this is empty and the manifest carries
/// one layer. A thin archive references its payloads by kappa, and each must be
/// published alongside or a later pull resolves an archive whose content is
/// missing.
fn external_payloads(inspection: &HoloInspection) -> BTreeSet<String> {
    let Some(directory) = inspection.directory.as_ref() else {
        return BTreeSet::new();
    };
    let embedded: BTreeSet<&str> = directory
        .blobs
        .iter()
        .map(|blob| blob.kappa.as_str())
        .collect();
    directory
        .layers
        .iter()
        .map(|layer| layer.content_kappa.as_str())
        .filter(|kappa| !embedded.contains(kappa))
        .map(str::to_owned)
        .collect()
}

pub fn push(
    publish: &dyn LayerPublish,
    store: &ObjectStore,
    reference: &ArtifactRef,
    archive_bytes: &[u8],
    inspection: &HoloInspection,
    force: bool,
    on_progress: &mut dyn FnMut(PushProgress),
) -> Result<PushReport> {
    let repository = reference.repository();

    // Checked before anything is written, so a refused push leaves the
    // registry untouched rather than half-populated.
    let replaced_existing_tag = publish.tag_exists(&repository, &reference.tag)?;
    if replaced_existing_tag && !force {
        return Err(LiveError::Conflict(format!(
            "{} already exists; pass --force to move the tag to this artifact",
            reference.display()
        )));
    }

    let archive_kappa = format!("blake3:{}", blake3::hash(archive_bytes).to_hex());
    let external = external_payloads(inspection);

    // Gather every payload before publishing anything, so a thin archive with
    // a missing payload fails before the registry has been written to.
    let mut payloads = Vec::with_capacity(external.len());
    for kappa in &external {
        let bytes = store.get_cached(kappa)?.ok_or_else(|| {
            LiveError::NotFound(format!(
                "archive references payload {kappa} which is not in the local store; \
                 import or compile the archive fat before publishing it"
            ))
        })?;
        payloads.push((kappa.clone(), bytes));
    }

    let total = payloads.len() + 1;
    let mut bytes_transferred = 0_u64;

    publish.put_blob(&repository, &archive_kappa, HOLO_MEDIA_TYPE, archive_bytes)?;
    let archive_size = archive_bytes.len().try_into().unwrap_or(u64::MAX);
    bytes_transferred = bytes_transferred.saturating_add(archive_size);
    on_progress(PushProgress {
        kappa: archive_kappa.clone(),
        index: 0,
        total,
        bytes: archive_size,
    });

    let mut layers = vec![ArtifactLayer {
        kappa: archive_kappa.clone(),
        media_type: HOLO_MEDIA_TYPE.to_owned(),
        size: archive_size,
        role: LayerRole::Archive,
    }];

    for (index, (kappa, bytes)) in payloads.iter().enumerate() {
        publish.put_blob(&repository, kappa, LAYER_MEDIA_TYPE, bytes)?;
        let size = bytes.len().try_into().unwrap_or(u64::MAX);
        bytes_transferred = bytes_transferred.saturating_add(size);
        layers.push(ArtifactLayer {
            kappa: kappa.clone(),
            media_type: LAYER_MEDIA_TYPE.to_owned(),
            size,
            role: LayerRole::Layer,
        });
        on_progress(PushProgress {
            kappa: kappa.clone(),
            index: index + 1,
            total,
            bytes: size,
        });
    }

    let layers_published = layers.len();
    let manifest = ArtifactManifest {
        layers,
        kind: Some("holo".to_owned()),
        name: Some(reference.name.clone()),
        tag: Some(reference.tag.clone()),
    };
    // The manifest goes last: until it exists the tag resolves to nothing, so
    // an interrupted push leaves unreferenced blobs rather than a tag pointing
    // at an incomplete artifact.
    publish.put_manifest(&repository, &reference.tag, &manifest.encode()?)?;

    Ok(PushReport {
        reference: reference.display(),
        archive_kappa,
        layers_published,
        bytes_transferred,
        replaced_existing_tag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;
    use crate::protocol::{HoloBlob, HoloDirectory, HoloLayer};
    use std::cell::RefCell;

    const ARCHIVE_BYTES: &[u8] = b"THIN-HOLO-ARCHIVE";
    const PAYLOAD_BYTES: &[u8] = b"PAYLOAD-CONTENT";

    fn kappa(bytes: &[u8]) -> String {
        format!("blake3:{}", blake3::hash(bytes).to_hex())
    }

    /// Records the exact order of registry writes, so a test can assert the
    /// manifest lands last.
    #[derive(Default)]
    struct FakeRegistry {
        writes: RefCell<Vec<String>>,
        existing_tag: bool,
    }

    impl LayerPublish for FakeRegistry {
        fn put_blob(
            &self,
            _repository: &str,
            kappa: &str,
            _media_type: &str,
            _bytes: &[u8],
        ) -> Result<()> {
            self.writes.borrow_mut().push(format!("blob:{kappa}"));
            Ok(())
        }

        fn put_manifest(&self, _repository: &str, tag: &str, _body: &[u8]) -> Result<()> {
            self.writes.borrow_mut().push(format!("manifest:{tag}"));
            Ok(())
        }

        fn tag_exists(&self, _repository: &str, _tag: &str) -> Result<bool> {
            Ok(self.existing_tag)
        }
    }

    fn store(label: &str) -> (ObjectStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hologram-push-{label}-{}-{}",
            std::process::id(),
            crate::util::now_millis()
        ));
        (ObjectStore::open(&root).expect("open"), root)
    }

    fn reference() -> ArtifactRef {
        ArtifactRef::parse("demo:v1", &RegistryConfig::default()).expect("parse")
    }

    /// An archive whose single layer is embedded: nothing external to publish.
    fn fat_inspection() -> HoloInspection {
        inspection(vec![HoloBlob {
            kappa: kappa(PAYLOAD_BYTES),
            byte_length: 15,
        }])
    }

    /// An archive that references its payload without embedding it.
    fn thin_inspection() -> HoloInspection {
        inspection(Vec::new())
    }

    fn inspection(blobs: Vec<HoloBlob>) -> HoloInspection {
        HoloInspection {
            kappa: kappa(ARCHIVE_BYTES),
            application_kappa: None,
            name: "demo".to_owned(),
            format_version: 4,
            byte_length: 17,
            archive_fingerprint: String::new(),
            footer_verified: true,
            sections: Vec::new(),
            directory: Some(HoloDirectory {
                schema_version: 1,
                primary_layer: Some(0),
                requires_kappa: String::new(),
                layers: vec![HoloLayer {
                    position: 0,
                    kind: "wasm".to_owned(),
                    content_kappa: kappa(PAYLOAD_BYTES),
                    entry: "main".to_owned(),
                    contract: None,
                    architecture: None,
                    surface: None,
                    engine: None,
                }],
                children: Vec::new(),
                blobs,
            }),
            directory_embedded: true,
        }
    }

    #[test]
    fn a_fat_archive_publishes_one_layer() {
        let (store, root) = store("fat");
        let registry = FakeRegistry::default();

        let report = push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &fat_inspection(),
            false,
            &mut |_| {},
        )
        .expect("push");

        assert_eq!(
            report.layers_published, 1,
            "an embedded payload needs no separate layer"
        );
        assert_eq!(report.archive_kappa, kappa(ARCHIVE_BYTES));
        assert!(!report.replaced_existing_tag);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_thin_archive_publishes_its_referenced_payload() {
        let (store, root) = store("thin");
        store
            .cache_addressed(&kappa(PAYLOAD_BYTES), PAYLOAD_BYTES)
            .expect("seed payload");
        let registry = FakeRegistry::default();

        let report = push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &thin_inspection(),
            false,
            &mut |_| {},
        )
        .expect("push");

        assert_eq!(
            report.layers_published, 2,
            "the archive plus the payload it references"
        );
        assert_eq!(report.bytes_transferred, 32);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_manifest_is_written_after_every_blob() {
        // Until the manifest exists the tag resolves to nothing, so an
        // interrupted push leaves unreferenced blobs rather than a tag
        // pointing at an artifact whose content is missing.
        let (store, root) = store("order");
        store
            .cache_addressed(&kappa(PAYLOAD_BYTES), PAYLOAD_BYTES)
            .expect("seed payload");
        let registry = FakeRegistry::default();

        push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &thin_inspection(),
            false,
            &mut |_| {},
        )
        .expect("push");

        let writes = registry.writes.borrow();
        assert_eq!(writes.len(), 3);
        assert!(
            writes[..2].iter().all(|write| write.starts_with("blob:")),
            "blobs first: {writes:?}"
        );
        assert!(
            writes[2].starts_with("manifest:"),
            "manifest last: {writes:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_thin_archive_missing_its_payload_is_refused_before_any_write() {
        // Publishing this would create exactly the dangling manifest the pull
        // path exists to reject, so it fails before the registry is touched.
        let (store, root) = store("missing");
        let registry = FakeRegistry::default();

        let error = push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &thin_inspection(),
            false,
            &mut |_| {},
        )
        .expect_err("a missing payload must refuse the push");

        assert!(
            format!("{error}").contains(&kappa(PAYLOAD_BYTES)),
            "the error names the missing payload: {error}"
        );
        assert!(
            registry.writes.borrow().is_empty(),
            "a refused push must not write anything"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_tag_is_not_moved_without_force() {
        // Tags upstream are mutable, so a re-PUT silently repoints a name.
        // Requiring --force is what stops a publish from quietly replacing
        // what someone else is already pulling.
        let (store, root) = store("exists");
        let registry = FakeRegistry {
            existing_tag: true,
            ..FakeRegistry::default()
        };

        let error = push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &fat_inspection(),
            false,
            &mut |_| {},
        )
        .expect_err("an existing tag must not move silently");

        assert!(matches!(error, LiveError::Conflict(_)), "got {error:?}");
        assert!(
            registry.writes.borrow().is_empty(),
            "a refused push must not write anything"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn force_moves_an_existing_tag_and_says_so() {
        let (store, root) = store("force");
        let registry = FakeRegistry {
            existing_tag: true,
            ..FakeRegistry::default()
        };

        let report = push(
            &registry,
            &store,
            &reference(),
            ARCHIVE_BYTES,
            &fat_inspection(),
            true,
            &mut |_| {},
        )
        .expect("force push");

        assert!(
            report.replaced_existing_tag,
            "the report must record that a tag was moved"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
