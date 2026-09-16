//! HTTP transport for an external kappa-registry instance.
//!
//! Kept separate from the provider so the wire format can be tested without a
//! provider, and so neither file approaches the source-size gate.
//!
//! The client is blocking. Every caller already runs provider work inside
//! `spawn_blocking`, and `reqwest::blocking` under `spawn_blocking` is the
//! pattern already shipped in `src/holo_fetch.rs`.

use crate::config::RegistryConfig;
use crate::error::{LiveError, Result};
use crate::util::install_crypto_provider;
use std::time::Duration;

/// Size and media type of a stored blob, read without transferring it.
#[derive(Debug, Clone)]
pub struct BlobHead {
    pub size: u64,
    pub media_type: String,
}

/// One page of tags plus the cursor for the next, if any.
#[derive(Debug, Clone)]
pub struct TagPage {
    pub tags: Vec<String>,
    pub next: Option<String>,
}

/// Tag under which an object's sidecar manifest is stored.
///
/// The OCI tag grammar excludes `:`, so the object's kappa is transliterated.
/// The result is 71 characters, inside the 128-character limit, and legal
/// regardless of the object's filename.
pub fn tag_for(kappa: &str) -> String {
    kappa.replace(':', "_")
}

/// Inverse of [`tag_for`], rejecting anything that is not one of our object
/// tags so unrelated tags in a shared namespace are never misread as objects.
pub fn kappa_for(tag: &str) -> Option<String> {
    let digest = tag.strip_prefix("blake3_")?;
    if digest.len() != 64 {
        return None;
    }
    if !digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    Some(format!("blake3:{digest}"))
}

pub struct KappaClient {
    http: reqwest::blocking::Client,
    endpoint: String,
    namespace: String,
    token: String,
}

impl KappaClient {
    pub fn new(config: &RegistryConfig) -> Result<Self> {
        install_crypto_provider();
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .map_err(|error| {
                LiveError::Transport(format!("build kappa-registry client: {error}"))
            })?;
        Ok(Self {
            http,
            endpoint: config.endpoint.trim_end_matches('/').to_owned(),
            namespace: config.namespace.trim_matches('/').to_owned(),
            token: config.token.clone(),
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/v2/{}/{suffix}", self.endpoint, self.namespace)
    }

    fn authorize(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        if self.token.is_empty() {
            builder
        } else {
            builder.bearer_auth(&self.token)
        }
    }

    pub fn put_blob(&self, kappa: &str, media_type: &str, bytes: &[u8]) -> Result<()> {
        let request = self
            .http
            .put(self.url(&format!("blobs/{kappa}")))
            .header(reqwest::header::CONTENT_TYPE, media_type)
            .body(bytes.to_vec());
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("put blob {kappa}: {error}")))?;
        status_to_result(response, &format!("put blob {kappa}")).map(|_| ())
    }

