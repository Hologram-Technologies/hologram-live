//! The named-artifact distribution format.
//!
//! One OCI manifest per artifact version. Its layers are a thin `.holo`
//! archive plus the kappa-addressed payload blobs that archive references, so
//! two artifacts sharing weights or a tokenizer transfer them once.
//!
//! A fat archive published as a single `archive` layer with no payloads is
//! valid and simpler; it forfeits deduplication.

use crate::error::{LiveError, Result};

const ARTIFACT_TYPE: &str = "application/vnd.hologram.artifact.v1+json";
const ANNOTATION_ROLE: &str = "dev.hologram.role";
const ANNOTATION_KIND: &str = "dev.hologram.kind";
const ANNOTATION_NAME: &str = "dev.hologram.name";
const ANNOTATION_TAG: &str = "dev.hologram.tag";
/// A layer named by sha256 on the wire keeps its kappa here, so a pull can
/// find it in the local store and a Docker client can fetch it at all.
const ANNOTATION_KAPPA: &str = "dev.hologram.kappa";
/// The empty JSON object every OCI artifact uses as its config when it has
/// none of its own, and its digest and size. Naming the archive here instead
/// gave the config and layer 0 one digest, and a client resolving the
/// descriptor by digest then read the config's media type for the layer:
/// `crane validate` reported a mismatch on every artifact we published.
pub const EMPTY_CONFIG: &[u8] = b"{}";
pub const EMPTY_CONFIG_DIGEST: &str =
    "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
const EMPTY_CONFIG_TYPE: &str = "application/vnd.oci.empty.v1+json";

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
    /// The same bytes named by SHA-256, when known. It is what the
    /// manifest carries as `digest`: `docker`, `crane`, `containerd` and
    /// `skopeo` accept sha256 and sha512 and nothing else, and the registry
    /// serves one blob under both names (ADR 030). `None` for a manifest
    /// published before this field existed.
    pub sha256: Option<String>,
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
            let digest = entry
                .get("digest")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    LiveError::Protocol("artifact manifest layer has no digest".to_owned())
                })?;
            let layer_annotation = |key: &str| -> Option<&str> {
                entry
                    .get("annotations")
                    .and_then(|value| value.get(key))
                    .and_then(|value| value.as_str())
            };
            // Identity is blake3 everywhere in this system. A manifest may
            // name a layer by sha256 on the wire, for clients that speak
            // nothing else, but then it must say which kappa that is: a
            // foreign axis alone would match nothing in the local store, so
            // reject it at the boundary rather than fail later with a
            // confusing cache miss.
            let (kappa, sha256) = if is_blake3(digest) {
                (digest.to_owned(), None)
            } else if is_sha256(digest) {
                match layer_annotation(ANNOTATION_KAPPA) {
                    Some(kappa) if is_blake3(kappa) => (kappa.to_owned(), Some(digest.to_owned())),
                    _ => {
                        return Err(LiveError::Protocol(format!(
                            "artifact manifest layer {digest:?} names no blake3 kappa in {ANNOTATION_KAPPA}"
                        )))
                    }
                }
            } else {
                return Err(LiveError::Protocol(format!(
                    "artifact manifest layer digest {digest:?} is neither a blake3 kappa nor sha256"
                )));
            };
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
                kappa,
                sha256,
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

    /// Encode this manifest for publication.
    ///
    /// The inverse of [`decode`](Self::decode). Roles become explicit
    /// annotations on every layer, including payloads: `decode` tolerates an
    /// unannotated layer for compatibility, but anything this code publishes
    /// says what it is.
    pub fn encode(&self) -> Result<Vec<u8>> {
        // Still checked, and still the reason a manifest with no archive cannot
        // be published, even though the config no longer names it.
        self.archive()?;

        let layers: Vec<serde_json::Value> = self
            .layers
            .iter()
            .map(|layer| {
                let role = match layer.role {
                    LayerRole::Archive => "archive",
                    LayerRole::Layer => "layer",
                };
                // On the wire the layer is named by sha256 when that is
                // known, so every OCI client can fetch it; the kappa rides in
                // an annotation so a pull still finds it in the local store.
                let mut annotations = serde_json::json!({ ANNOTATION_ROLE: role });
                if layer.sha256.is_some() {
                    annotations[ANNOTATION_KAPPA] = serde_json::Value::String(layer.kappa.clone());
                }
                serde_json::json!({
                    "mediaType": layer.media_type,
                    "digest": layer.wire_digest(),
                    "size": layer.size,
                    "annotations": annotations,
                })
            })
            .collect();

        let mut annotations = serde_json::Map::new();
        for (key, value) in [
            (ANNOTATION_KIND, self.kind.as_ref()),
            (ANNOTATION_NAME, self.name.as_ref()),
            (ANNOTATION_TAG, self.tag.as_ref()),
        ] {
            if let Some(value) = value {
                annotations.insert(key.to_owned(), serde_json::Value::String(value.clone()));
            }
        }

        let document = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": ARTIFACT_TYPE,
            "config": {
                "mediaType": EMPTY_CONFIG_TYPE,
                "digest": EMPTY_CONFIG_DIGEST,
                "size": EMPTY_CONFIG.len(),
            },
            "layers": layers,
            "annotations": serde_json::Value::Object(annotations),
        });
        serde_json::to_vec(&document).map_err(Into::into)
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

