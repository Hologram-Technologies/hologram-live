//! Routes 2, 3 and R: the catalogue, a repository's tags, a manifest's referrers.
//!
//! Paging edge cases (`n=0`, an `n` over the limit, a `last` that does not
//! exist) are recalled from the reference and the OCI text. Gate B scenarios
//! `tags-paging`, `catalog-paging` and `referrers-present` record them.

use super::error::{Context, ErrorCode, OciError};
use super::media::OCI_INDEX;
use super::path::query_param;
use crate::oci_store::{Digest, OciStore, OciStoreError, RepoName};
use axum::body::Body;
use axum::http::header::{HeaderName, CONTENT_LENGTH, CONTENT_TYPE, LINK};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use std::sync::Arc;

/// What the catalogue returns when `n` is absent: the reference's
/// `catalog.maxentries` default.
const CATALOG_DEFAULT: usize = 100;
/// More than any repository holds; "all of them" without an unbounded read.
const TAGS_DEFAULT: usize = 1_000_000;

struct Page {
    /// How many to return. `None` when the client did not say.
    n: Option<usize>,
    last: Option<String>,
}

fn page(query: Option<&str>) -> Result<Page, OciError> {
    let n = query_param(query, "n")
        .map(|text| {
            text.parse::<usize>()
                .map_err(|_| OciError::new(ErrorCode::PaginationNumberInvalid).with_detail(json!({ "n": text })))
        })
        .transpose()?;
    let last = query_param(query, "last").filter(|last| !last.is_empty());
    Ok(Page { n, last })
}

/// Keep `limit` of the `limit + 1` that were read; the extra one only says
/// whether a next page exists.
fn trim<T>(mut items: Vec<T>, limit: usize) -> (Vec<T>, bool) {
    let more = items.len() > limit;
    items.truncate(limit);
    (items, more)
}

fn respond(body: &Value, content_type: &'static str, next: Option<String>) -> Response {
    // The reference's encoder ends the body with a newline (gate B, every
    // listing).
    let text = body.to_string() + "\n";
    let mut response = Response::new(Body::empty());
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(text.len()));
    if let Some(value) = next.and_then(|next| HeaderValue::from_str(&next).ok()) {
        headers.insert(LINK, value);
    }
    *response.body_mut() = Body::from(text);
    response
}

fn link_next(path: &str, last: &str, n: usize) -> String {
    format!("<{path}?last={last}&n={n}>; rel=\"next\"")
}

async fn blocking<T, F>(context: Context, call: F) -> Result<T, OciError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, OciStoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|error| OciError::internal(&error))?
        .map_err(|error| OciError::from_store(error, context))
}

/// Route 3. Order is byte-wise lexical: `v10` sorts before `v2`.
///
/// # Errors
///
/// `NAME_UNKNOWN`; `PAGINATION_NUMBER_INVALID`.
pub async fn tags(
    store: Arc<OciStore>,
    repo: RepoName,
    query: Option<&str>,
) -> Result<Response, OciError> {
    let Page { n, last } = page(query)?;
    let limit = n.unwrap_or(TAGS_DEFAULT);
    let name = repo.clone();
    let found = blocking(Context::Tags, move || {
        let tags = store.tags_page(&name, last.as_deref(), limit.saturating_add(1))?;
        // The reference knows a repository by its manifests: one that holds
        // blobs only is `NAME_UNKNOWN` here (gate B, every push scenario).
        if tags.is_empty() && last.is_none() && !holds_a_manifest(&store, &name)? {
            return Err(OciStoreError::UnknownRepository(name.as_str().to_owned()));
        }
        Ok(tags)
    })
    .await?;
    let (tags, more) = trim(found, limit);
    let next = match (more, tags.last()) {
        (true, Some(last)) => Some(link_next(&format!("/v2/{repo}/tags/list"), last.as_str(), limit)),
        _ => None,
    };
    let tags: Vec<&str> = tags.iter().map(crate::oci_store::Tag::as_str).collect();
    Ok(respond(
        &json!({ "name": repo.as_str(), "tags": tags }),
        "application/json",
        next,
    ))
}

fn holds_a_manifest(store: &OciStore, repo: &RepoName) -> Result<bool, OciStoreError> {
    let mut after = None;
    loop {
        let page = store.links_page(repo, after.as_ref(), 1000)?;
        if page.iter().any(|(_, link)| link.kind != crate::oci_store::LinkKind::Blob) {
            return Ok(true);
        }
        match page.last() {
            Some((digest, _)) if page.len() == 1000 => after = Some(digest.clone()),
            _ => return Ok(false),
        }
    }
}

