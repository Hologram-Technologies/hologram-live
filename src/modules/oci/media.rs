//! What a manifest body says, read without ever rewriting it.
//!
//! The bytes a client pushed are the bytes that are stored and hashed. This
//! file only reads them, to learn what the store must check: which blobs the
//! manifest needs, and whether it names a subject.

use super::error::{ErrorCode, OciError};
use crate::oci_store::{Digest, LinkKind, ManifestPlan, SubjectPlan};
use serde_json::{json, Map, Value};

pub const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub const DOCKER_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
pub const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
pub const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";

fn invalid(reason: &str) -> OciError {
    OciError::new(ErrorCode::ManifestInvalid).with_detail(json!(reason))
}

/// The media type to store the manifest under, and the plan for the store.
///
/// Accepted: Docker manifest v2 and list, OCI manifest and index, with any
/// `artifactType` and any layer media type. Schema 1 is refused. Without a
/// `Content-Type` the body's own `mediaType` decides, then its shape; what the
/// reference does there is gate B scenario `manifest-no-content-type`.
///
/// # Errors
///
/// `MANIFEST_INVALID`, with the reason in `detail`.
pub fn plan(content_type: Option<&str>, bytes: &[u8]) -> Result<(String, ManifestPlan), OciError> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| invalid("the body is not JSON"))?;
    let body = body
        .as_object()
        .ok_or_else(|| invalid("the body is not a JSON object"))?;
    if body.get("schemaVersion").and_then(Value::as_u64) != Some(2) {
        return Err(invalid("schemaVersion must be 2"));
    }
    // `Content-Type` decides, alone: the reference has no fallback to the
    // body's `mediaType` or to its shape (gate B, `manifest-put-invalid`).
    let sent = content_type
        .map(|value| value.split(';').next().unwrap_or_default().trim())
        .unwrap_or_default();
    let kind = match sent {
        DOCKER_MANIFEST | OCI_MANIFEST => LinkKind::Manifest,
        DOCKER_LIST | OCI_INDEX => LinkKind::Index,
        _ => {
            return Err(OciError::new(ErrorCode::ManifestInvalid).with_detail(json!(format!(
                "unsupported manifest media type and no default available: {sent}"
            ))));
        }
    };
    let media_type = sent;
    if body.get("mediaType").and_then(Value::as_str).is_some_and(|declared| declared != sent) {
        return Err(invalid("mediaType does not match Content-Type"));
    }

    let mut must_exist = Vec::new();
    if kind == LinkKind::Manifest {
        if let Some(config) = body.get("config") {
            must_exist.push(descriptor_digest(config)?);
        }
        for layer in array(body, "layers")? {
            // A foreign layer lives at its `urls`, not here.
            let foreign = layer
                .get("urls")
                .and_then(Value::as_array)
                .is_some_and(|urls| !urls.is_empty());
            let digest = descriptor_digest(layer)?;
            if !foreign {
                must_exist.push(digest);
            }
        }
    } else {
        // An index's children must be in this repository: the reference
        // refuses an index with a missing child (gate B, `index-missing-child`).
        for child in array(body, "manifests")? {
            must_exist.push(descriptor_digest(child)?);
        }
    }

    let subject = match body.get("subject") {
        None | Some(Value::Null) => None,
        Some(subject) => Some(SubjectPlan {
            subject: descriptor_digest(subject)?,
            artifact_type: artifact_type(body),
            annotations: body.get("annotations").and_then(Value::as_object).cloned(),
        }),
    };
    Ok((
        media_type.to_owned(),
        ManifestPlan {
            kind,
            must_exist,
            subject,
        },
    ))
}

/// `artifactType`, or for an image manifest without one, its config's media
/// type: what the referrers listing reports for it.
fn artifact_type(body: &Map<String, Value>) -> Option<String> {
    body.get("artifactType")
        .and_then(Value::as_str)
        .or_else(|| body.get("config")?.get("mediaType")?.as_str())
        .map(str::to_owned)
}

fn array<'a>(body: &'a Map<String, Value>, field: &str) -> Result<&'a [Value], OciError> {
    match body.get(field) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(invalid("a descriptor list is not an array")),
    }
}