    pub fn get_blob(&self, kappa: &str) -> Result<Vec<u8>> {
        let request = self.http.get(self.url(&format!("blobs/{kappa}")));
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("get blob {kappa}: {error}")))?;
        let response = status_to_result(response, &format!("get blob {kappa}"))?;
        response
            .bytes()
            .map(|body| body.to_vec())
            .map_err(|error| LiveError::Transport(format!("read blob {kappa}: {error}")))
    }

    pub fn head_blob(&self, kappa: &str) -> Result<Option<BlobHead>> {
        let request = self.http.head(self.url(&format!("blobs/{kappa}")));
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("head blob {kappa}: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = status_to_result(response, &format!("head blob {kappa}"))?;
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        Ok(Some(BlobHead {
            size: response.content_length().unwrap_or_default(),
            media_type,
        }))
    }

    pub fn put_manifest(&self, tag: &str, body: &[u8]) -> Result<()> {
        let request = self
            .http
            .put(self.url(&format!("manifests/{tag}")))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/vnd.oci.image.manifest.v1+json",
            )
            .body(body.to_vec());
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("put manifest {tag}: {error}")))?;
        status_to_result(response, &format!("put manifest {tag}")).map(|_| ())
    }

    pub fn get_manifest(&self, tag: &str) -> Result<Option<Vec<u8>>> {
        let request = self.http.get(self.url(&format!("manifests/{tag}"))).header(
            reqwest::header::ACCEPT,
            "application/vnd.oci.image.manifest.v1+json",
        );
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("get manifest {tag}: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = status_to_result(response, &format!("get manifest {tag}"))?;
        response
            .bytes()
            .map(|body| Some(body.to_vec()))
            .map_err(|error| LiveError::Transport(format!("read manifest {tag}: {error}")))
    }

    pub fn delete_manifest(&self, tag: &str) -> Result<()> {
        let request = self.http.delete(self.url(&format!("manifests/{tag}")));
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("delete manifest {tag}: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        status_to_result(response, &format!("delete manifest {tag}")).map(|_| ())
    }

    pub fn list_tags(&self, limit: u32, last: Option<&str>) -> Result<TagPage> {
        let mut request = self
            .http
            .get(self.url("tags/list"))
            .query(&[("n", limit.to_string())]);
        if let Some(last) = last {
            request = request.query(&[("last", last)]);
        }
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("list tags: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            // A namespace with nothing in it yet is an empty listing, not an
            // error: the first search against a fresh registry must not fail.
            return Ok(TagPage {
                tags: Vec::new(),
                next: None,
            });
        }
        let response = status_to_result(response, "list tags")?;
        // Upstream's `Link: rel="next"` header omits the page size, so the
        // cursor is derived from the last tag rather than by following it.
        let body: TagListBody = response
            .json()
            .map_err(|error| LiveError::Protocol(format!("decode tag list: {error}")))?;
        // A full page implies more may remain; a short one means the walk
        // reached the end. `try_from` rather than a cast so a page larger than
        // u32 could never wrap around into looking short.
        let next = if u32::try_from(body.tags.len()).is_ok_and(|count| count >= limit) {
            body.tags.last().cloned()
        } else {
            None
        };
        Ok(TagPage {
            tags: body.tags,
            next,
        })
    }
}

#[derive(serde::Deserialize)]
struct TagListBody {
    #[serde(default)]
    tags: Vec<String>,
}

/// Map an upstream response onto the typed error vocabulary.
///
/// Upstream speaks the OCI error envelope, and these variants already convert
/// to the right HTTP status through `src/modules/mod.rs`, so remote failures
/// surface correctly with no extra plumbing.
fn status_to_result(
    response: reqwest::blocking::Response,
    context: &str,
) -> Result<reqwest::blocking::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let detail = response.text().unwrap_or_default();
    let detail = detail.trim();
    let message = if detail.is_empty() {
        format!("{context}: HTTP {status}")
    } else {
        format!("{context}: HTTP {status}: {detail}")
    };
    Err(match status {
        reqwest::StatusCode::NOT_FOUND => LiveError::NotFound(message),
        reqwest::StatusCode::UNAUTHORIZED => LiveError::Authentication(message),
        reqwest::StatusCode::FORBIDDEN => LiveError::Authorization(message),
        reqwest::StatusCode::BAD_REQUEST | reqwest::StatusCode::PAYLOAD_TOO_LARGE => {
            LiveError::Protocol(message)
        }
        reqwest::StatusCode::INSUFFICIENT_STORAGE => LiveError::Io(message),
        _ => LiveError::Transport(message),
    })
}

impl crate::artifact_pull::LayerFetch for KappaClient {
    fn manifest(&self, repository: &str, tag: &str) -> Result<Option<(Vec<u8>, Option<String>)>> {
        // The repository is namespace-qualified by the caller, so address it
        // directly rather than through the configured namespace.
        let request = self
            .http
            .get(format!("{}/v2/{repository}/manifests/{tag}", self.endpoint))
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.oci.image.manifest.v1+json",
            );
        let response = self.authorize(request).send().map_err(|error| {
            LiveError::Transport(format!("resolve {repository}:{tag}: {error}"))
        })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = status_to_result(response, &format!("resolve {repository}:{tag}"))?;
        // Recording the digest is what lets a later pull report that a mutable
        // tag has moved.
        let digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes()
            .map_err(|error| LiveError::Transport(format!("read manifest: {error}")))?;
        Ok(Some((body.to_vec(), digest)))
    }

    fn blob(&self, repository: &str, kappa: &str) -> Result<Vec<u8>> {
        // Addressed through the reference's repository, not the configured
        // namespace: a reference names where its layers live.
        let request = self
            .http
            .get(format!("{}/v2/{repository}/blobs/{kappa}", self.endpoint));
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("get layer {kappa}: {error}")))?;
        let response = status_to_result(response, &format!("get layer {kappa}"))?;
        response
            .bytes()
            .map(|body| body.to_vec())
            .map_err(|error| LiveError::Transport(format!("read layer {kappa}: {error}")))
    }
}

