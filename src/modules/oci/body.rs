//! A request body into an upload session, one frame at a time.

use super::error::{Context, ErrorCode, OciError};
use crate::oci_store::{Framer, OciStore, UploadId};
use axum::body::Body;
use std::sync::Arc;
use tokio_stream::StreamExt;

/// Append `body` to upload `id` from `offset`. Returns the new offset.
///
/// The HTTP stack hands over pieces of a few KiB; the store wants few, large
/// writes. `Framer` turns the first into the second. One frame is in memory
/// per upload, and the next piece is not polled while a frame is being
/// written, so a slow disk slows the client instead of filling memory.
///
/// # Errors
///
/// `RANGE_INVALID` when `offset` is not where the session stands;
/// `BLOB_UPLOAD_UNKNOWN`; `BLOB_UPLOAD_INVALID` when the client goes away
/// mid-body (what it sent stays: the client resumes from the status route).
pub async fn stream_into(
    store: &Arc<OciStore>,
    id: &UploadId,
    mut offset: u64,
    body: Body,
) -> Result<u64, OciError> {
    let mut framer = Framer::default();
    let mut stream = body.into_data_stream();
    while let Some(piece) = stream.next().await {
        let piece = piece.map_err(|error| {
            tracing::debug!(%error, upload = %id, "the client went away mid-body");
            OciError::new(ErrorCode::BlobUploadInvalid)
        })?;
        let full: Vec<bytes::Bytes> = framer.push(&piece).collect();
        for frame in full {
            offset = append(store, id, offset, frame).await?;
        }
    }
    if let Some(tail) = framer.finish() {
        offset = append(store, id, offset, tail).await?;
    }
    Ok(offset)
}

async fn append(
    store: &Arc<OciStore>,
    id: &UploadId,
    offset: u64,
    frame: bytes::Bytes,
) -> Result<u64, OciError> {
    let (store, id) = (store.clone(), id.clone());
    tokio::task::spawn_blocking(move || store.upload_append(&id, offset, &frame))
        .await
        // A panic in the store is a 500, not a hung connection.
        .map_err(|error| OciError::internal(&error))?
        .map_err(|error| OciError::from_store(error, Context::Upload))
}
