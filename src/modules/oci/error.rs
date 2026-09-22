//! The registry's error envelope, and the one place a store error becomes a
//! registry error code.
//!
//! Messages and `detail` payloads follow `contracts/registry-api.md`. The
//! contract marks them as recalled: golden transcript 4 (gate B) records what
//! the reference sends, and corrects this file where they differ.

use super::respond::stamp_version;
use crate::oci_store::OciStoreError;
use axum::http::header::{HeaderName, ALLOW, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::borrow::Cow;

/// The 18 documented codes, and `UNKNOWN` for an internal failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    BlobUnknown,
    BlobUploadInvalid,
    BlobUploadUnknown,
    DigestInvalid,
    ManifestBlobUnknown,
    ManifestInvalid,
    ManifestUnknown,
    ManifestUnverified,
    NameInvalid,
    NameUnknown,
    PaginationNumberInvalid,
    RangeInvalid,
    SizeInvalid,
    TagInvalid,
    Unauthorized,
    Denied,
    Unsupported,
    TooManyRequests,
    Unknown,
}

impl ErrorCode {
    pub const ALL: [Self; 19] = [
        Self::BlobUnknown,
        Self::BlobUploadInvalid,
        Self::BlobUploadUnknown,
        Self::DigestInvalid,
        Self::ManifestBlobUnknown,
        Self::ManifestInvalid,
        Self::ManifestUnknown,
        Self::ManifestUnverified,
        Self::NameInvalid,
        Self::NameUnknown,
        Self::PaginationNumberInvalid,
        Self::RangeInvalid,
        Self::SizeInvalid,
        Self::TagInvalid,
        Self::Unauthorized,
        Self::Denied,
        Self::Unsupported,
        Self::TooManyRequests,
        Self::Unknown,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BlobUnknown => "BLOB_UNKNOWN",
            Self::BlobUploadInvalid => "BLOB_UPLOAD_INVALID",
            Self::BlobUploadUnknown => "BLOB_UPLOAD_UNKNOWN",
            Self::DigestInvalid => "DIGEST_INVALID",
            Self::ManifestBlobUnknown => "MANIFEST_BLOB_UNKNOWN",
            Self::ManifestInvalid => "MANIFEST_INVALID",
            Self::ManifestUnknown => "MANIFEST_UNKNOWN",
            Self::ManifestUnverified => "MANIFEST_UNVERIFIED",
            Self::NameInvalid => "NAME_INVALID",
            Self::NameUnknown => "NAME_UNKNOWN",
            Self::PaginationNumberInvalid => "PAGINATION_NUMBER_INVALID",
            Self::RangeInvalid => "RANGE_INVALID",
            Self::SizeInvalid => "SIZE_INVALID",
            Self::TagInvalid => "TAG_INVALID",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Denied => "DENIED",
            Self::Unsupported => "UNSUPPORTED",
            Self::TooManyRequests => "TOOMANYREQUESTS",
            Self::Unknown => "UNKNOWN",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Self::BlobUnknown
            | Self::BlobUploadInvalid
            | Self::BlobUploadUnknown
            | Self::ManifestUnknown
            | Self::NameUnknown => StatusCode::NOT_FOUND,
            Self::DigestInvalid
            | Self::ManifestBlobUnknown
            | Self::ManifestInvalid
            | Self::ManifestUnverified
            | Self::NameInvalid
            | Self::PaginationNumberInvalid
            | Self::SizeInvalid
            | Self::TagInvalid => StatusCode::BAD_REQUEST,
            Self::RangeInvalid => StatusCode::RANGE_NOT_SATISFIABLE,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Denied => StatusCode::FORBIDDEN,
            Self::Unsupported => StatusCode::METHOD_NOT_ALLOWED,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Unknown => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn default_message(self) -> &'static str {
        match self {
            Self::BlobUnknown | Self::ManifestBlobUnknown => "blob unknown to registry",
            Self::BlobUploadInvalid => "blob upload invalid",
            Self::BlobUploadUnknown => "blob upload unknown to registry",
            Self::DigestInvalid => "provided digest did not match uploaded content",
            Self::ManifestInvalid => "manifest invalid",
            Self::ManifestUnknown => "manifest unknown",
            Self::ManifestUnverified => "manifest failed signature verification",
            Self::NameInvalid => "invalid repository name",
            Self::NameUnknown => "repository name not known to registry",
            Self::PaginationNumberInvalid => "invalid number of results requested",
            Self::RangeInvalid => "invalid content range",
            Self::SizeInvalid => "provided length did not match content length",
            Self::TagInvalid => "manifest tag did not match URI",
            Self::Unauthorized => "authentication required",
            Self::Denied => "requested access to the resource is denied",
            Self::Unsupported => "The operation is unsupported.",
            Self::TooManyRequests => "too many requests",
            Self::Unknown => "unknown error",
        }
    }
}

