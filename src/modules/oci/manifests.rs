//! Routes 4, 5 and 6: a manifest by tag or by digest, out and in.

use super::error::{Context, ErrorCode, OciError};
use super::media;
use super::respond::{not_modified, stamp_digest};
use crate::oci_store::{OciStore, OciStoreError, Reference, RepoName};
use axum::body::Body;
use axum::http::header::{HeaderName, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use std::sync::Arc;
use tokio_stream::StreamExt;

/// The largest manifest taken. The only body this module ever holds whole.
const MANIFEST_MAX: usize = 4 * 1024 * 1024;

/// Serve what is stored, under the media type it was pushed with.
///
/// A tag is resolved to a digest once, inside the store, and that digest is
/// what is read: the answer is one whole manifest, the old or the new, never a
/// mix, however the tag moves meanwhile. What `Accept` changes is gate B
/// scenario `manifest-accept`; until it is recorded, `Accept` changes nothing.
///
/// # Errors
///
/// `MANIFEST_UNKNOWN` when the tag, the digest or the repository is not known;
/// `UNKNOWN` when the store fails.
pub async fn get(
    store: Arc<OciStore>,
    repo: RepoName,
    reference: Reference,
    headers: &HeaderMap,
    head: bool,
) -> Result<Response, OciError> {
    let asked = reference.clone();
    let name = repo.clone();
    let manifest = tokio::task::spawn_blocking(move || store.manifest_get(&repo, &reference))
        .await
        .map_err(|error| OciError::internal(&error))?
        .map_err(|error| match error {
            // In the reference's words (gate B, `errors-read`).
            OciStoreError::NotInRepository { .. } | OciStoreError::UnknownRepository(_) => {
                let detail = match &asked {
                    Reference::Tag(tag) => format!("unknown tag={tag}"),
                    Reference::Digest(digest) => format!("unknown manifest name={name} revision={digest}"),
                };
                OciError::new(ErrorCode::ManifestUnknown).with_detail(serde_json::json!(detail))
            }
            other => OciError::from_store(other, Context::Manifest),
        })?;
    // A conditional request is answered first, whatever `Accept` says (gate
    // B, `manifest-read`): a proxy polling a tag it holds sends no `Accept`.
    if not_modified(headers, &manifest.digest) {
        // Bare, as the reference answers it (gate B, `manifest-read`).
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        return Ok(response);
    }
    // The reference serves an OCI manifest or index only to a client whose
    // `Accept` names that type; `*/*` is not enough (gate B, `manifest-read`).
    // Docker types are served to anyone.
    let refusal = match manifest.media_type.as_str() {
        media::OCI_MANIFEST => Some("OCI manifest found, but accept header does not support OCI manifests"),
        media::OCI_INDEX => Some("OCI index found, but accept header does not support OCI indexes"),
        _ => None,
    };
    if let Some(message) = refusal {
        let accepted = headers
            .get_all(axum::http::header::ACCEPT)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .any(|entry| entry.split(';').next().unwrap_or_default().trim() == manifest.media_type);
        if !accepted {
            return Err(OciError::new(ErrorCode::ManifestUnknown).with_message(message));
        }
    }

    let mut response = Response::new(Body::empty());
    let out = response.headers_mut();
    stamp_digest(out, &manifest.digest);
    out.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&manifest.media_type).map_err(|error| OciError::internal(&error))?,
    );
    out.insert(CONTENT_LENGTH, HeaderValue::from(manifest.bytes.len()));
    if !head {
        *response.body_mut() = Body::from(manifest.bytes);
    }
    Ok(response)
}

/// Route 6: store a manifest, byte for byte as it was sent.
///
/// # Errors
///
/// `MANIFEST_INVALID` (also over 4 MiB); `DIGEST_INVALID` when the path names a
/// digest the body does not hash to; `MANIFEST_BLOB_UNKNOWN` with the missing
/// digests in `detail`.
pub async fn put(
    store: Arc<OciStore>,
    repo: RepoName,
    reference: Reference,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, OciError> {
    let bytes = whole(body).await?;
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let (media_type, plan) = media::plan(content_type, &bytes)?;
    let subject = plan.subject.as_ref().map(|plan| plan.subject.clone());
    let target = repo.clone();
    let digest = tokio::task::spawn_blocking(move || {
        store.manifest_put(&target, &reference, &media_type, &bytes, &plan)
    })
    .await
    .map_err(|error| OciError::internal(&error))?
    .map_err(|error| OciError::from_store(error, Context::Manifest))?;

    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::CREATED;
    let out = response.headers_mut();
    stamp_digest(out, &digest);
    out.remove(axum::http::header::ETAG);
    out.insert(
        LOCATION,
        HeaderValue::from_str(&format!("/v2/{repo}/manifests/{digest}"))
            .map_err(|error| OciError::internal(&error))?,
    );
    if let Some(subject) = subject {
        // Tells the client the referrers listing is kept here, so it need not
        // maintain the fallback tag.
        if let Ok(value) = HeaderValue::from_str(subject.as_str()) {
            out.insert(HeaderName::from_static("oci-subject"), value);
        }
    }
    out.insert(CONTENT_LENGTH, HeaderValue::from(0_u64));
    Ok(response)
}

/// Read a manifest body, refusing it as soon as it passes the cap.
async fn whole(body: Body) -> Result<Vec<u8>, OciError> {
    let mut stream = body.into_data_stream();
    let mut bytes = Vec::new();
    while let Some(piece) = stream.next().await {
        let piece = piece.map_err(|_| OciError::new(ErrorCode::ManifestInvalid))?;
        if bytes.len() + piece.len() > MANIFEST_MAX {
            // 400, and in these words, as the reference answers it; the OCI
            // text suggests 413 (gate B, `manifest-put-invalid`).
            return Err(OciError::new(ErrorCode::ManifestInvalid)
                .with_detail(serde_json::json!("http: request body too large")));
        }
        bytes.extend_from_slice(&piece);
    }
    Ok(bytes)
}