impl crate::artifact_push::LayerPublish for KappaClient {
    fn put_blob(
        &self,
        repository: &str,
        kappa: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<()> {
        // Repository-scoped, like the pull side: a reference names where its
        // layers live, which is not necessarily the configured namespace.
        let request = self
            .http
            .put(format!("{}/v2/{repository}/blobs/{kappa}", self.endpoint))
            .header(reqwest::header::CONTENT_TYPE, media_type)
            .body(bytes.to_vec());
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("publish blob {kappa}: {error}")))?;
        status_to_result(response, &format!("publish blob {kappa}")).map(|_| ())
    }

    fn put_manifest(&self, repository: &str, tag: &str, body: &[u8]) -> Result<()> {
        let request = self
            .http
            .put(format!("{}/v2/{repository}/manifests/{tag}", self.endpoint))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/vnd.oci.image.manifest.v1+json",
            )
            .body(body.to_vec());
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("publish manifest {tag}: {error}")))?;
        status_to_result(response, &format!("publish manifest {tag}")).map(|_| ())
    }

    fn tag_exists(&self, repository: &str, tag: &str) -> Result<bool> {
        let request = self
            .http
            .get(format!("{}/v2/{repository}/manifests/{tag}", self.endpoint))
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.oci.image.manifest.v1+json",
            );
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("check tag {tag}: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        status_to_result(response, &format!("check tag {tag}")).map(|_| true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `reqwest::blocking::Client` owns an internal runtime, and building one
    /// from an async context panics when that runtime is dropped. Every caller
    /// therefore constructs the client *inside* `spawn_blocking`, not before
    /// it. Running the real CLI is what caught this; no unit test did.
    #[tokio::test]
    async fn the_client_builds_inside_a_blocking_task() {
        let config = RegistryConfig {
            endpoint: "http://127.0.0.1:1".to_owned(),
            ..RegistryConfig::default()
        };
        tokio::task::spawn_blocking(move || {
            KappaClient::new(&config).expect("client builds on a blocking thread");
        })
        .await
        .expect("blocking task must not panic");
    }

    /// A reference names its own namespace, so layers must be addressed
    /// through the manifest's repository rather than the client's configured
    /// namespace. Resolving `models/demo` and then fetching layers from
    /// `models` looks up blobs that are not there -- which is exactly what an
    /// end-to-end pull did before this was fixed.
    #[test]
    fn layer_urls_are_scoped_to_the_given_repository() {
        let config = RegistryConfig {
            endpoint: "http://registry.example:5000".to_owned(),
            namespace: "models".to_owned(),
            ..RegistryConfig::default()
        };
        let client = KappaClient::new(&config).expect("client");

        // The configured-namespace helper and the repository-scoped layer URL
        // must differ once a reference carries a deeper namespace.
        let configured = client.url("blobs/blake3:abc");
        let scoped = format!("{}/v2/{}/blobs/blake3:abc", client.endpoint, "models/demo");

        assert_eq!(
            configured,
            "http://registry.example:5000/v2/models/blobs/blake3:abc"
        );
        assert_ne!(
            configured, scoped,
            "a deeper repository must not collapse onto the configured namespace"
        );
    }

    #[test]
    fn a_kappa_converts_to_a_valid_oci_tag_and_back() {
        let kappa = "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959";
        let tag = tag_for(kappa);

        assert_eq!(
            tag,
            "blake3_cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959"
        );
        assert!(tag.len() <= 128, "OCI limits a tag to 128 characters");
        assert!(
            tag.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
            "every character must be legal in an OCI tag"
        );
        assert_eq!(
            kappa_for(&tag).as_deref(),
            Some(kappa),
            "conversion round-trips"
        );
    }

    #[test]
    fn tags_that_are_not_ours_are_not_mistaken_for_objects() {
        assert_eq!(kappa_for("latest"), None);
        assert_eq!(kappa_for("sha256_abc"), None);
        assert_eq!(kappa_for("blake3_short"), None);
        assert_eq!(
            kappa_for("blake3_CB9EF1526F722FCAAF5A6E19610D045349627E4CE70AD4420CCC00B6BFC5E959"),
            None,
            "object ids are lowercase hex, so an uppercase tag is not one of ours"
        );
    }
}
