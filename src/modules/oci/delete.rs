//! Routes 7 and 10: remove a manifest, a tag or a blob from one repository.
//!
//! Delete removes links and tags. It never touches bytes: another repository
//! may link the same blob, and a pull that is under way holds its file open.
//! The bytes go at the next garbage collection.

use super::error::{Context, OciError};
use crate::oci_store::{Digest, OciStore, OciStoreError, Reference, RepoName};
use axum::body::Body;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use std::sync::Arc;

fn accepted() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::ACCEPTED;
    response
        .headers_mut()
        .insert(CONTENT_LENGTH, HeaderValue::from(0_u64));
    response
}

async fn blocking<F>(context: Context, call: F) -> Result<Response, OciError>
where
    F: FnOnce() -> Result<(), OciStoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|error| OciError::internal(&error))?
        .map_err(|error| OciError::from_store(error, context))?;
    Ok(accepted())
}

/// Route 7. By digest: the manifest and every tag that points at it. By tag:
/// the tag only; the manifest stays reachable by digest.
///
/// # Errors
///
/// `MANIFEST_UNKNOWN`.
pub async fn manifest(
    store: Arc<OciStore>,
    repo: RepoName,
    reference: Reference,
) -> Result<Response, OciError> {
    blocking(Context::Manifest, move || match &reference {
        Reference::Digest(digest) => store.manifest_delete(&repo, digest),
        Reference::Tag(tag) => store.tag_delete(&repo, tag),
    })
    .await
}

/// Route 10.
///
/// # Errors
///
/// `BLOB_UNKNOWN` when this repository does not link the blob.
pub async fn blob(
    store: Arc<OciStore>,
    repo: RepoName,
    digest: Digest,
) -> Result<Response, OciError> {
    blocking(Context::Blob, move || {
        // Asked by either name, the link is under the name it is stored by.
        let stored_as = store.blob_stat(&repo, &digest)?.stored_as;
        store.link_remove(&repo, &stored_as)?;
        if stored_as != digest {
            store.link_remove(&repo, &digest)?;
        }
        Ok(())
    })
    .await
}
