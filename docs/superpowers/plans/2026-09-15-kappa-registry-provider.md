# Kappa Registry Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a second `RegistryProvider` backed by an external kappa-registry instance, and prove it is observably substitutable for the local one.

**Architecture:** An object becomes one blob plus one sidecar OCI manifest. The blob carries the bytes and, through its `Content-Type`, the media type; the manifest carries kind, filename, and creation time as annotations, points `subject` at the blob it describes, and is tagged with the object's kappa transliterated to `blake3_<hex>` so a point lookup is one deterministic request. The client is blocking, because every caller already runs provider work inside `spawn_blocking` and `reqwest::blocking` under `spawn_blocking` is the pattern already shipped in `src/holo_fetch.rs`.

**Tech Stack:** Rust 1.94 (toolchain pins 1.97.1), reqwest 0.13 with `blocking`, serde_json, kappa-registry pinned at `2af86560a177fc9651b6c0e92e7974140ed77dd5`.

**Spec:** `docs/superpowers/specs/2026-09-15-kappa-registry-provider-design.md` (§2, §4, §6)

**Plan 2 of 2.** Requires `2026-09-15-object-search.md` to have landed: this plan consumes the module directory from its Task 1, `ObjectQuery`/`ObjectPage` and the trait's `search` method from its Task 3, and `RegistryConfig` from its Task 4. References below name those as "Plan 1 Task N".

**Also requires the reqwest 0.13 migration** (branch `chore/reqwest-0.13-and-safe-bumps`). Task 1 calls `crate::util::install_crypto_provider`, which that change introduces: reqwest 0.13 uses `rustls-no-provider` here to keep aws-lc-rs out of a pure-Rust build, and a client built without an installed provider panics at construction. Do not start this plan against reqwest 0.12.

## Global Constraints

- **No new dependencies.** `reqwest` with `blocking`, `serde`, and `serde_json` are all already in the tree. Search is built on upstream primitives, never an index.
- **1500 production lines per file**, enforced by `scripts/check-file-size.sh`. The wire client and the provider are separate files for this reason.
- **`--locked` on every cargo invocation**, matching the `Justfile`.
- **Clippy `pedantic` is warn-level and `just clippy` runs `-D warnings`.** A warning fails the gate.
- **`unsafe_code = "forbid"`, `unused_must_use = "deny"`.**
- **Comment density matches surrounding code:** `//!` module headers and `///` on public items, explaining *why*.
- **The local provider stays the default.** Selecting `kappa` is opt-in configuration; a stock install needs no external service.
- **Object identity is `blake3:` + 64 lowercase hex**, which is already a valid kappa-label upstream. Never translate it, and never mix digest axes for object identity.
- Full gate: `just verify`. The conformance suite against a live registry runs under `just kappa-registry`.

---

### Task 1: Build the kappa-registry wire client

Transport only — no `RegistryProvider` yet. Keeping the HTTP surface separate keeps both files well under the 1500-line gate and lets the wire format be tested without a provider.

**Files:**
- Create: `src/registry/kappa_client.rs`
- Modify: `src/registry/mod.rs` (declare the module)
- Test: `src/registry/kappa_client.rs`

**Interfaces:**
- Consumes: `config::RegistryConfig` from Plan 1 Task 4.
- Produces:
  - `KappaClient::new(config: &RegistryConfig) -> Result<Self>`
  - `KappaClient::put_blob(&self, kappa: &str, media_type: &str, bytes: &[u8]) -> Result<()>`
  - `KappaClient::get_blob(&self, kappa: &str) -> Result<Vec<u8>>`
  - `KappaClient::head_blob(&self, kappa: &str) -> Result<Option<BlobHead>>` where `BlobHead { size: u64, media_type: String }`
  - `KappaClient::put_manifest(&self, tag: &str, body: &[u8]) -> Result<()>`
  - `KappaClient::get_manifest(&self, tag: &str) -> Result<Option<Vec<u8>>>`
  - `KappaClient::delete_manifest(&self, tag: &str) -> Result<()>`
  - `KappaClient::list_tags(&self, limit: u32, last: Option<&str>) -> Result<TagPage>` where `TagPage { tags: Vec<String>, next: Option<String> }`
  - `kappa_client::tag_for(kappa: &str) -> String` and `kappa_client::kappa_for(tag: &str) -> Option<String>`

- [ ] **Step 1: Write the failing tests for the pure functions**