/// Which route asked the store. One store error maps differently by route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Blob,
    Manifest,
    Upload,
    Tags,
}

/// A refusal under `/v2/`. Boxed, so a `Result` that carries one stays small.
#[derive(Debug)]
pub struct OciError(Box<Refusal>);

#[derive(Debug)]
struct Refusal {
    /// `None` is the one answer without an envelope: a path no route claims.
    code: Option<ErrorCode>,
    /// For the one code that is sent with two statuses.
    status: Option<StatusCode>,
    message: Cow<'static, str>,
    detail: Value,
    /// Send `"detail": null` rather than leave it out, as the reference does
    /// where Go encodes a nil slice (the login challenge on the base route).
    null_detail: bool,
    headers: HeaderMap,
}

impl OciError {
    pub fn new(code: ErrorCode) -> Self {
        Self(Box::new(Refusal {
            code: Some(code),
            status: None,
            message: Cow::Borrowed(code.default_message()),
            detail: Value::Null,
            null_detail: false,
            headers: HeaderMap::new(),
        }))
    }

    #[must_use]
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.0.detail = detail;
        self
    }

    /// As [`OciError::with_detail`], and a null detail is sent as `null`.
    #[must_use]
    pub fn with_detail_even_null(mut self, detail: Value) -> Self {
        self.0.detail = detail;
        self.0.null_detail = true;
        self
    }

    /// Answer with `status` instead of the code's own.
    #[must_use]
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.0.status = Some(status);
        self
    }

    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.0.headers.insert(name, value);
        self
    }

    /// A path under `/v2/` that is no route. The reference answers from its
    /// router, before the registry's own error shape exists: plain text.
    pub fn unknown_route() -> Self {
        Self(Box::new(Refusal {
            code: None,
            status: None,
            message: Cow::Borrowed("404 page not found\n"),
            detail: Value::Null,
            null_detail: false,
            headers: HeaderMap::new(),
        }))
    }

    /// A known path with a method it does not take. Like the plain 404, the
    /// reference answers this from its router, in plain text, with `Allow`
    /// (gate B, `unknown-route`).
    pub fn wrong_method(allow: &'static str) -> Self {
        Self(Box::new(Refusal {
            code: None,
            status: Some(StatusCode::METHOD_NOT_ALLOWED),
            message: Cow::Borrowed("Method not allowed\n"),
            detail: Value::Null,
            null_detail: false,
            headers: HeaderMap::new(),
        }))
        .with_header(ALLOW, HeaderValue::from_static(allow))
    }

    /// `BLOB_UPLOAD_UNKNOWN`, with the reference's detail (gate B).
    pub fn upload_unknown() -> Self {
        Self::new(ErrorCode::BlobUploadUnknown).with_detail(json!("blob upload unknown"))
    }

    /// Answer with `message` instead of the code's own.
    #[must_use]
    pub fn with_message(mut self, message: &'static str) -> Self {
        self.0.message = Cow::Borrowed(message);
        self
    }

    /// An internal failure. The cause is logged here and never sent.
    pub fn internal(cause: &dyn std::fmt::Display) -> Self {
        tracing::error!(%cause, "registry request failed");
        Self::new(ErrorCode::Unknown)
    }

    pub fn code(&self) -> Option<ErrorCode> {
        self.0.code
    }

    pub fn status(&self) -> StatusCode {
        self.0
            .status
            .unwrap_or_else(|| self.0.code.map_or(StatusCode::NOT_FOUND, ErrorCode::status))
    }

    pub fn from_store(error: OciStoreError, context: Context) -> Self {
        match error {
            OciStoreError::Invalid { what, value } => match what {
                "digest" => Self::new(ErrorCode::DigestInvalid).with_detail(json!(digest_fault(&value))),
                "repository name" => Self::new(ErrorCode::NameInvalid).with_detail(json!(value)),
                "tag" => Self::new(ErrorCode::TagInvalid).with_detail(json!(value)),
                "upload id" => Self::upload_unknown(),
                other => Self::internal(&format!("invalid {other}: {value:?}")),
            },
            OciStoreError::NotInRepository { repo, digest } => match context {
                Context::Manifest | Context::Tags => Self::new(ErrorCode::ManifestUnknown)
                    .with_detail(json!({ "Name": repo, "Revision": digest })),
                Context::Blob | Context::Upload => {
                    Self::new(ErrorCode::BlobUnknown).with_detail(json!(digest))
                }
            },
            OciStoreError::UnknownRepository(repo) => match context {
                Context::Tags => {
                    Self::new(ErrorCode::NameUnknown).with_detail(json!({ "name": repo }))
                }
                Context::Manifest => {
                    Self::new(ErrorCode::ManifestUnknown).with_detail(json!({ "Name": repo }))
                }
                Context::Blob | Context::Upload => Self::new(ErrorCode::BlobUnknown),
            },
            OciStoreError::UnknownUpload(_) => Self::upload_unknown(),
            // In the reference's words (gate B, `digest-mismatch`); a manifest
            // whose digest disagrees gets no detail.
            OciStoreError::DigestMismatch { claimed } => match context {
                Context::Manifest => Self::new(ErrorCode::DigestInvalid),
                _ => Self::new(ErrorCode::DigestInvalid).with_detail(json!(format!(
                    "invalid digest for referenced layer: {claimed}, content does not match digest"
                ))),
            },
            // The reference sends no `Range` with this refusal (gate B,
            // `push-chunked`); a client asks the status route where it stands.
            OciStoreError::OffsetMismatch { .. } => Self::new(ErrorCode::RangeInvalid),
            OciStoreError::MissingReferences(digests) => {
                Self::new(ErrorCode::ManifestBlobUnknown).with_detail(json!(digests))
            }
            error @ (OciStoreError::Io(_) | OciStoreError::Locked | OciStoreError::Layout(_)) => {
                Self::internal(&error)
            }
        }
    }
}