/// Route 2.
///
/// # Errors
///
/// `PAGINATION_NUMBER_INVALID`.
pub async fn catalog(store: Arc<OciStore>, query: Option<&str>) -> Result<Response, OciError> {
    let Page { n, last } = page(query)?;
    let limit = n.unwrap_or(CATALOG_DEFAULT);
    let found = blocking(Context::Tags, move || {
        // A repository that holds blobs and no manifest is not in the
        // reference's catalogue (gate B, every push scenario). The scan is per
        // repository; a manifest count kept at write time would remove it.
        let mut out = Vec::new();
        let mut after = last;
        loop {
            let page = store.repos_page(after.as_deref(), 1000)?;
            let exhausted = page.len() < 1000;
            after = page.last().map(|repo| repo.as_str().to_owned());
            for repo in page {
                if holds_a_manifest(&store, &repo)? {
                    out.push(repo);
                    if out.len() > limit {
                        return Ok(out);
                    }
                }
            }
            if exhausted {
                return Ok(out);
            }
        }
    })
    .await?;
    let (repos, more) = trim(found, limit);
    let next = match (more, repos.last()) {
        (true, Some(last)) => Some(link_next("/v2/_catalog", last.as_str(), limit)),
        _ => None,
    };
    let repos: Vec<&str> = repos.iter().map(RepoName::as_str).collect();
    Ok(respond(
        &json!({ "repositories": repos }),
        "application/json",
        next,
    ))
}

/// Route R: an OCI index of the manifests whose `subject` is `digest`. An
/// empty index, never 404, when there are none or the subject is not here
/// yet: a referrer may be pushed before its subject.
///
/// # Errors
///
/// `UNKNOWN` when the store fails.
pub async fn referrers(
    store: Arc<OciStore>,
    repo: RepoName,
    digest: Digest,
    query: Option<&str>,
) -> Result<Response, OciError> {
    let wanted = query_param(query, "artifactType").filter(|wanted| !wanted.is_empty());
    let found = blocking(Context::Manifest, move || store.referrers_of(&repo, &digest)).await?;
    let manifests: Vec<Value> = found
        .into_iter()
        .filter(|referrer| {
            wanted
                .as_ref()
                .is_none_or(|wanted| referrer.artifact_type.as_ref() == Some(wanted))
        })
        .map(|referrer| {
            let mut descriptor = json!({
                "mediaType": referrer.media_type,
                "digest": referrer.digest,
                "size": referrer.size,
            });
            if let Some(artifact_type) = referrer.artifact_type {
                descriptor["artifactType"] = json!(artifact_type);
            }
            if let Some(annotations) = referrer.annotations {
                descriptor["annotations"] = Value::Object(annotations);
            }
            descriptor
        })
        .collect();
    let mut response = respond(
        &json!({ "schemaVersion": 2, "mediaType": OCI_INDEX, "manifests": manifests }),
        OCI_INDEX,
        None,
    );
    if wanted.is_some() {
        response.headers_mut().insert(
            HeaderName::from_static("oci-filters-applied"),
            HeaderValue::from_static("artifactType"),
        );
    }
    *response.status_mut() = StatusCode::OK;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_query() {
        let parsed = page(Some("n=5&last=v1")).expect("page");
        assert_eq!((parsed.n, parsed.last.as_deref()), (Some(5), Some("v1")));
        let parsed = page(None).expect("page");
        assert_eq!((parsed.n, parsed.last), (None, None));
        assert_eq!(page(Some("n=0")).expect("page").n, Some(0));
        for bad in ["n=-1", "n=many", "n="] {
            assert_eq!(
                page(Some(bad)).err().and_then(|error| error.code()),
                Some(ErrorCode::PaginationNumberInvalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn one_extra_item_means_a_next_page() {
        assert_eq!(trim(vec![1, 2, 3], 2), (vec![1, 2], true));
        assert_eq!(trim(vec![1, 2], 2), (vec![1, 2], false));
        assert_eq!(trim(Vec::<u8>::new(), 0), (vec![], false));
        assert_eq!(
            link_next("/v2/a/b/tags/list", "v1", 2),
            "</v2/a/b/tags/list?last=v1&n=2>; rel=\"next\""
        );
    }
}