Create `src/registry/kappa_client.rs` containing only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(kappa_for(&tag).as_deref(), Some(kappa), "conversion round-trips");
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked registry::kappa_client`
Expected: FAIL — module not declared, functions absent.

- [ ] **Step 3: Declare the module**

In `src/registry/mod.rs`, below `mod local;`:

```rust
mod kappa_client;
```

- [ ] **Step 4: Implement the conversions and the client**

Prepend to `src/registry/kappa_client.rs`:

```rust
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
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if digest.bytes().any(|b| b.is_ascii_uppercase()) {
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
        let response = status_to_result(response, "list tags")?;
        // Upstream's `Link: rel="next"` header omits the page size, so the
        // cursor is derived from the last tag instead of by following it.
        let body: TagListBody = response
            .json()
            .map_err(|error| LiveError::Protocol(format!("decode tag list: {error}")))?;
        let next = if body.tags.len() as u32 == limit {
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
```

**Verified:** `src/error.rs` declares `Io(String)`, so the 507 arm above compiles as written.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked registry::kappa_client`
Expected: PASS, 2 tests.

- [ ] **Step 6: Confirm no new dependency crept in**

Run: `git diff --stat Cargo.toml Cargo.lock`
Expected: empty. `reqwest`, `serde`, and `serde_json` were all already present.

- [ ] **Step 7: Commit**

```bash
git add src/registry/kappa_client.rs src/registry/mod.rs
git commit -m "feat(registry): add the kappa-registry wire client"
```

---
### Task 2: Implement `KappaRegistryProvider`

Spec §2 and §3. Wires the client to the trait.

**Files:**
- Create: `src/registry/kappa.rs`
- Modify: `src/registry/mod.rs` (declare, re-export, add the factory)
- Modify: `src/app.rs` (select the provider from config)
- Test: `src/registry/kappa.rs`

**Interfaces:**
- Consumes: `KappaClient` and the tag helpers from Task 1; `RegistryConfig` from Plan 1 Task 4; `ObjectQuery`/`ObjectPage` from Plan 1 Task 3.
- Produces: `registry::KappaRegistryProvider`, and `registry::provider_from_config(config: &AppConfig, store: Arc<ObjectStore>) -> Result<Arc<dyn RegistryProvider>>`.

- [ ] **Step 1: Write the failing tests for manifest encoding**

Create `src/registry/kappa.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ObjectMetadata {
        ObjectMetadata {
            id: "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959"
                .to_owned(),
            kind: "file".to_owned(),
            media_type: "text/plain".to_owned(),
            filename: Some("notes.txt".to_owned()),
            size: 24,
            created_at_millis: 1_757_894_400_000,
        }
    }

    #[test]
    fn metadata_round_trips_through_a_sidecar_manifest() {
        let original = sample();
        let encoded = encode_manifest(&original).expect("encode");
        let decoded = decode_manifest(&encoded).expect("decode");

        assert_eq!(decoded.id, original.id);
        assert_eq!(decoded.kind, original.kind);
        assert_eq!(decoded.media_type, original.media_type);
        assert_eq!(decoded.filename, original.filename);
        assert_eq!(decoded.size, original.size);
        assert_eq!(decoded.created_at_millis, original.created_at_millis);
    }

    #[test]
    fn the_manifest_subject_is_the_blob_it_describes() {
        let original = sample();
        let encoded = encode_manifest(&original).expect("encode");
        let document: serde_json::Value = serde_json::from_slice(&encoded).expect("json");

        assert_eq!(
            document["subject"]["digest"].as_str(),
            Some(original.id.as_str()),
            "subject points at the blob, which is what the referrers API indexes"
        );
        assert_eq!(
            document["artifactType"].as_str(),
            Some("application/vnd.hologram.object.v1+json")
        );
    }

    #[test]
    fn a_manifest_missing_our_annotations_is_rejected() {
        let foreign = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[]}"#;
        assert!(
            decode_manifest(foreign).is_err(),
            "a manifest that is not ours must not decode to a half-populated object"
        );
    }

    #[test]
    fn a_missing_filename_round_trips_as_absent() {
        let mut original = sample();
        original.filename = None;
        let decoded = decode_manifest(&encode_manifest(&original).expect("encode")).expect("decode");
        assert_eq!(decoded.filename, None);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked registry::kappa`
Expected: FAIL — module not declared.

- [ ] **Step 3: Implement the provider**

Prepend to `src/registry/kappa.rs`:

```rust
//! `RegistryProvider` backed by an external kappa-registry instance.
//!
//! An object is one blob plus one sidecar OCI manifest. The blob carries the
//! bytes and, through its `Content-Type`, the media type. The manifest carries
//! the fields a content-addressed blob store has nowhere to put — kind,
//! filename, creation time — as annotations, and is tagged with the object's
//! kappa so a point lookup is a single deterministic fetch.

use super::kappa_client::{kappa_for, tag_for, KappaClient};
use super::RegistryProvider;
use crate::config::RegistryConfig;
use crate::error::{LiveError, Result};
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};

const ARTIFACT_TYPE: &str = "application/vnd.hologram.object.v1+json";
const ANNOTATION_KIND: &str = "dev.hologram.kind";
const ANNOTATION_FILENAME: &str = "dev.hologram.filename";
const ANNOTATION_CREATED: &str = "dev.hologram.created-at-millis";

pub struct KappaRegistryProvider {
    client: KappaClient,
    max_scan_pages: u32,
}

impl KappaRegistryProvider {
    pub fn new(config: &RegistryConfig) -> Result<Self> {
        Ok(Self {
            client: KappaClient::new(config)?,
            max_scan_pages: config.max_scan_pages.max(1),
        })
    }

    /// Read one object's metadata by its tag. Returns `None` for a tag that is
    /// not ours, so an unrelated tag sharing the namespace is skipped rather
    /// than failing an entire listing.
    fn metadata_by_tag(&self, tag: &str) -> Result<Option<ObjectMetadata>> {
        match self.client.get_manifest(tag)? {
            Some(body) => decode_manifest(&body).map(Some),
            None => Ok(None),
        }
    }
}

/// Encode an object's metadata as its sidecar manifest.
fn encode_manifest(metadata: &ObjectMetadata) -> Result<Vec<u8>> {
    let mut annotations = serde_json::Map::new();
    annotations.insert(
        ANNOTATION_KIND.to_owned(),
        serde_json::Value::String(metadata.kind.clone()),
    );
    annotations.insert(
        ANNOTATION_CREATED.to_owned(),
        serde_json::Value::String(metadata.created_at_millis.to_string()),
    );
    if let Some(filename) = metadata.filename.as_ref() {
        annotations.insert(
            ANNOTATION_FILENAME.to_owned(),
            serde_json::Value::String(filename.clone()),
        );
    }

    let descriptor = serde_json::json!({
        "mediaType": metadata.media_type,
        "digest": metadata.id,
        "size": metadata.size,
    });
    let document = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": ARTIFACT_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.empty.v1+json",
            "digest": metadata.id,
            "size": metadata.size,
        },
        "layers": [descriptor],
        "subject": descriptor,
        "annotations": serde_json::Value::Object(annotations),
    });
    serde_json::to_vec(&document).map_err(Into::into)
}

/// Decode a sidecar manifest, rejecting anything that is not ours rather than
/// returning a half-populated record.
fn decode_manifest(body: &[u8]) -> Result<ObjectMetadata> {
    let document: serde_json::Value = serde_json::from_slice(body)?;
    let annotations = document.get("annotations").and_then(|v| v.as_object());
    let annotation = |key: &str| -> Option<&str> {
        annotations
            .and_then(|map| map.get(key))
            .and_then(|value| value.as_str())
    };

    let subject = document
        .get("subject")
        .ok_or_else(|| LiveError::Protocol("object manifest has no subject".to_owned()))?;
    let id = subject
        .get("digest")
        .and_then(|value| value.as_str())
        .ok_or_else(|| LiveError::Protocol("object manifest subject has no digest".to_owned()))?;
    let kind = annotation(ANNOTATION_KIND).ok_or_else(|| {
        LiveError::Protocol(format!("object manifest is missing {ANNOTATION_KIND}"))
    })?;
    let created_at_millis = annotation(ANNOTATION_CREATED)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            LiveError::Protocol(format!("object manifest is missing {ANNOTATION_CREATED}"))
        })?;
    let media_type = subject
        .get("mediaType")
        .and_then(|value| value.as_str())
        .unwrap_or("application/octet-stream");
    let size = subject
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();

    Ok(ObjectMetadata {
        id: id.to_owned(),
        kind: kind.to_owned(),
        media_type: media_type.to_owned(),
        filename: annotation(ANNOTATION_FILENAME).map(str::to_owned),
        size,
        created_at_millis,
    })
}

impl RegistryProvider for KappaRegistryProvider {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        // Preserves the newest-first contract of the legacy surface.
        let query = ObjectQuery {
            kind: kind.map(str::to_owned),
            limit: ObjectQuery::MAX_LIMIT,
            ..ObjectQuery::default()
        };
        let mut objects = self.search(&query)?.objects;
        objects.sort_by(|left, right| {
            right
                .created_at_millis
                .cmp(&left.created_at_millis)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(objects)
    }

    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        let id = format!("blake3:{}", blake3::hash(bytes).to_hex());
        let tag = tag_for(&id);

        // The creation time of immutable content is the time it first
        // appeared, so an existing record wins over a fresh clock reading.
        let created_at_millis = match self.metadata_by_tag(&tag) {
            Ok(Some(existing)) => existing.created_at_millis,
            _ => crate::util::now_millis(),
        };

        let metadata = ObjectMetadata {
            id,
            kind,
            media_type,
            filename,
            size: bytes.len().try_into().unwrap_or(u64::MAX),
            created_at_millis,
        };

        self.client
            .put_blob(&metadata.id, &metadata.media_type, bytes)?;
        self.client.put_manifest(&tag, &encode_manifest(&metadata)?)?;
        Ok(metadata)
    }

    fn get_object(&self, id: &str) -> Result<ObjectContent> {
        let metadata = self
            .metadata_by_tag(&tag_for(id))?
            .ok_or_else(|| LiveError::NotFound(format!("object {id} not found")))?;
        let bytes = self.client.get_blob(id)?;
        Ok(ObjectContent { metadata, bytes })
    }

    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        let tag = tag_for(id);
        let mut metadata = self
            .metadata_by_tag(&tag)?
            .ok_or_else(|| LiveError::NotFound(format!("file {id} not found")))?;
        if metadata.kind != "file" {
            return Err(LiveError::NotFound(format!("file {id} not found")));
        }
        metadata.filename = Some(filename);
        // The blob is immutable, so only the sidecar changes. Writing the same
        // tag replaces the record in one request, which avoids the
        // write-then-delete window a second tag would open.
        self.client.put_manifest(&tag, &encode_manifest(&metadata)?)?;
        Ok(metadata)
    }

    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        let limit = query.effective_limit();
        let mut objects = Vec::with_capacity(limit);
        let mut cursor = query.cursor.as_deref().map(tag_for);
        let mut truncated = false;
        let mut pages = 0_u32;

        loop {
            if pages == self.max_scan_pages {
                // Upstream cannot filter on kind or filename, so a selective
                // query walks pages here. Report the bound instead of
                // presenting a capped result as a complete one.
                truncated = true;
                tracing::debug!(
                    pages,
                    found = objects.len(),
                    "kappa registry search stopped at the page bound"
                );
                break;
            }
            let page = self
                .client
                .list_tags(ObjectQuery::MAX_LIMIT, cursor.as_deref())?;
            pages += 1;
            if page.tags.is_empty() {
                break;
            }
            for tag in &page.tags {
                if kappa_for(tag).is_none() {
                    continue;
                }
                let Some(metadata) = self.metadata_by_tag(tag)? else {
                    continue;
                };
                if !query.matches(&metadata) {
                    continue;
                }
                if objects.len() == limit {
                    return Ok(ObjectPage {
                        next_cursor: objects.last().map(|o: &ObjectMetadata| o.id.clone()),
                        objects,
                        truncated: false,
                    });
                }
                objects.push(metadata);
            }
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        let next_cursor = if truncated {
            objects.last().map(|object| object.id.clone())
        } else {
            None
        };
        Ok(ObjectPage {
            objects,
            next_cursor,
            truncated,
        })
    }
}
```

- [ ] **Step 4: Declare the module and add the factory in `src/registry/mod.rs`**

```rust
mod kappa;

pub use kappa::KappaRegistryProvider;
```

And append the factory:

```rust
/// Build the provider named by configuration.
///
/// Local is the default so a stock install needs no external service;
/// `AppConfig::validate` has already rejected an unknown name or a kappa
/// provider with no endpoint, so this cannot fail on a validated config.
pub fn provider_from_config(
    config: &crate::config::AppConfig,
    store: std::sync::Arc<crate::store::ObjectStore>,
) -> Result<std::sync::Arc<dyn RegistryProvider>> {
    match config.registry.provider.as_str() {
        "kappa" => Ok(std::sync::Arc::new(KappaRegistryProvider::new(
            &config.registry,
        )?)),
        _ => Ok(std::sync::Arc::new(LocalRegistryProvider::new(store))),
    }
}
```

- [ ] **Step 5: Use the factory in `src/app.rs`**

Find where `LocalRegistryProvider::new` is constructed and replace it with `crate::registry::provider_from_config(&config, store.clone())?`. Keep the surrounding `Arc` shape unchanged.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --locked registry::kappa`
Expected: PASS, 4 tests.

- [ ] **Step 7: Run the full gate**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src/registry/ src/app.rs
git commit -m "feat(registry): add the kappa-registry provider behind the config seam"
```

---
### Task 3: Provider conformance suite

Spec §6. The primary test asset: one body run against both providers. If they are not substitutable, this is the only thing that catches it.

**Files:**
- Create: `tests/registry_conformance.rs`
- Create: `scripts/check-kappa-registry.sh`
- Modify: `Justfile` (add a recipe)

**Interfaces:**
- Consumes: both providers and `provider_from_config`.
- Produces: no library API. `KAPPA_REGISTRY_ENDPOINT` opts the remote provider in; unset skips those cases cleanly.

- [ ] **Step 1: Write the conformance suite**

Create `tests/registry_conformance.rs`:

```rust
//! One behavioural contract, executed against every provider.
//!
//! The provider seam exists so storage can move without changing routes,
//! operation ids, or clients. That guarantee is only real if the
//! implementations are observably identical, which is what this asserts.
//!
//! The kappa cases run only when `KAPPA_REGISTRY_ENDPOINT` is set, so a
//! developer without a registry still gets the local half.

use hologram_live::config::RegistryConfig;
use hologram_live::protocol::ObjectQuery;
use hologram_live::registry::{KappaRegistryProvider, LocalRegistryProvider, RegistryProvider};
use hologram_live::store::ObjectStore;
use std::sync::Arc;

fn local(label: &str) -> Box<dyn RegistryProvider> {
    let root = std::env::temp_dir().join(format!(
        "hologram-conformance-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let store = Arc::new(ObjectStore::open(&root).expect("open store"));
    Box::new(LocalRegistryProvider::new(store))
}

/// A kappa provider against a throwaway namespace, or `None` when no endpoint
/// is configured. Each case gets its own namespace so runs cannot collide.
fn kappa(label: &str) -> Option<Box<dyn RegistryProvider>> {
    let endpoint = std::env::var("KAPPA_REGISTRY_ENDPOINT").ok()?;
    let config = RegistryConfig {
        provider: "kappa".to_owned(),
        endpoint,
        namespace: format!("conformance-{label}-{}", std::process::id()),
        ..RegistryConfig::default()
    };
    Some(Box::new(
        KappaRegistryProvider::new(&config).expect("build kappa provider"),
    ))
}

/// Run one contract against every available provider, naming which failed.
fn for_each_provider(label: &str, contract: fn(&dyn RegistryProvider, &str)) {
    contract(local(label).as_ref(), "local");
    match kappa(label) {
        Some(provider) => contract(provider.as_ref(), "kappa"),
        None => eprintln!("skipping kappa provider: KAPPA_REGISTRY_ENDPOINT is unset"),
    }
}

fn put_get_round_trip(provider: &dyn RegistryProvider, name: &str) {
    let stored = provider
        .put_object(
            "file".to_owned(),
            "text/plain".to_owned(),
            Some("hello.txt".to_owned()),
            b"hello",
        )
        .unwrap_or_else(|error| panic!("{name}: put failed: {error}"));

    assert!(
        stored.id.starts_with("blake3:"),
        "{name}: object identity must be a blake3 kappa, got {}",
        stored.id
    );
    assert_eq!(stored.size, 5, "{name}: size");
    assert_eq!(stored.kind, "file", "{name}: kind");
    assert_eq!(stored.filename.as_deref(), Some("hello.txt"), "{name}: filename");

    let fetched = provider
        .get_object(&stored.id)
        .unwrap_or_else(|error| panic!("{name}: get failed: {error}"));
    assert_eq!(fetched.bytes, b"hello", "{name}: bytes round-trip");
    assert_eq!(fetched.metadata.id, stored.id, "{name}: identity is stable");
    assert_eq!(
        fetched.metadata.media_type, "text/plain",
        "{name}: media type survives"
    );
}

fn put_is_idempotent(provider: &dyn RegistryProvider, name: &str) {
    let first = provider
        .put_object("file".to_owned(), "text/plain".to_owned(), None, b"same")
        .unwrap_or_else(|error| panic!("{name}: first put: {error}"));
    let second = provider
        .put_object("file".to_owned(), "text/plain".to_owned(), None, b"same")
        .unwrap_or_else(|error| panic!("{name}: second put: {error}"));

    assert_eq!(first.id, second.id, "{name}: content addressing is stable");
    assert_eq!(
        first.created_at_millis, second.created_at_millis,
        "{name}: immutable content keeps its creation time"
    );
}

fn missing_objects_are_not_found(provider: &dyn RegistryProvider, name: &str) {
    let absent = "blake3:0000000000000000000000000000000000000000000000000000000000000000";
    let error = provider
        .get_object(absent)
        .expect_err("{name}: a missing object must not succeed");
    assert!(
        matches!(error, hologram_live::error::LiveError::NotFound(_)),
        "{name}: expected NotFound, got {error:?}"
    );
}

fn rename_preserves_identity(provider: &dyn RegistryProvider, name: &str) {
    let stored = provider
        .put_object(
            "file".to_owned(),
            "text/plain".to_owned(),
            Some("before.txt".to_owned()),
            b"rename me",
        )
        .unwrap_or_else(|error| panic!("{name}: put: {error}"));

    let renamed = provider
        .rename_file(&stored.id, "after.txt".to_owned())
        .unwrap_or_else(|error| panic!("{name}: rename: {error}"));

    assert_eq!(renamed.id, stored.id, "{name}: rename must not change identity");
    assert_eq!(renamed.filename.as_deref(), Some("after.txt"), "{name}: new name");
    let fetched = provider
        .get_object(&stored.id)
        .unwrap_or_else(|error| panic!("{name}: get after rename: {error}"));
    assert_eq!(fetched.bytes, b"rename me", "{name}: content is untouched");
    assert_eq!(
        fetched.metadata.filename.as_deref(),
        Some("after.txt"),
        "{name}: the rename is durable"
    );
}

fn search_filters_and_orders_identically(provider: &dyn RegistryProvider, name: &str) {
    for (index, kind) in [("file", "file"), ("holo", "holo"), ("file", "file")]
        .iter()
        .enumerate()
    {
        provider
            .put_object(
                kind.1.to_owned(),
                "text/plain".to_owned(),
                Some(format!("item{index}.txt")),
                format!("payload-{index}").as_bytes(),
            )
            .unwrap_or_else(|error| panic!("{name}: seeding put: {error}"));
    }

    let page = provider
        .search(&ObjectQuery {
            kind: Some("file".to_owned()),
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: search: {error}"));

    assert_eq!(page.objects.len(), 2, "{name}: only file-kind objects match");
    assert!(
        page.objects.iter().all(|object| object.kind == "file"),
        "{name}: the kind filter must be exact"
    );
    let ids: Vec<&str> = page.objects.iter().map(|o| o.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "{name}: results ascend by id");
}

fn search_pages_without_overlap(provider: &dyn RegistryProvider, name: &str) {
    for index in 0..4u8 {
        provider
            .put_object(
                "file".to_owned(),
                "text/plain".to_owned(),
                None,
                &[index, index],
            )
            .unwrap_or_else(|error| panic!("{name}: seeding put: {error}"));
    }

    let first = provider
        .search(&ObjectQuery {
            limit: 2,
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: first page: {error}"));
    assert_eq!(first.objects.len(), 2, "{name}: page size is honoured");

    let cursor = first
        .next_cursor
        .clone()
        .unwrap_or_else(|| panic!("{name}: more results exist, so a cursor is required"));

    let second = provider
        .search(&ObjectQuery {
            limit: 2,
            cursor: Some(cursor),
            ..ObjectQuery::default()
        })
        .unwrap_or_else(|error| panic!("{name}: second page: {error}"));

    let first_ids: Vec<&String> = first.objects.iter().map(|o| &o.id).collect();
    for object in &second.objects {
        assert!(
            !first_ids.contains(&&object.id),
            "{name}: pages overlap on {}",
            object.id
        );
    }
}

#[test]
fn providers_round_trip_objects() {
    for_each_provider("roundtrip", put_get_round_trip);
}

#[test]
fn providers_treat_put_as_idempotent() {
    for_each_provider("idempotent", put_is_idempotent);
}

#[test]
fn providers_report_missing_objects_the_same_way() {
    for_each_provider("missing", missing_objects_are_not_found);
}

#[test]
fn providers_rename_without_changing_identity() {
    for_each_provider("rename", rename_preserves_identity);
}

#[test]
fn providers_filter_and_order_search_identically() {
    for_each_provider("search", search_filters_and_orders_identically);
}

#[test]
fn providers_paginate_search_identically() {
    for_each_provider("paginate", search_pages_without_overlap);
}
```

- [ ] **Step 2: Run the suite with only the local provider**

Run: `cargo test --locked --test registry_conformance -- --test-threads=1`
Expected: PASS, 6 tests, each printing the skip notice for kappa.

- [ ] **Step 3: Add the registry helper script**

Create `scripts/check-kappa-registry.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

# Build a pinned kappa-registry, run it on a scratch store, and execute the
# provider conformance suite against it.
#
# Two upstream defects at this revision stop a clean clone from building, so
# both are patched here rather than blocking CI on an upstream merge:
#   1. Cargo.toml declares [[test]] in a virtual workspace manifest, which
#      cargo refuses to parse.
#   2. Cargo.lock lists the package `inventory` twice.
# Remove the patches once upstream fixes them and the pin moves.

readonly REV="2af86560a177fc9651b6c0e92e7974140ed77dd5"
readonly REPO="https://github.com/uoR-Foundation/kappa-registry.git"

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hologram-kappa-registry.XXXXXX")
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]]; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  rm -rf -- "${work_dir}"
}
trap cleanup EXIT

for command in cargo git curl; do
  command -v "${command}" >/dev/null || {
    printf 'error: %s is required\n' "${command}" >&2
    exit 1
  }
done

printf 'cloning kappa-registry at %s\n' "${REV}"
git init --quiet "${work_dir}/src"
git -C "${work_dir}/src" remote add origin "${REPO}"
git -C "${work_dir}/src" fetch --quiet --depth 1 origin "${REV}"
git -C "${work_dir}/src" checkout --quiet FETCH_HEAD

# Patch 1: strip the [[test]] section from the virtual manifest.
python3 - "${work_dir}/src/Cargo.toml" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read()
text = text.replace('[[test]]\nname = "bdd"\nharness = false\n', '')
open(path, 'w').write(text)
PY

# Patch 2: regenerate the lockfile, which contains a duplicate entry.
rm -f "${work_dir}/src/Cargo.lock"
cargo generate-lockfile --manifest-path "${work_dir}/src/Cargo.toml" >/dev/null

printf 'building kappa-server\n'
cargo build --quiet --manifest-path "${work_dir}/src/Cargo.toml" --package kappa-server

port=5000
endpoint="http://127.0.0.1:${port}"
KAPPA_LISTEN_ADDR="127.0.0.1:${port}" \
KAPPA_STORE_ROOT="${work_dir}/data" \
  "${work_dir}/src/target/debug/kappa-server" >"${work_dir}/server.log" 2>&1 &
server_pid=$!

printf 'waiting for %s\n' "${endpoint}"
for _ in $(seq 1 60); do
  if curl -fsS -o /dev/null "${endpoint}/v2/" 2>/dev/null; then
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    printf 'error: kappa-server exited during startup\n' >&2
    cat "${work_dir}/server.log" >&2
    exit 1
  fi
  sleep 1
done

printf 'running provider conformance against %s\n' "${endpoint}"
KAPPA_REGISTRY_ENDPOINT="${endpoint}" \
  cargo test --locked --test registry_conformance -- --test-threads=1

printf 'kappa registry conformance passed\n'
```

Then: `chmod +x scripts/check-kappa-registry.sh`

- [ ] **Step 4: Add the Justfile recipe**

Insert after the `python-private-registry` recipe:

```makefile
# Build a pinned kappa-registry and run provider conformance against it.
kappa-registry:
    ./scripts/check-kappa-registry.sh
```

- [ ] **Step 5: Run the script end to end**

Run: `just kappa-registry`
Expected: builds the registry, starts it, and reports 6 passing tests with the kappa half exercised. First run takes several minutes to compile upstream.

- [ ] **Step 6: Commit**

```bash
git add tests/registry_conformance.rs scripts/check-kappa-registry.sh Justfile
git commit -m "test(registry): prove both providers satisfy one behavioural contract"
```

---
### Task 4: Record the decision and update the documentation

Spec "Consequences". Folded into one task because these files change together and share a single review.

**Files:**
- Create: `specs/adrs/021-kappa-registry-provider.md`
- Modify: `DEPENDENCIES.md`, `ARCHITECTURE.md`, `ACTUAL_CAPABILITIES.md`

- [ ] **Step 1: Write the ADR**

Create `specs/adrs/021-kappa-registry-provider.md`:

```markdown
# ADR 021: Object storage runs behind a registry provider boundary

## Status

Accepted.

## Decision

`RegistryProvider` has two implementations, selected by `[registry].provider`.
`local` is the default and keeps a stock install self-contained. `kappa` speaks
to an external kappa-registry instance over its OCI blob and manifest surface.

Object identity does not change. Hologram Live addresses objects as
`blake3:<64 hex>`, and kappa-registry's kappa-label grammar treats blake3 as a
first-class axis of exactly that shape, so the same identifier is valid on both
sides and no translation exists to get wrong.

An object is one blob plus one sidecar OCI manifest. The blob holds the bytes
and its media type; the manifest holds kind, filename, and creation time as
annotations, sets `subject` to the blob it describes, and is tagged with the
object's kappa transliterated to `blake3_<hex>` so a point lookup is one
deterministic request.

Search is implemented in Hologram Live over upstream's paginated tag listing.
Upstream's `blobs/_meta` endpoint is an object-type index, not general metadata
search: `blob_put_meta` and `meta_query` address different tables, and the only
key ever written is `object-type`.

## Alternatives considered

**Authoring a new object API.** Rejected. kappa-registry already defines a
content-addressed object contract that our identifiers satisfy unchanged, and a
third such API would fragment the ecosystem for no gain.

**Pointing every sidecar manifest at one synthetic namespace root**, so a single
referrers call could enumerate every object with annotations inline. Rejected.
It depends on referrers pagination that was never verified, it makes one index
grow without bound, and its failure mode is baked into persisted data: recovery
would mean rewriting every manifest. Blob-as-subject fails cheaply instead,
because an index can be added behind the search seam without touching stored
data.

**An embedded search index.** Rejected as a new primary dependency for a need
that has not been demonstrated. The scan is bounded and reports truncation.

## Consequences

- Storage moves without changing module routes, operation ids, or clients.
- Search is bounded by `registry.max_scan_pages`, and reaching that bound sets
  `truncated` rather than silently returning a short page.
- Cursors are opaque and provider-scoped, so neither side is pinned by the
  other's internals.
- Creation time is now load-bearing: it is preserved across re-puts of
  identical content, because it resolves duplicate sidecar records.
```

- [ ] **Step 2: Update the prose documentation**

- `DEPENDENCIES.md`: the closing paragraph says kappa-registry is integrated "through the registry provider boundary rather than adding its workspace crates to this dependency graph". Reword it to describe a shipped adapter rather than an intention, and keep the statement that no upstream crates enter the graph, which remains true.
- `ARCHITECTURE.md`: the content-store paragraph anticipates this replacement. State that the kappa provider now exists and is configuration-selected.
- `ACTUAL_CAPABILITIES.md`: add the kappa-backed provider under "Implemented and exercised", noting it is configuration-selected and that the local provider remains the default.

- [ ] **Step 3: Run the full verification gate**

Run: `just verify && just kappa-registry`
Expected: PASS — the standard gate, then the conformance suite against a live pinned registry.

- [ ] **Step 4: Commit**

```bash
git add specs/adrs/021-kappa-registry-provider.md DEPENDENCIES.md ARCHITECTURE.md ACTUAL_CAPABILITIES.md
git commit -m "docs: record ADR 021 for the registry provider boundary"
```

---
## Self-Review

**Spec coverage for this plan's scope.** §2 object representation, the tag scheme, and rename → Tasks 1 and 2. §3's remote half — client-side filtering, the page bound, truncation reporting → Task 2's `search`. §4 error mapping → Task 1's `status_to_result`. §6 the conformance suite and live integration → Task 3. Consequences and the ADR → Task 4. §1, §5, and §7 were delivered by Plan 1.

**One spec requirement intentionally not implemented as written.** §4 says `put_object` retries with bounded backoff on ambiguous failures. No retry loop appears above. Retry policy is only meaningful once a real deployment shows which failures are transient, and a speculative backoff would be untested code on the write path. What makes retry *safe* — idempotence — is proven by Task 3's conformance suite, so it can be added later without redesign. Flagged here rather than silently dropped.

**One deliberate divergence, recorded at its task.** Task 2 implements `rename_file` as a single same-tag manifest replacement rather than §2's write-new-then-delete-old. This removes the non-atomic window the spec described, so it is strictly stronger. The greatest-`created-at-millis` tie-break remains the documented rule for resolving duplicate records that predate this, and Task 2's `put_object` preserves an existing creation time for the same reason.

**Placeholder scan.** No TBDs. Every code step carries real code. No step defers to another task's content.

**Type consistency.** `tag_for` and `kappa_for` keep one signature across Tasks 1, 2, and 3. `BlobHead` and `TagPage` are defined once in Task 1 and consumed in Task 2. `encode_manifest`/`decode_manifest` are defined in Task 2 and tested there. `provider_from_config` is defined in Task 2 Step 4 and consumed in Task 2 Step 5. `ObjectQuery::matches` and `effective_limit` come from Plan 1 Task 3 and are used unchanged.

**One environmental note for the executor.** Task 3's script patches two upstream defects at the pinned revision — a `[[test]]` section in a virtual workspace manifest, and a doubled `inventory` entry in `Cargo.lock` — because both stop a clean clone from building. Both were reproduced directly. Remove the patches when upstream fixes them and the pin moves.