/// Why a digest is outside the grammar, in the reference's words (gate B,
/// `errors-read`).
fn digest_fault(value: &str) -> &'static str {
    match value.split_once(':') {
        Some((algorithm, hex)) if ["sha256", "sha512", "blake3"].contains(&algorithm) => {
            if hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                "invalid checksum digest length"
            } else {
                "invalid checksum digest format"
            }
        }
        Some(_) => "unsupported digest algorithm",
        None => "invalid checksum digest format",
    }
}

impl IntoResponse for OciError {
    fn into_response(self) -> Response {
        let status = self.status();
        let Refusal {
            code,
            message,
            detail,
            null_detail,
            headers,
            status: _,
        } = *self.0;
        let mut response = match code {
            None => {
                let mut response = (status, message.into_owned()).into_response();
                response.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                );
                // As the reference's router sends it (gate B, `unknown-route`).
                response.headers_mut().insert(
                    axum::http::header::X_CONTENT_TYPE_OPTIONS,
                    HeaderValue::from_static("nosniff"),
                );
                response
            }
            Some(code) => {
                let entry = |detail: Value| {
                    let mut entry = json!({ "code": code.as_str(), "message": message });
                    if null_detail || !detail.is_null() {
                        entry["detail"] = detail;
                    }
                    entry
                };
                // The reference reports each missing blob as an error of its
                // own, its digest in `detail` (gate B, `manifest-put-invalid`).
                let entries: Vec<Value> = match detail {
                    Value::Array(digests) if code == ErrorCode::ManifestBlobUnknown => {
                        digests.into_iter().map(entry).collect()
                    }
                    other => vec![entry(other)],
                };
                // The reference's encoder ends the body with a newline; clients
                // see it in Content-Length (gate B, every error).
                let body = json!({ "errors": entries }).to_string() + "\n";
                let mut response = (status, body).into_response();
                response
                    .headers_mut()
                    .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                response
            }
        };
        response.headers_mut().extend(headers);
        // The module's layer adds this too; an error built and answered in a
        // test, or by a caller outside the layer, still carries it.
        stamp_version(response.headers_mut());
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parts(error: OciError) -> (StatusCode, HeaderMap, Value) {
        let response = error.into_response();
        let (head, body) = response.into_parts();
        let mut stream = body.into_data_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            bytes.extend_from_slice(&chunk.expect("chunk"));
        }
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (head.status, head.headers, value)
    }

    #[tokio::test]
    async fn every_code_answers_in_the_registry_envelope() {
        for code in ErrorCode::ALL {
            let (status, headers, body) = parts(OciError::new(code)).await;
            assert_eq!(status, code.status(), "{code:?}");
            assert_eq!(headers[CONTENT_TYPE], "application/json", "{code:?}");
            assert_eq!(
                headers["docker-distribution-api-version"], "registry/2.0",
                "{code:?}"
            );
            let errors = body["errors"].as_array().expect("errors array");
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0]["code"], code.as_str());
            assert_eq!(errors[0]["message"], code.default_message());
            assert!(errors[0].get("detail").is_none(), "no detail, no key");
        }
    }

    #[test]
    fn the_status_of_each_code_is_the_documented_one() {
        let expected = [
            ("BLOB_UNKNOWN", 404),
            ("BLOB_UPLOAD_INVALID", 404),
            ("BLOB_UPLOAD_UNKNOWN", 404),
            ("DIGEST_INVALID", 400),
            ("MANIFEST_BLOB_UNKNOWN", 400),
            ("MANIFEST_INVALID", 400),
            ("MANIFEST_UNKNOWN", 404),
            ("MANIFEST_UNVERIFIED", 400),
            ("NAME_INVALID", 400),
            ("NAME_UNKNOWN", 404),
            ("PAGINATION_NUMBER_INVALID", 400),
            ("RANGE_INVALID", 416),
            ("SIZE_INVALID", 400),
            ("TAG_INVALID", 400),
            ("UNAUTHORIZED", 401),
            ("DENIED", 403),
            ("UNSUPPORTED", 405),
            ("TOOMANYREQUESTS", 429),
            ("UNKNOWN", 500),
        ];
        for (code, (name, status)) in ErrorCode::ALL.iter().zip(expected) {
            assert_eq!(code.as_str(), name);
            assert_eq!(code.status().as_u16(), status, "{name}");
        }
    }

    #[test]
    fn one_store_error_maps_by_route() {
        let missing = || OciStoreError::NotInRepository {
            repo: "app".to_owned(),
            digest: "sha256:00".to_owned(),
        };
        let code = |error, context| OciError::from_store(error, context).code();
        assert_eq!(
            code(missing(), Context::Blob),
            Some(ErrorCode::BlobUnknown)
        );
        assert_eq!(
            code(missing(), Context::Manifest),
            Some(ErrorCode::ManifestUnknown)
        );
        let unknown = || OciStoreError::UnknownRepository("app".to_owned());
        assert_eq!(code(unknown(), Context::Tags), Some(ErrorCode::NameUnknown));
        assert_eq!(
            code(unknown(), Context::Manifest),
            Some(ErrorCode::ManifestUnknown)
        );
        assert_eq!(code(unknown(), Context::Blob), Some(ErrorCode::BlobUnknown));
        let invalid = |what| OciStoreError::Invalid {
            what,
            value: "x".to_owned(),
        };
        assert_eq!(
            code(invalid("digest"), Context::Blob),
            Some(ErrorCode::DigestInvalid)
        );
        assert_eq!(
            code(invalid("repository name"), Context::Blob),
            Some(ErrorCode::NameInvalid)
        );
        assert_eq!(
            code(invalid("tag"), Context::Manifest),
            Some(ErrorCode::TagInvalid)
        );
        assert_eq!(
            code(invalid("upload id"), Context::Upload),
            Some(ErrorCode::BlobUploadUnknown)
        );
        assert_eq!(
            code(
                OciStoreError::UnknownUpload("u".to_owned()),
                Context::Upload
            ),
            Some(ErrorCode::BlobUploadUnknown)
        );
        assert_eq!(
            code(
                OciStoreError::DigestMismatch {
                    claimed: "sha256:00".to_owned()
                },
                Context::Upload
            ),
            Some(ErrorCode::DigestInvalid)
        );
        assert_eq!(
            code(
                OciStoreError::MissingReferences(vec!["sha256:00".to_owned()]),
                Context::Manifest
            ),
            Some(ErrorCode::ManifestBlobUnknown)
        );
    }

    #[tokio::test]
    async fn a_stale_offset_is_416_without_a_range() {
        let error = OciError::from_store(
            OciStoreError::OffsetMismatch {
                expected: 100,
                got: 7,
            },
            Context::Upload,
        );
        let (status, headers, _) = parts(error).await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert!(headers.get("range").is_none());
    }

    #[tokio::test]
    async fn an_internal_failure_never_sends_its_cause() {
        let error = OciError::from_store(
            OciStoreError::Io("open /secret/path: denied".to_owned()),
            Context::Blob,
        );
        let (status, _, body) = parts(error).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["errors"][0]["code"], "UNKNOWN");
        assert!(!body.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn an_unknown_route_is_the_reference_routers_plain_404() {
        let response = OciError::unknown_route().into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn a_wrong_method_names_the_right_ones_in_plain_text() {
        let (status, headers, body) = parts(OciError::wrong_method("GET, HEAD")).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(headers[ALLOW], "GET, HEAD");
        assert_eq!(headers[CONTENT_TYPE], "text/plain; charset=utf-8");
        assert!(body.is_null(), "the reference's router answers before the envelope exists");
    }

    #[test]
    fn a_digest_fault_is_named_in_the_references_words() {
        assert_eq!(digest_fault("sha256:abc"), "invalid checksum digest length");
        assert_eq!(digest_fault("sha256:xyz"), "invalid checksum digest format");
        assert_eq!(digest_fault("md5:d41d8cd98f00b204e9800998ecf8427e"), "unsupported digest algorithm");
    }
}
