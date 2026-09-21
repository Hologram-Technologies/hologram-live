//! Routes 8 and 9: a blob's bytes, streamed, whole or one range of them.

use super::error::{Context, OciError};
use super::respond::{byte_range, not_modified, stamp_digest, ByteRange};
use crate::oci_store::{AsyncBlob, BlobRead, Digest, OciStore, RepoName};
use axum::body::Body;
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tokio_util::io::ReaderStream;

/// One read from disk, and at most this much of a blob in memory per request.
const CHUNK: usize = 1 << 20;

/// # Errors
///
/// `BLOB_UNKNOWN` when `repo` holds no link to `digest`; `UNKNOWN` when the
/// store fails. A range that is malformed or past the end is a 416 answer,
/// not an error: the reference answers it in plain text.
pub async fn get(
    store: Arc<OciStore>,
    repo: RepoName,
    digest: Digest,
    headers: &HeaderMap,
    head: bool,
) -> Result<Response, OciError> {
    let asked = digest.clone();
    // A `HEAD` never opens the file.
    let (size, reader) = tokio::task::spawn_blocking(move || {
        if head {
            store.blob_stat(&repo, &digest).map(|stat| (stat.size, None))
        } else {
            store
                .blob_open(&repo, &digest)
                .map(|(stat, reader)| (stat.size, Some(reader)))
        }
    })
    .await
    .map_err(|error| OciError::internal(&error))?
    .map_err(|error| OciError::from_store(error, Context::Blob))?;

    let mut response = Response::new(Body::empty());
    let out = response.headers_mut();
    // The digest the client asked by, which is the one it will verify against.
    stamp_digest(out, &asked);
    if not_modified(headers, &asked) {
        out.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=31536000"));
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        return Ok(response);
    }
    let (start, len) = match byte_range(headers.get(RANGE), size) {
        ByteRange::Whole => (0, size),
        ByteRange::Part { start, len } => {
            out.insert(
                CONTENT_RANGE,
                header(&format!("bytes {start}-{}/{size}", start + len - 1))?,
            );
            (start, len)
        }
        // The reference's file server answers these, in plain text, before
        // the registry's error shape (gate B, `blob-read`).
        ByteRange::Unsatisfiable => {
            let refused = refuse_range(&asked, "invalid range: failed to overlap\n");
            let mut refused = refused;
            refused
                .headers_mut()
                .insert(CONTENT_RANGE, header(&format!("bytes */{size}"))?);
            return Ok(refused);
        }
        ByteRange::Malformed => return Ok(refuse_range(&asked, "invalid range\n")),
    };
    out.insert(CONTENT_LENGTH, HeaderValue::from(len));
    out.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    out.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    // Content under a digest never changes; the reference says a year too.
    out.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=31536000"));
    if len != size {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    if let Some(reader) = reader {
        *response.body_mut() = body(reader, start, len);
    }
    Ok(response)
}

fn refuse_range(digest: &Digest, text: &'static str) -> Response {
    let mut response = (StatusCode::RANGE_NOT_SATISFIABLE, text).into_response();
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    stamp_digest(headers, digest);
    headers.remove(axum::http::header::ETAG);
    response
}

/// `len` bytes of `reader` from `start`, read a chunk at a time off the runtime.
fn body(reader: Box<dyn BlobRead>, start: u64, len: u64) -> Body {
    Body::from_stream(ReaderStream::with_capacity(
        AsyncBlob::new(reader, start, len),
        CHUNK,
    ))
}

fn header(value: &str) -> Result<HeaderValue, OciError> {
    HeaderValue::from_str(value).map_err(|error| OciError::internal(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 64 MiB of a repeating pattern that is never held in memory, counting
    /// every byte handed out.
    struct Counting {
        at: u64,
        size: u64,
        read: Arc<AtomicU64>,
    }

    fn pattern(at: u64) -> u8 {
        u8::try_from(at % 251).unwrap_or(0)
    }

    impl Read for Counting {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let left = usize::try_from(self.size - self.at).unwrap_or(usize::MAX);
            let n = buffer.len().min(left);
            for (offset, byte) in buffer[..n].iter_mut().enumerate() {
                *byte = pattern(self.at + offset as u64);
            }
            self.at += n as u64;
            self.read.fetch_add(n as u64, Ordering::Relaxed);
            Ok(n)
        }
    }

    impl Seek for Counting {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            if let SeekFrom::Start(at) = to {
                self.at = at;
            }
            Ok(self.at)
        }
    }

    #[tokio::test]
    async fn a_range_reads_only_what_it_serves() {
        const MIB: u64 = 1 << 20;
        let read = Arc::new(AtomicU64::new(0));
        let reader = Counting {
            at: 0,
            size: 64 * MIB,
            read: read.clone(),
        };
        let start = 40 * MIB + 7;
        // Counted as it streams: the test never holds the body either.
        let mut stream = body(Box::new(reader), start, MIB).into_data_stream();
        let (mut served, mut first) = (0_u64, None);
        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            let chunk = chunk.expect("chunk");
            first = first.or_else(|| chunk.first().copied());
            served += chunk.len() as u64;
        }
        assert_eq!(served, MIB);
        assert_eq!(first, Some(pattern(start)), "the range starts where asked");
        assert!(
            read.load(Ordering::Relaxed) <= 2 * MIB,
            "read {} bytes to serve 1 MiB",
            read.load(Ordering::Relaxed)
        );
    }
}
