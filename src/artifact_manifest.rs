//! The named-artifact distribution format.
//!
//! One OCI manifest per artifact version. Its layers are a thin `.holo`
//! archive plus the kappa-addressed payload blobs that archive references, so
//! two artifacts sharing weights or a tokenizer transfer them once.
//!
//! A fat archive published as a single `archive` layer with no payloads is
//! valid and simpler; it forfeits deduplication.

use crate::error::{LiveError, Result};

const ANNOTATION_ROLE: &str = "dev.hologram.role";
const ANNOTATION_KIND: &str = "dev.hologram.kind";
const ANNOTATION_NAME: &str = "dev.hologram.name";
const ANNOTATION_TAG: &str = "dev.hologram.tag";

/// What a layer contributes to the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerRole {
    /// The `.holo` archive itself. Exactly one is required.
    Archive,
    /// A kappa-addressed payload the archive references.
    Layer,
}

#[derive(Debug, Clone)]
pub struct ArtifactLayer {
    pub kappa: String,
    pub media_type: String,
    pub size: u64,
    pub role: LayerRole,
}

#[derive(Debug, Clone)]
pub struct ArtifactManifest {
    pub layers: Vec<ArtifactLayer>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
}

impl ArtifactManifest {
    pub fn decode(body: &[u8]) -> Result<Self> {
        let document: serde_json::Value = serde_json::from_slice(body)?;
        let annotations = document.get("annotations").and_then(|v| v.as_object());
        let annotation = |key: &str| -> Option<String> {
            annotations
                .and_then(|map| map.get(key))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };

        let entries = document
            .get("layers")
            .and_then(|value| value.as_array())
            .ok_or_else(|| LiveError::Protocol("artifact manifest has no layers".to_owned()))?;

        let mut layers = Vec::with_capacity(entries.len());
        for entry in entries {
            let kappa = entry
                .get("digest")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    LiveError::Protocol("artifact manifest layer has no digest".to_owned())
                })?;
            // Identity is blake3 everywhere in this system. A foreign axis
            // would not match anything in the local store, so reject it at the
            // boundary rather than failing later with a confusing cache miss.
            if !is_blake3(kappa) {
                return Err(LiveError::Protocol(format!(
                    "artifact manifest layer digest {kappa:?} is not a blake3 kappa"
                )));
            }
            let role = match entry
                .get("annotations")
                .and_then(|value| value.get(ANNOTATION_ROLE))
                .and_then(|value| value.as_str())
            {
                Some("archive") => LayerRole::Archive,
                // An unannotated layer is a payload: the archive layer must
                // declare itself, so absence is unambiguous.
                _ => LayerRole::Layer,
            };
            layers.push(ArtifactLayer {
                kappa: kappa.to_owned(),
                media_type: entry
                    .get("mediaType")
                    .and_then(|value| value.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_owned(),
                size: entry
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default(),
                role,
            });
        }

        Ok(Self {
            layers,
            kind: annotation(ANNOTATION_KIND),
            name: annotation(ANNOTATION_NAME),
            tag: annotation(ANNOTATION_TAG),
        })
    }

    /// The single archive layer, or a typed error naming what was wrong.
    pub fn archive(&self) -> Result<&ArtifactLayer> {
        let mut found = self
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Archive);
        let archive = found.next().ok_or_else(|| {
            LiveError::Protocol("artifact manifest declares no archive layer".to_owned())
        })?;
        if found.next().is_some() {
            return Err(LiveError::Protocol(
                "artifact manifest declares more than one archive layer".to_owned(),
            ));
        }
        Ok(archive)
    }

    pub fn payloads(&self) -> impl Iterator<Item = &ArtifactLayer> {
        self.layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Layer)
    }
}

fn is_blake3(kappa: &str) -> bool {
    kappa.strip_prefix("blake3:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVE: &str = "blake3:5428a65f5815d145cd5f5b91f9d0d4c902b0376f268a41027e13bc1cf96f5429";
    const WEIGHTS: &str = "blake3:133f4a5d15cc232345f93adf581818b8910412c4b205e5f971faf93859037815";

    fn document() -> String {
        format!(
            r#"{{"schemaVersion":2,
                "mediaType":"application/vnd.oci.image.manifest.v1+json",
                "artifactType":"application/vnd.hologram.artifact.v1+json",
                "config":{{"mediaType":"application/vnd.oci.empty.v1+json","digest":"{ARCHIVE}","size":30}},
                "layers":[
                  {{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                    "annotations":{{"dev.hologram.role":"archive"}}}},
                  {{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30,
                    "annotations":{{"dev.hologram.role":"layer"}}}}],
                "annotations":{{"dev.hologram.kind":"inference-model","dev.hologram.name":"qwen3.5","dev.hologram.tag":"4b"}}}}"#
        )
    }

    #[test]
    fn a_multi_layer_manifest_decodes_into_roles() {
        let manifest = ArtifactManifest::decode(document().as_bytes()).expect("decode");

        assert_eq!(manifest.layers.len(), 2);
        assert_eq!(manifest.archive().expect("archive").kappa, ARCHIVE);
        let payloads: Vec<&ArtifactLayer> = manifest.payloads().collect();
        assert_eq!(payloads.len(), 1, "the archive is not a payload layer");
        assert_eq!(payloads[0].kappa, WEIGHTS);
        assert_eq!(manifest.kind.as_deref(), Some("inference-model"));
        assert_eq!(manifest.name.as_deref(), Some("qwen3.5"));
        assert_eq!(manifest.tag.as_deref(), Some("4b"));
    }

    #[test]
    fn a_single_fat_archive_layer_is_valid_and_has_no_payloads() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[{{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                   "annotations":{{"dev.hologram.role":"archive"}}}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert_eq!(manifest.archive().expect("archive").kappa, ARCHIVE);
        assert_eq!(manifest.payloads().count(), 0);
    }

    #[test]
    fn a_manifest_with_no_archive_layer_is_rejected() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[{{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30,
                   "annotations":{{"dev.hologram.role":"layer"}}}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert!(
            manifest.archive().is_err(),
            "payload layers alone are not a usable artifact"
        );
    }

    #[test]
    fn two_archive_layers_are_rejected() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[
                   {{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                     "annotations":{{"dev.hologram.role":"archive"}}}},
                   {{"mediaType":"application/vnd.hologram.holo","digest":"{WEIGHTS}","size":30,
                     "annotations":{{"dev.hologram.role":"archive"}}}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert!(
            manifest.archive().is_err(),
            "an ambiguous artifact must not silently pick one archive"
        );
    }

    #[test]
    fn a_layer_digest_on_a_foreign_axis_is_rejected() {
        // Object identity is blake3 throughout. A sha256 layer would silently
        // break local cache lookups, so it must fail at decode.
        let body = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
            "layers":[{"mediaType":"application/vnd.hologram.holo",
              "digest":"sha256:5ca494d6ec8cb393acefd75cf0f332f6fa3690ea2a1804e92e27373b95ea8373","size":30,
              "annotations":{"dev.hologram.role":"archive"}}]}"#;
        assert!(ArtifactManifest::decode(body.as_bytes()).is_err());
    }

    #[test]
    fn a_layer_without_a_role_annotation_defaults_to_payload() {
        // Upstream publishes manifests whose payload layers carry no role.
        // Treating an unannotated layer as a payload keeps those readable,
        // while the archive layer must still declare itself.
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[
                   {{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                     "annotations":{{"dev.hologram.role":"archive"}}}},
                   {{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert_eq!(manifest.payloads().count(), 1);
    }
}
