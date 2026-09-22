//! Parsing what follows `/v2/`.
//!
//! A repository name may contain slashes, and a router cannot put a wildcard
//! in the middle of a path, so the module takes the whole remainder and reads
//! it here. Every route is anchored at its **end**: the reference, digest or
//! upload id is the last segment and holds no slash; what precedes the route's
//! keyword is the repository name. `team/tags/app/manifests/latest` is a
//! manifest of `team/tags/app`, as it is for the reference's router.

use super::error::{Context, OciError};
use crate::oci_store::{Digest, Reference, RepoName, UploadId};
use axum::http::Method;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestVerb {
    Get,
    Head,
    Put,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobVerb {
    Get,
    Head,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadVerb {
    /// `GET` and `HEAD` both ask where the upload stands.
    Status,
    Patch,
    Put,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Base,
    Catalog,
    TagsList {
        repo: RepoName,
    },
    Manifest {
        repo: RepoName,
        reference: Reference,
        verb: ManifestVerb,
    },
    Blob {
        repo: RepoName,
        digest: Digest,
        verb: BlobVerb,
    },
    UploadStart {
        repo: RepoName,
    },
    Upload {
        repo: RepoName,
        id: UploadId,
        verb: UploadVerb,
    },
    Referrers {
        repo: RepoName,
        digest: Digest,
    },
    /// `OPTIONS` on a known path: the methods it takes.
    Options {
        allow: &'static str,
    },
}

impl Route {
    /// The repository the route acts on, if any.
    pub fn repo(&self) -> Option<&RepoName> {
        match self {
            Self::TagsList { repo }
            | Self::Manifest { repo, .. }
            | Self::Blob { repo, .. }
            | Self::UploadStart { repo }
            | Self::Upload { repo, .. }
            | Self::Referrers { repo, .. } => Some(repo),
            Self::Base | Self::Catalog | Self::Options { .. } => None,
        }
    }
}

// The reference's method handler lists them sorted.
const ALLOW_GET: &str = "GET";
const ALLOW_POST: &str = "POST";
const ALLOW_MANIFEST: &str = "DELETE, GET, HEAD, PUT";
const ALLOW_BLOB: &str = "DELETE, GET, HEAD";
const ALLOW_UPLOAD: &str = "DELETE, GET, HEAD, PATCH, PUT";

/// Undo a path's percent-encoding once. `None` when the path hides a slash
/// inside a component (`%2F`), or is not valid encoding or UTF-8: such a path
/// is no route.
pub fn decode(raw: &str) -> Option<String> {
    percent_decode(raw, false)
}

/// The value of `name` in a query string, decoded. A repository name in
/// `from=` arrives as `myorg%2Fother`, so a slash is allowed here.
pub fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name)
            .then(|| percent_decode(value, true))
            .flatten()
    })
}