impl ArtifactLayer {
    /// The name the manifest carries: sha256 when known, else the kappa.
    pub fn wire_digest(&self) -> &str {
        self.sha256.as_deref().unwrap_or(&self.kappa)
    }
}

fn is_sha256(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
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
    fn a_manifest_round_trips_through_encode_and_decode() {
        let original = ArtifactManifest::decode(document().as_bytes()).expect("decode");
        let reencoded = original.encode().expect("encode");
        let decoded = ArtifactManifest::decode(&reencoded).expect("decode again");

        assert_eq!(decoded.layers.len(), original.layers.len());
        assert_eq!(decoded.archive().expect("archive").kappa, ARCHIVE);
        assert_eq!(decoded.payloads().count(), 1);
        assert_eq!(decoded.kind, original.kind);
        assert_eq!(decoded.name, original.name);
        assert_eq!(decoded.tag, original.tag);
        for (before, after) in original.layers.iter().zip(decoded.layers.iter()) {
            assert_eq!(before.kappa, after.kappa);
            assert_eq!(before.media_type, after.media_type);
            assert_eq!(before.size, after.size);
            assert_eq!(before.role, after.role, "roles must survive the round trip");
        }
    }

    #[test]
    fn encoding_marks_every_layer_role_explicitly() {
        // decode tolerates an unannotated payload for compatibility with
        // manifests published elsewhere. Anything we publish should still say
        // what each layer is, so a reader never has to rely on that default.
        let manifest = ArtifactManifest::decode(document().as_bytes()).expect("decode");
        let encoded = manifest.encode().expect("encode");
        let value: serde_json::Value = serde_json::from_slice(&encoded).expect("json");

        let layers = value["layers"].as_array().expect("layers");
        assert_eq!(layers.len(), 2);
        for layer in layers {
            assert!(
                layer["annotations"][ANNOTATION_ROLE].is_string(),
                "every published layer declares its role: {layer}"
            );
        }
        assert_eq!(
            value["artifactType"].as_str(),
            Some("application/vnd.hologram.artifact.v1+json")
        );
    }

    #[test]
    fn a_manifest_with_no_archive_layer_cannot_be_encoded() {
        // Publishing an artifact with no archive would create exactly the
        // dangling manifest the pull path exists to reject.
        let manifest = ArtifactManifest {
            layers: vec![ArtifactLayer {
                kappa: WEIGHTS.to_owned(),
                sha256: None,
                media_type: "application/octet-stream".to_owned(),
                size: 30,
                role: LayerRole::Layer,
            }],
            kind: None,
            name: None,
            tag: None,
        };
        assert!(manifest.encode().is_err());
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
    const ARCHIVE_SHA: &str =
        "sha256:2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";
    const WEIGHTS_SHA: &str =
        "sha256:fcde2b2edba56bf408601fb721fe9b5c338d10ee429ea04fae5511b68fbf8fb9";

    #[test]
    fn a_layer_with_a_sha256_is_named_by_it_on_the_wire_and_keeps_its_kappa() {
        let manifest = ArtifactManifest {
            layers: vec![
                ArtifactLayer {
                    kappa: ARCHIVE.to_owned(),
                    sha256: Some(ARCHIVE_SHA.to_owned()),
                    media_type: "application/vnd.hologram.holo".to_owned(),
                    size: 30,
                    role: LayerRole::Archive,
                },
                ArtifactLayer {
                    kappa: WEIGHTS.to_owned(),
                    sha256: Some(WEIGHTS_SHA.to_owned()),
                    media_type: "application/octet-stream".to_owned(),
                    size: 30,
                    role: LayerRole::Layer,
                },
            ],
            kind: Some("holo".to_owned()),
            name: Some("demo".to_owned()),
            tag: Some("v1".to_owned()),
        };
        let encoded = manifest.encode().expect("encode");
        let value: serde_json::Value = serde_json::from_slice(&encoded).expect("json");

        // What a Docker client sees: sha256 everywhere, and a config that is the
        // empty object rather than the archive under another name.
        assert_eq!(
            value["config"]["digest"].as_str(),
            Some(EMPTY_CONFIG_DIGEST)
        );
        for layer in value["layers"].as_array().expect("layers") {
            assert!(
                layer["digest"].as_str().unwrap().starts_with("sha256:"),
                "{layer}"
            );
            assert!(layer["annotations"][ANNOTATION_KAPPA]
                .as_str()
                .unwrap()
                .starts_with("blake3:"));
        }

        // What a pull sees: the kappas, unchanged.
        let decoded = ArtifactManifest::decode(&encoded).expect("decode");
        assert_eq!(decoded.archive().expect("archive").kappa, ARCHIVE);
        assert_eq!(
            decoded.archive().expect("archive").sha256.as_deref(),
            Some(ARCHIVE_SHA)
        );
        let payloads: Vec<&ArtifactLayer> = decoded.payloads().collect();
        assert_eq!(payloads[0].kappa, WEIGHTS);
        assert_eq!(payloads[0].sha256.as_deref(), Some(WEIGHTS_SHA));
    }

    #[test]
    fn a_sha256_layer_without_its_kappa_is_rejected() {
        // A foreign name alone would match nothing in the local store.
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[{{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE_SHA}","size":30,
                   "annotations":{{"dev.hologram.role":"archive"}}}}]}}"#
        );
        let error = ArtifactManifest::decode(body.as_bytes()).expect_err("no kappa");
        assert!(error.to_string().contains(ANNOTATION_KAPPA), "{error}");
    }

    #[test]
    fn the_config_is_the_empty_object_and_never_the_archive() {
        // One digest for both the config and layer 0 made a client resolving
        // the descriptor read the config's media type for the layer, and
        // `crane validate` refused every artifact we published.
        let manifest = ArtifactManifest::decode(document().as_bytes()).expect("decode");
        let value: serde_json::Value =
            serde_json::from_slice(&manifest.encode().expect("encode")).expect("json");

        assert_eq!(
            value["config"]["digest"].as_str(),
            Some(EMPTY_CONFIG_DIGEST)
        );
        assert_eq!(value["config"]["size"].as_u64(), Some(2));
        assert_eq!(
            value["config"]["mediaType"].as_str(),
            Some(EMPTY_CONFIG_TYPE)
        );
        let archive = value["layers"][0]["digest"].as_str().expect("layer 0");
        assert_ne!(
            value["config"]["digest"].as_str(),
            Some(archive),
            "the config and a layer must not share a digest"
        );
        assert_eq!(
            EMPTY_CONFIG_DIGEST,
            format!(
                "sha256:{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(EMPTY_CONFIG)
            ),
            "the constant must be the digest of the bytes push uploads"
        );
    }

    #[test]
    fn a_manifest_published_before_sha256_still_decodes() {
        // Every layer named by blake3 alone, as the hub's daily index was.
        let manifest = ArtifactManifest::decode(document().as_bytes()).expect("decode");
        for layer in &manifest.layers {
            assert!(layer.sha256.is_none());
            assert_eq!(layer.wire_digest(), layer.kappa);
        }
    }
}
