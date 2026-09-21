//! Routes 11 to 16: a blob in, in one request or in many, or mounted from
//! another repository without moving a byte. No layer is ever in memory.

use super::body::stream_into;
use super::error::{Context, ErrorCode, OciError};
use super::path::query_param;
use super::respond::stamp_digest;
use crate::oci_store::{Digest, LinkKind, OciStore, OciStoreError, RepoName, UploadId};
use axum::body::Body;
use axum::http::header::{HeaderName, CONTENT_LENGTH, CONTENT_RANGE, LOCATION, RANGE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use std::sync::Arc;

const UPLOAD_UUID: &str = "docker-upload-uuid";

/// Run a store call off the runtime. A panic in the store is a 500.
async fn blocking<T, F>(call: F) -> Result<T, OciError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, OciStoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|error| OciError::internal(&error))?
        .map_err(|error| OciError::from_store(error, Context::Upload))
}

fn value(text: &str) -> Result<HeaderValue, OciError> {
    HeaderValue::from_str(text).map_err(|error| OciError::internal(&error))
}

/// `202`, or `204` for the status route: where the session is and how far.
fn session_response(
    status: StatusCode,
    repo: &RepoName,
    id: &UploadId,
    received: u64,
) -> Result<Response, OciError> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(LOCATION, value(&format!("/v2/{repo}/blobs/uploads/{id}"))?);
    // No `bytes=` prefix here, and an empty upload is `0-0`: the protocol's
    // own form, not HTTP's.
    headers.insert(RANGE, value(&format!("0-{}", received.saturating_sub(1)))?);
    headers.insert(HeaderName::from_static(UPLOAD_UUID), value(id.as_str())?);
    headers.insert(CONTENT_LENGTH, HeaderValue::from(0_u64));
    Ok(response)
}

/// `201`: the blob exists in `repo` under `digest`.
fn created(repo: &RepoName, digest: &Digest) -> Result<Response, OciError> {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::CREATED;
    let headers = response.headers_mut();
    headers.insert(LOCATION, value(&format!("/v2/{repo}/blobs/{digest}"))?);
    stamp_digest(headers, digest);
    headers.remove(axum::http::header::ETAG);
    headers.insert(CONTENT_LENGTH, HeaderValue::from(0_u64));
    Ok(response)
}

fn digest_param(query: Option<&str>, name: &str) -> Result<Option<Digest>, OciError> {
    query_param(query, name)
        .map(|text| {
            // In the reference's words for a query value (gate B, `push-chunked`).
            Digest::parse(&text).map_err(|_| {
                OciError::new(ErrorCode::DigestInvalid)
                    .with_detail(serde_json::json!("digest parsing failed"))
            })
        })
        .transpose()
}

/// The session `id`, which must belong to `repo`: an upload id is not a way
/// into another repository.
async fn session_of(
    store: &Arc<OciStore>,
    repo: &RepoName,
    id: &UploadId,
) -> Result<u64, OciError> {
    let (store, id) = (store.clone(), id.clone());
    let status = blocking(move || store.upload_status(&id)).await?;
    if status.repo != *repo {
        return Err(OciError::new(ErrorCode::BlobUploadUnknown));
    }
    Ok(status.received)
}

/// Routes 11, 11b and 12: `POST …/blobs/uploads/`.
///
/// # Errors
///
/// `DIGEST_INVALID` for a malformed or mismatched digest; `UNKNOWN`.
pub async fn start(
    store: Arc<OciStore>,
    repo: RepoName,
    query: Option<&str>,
    body: Body,
) -> Result<Response, OciError> {
    // A `mount` that is not a digest is ignored, as the reference ignores it
    // (gate B, `mount`): the client gets an ordinary session.
    if let Some(digest) = digest_param(query, "mount").ok().flatten() {
        let from = query_param(query, "from").and_then(|name| RepoName::parse(&name).ok());
        if let Some(from) = from {
            if let Some(response) = mount(&store, &repo, &from, &digest).await? {
                return Ok(response);
            }
        }
        // No source, or the source does not link it: an ordinary session. A
        // mount never succeeds on a digest alone, or any blob could be read
        // by guessing its name.
    }
    let claimed = digest_param(query, "digest")?;
    let id = {
        let (store, repo) = (store.clone(), repo.clone());
        blocking(move || store.upload_begin(&repo)).await?
    };
    match claimed {
        // Monolithic: the whole blob in this request, still streamed.
        Some(claimed) => {
            stream_into(&store, &id, 0, body).await?;
            finish(store, &repo, id, claimed).await
        }
        None => session_response(StatusCode::ACCEPTED, &repo, &id, 0),
    }
}