fn percent_decode(raw: &str, slash_allowed: bool) -> Option<String> {
    if !raw.contains('%') {
        return Some(raw.to_owned());
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = raw.get(at + 1..at + 3)?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == b'/' && !slash_allowed {
                return None;
            }
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Read `rest`, the decoded path after `/v2/`, into a route.
///
/// # Errors
///
/// A plain 404 for a path that is no route; `NAME_INVALID`, `DIGEST_INVALID`,
/// `TAG_INVALID` or `BLOB_UPLOAD_UNKNOWN` for a route whose parts are outside
/// the grammar; 405 with `Allow` for a route that does not take `method`.
pub fn parse(method: &Method, rest: &str) -> Result<Route, OciError> {
    if rest.is_empty() {
        // The reference's base handler takes every method (gate B, `base`).
        return Ok(Route::Base);
    }
    if rest == "_catalog" {
        return pick(method, ALLOW_GET, |method| {
            (method == Method::GET).then_some(Route::Catalog)
        });
    }
    if let Some(name) = rest.strip_suffix("/tags/list") {
        let repo = repo(name)?;
        return pick(method, ALLOW_GET, |method| {
            (method == Method::GET).then_some(Route::TagsList { repo })
        });
    }
    if let Some(name) = rest.strip_suffix("/blobs/uploads/") {
        let repo = repo(name)?;
        // The reference routes this to the upload with an empty id (gate B).
        if [Method::GET, Method::HEAD, Method::PATCH, Method::PUT, Method::DELETE].contains(method) {
            return Err(OciError::upload_unknown());
        }
        return pick(method, ALLOW_POST, |method| {
            (method == Method::POST).then_some(Route::UploadStart { repo })
        });
    }
    let (head, tail) = rest.rsplit_once('/').ok_or_else(OciError::unknown_route)?;
    if tail.is_empty() {
        return Err(OciError::unknown_route());
    }
    // `/blobs/uploads` before `/blobs`: both end in a keyword, and only the
    // longer one leaves the right repository name.
    if let Some(name) = head.strip_suffix("/blobs/uploads") {
        let repo = repo(name)?;
        let id = UploadId::parse(tail).map_err(|error| OciError::from_store(error, Context::Upload))?;
        return pick(method, ALLOW_UPLOAD, |method| {
            let verb = match *method {
                Method::GET | Method::HEAD => UploadVerb::Status,
                Method::PATCH => UploadVerb::Patch,
                Method::PUT => UploadVerb::Put,
                Method::DELETE => UploadVerb::Delete,
                _ => return None,
            };
            Some(Route::Upload { repo, id, verb })
        });
    }
    if let Some(name) = head.strip_suffix("/manifests") {
        let repo = repo(name)?;
        // A tag outside the grammar is no route at all, as it is for the
        // reference's router: the OCI conformance suite asks for the tag
        // `.INVALID_MANIFEST_NAME` and requires 404. A malformed digest is
        // `DIGEST_INVALID`; it is never read as a tag.
        let reference = Reference::parse(tail).map_err(|error| {
            if tail.contains(':') {
                OciError::from_store(error, Context::Manifest)
            } else {
                OciError::unknown_route()
            }
        })?;
        return pick(method, ALLOW_MANIFEST, |method| {
            let verb = match *method {
                Method::GET => ManifestVerb::Get,
                Method::HEAD => ManifestVerb::Head,
                Method::PUT => ManifestVerb::Put,
                Method::DELETE => ManifestVerb::Delete,
                _ => return None,
            };
            Some(Route::Manifest {
                repo,
                reference,
                verb,
            })
        });
    }
    if let Some(name) = head.strip_suffix("/blobs") {
        let repo = repo(name)?;
        let digest = digest(tail)?;
        return pick(method, ALLOW_BLOB, |method| {
            let verb = match *method {
                Method::GET => BlobVerb::Get,
                Method::HEAD => BlobVerb::Head,
                Method::DELETE => BlobVerb::Delete,
                _ => return None,
            };
            Some(Route::Blob { repo, digest, verb })
        });
    }
    if let Some(name) = head.strip_suffix("/referrers") {
        let repo = repo(name)?;
        let digest = digest(tail)?;
        return pick(method, ALLOW_GET, |method| {
            (method == Method::GET).then_some(Route::Referrers { repo, digest })
        });
    }
    Err(OciError::unknown_route())
}

/// The route for `method`, or `OPTIONS`' answer, or 405 naming what is allowed.
fn pick(
    method: &Method,
    allow: &'static str,
    route: impl FnOnce(&Method) -> Option<Route>,
) -> Result<Route, OciError> {
    if method == Method::OPTIONS {
        return Ok(Route::Options { allow });
    }
    route(method).ok_or_else(|| OciError::wrong_method(allow))
}

/// A name outside the grammar is no route at all: the reference's router has
/// the grammar in its patterns, so such a path matches nothing and is the
/// plain 404 (gate B, `names`: `Foo`, `a..b`, `-a`, `_catalog`).
fn repo(name: &str) -> Result<RepoName, OciError> {
    RepoName::parse(name).map_err(|_| OciError::unknown_route())
}

fn digest(value: &str) -> Result<Digest, OciError> {
    Digest::parse(value).map_err(|error| OciError::from_store(error, Context::Blob))
}

#[cfg(test)]
mod tests {
    use super::super::error::ErrorCode;
    use super::*;
    use axum::http::StatusCode;

    const HEX: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const UUID: &str = "6b3c1f0e-8f4a-4b7e-9d2a-0c1e2f3a4b5c";

    fn get(rest: &str) -> Result<Route, OciError> {
        parse(&Method::GET, rest)
    }

    fn name(value: &str) -> RepoName {
        RepoName::parse(value).expect("repository name")
    }

    /// The nine rows of `contracts/registry-api.md`, "Path parsing".
    #[test]
    fn the_contract_table() {
        let digest = format!("sha256:{HEX}");
        assert_eq!(
            get("team/tags/app/manifests/latest").expect("row 1"),
            Route::Manifest {
                repo: name("team/tags/app"),
                reference: Reference::parse("latest").expect("tag"),
                verb: ManifestVerb::Get,
            }
        );
        assert_eq!(
            get(&format!("a/blobs/b/blobs/{digest}")).expect("row 2"),
            Route::Blob {
                repo: name("a/blobs/b"),
                digest: Digest::parse(&digest).expect("digest"),
                verb: BlobVerb::Get,
            }
        );
        assert_eq!(
            get("x/manifests/y/tags/list").expect("row 3"),
            Route::TagsList {
                repo: name("x/manifests/y")
            }
        );
        assert_eq!(
            parse(&Method::POST, "foo/blobs/uploads/").expect("row 4"),
            Route::UploadStart { repo: name("foo") }
        );
        for (method, verb) in [
            (Method::GET, UploadVerb::Status),
            (Method::HEAD, UploadVerb::Status),
            (Method::PATCH, UploadVerb::Patch),
            (Method::PUT, UploadVerb::Put),
            (Method::DELETE, UploadVerb::Delete),
        ] {
            assert_eq!(
                parse(&method, &format!("foo/blobs/uploads/{UUID}")).expect("row 5"),
                Route::Upload {
                    repo: name("foo"),
                    id: UploadId::parse(UUID).expect("upload id"),
                    verb,
                }
            );
        }
        assert_eq!(
            parse(&Method::POST, "blobs/uploads/blobs/uploads/").expect("row 6"),
            Route::UploadStart {
                repo: name("blobs/uploads")
            }
        );
        let row_7 = get("Foo/manifests/latest").expect_err("row 7");
        assert_eq!((row_7.code(), row_7.status()), (None, StatusCode::NOT_FOUND));
        let row_8 = get("foo/manifests/a/b").expect_err("row 8");
        assert_eq!((row_8.code(), row_8.status()), (None, StatusCode::NOT_FOUND));
        let row_9 = get("_catalog/manifests/x").expect_err("row 9");
        assert_eq!((row_9.code(), row_9.status()), (None, StatusCode::NOT_FOUND));
    }

    #[test]
    fn the_name_separators_the_reference_allows() {
        for good in ["a__b", "a---b", "a.b", "a_b"] {
            assert!(get(&format!("{good}/tags/list")).is_ok(), "{good}");
        }
        for bad in ["a___b", "a..b", "-a", "a-", "a//b"] {
            let error = get(&format!("{bad}/tags/list")).expect_err(bad);
            assert_eq!((error.code(), error.status()), (None, StatusCode::NOT_FOUND), "{bad}");
        }
    }

    /// The OCI conformance suite's default namespaces (its README:
    /// `OCI_NAMESPACE`, `OCI_CROSSMOUNT_NAMESPACE`).
    #[test]
    fn the_conformance_suites_repository_names() {
        for repo in ["myorg/myrepo", "myorg/other"] {
            assert!(get(&format!("{repo}/tags/list")).is_ok());
            assert!(get(&format!("{repo}/manifests/tagtest0")).is_ok());
            assert!(get(&format!("{repo}/blobs/sha256:{HEX}")).is_ok());
            assert!(get(&format!("{repo}/referrers/sha256:{HEX}")).is_ok());
            assert!(parse(&Method::POST, &format!("{repo}/blobs/uploads/")).is_ok());
        }
    }

    #[test]
    fn base_and_catalogue() {
        // The reference's base handler takes every method.
        assert_eq!(parse(&Method::POST, "").expect("base"), Route::Base);
        assert_eq!(parse(&Method::OPTIONS, "").expect("base"), Route::Base);
        assert_eq!(get("").expect("base"), Route::Base);
        assert_eq!(get("_catalog").expect("catalogue"), Route::Catalog);
    }

    #[test]
    fn a_wrong_method_is_405_with_allow() {
        for (method, rest, allow) in [
            (Method::DELETE, "foo/tags/list".to_owned(), "GET"),
            (Method::POST, "foo/manifests/latest".to_owned(), "DELETE, GET, HEAD, PUT"),
            (Method::PUT, format!("foo/blobs/sha256:{HEX}"), "DELETE, GET, HEAD"),
            (
                Method::POST,
                format!("foo/blobs/uploads/{UUID}"),
                "DELETE, GET, HEAD, PATCH, PUT",
            ),
        ] {
            let error = parse(&method, &rest).expect_err("wrong method");
            assert_eq!(error.status(), StatusCode::METHOD_NOT_ALLOWED, "{rest}");
            let response = axum::response::IntoResponse::into_response(error);
            assert_eq!(response.headers()["allow"], allow, "{rest}");
        }
    }

    #[test]
    fn options_names_the_methods() {
        assert_eq!(
            parse(&Method::OPTIONS, "foo/manifests/latest").expect("options"),
            Route::Options {
                allow: "DELETE, GET, HEAD, PUT"
            }
        );
    }

    #[test]
    fn parts_outside_the_grammar() {
        assert_eq!(
            get("foo/blobs/sha256:short").expect_err("digest").code(),
            Some(ErrorCode::DigestInvalid)
        );
        assert_eq!(
            get("foo/manifests/sha256:short").expect_err("digest").code(),
            Some(ErrorCode::DigestInvalid),
            "a malformed digest is never read as a tag"
        );
        assert_eq!(
            get("foo/blobs/uploads/not-a-uuid")
                .expect_err("upload id")
                .code(),
            Some(ErrorCode::BlobUploadUnknown)
        );
        assert!(get(&format!("foo/blobs/blake3:{HEX}")).is_ok());
    }

    #[test]
    fn a_tag_outside_the_grammar_is_404_for_every_method() {
        for method in [Method::GET, Method::HEAD, Method::PUT, Method::DELETE] {
            let error =
                parse(&method, "myorg/myrepo/manifests/.INVALID_MANIFEST_NAME").expect_err("tag");
            assert_eq!((error.code(), error.status()), (None, StatusCode::NOT_FOUND));
        }
    }

    #[test]
    fn what_is_no_route() {
        for rest in ["foo", "foo/", "manifests/latest", "tags/list", "foo/bar/baz", "foo/blobs/"] {
            let error = get(rest).expect_err(rest);
            assert_eq!((error.code(), error.status()), (None, StatusCode::NOT_FOUND), "{rest}");
        }
    }

    #[test]
    fn query_values_are_decoded_and_may_hold_a_slash() {
        let query = Some("mount=sha256%3Aabc&from=myorg%2Fother&empty");
        assert_eq!(query_param(query, "mount").as_deref(), Some("sha256:abc"));
        assert_eq!(query_param(query, "from").as_deref(), Some("myorg/other"));
        assert_eq!(query_param(query, "empty").as_deref(), Some(""));
        assert_eq!(query_param(query, "digest"), None);
        assert_eq!(query_param(None, "digest"), None);
    }

    #[test]
    fn percent_encoding_is_undone_once() {
        assert_eq!(
            decode("foo/blobs/sha256%3Aabc").as_deref(),
            Some("foo/blobs/sha256:abc")
        );
        assert_eq!(decode("plain/path").as_deref(), Some("plain/path"));
        assert_eq!(decode("foo%2Fbar/tags/list"), None, "a hidden slash");
        assert_eq!(decode("foo%2fbar/tags/list"), None);
        assert_eq!(decode("foo%zz"), None);
        assert_eq!(decode("foo%2"), None);
        assert_eq!(decode("foo%252Fbar").as_deref(), Some("foo%2Fbar"), "once, not twice");
    }
}