fn descriptor_digest(descriptor: &Value) -> Result<Digest, OciError> {
    descriptor
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|text| Digest::parse(text).ok())
        .ok_or_else(|| invalid("a descriptor has no valid digest"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn digests(plan: &ManifestPlan) -> Vec<&str> {
        plan.must_exist.iter().map(Digest::as_str).collect()
    }

    #[test]
    fn an_image_manifest_needs_its_config_and_its_layers() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"{OCI_MANIFEST}","config":{{"digest":"{A}"}},"layers":[{{"digest":"{B}"}},{{"digest":"{C}"}}]}}"#
        );
        let (media_type, plan) = plan(Some(OCI_MANIFEST), body.as_bytes()).expect("plan");
        assert_eq!(media_type, OCI_MANIFEST);
        assert_eq!(plan.kind, LinkKind::Manifest);
        assert_eq!(digests(&plan), [A, B, C], "blake3 descriptors are accepted");
        assert!(plan.subject.is_none());
    }

    #[test]
    fn a_foreign_layer_is_not_required() {
        let body = format!(
            r#"{{"schemaVersion":2,"config":{{"digest":"{A}"}},"layers":[{{"digest":"{B}","urls":["https://example.com/l"]}}]}}"#
        );
        let (_, plan) = plan(Some(DOCKER_MANIFEST), body.as_bytes()).expect("plan");
        assert_eq!(digests(&plan), [A]);
    }

    #[test]
    fn an_index_requires_its_children() {
        let body = format!(r#"{{"schemaVersion":2,"manifests":[{{"digest":"{A}"}}]}}"#);
        for media_type in [OCI_INDEX, DOCKER_LIST] {
            let (_, plan) = plan(Some(media_type), body.as_bytes()).expect("plan");
            assert_eq!(plan.kind, LinkKind::Index);
            assert_eq!(digests(&plan), [A]);
        }
    }

    #[test]
    fn a_subject_makes_a_referrer_with_its_artifact_type() {
        let body = format!(
            r#"{{"schemaVersion":2,"artifactType":"application/vnd.example.sbom","config":{{"mediaType":"application/vnd.oci.empty.v1+json","digest":"{A}"}},"layers":[],"subject":{{"digest":"{B}"}},"annotations":{{"k":"v"}}}}"#
        );
        let (_, plan) = plan(Some(OCI_MANIFEST), body.as_bytes()).expect("plan");
        let subject = plan.subject.expect("subject");
        assert_eq!(subject.subject.as_str(), B);
        assert_eq!(
            subject.artifact_type.as_deref(),
            Some("application/vnd.example.sbom")
        );
        assert_eq!(subject.annotations.expect("annotations")["k"], "v");

        let body = format!(
            r#"{{"schemaVersion":2,"config":{{"mediaType":"application/vnd.example.config","digest":"{A}"}},"subject":{{"digest":"{B}"}}}}"#
        );
        let (_, plan) = super::plan(Some(OCI_MANIFEST), body.as_bytes()).expect("plan");
        assert_eq!(
            plan.subject.expect("subject").artifact_type.as_deref(),
            Some("application/vnd.example.config"),
            "without artifactType, the config's media type"
        );
    }

    #[test]
    fn the_media_type_comes_from_the_header_alone() {
        let bare = br#"{"schemaVersion":2,"layers":[]}"#;
        let with_parameter = format!("{OCI_MANIFEST}; charset=utf-8");
        assert_eq!(plan(Some(&with_parameter), bare).expect("plan").0, OCI_MANIFEST);
        for missing in [None, Some(""), Some("application/json")] {
            let error = plan(missing, bare).expect_err("no fallback");
            assert_eq!(error.code(), Some(ErrorCode::ManifestInvalid));
        }
    }

    #[test]
    fn what_is_refused() {
        let refused = |content_type: Option<&str>, body: &str| {
            let error = plan(content_type, body.as_bytes()).expect_err(body);
            assert_eq!(error.code(), Some(ErrorCode::ManifestInvalid), "{body}");
        };
        refused(Some(OCI_MANIFEST), "not json");
        refused(Some(OCI_MANIFEST), "[]");
        refused(Some(OCI_MANIFEST), r#"{"schemaVersion":1}"#);
        refused(
            Some("application/vnd.docker.distribution.manifest.v1+prettyjws"),
            r#"{"schemaVersion":2}"#,
        );
        refused(
            Some(OCI_MANIFEST),
            &format!(r#"{{"schemaVersion":2,"mediaType":"{OCI_INDEX}"}}"#),
        );
        refused(
            Some(OCI_MANIFEST),
            r#"{"schemaVersion":2,"layers":[{"digest":"sha256:short"}]}"#,
        );
        refused(Some(OCI_MANIFEST), r#"{"schemaVersion":2,"layers":{}}"#);
    }
}