/// Link `digest` into `repo` when `from` links it. `None` when it does not.
async fn mount(
    store: &Arc<OciStore>,
    repo: &RepoName,
    from: &RepoName,
    digest: &Digest,
) -> Result<Option<Response>, OciError> {
    let (store, target, from, asked) = (store.clone(), repo.clone(), from.clone(), digest.clone());
    let mounted = blocking(move || match store.blob_stat(&from, &asked) {
        Ok(stat) => store
            .link_add(&target, &stat.stored_as, LinkKind::Blob, None)
            .map(|()| true),
        Err(OciStoreError::NotInRepository { .. } | OciStoreError::UnknownRepository(_)) => {
            Ok(false)
        }
        Err(error) => Err(error),
    })
    .await?;
    mounted.then(|| created(repo, digest)).transpose()
}

/// Route 13: where the upload stands.
///
/// # Errors
///
/// `BLOB_UPLOAD_UNKNOWN`.
pub async fn status(
    store: Arc<OciStore>,
    repo: RepoName,
    id: UploadId,
) -> Result<Response, OciError> {
    let received = session_of(&store, &repo, &id).await?;
    session_response(StatusCode::NO_CONTENT, &repo, &id, received)
}

/// Route 14: one chunk.
///
/// # Errors
///
/// `RANGE_INVALID` with the true `Range` when `Content-Range` does not start
/// where the session stands; `BLOB_UPLOAD_UNKNOWN`.
pub async fn patch(
    store: Arc<OciStore>,
    repo: RepoName,
    id: UploadId,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, OciError> {
    let received = session_of(&store, &repo, &id).await?;
    // Without `Content-Range` the chunk goes at the end, which is what
    // `docker` sends. With it, the start must be the end.
    if let Some(start) = content_range_start(headers) {
        if start != received {
            return Err(OciError::from_store(
                OciStoreError::OffsetMismatch {
                    expected: received,
                    got: start,
                },
                Context::Upload,
            ));
        }
    }
    let received = stream_into(&store, &id, received, body).await?;
    session_response(StatusCode::ACCEPTED, &repo, &id, received)
}

/// Route 15: the last chunk, if any, and the digest the whole must hash to.
///
/// # Errors
///
/// `DIGEST_INVALID` when `digest` is missing, malformed, or not what the
/// bytes hash to; `BLOB_UPLOAD_UNKNOWN`.
pub async fn put(
    store: Arc<OciStore>,
    repo: RepoName,
    id: UploadId,
    query: Option<&str>,
    body: Body,
) -> Result<Response, OciError> {
    let claimed =
        digest_param(query, "digest")?.ok_or_else(|| {
            OciError::new(ErrorCode::DigestInvalid).with_detail(serde_json::json!("digest missing"))
        })?;
    let received = session_of(&store, &repo, &id).await?;
    stream_into(&store, &id, received, body).await?;
    finish(store, &repo, id, claimed).await
}

/// Close the session. The store reads the whole blob back to verify it, so
/// this may take as long as the blob is large; there is no timeout on it.
async fn finish(
    store: Arc<OciStore>,
    repo: &RepoName,
    id: UploadId,
    claimed: Digest,
) -> Result<Response, OciError> {
    let asked = claimed.clone();
    blocking(move || store.upload_finish(&id, &claimed)).await?;
    created(repo, &asked)
}

/// Route 16: abandon the upload.
///
/// # Errors
///
/// `BLOB_UPLOAD_UNKNOWN`.
pub async fn cancel(
    store: Arc<OciStore>,
    repo: RepoName,
    id: UploadId,
) -> Result<Response, OciError> {
    session_of(&store, &repo, &id).await?;
    blocking(move || store.upload_cancel(&id)).await?;
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    Ok(response)
}

/// The first byte position in `Content-Range`: `<start>-<end>`, as the
/// protocol writes it, or HTTP's `bytes <start>-<end>/<total>`.
fn content_range_start(headers: &HeaderMap) -> Option<u64> {
    let text = headers.get(CONTENT_RANGE)?.to_str().ok()?.trim();
    let text = text.strip_prefix("bytes").map_or(text, str::trim_start);
    text.split_once('-')?.0.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_in_both_spellings() {
        let start = |text: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_RANGE, HeaderValue::from_str(text).expect("header"));
            content_range_start(&headers)
        };
        assert_eq!(start("0-1023"), Some(0));
        assert_eq!(start("1024-2047"), Some(1024));
        assert_eq!(start("bytes 1024-2047/4096"), Some(1024));
        assert_eq!(start("nonsense"), None);
        assert_eq!(content_range_start(&HeaderMap::new()), None);
    }
}
