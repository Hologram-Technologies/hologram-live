//! Header helpers shared by the registry routes.

use crate::oci_store::Digest;
use axum::http::header::{HeaderName, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue};

const VERSION_NAME: &str = "docker-distribution-api-version";
const VERSION_VALUE: &str = "registry/2.0";
const CONTENT_DIGEST: &str = "docker-content-digest";

/// Every response under `/v2/`, errors included, carries this.
pub fn stamp_version(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static(VERSION_NAME),
        HeaderValue::from_static(VERSION_VALUE),
    );
}

/// `Docker-Content-Digest` and the quoted `ETag` for `digest`.
pub fn stamp_digest(headers: &mut HeaderMap, digest: &Digest) {
    // A parsed digest is `algorithm:hex`, which is always a valid header value.
    if let Ok(value) = HeaderValue::from_str(digest.as_str()) {
        headers.insert(HeaderName::from_static(CONTENT_DIGEST), value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("\"{digest}\"")) {
        headers.insert(axum::http::header::ETAG, value);
    }
}

/// Whether `If-None-Match` names `digest`, so the answer is 304.
pub fn not_modified(headers: &HeaderMap, digest: &Digest) -> bool {
    headers
        .get_all(IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|entry| entry.trim().trim_start_matches("W/").trim_matches('"'))
        .any(|entry| entry == "*" || entry == digest.as_str())
}

/// What a `Range` header asks of a body of `size` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// No header, or one this server may ignore: the whole body, 200.
    Whole,
    /// One satisfiable range: 206.
    Part { start: u64, len: u64 },
    /// Malformed, or wholly past the end: 416.
    Unsatisfiable,
}

/// Read a blob `GET`'s `Range` header the way the reference's file server does.
///
/// One range is served. More than one is answered with the whole body, which
/// the HTTP specification allows; the reference sends `multipart/byteranges`,
/// which no registry client asks for. Gate B scenario `blob-range-forms`
/// records the difference.
pub fn byte_range(header: Option<&HeaderValue>, size: u64) -> ByteRange {
    let Some(header) = header else {
        return ByteRange::Whole;
    };
    let Some(spec) = header
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("bytes="))
    else {
        return ByteRange::Unsatisfiable;
    };
    let mut ranges = Vec::new();
    let mut past_the_end = false;
    for part in spec.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        let Some((first, last)) = part.split_once('-') else {
            return ByteRange::Unsatisfiable;
        };
        let (first, last) = (first.trim(), last.trim());
        if first.is_empty() {
            // A suffix: the last `n` bytes.
            let Ok(n) = last.parse::<u64>() else {
                return ByteRange::Unsatisfiable;
            };
            let n = n.min(size);
            if n == 0 {
                past_the_end = true;
                continue;
            }
            ranges.push((size - n, n));
            continue;
        }
        let Ok(start) = first.parse::<u64>() else {
            return ByteRange::Unsatisfiable;
        };
        if start >= size {
            past_the_end = true;
            continue;
        }
        let end = if last.is_empty() {
            size - 1
        } else {
            match last.parse::<u64>() {
                Ok(end) if end >= start => end.min(size - 1),
                _ => return ByteRange::Unsatisfiable,
            }
        };
        ranges.push((start, end - start + 1));
    }
    match ranges.as_slice() {
        [] if past_the_end => ByteRange::Unsatisfiable,
        [(start, len)] => ByteRange::Part {
            start: *start,
            len: *len,
        },
        _ => ByteRange::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(value: &str, size: u64) -> ByteRange {
        byte_range(Some(&HeaderValue::from_str(value).expect("header")), size)
    }

    #[test]
    fn the_range_forms() {
        assert_eq!(byte_range(None, 100), ByteRange::Whole);
        assert_eq!(range("bytes=0-9", 100), ByteRange::Part { start: 0, len: 10 });
        assert_eq!(
            range("bytes=90-", 100),
            ByteRange::Part { start: 90, len: 10 }
        );
        assert_eq!(
            range("bytes=-10", 100),
            ByteRange::Part { start: 90, len: 10 }
        );
        assert_eq!(
            range("bytes=-500", 100),
            ByteRange::Part { start: 0, len: 100 },
            "a suffix longer than the body is the body"
        );
        assert_eq!(
            range("bytes=50-5000", 100),
            ByteRange::Part { start: 50, len: 50 },
            "the end is clamped"
        );
        assert_eq!(range("bytes=100-", 100), ByteRange::Unsatisfiable);
        assert_eq!(range("bytes=9-3", 100), ByteRange::Unsatisfiable);
        assert_eq!(range("bytes=a-b", 100), ByteRange::Unsatisfiable);
        assert_eq!(range("items=0-9", 100), ByteRange::Unsatisfiable);
        assert_eq!(range("bytes=0-0", 0), ByteRange::Unsatisfiable);
        assert_eq!(range("bytes=0-9,20-29", 100), ByteRange::Whole);
        assert_eq!(
            range("bytes=0-9,500-", 100),
            ByteRange::Part { start: 0, len: 10 },
            "a range past the end is dropped, not fatal, when another overlaps"
        );
    }

    #[test]
    fn if_none_match_forms() {
        let digest = Digest::sha256_of(b"x");
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(IF_NONE_MATCH, HeaderValue::from_str(value).expect("header"));
            not_modified(&headers, &digest)
        };
        assert!(with(&format!("\"{digest}\"")));
        assert!(with(&format!("W/\"{digest}\"")));
        assert!(with(&format!("\"other\", \"{digest}\"")));
        assert!(with("*"));
        assert!(!with("\"sha256:00\""));
        assert!(!not_modified(&HeaderMap::new(), &digest));
    }
}
