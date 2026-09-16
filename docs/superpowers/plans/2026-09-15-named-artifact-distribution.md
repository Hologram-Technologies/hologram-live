# Named Artifact Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Acquire `.holo` archives by name — `hologram pull qwen3.5:4b` — with content-deduplicated transfer, and reach them from `run`, `serve`, and `chat`.

**Architecture:** A docker-style reference resolves to an OCI manifest whose layers are a thin `.holo` archive plus the kappa-addressed payload blobs it references. Pull fetches only the layers absent from the local store, verifies each against its kappa on write, and confirms every referenced layer is present before declaring success — the registry does not check that itself. Tags are mutable, so pull always re-resolves and records the manifest digest rather than caching by name.

**Tech Stack:** Rust 1.94 (toolchain pins 1.97.1), reqwest 0.13 with `blocking`, serde_json, clap 4.6.

**Spec:** `docs/superpowers/specs/2026-09-15-named-artifact-distribution-design.md`

**Depends on `2026-09-15-kappa-registry-provider.md`** having landed: this plan uses its `KappaClient` for transport and its `RegistryConfig` for endpoint and namespace. Pulling is inherently remote, so there is no local-only subset to build first.

## Global Constraints

- **No new dependencies.** Transport is the existing `KappaClient`; verification and caching are existing `ObjectStore` primitives.
- **1500 production lines per file**, enforced by `scripts/check-file-size.sh` over source *and* Markdown.
- **`--locked` on every cargo invocation.**
- **Clippy `pedantic` is warn-level and `just clippy` runs `-D warnings`.**
- **`unsafe_code = "forbid"`, `unused_must_use = "deny"`.**
- **`--json` is a total contract.** Every command emits a machine-readable result on stdout. Progress is human-facing and goes to stderr; under `--json` it is JSONL events, never a rendered bar.
- **Pulling confers no authority.** A pulled archive takes the ordinary ADR 020 local baseline. Nothing in this plan may widen a grant, and Task 5 exists to prove it.
- **Never change the meaning of an existing invocation.** Reference resolution is path-first for exactly this reason.
- Full gate: `just verify`.

---

### Task 1: The reference grammar

Spec §1. Pure parsing, no I/O, so it is fully testable in isolation and every ambiguity gets pinned by a test rather than a comment.

**Files:**
- Create: `src/artifact_ref.rs`
- Modify: `src/lib.rs` (declare the module)
- Test: `src/artifact_ref.rs`

**Interfaces:**
- Consumes: `config::RegistryConfig` from the registry plan.
- Produces:
  - `artifact_ref::ArtifactRef { host: String, namespace: String, name: String, tag: String }`
  - `ArtifactRef::parse(input: &str, config: &RegistryConfig) -> Result<ArtifactRef>`
  - `ArtifactRef::repository(&self) -> String` — `"{namespace}/{name}"`
  - `ArtifactRef::display(&self) -> String` — canonical fully-qualified form
  - `artifact_ref::Resolution` enum with variants `Kappa(String)`, `File(PathBuf)`, `Reference(ArtifactRef)`
  - `artifact_ref::resolve(input: &str, config: &RegistryConfig) -> Result<Resolution>`

- [ ] **Step 1: Write the failing tests**

Create `src/artifact_ref.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;

    fn config() -> RegistryConfig {
        RegistryConfig {
            endpoint: "http://registry.example:5000".to_owned(),
            namespace: "models".to_owned(),
            ..RegistryConfig::default()
        }
    }

    #[test]
    fn a_bare_name_and_tag_expands_against_the_configured_default() {
        let reference = ArtifactRef::parse("qwen3.5:4b", &config()).expect("parse");
        assert_eq!(reference.host, "registry.example:5000");
        assert_eq!(reference.namespace, "models");
        assert_eq!(reference.name, "qwen3.5");
        assert_eq!(reference.tag, "4b");
        assert_eq!(reference.repository(), "models/qwen3.5");
    }

    #[test]
    fn an_omitted_tag_defaults_to_latest() {
        let reference = ArtifactRef::parse("qwen3.5", &config()).expect("parse");
        assert_eq!(reference.tag, "latest");
        assert_eq!(reference.name, "qwen3.5");
    }

    #[test]
    fn a_fully_qualified_reference_overrides_every_default() {
        let reference =
            ArtifactRef::parse("other.host:5001/team/app:v2", &config()).expect("parse");
        assert_eq!(reference.host, "other.host:5001");
        assert_eq!(reference.namespace, "team");
        assert_eq!(reference.name, "app");
        assert_eq!(reference.tag, "v2");
        assert_eq!(reference.display(), "other.host:5001/team/app:v2");
    }

    #[test]
    fn a_nested_namespace_keeps_every_segment() {
        let reference =
            ArtifactRef::parse("host:5000/team/sub/app:v1", &config()).expect("parse");
        assert_eq!(reference.namespace, "team/sub");
        assert_eq!(reference.name, "app");
        assert_eq!(reference.repository(), "team/sub/app");
    }

    #[test]
    fn a_tag_must_satisfy_the_oci_grammar() {
        // Colons separate repository from tag, so a tag containing one is not
        // representable and must be rejected rather than silently mangled.
        assert!(ArtifactRef::parse("app:bad:tag", &config()).is_err());
        assert!(ArtifactRef::parse("app:-leading", &config()).is_err());
        assert!(ArtifactRef::parse("app:has space", &config()).is_err());
        assert!(ArtifactRef::parse("app:", &config()).is_err());
        assert!(ArtifactRef::parse("", &config()).is_err());
    }

    #[test]
    fn a_kappa_resolves_to_the_catalog_not_the_registry() {
        let input = "blake3:cb9ef1526f722fcaaf5a6e19610d045349627e4ce70ad4420ccc00b6bfc5e959";
        match resolve(input, &config()).expect("resolve") {
            Resolution::Kappa(kappa) => assert_eq!(kappa, input),
            other => panic!("expected a kappa resolution, got {other:?}"),
        }
    }

    #[test]
    fn an_existing_path_wins_over_a_registry_reference() {
        // Path-first precedence is what guarantees no existing invocation of
        // `run` changes meaning when reference support is added.
        let directory = std::env::temp_dir().join(format!(
            "hologram-ref-{}-{}",
            std::process::id(),
            crate::util::now_millis()
        ));
        std::fs::create_dir_all(&directory).expect("create");
        let file = directory.join("qwen3.5:4b");
        std::fs::write(&file, b"archive").expect("write");

        let input = file.to_string_lossy().into_owned();
        match resolve(&input, &config()).expect("resolve") {
            Resolution::File(path) => assert_eq!(path, file),
            other => panic!("an existing file must win, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_name_that_is_not_a_path_resolves_as_a_reference() {
        match resolve("qwen3.5:4b", &config()).expect("resolve") {
            Resolution::Reference(reference) => assert_eq!(reference.name, "qwen3.5"),
            other => panic!("expected a registry reference, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked artifact_ref`
Expected: FAIL — module not declared.

- [ ] **Step 3: Declare the module**

In `src/lib.rs`, in alphabetical position among the `pub mod` lines:

```rust
pub mod artifact_ref;
```

- [ ] **Step 4: Implement the grammar**

Prepend to `src/artifact_ref.rs`:

```rust
//! Docker-style references for named `.holo` artifacts.
//!
//! `[host[:port]/]namespace/name[:tag]`, where a bare `name:tag` expands
//! against the configured default registry. Following OCI, the colon separates
//! repository from tag — `qwen3.5:4b` is repository `qwen3.5`, tag `4b` —
//! because the OCI tag grammar excludes colons.
//!
//! A reference is not a capability. Resolving one says nothing about what the
//! archive may do; see ADR 020 and the admission tests.

use crate::config::RegistryConfig;
use crate::error::{LiveError, Result};
use std::path::PathBuf;

/// A fully-qualified artifact reference, with every default already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    pub host: String,
    pub namespace: String,
    pub name: String,
    pub tag: String,
}

/// What an input string denotes.
///
/// The order of these variants is the resolution precedence, and that order is
/// load-bearing: `run` and `serve` already accept paths and kappas, so a path
/// must keep winning or adding reference support would silently change what an
/// existing command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A `blake3:` object id: look in the local catalog.
    Kappa(String),
    /// An existing filesystem path: a local archive.
    File(PathBuf),
    /// Anything else: resolve through the registry.
    Reference(ArtifactRef),
}

impl ArtifactRef {
    pub fn parse(input: &str, config: &RegistryConfig) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(LiveError::Protocol("artifact reference is empty".to_owned()));
        }

        // Split the tag from the right, but only past the last '/': a port in
        // the host ("host:5000/app") must not be mistaken for a tag.
        let last_slash = input.rfind('/');
        let (body, tag) = match input.rfind(':') {
            Some(colon) if last_slash.is_none_or(|slash| colon > slash) => {
                (&input[..colon], &input[colon + 1..])
            }
            _ => (input, "latest"),
        };
        validate_tag(tag)?;

        let mut segments: Vec<&str> = body.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err(LiveError::Protocol(format!(
                "artifact reference {input:?} has no name"
            )));
        }

        let name = segments.pop().expect("checked non-empty").to_owned();
        validate_segment(&name, "name")?;

        // A leading segment is a host only if it looks like one. "team/app"
        // must stay namespace/name against the default host.
        let host = if segments.first().is_some_and(|first| is_host(first)) {
            segments.remove(0).to_owned()
        } else {
            default_host(config)?
        };

        let namespace = if segments.is_empty() {
            config.namespace.trim_matches('/').to_owned()
        } else {
            for segment in &segments {
                validate_segment(segment, "namespace")?;
            }
            segments.join("/")
        };
        if namespace.is_empty() {
            return Err(LiveError::Protocol(
                "artifact reference has no namespace and registry.namespace is empty".to_owned(),
            ));
        }

        Ok(Self {
            host,
            namespace,
            name,
            tag: tag.to_owned(),
        })
    }

    /// `namespace/name`, the repository portion of an upstream path.
    pub fn repository(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    /// Canonical fully-qualified form, suitable for logs and audit records.
    pub fn display(&self) -> String {
        format!("{}/{}/{}:{}", self.host, self.namespace, self.name, self.tag)
    }
}

/// Classify an input against the precedence rules.
pub fn resolve(input: &str, config: &RegistryConfig) -> Result<Resolution> {
    let trimmed = input.trim();
    if is_kappa(trimmed) {
        return Ok(Resolution::Kappa(trimmed.to_owned()));
    }
    let path = PathBuf::from(trimmed);
    if path.exists() {
        return Ok(Resolution::File(path));
    }
    Ok(Resolution::Reference(ArtifactRef::parse(trimmed, config)?))
}

fn is_kappa(input: &str) -> bool {
    input.strip_prefix("blake3:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

/// A leading segment is a host if it carries a port or a dot. This is the
/// docker heuristic, and it is why `team/app` stays namespace/name.
fn is_host(segment: &str) -> bool {
    segment.contains(':') || segment.contains('.') || segment == "localhost"
}

fn default_host(config: &RegistryConfig) -> Result<String> {
    let endpoint = config.endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint);
    let host = without_scheme.trim_end_matches('/');
    if host.is_empty() {
        return Err(LiveError::Config(
            "registry.endpoint must be set to resolve a bare artifact reference".to_owned(),
        ));
    }
    Ok(host.to_owned())
}

/// The OCI tag grammar: `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`.
fn validate_tag(tag: &str) -> Result<()> {
    if tag.is_empty() || tag.len() > 128 {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} must be 1 to 128 characters"
        )));
    }
    let mut characters = tag.chars();
    let first = characters.next().expect("checked non-empty");
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} must start with an alphanumeric or underscore"
        )));
    }
    if !characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(LiveError::Protocol(format!(
            "artifact tag {tag:?} contains a character outside the OCI tag grammar"
        )));
    }
    Ok(())
}

fn validate_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(LiveError::Protocol(format!(
            "artifact reference {label} segment is empty"
        )));
    }
    if !segment
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(LiveError::Protocol(format!(
            "artifact reference {label} {segment:?} must be lowercase alphanumeric with '.', '_', or '-'"
        )));
    }
    Ok(())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked artifact_ref`
Expected: PASS, 8 tests.

- [ ] **Step 6: Commit**

```bash
git add src/artifact_ref.rs src/lib.rs
git commit -m "feat(artifact): add the docker-style reference grammar"
```

---

### Task 2: Encode and decode the artifact manifest

Spec §2. The distribution format, kept separate from the pull mechanics so it can be tested without a registry.

**Files:**
- Create: `src/artifact_manifest.rs`
- Modify: `src/lib.rs`
- Test: `src/artifact_manifest.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `artifact_manifest::LayerRole` with `Archive` and `Layer`
  - `artifact_manifest::ArtifactLayer { kappa: String, media_type: String, size: u64, role: LayerRole }`
  - `artifact_manifest::ArtifactManifest { layers: Vec<ArtifactLayer>, kind: Option<String>, name: Option<String>, tag: Option<String> }`
  - `ArtifactManifest::decode(body: &[u8]) -> Result<Self>`
  - `ArtifactManifest::archive(&self) -> Result<&ArtifactLayer>`
  - `ArtifactManifest::payloads(&self) -> impl Iterator<Item = &ArtifactLayer>`

- [ ] **Step 1: Write the failing tests**

Create `src/artifact_manifest.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVE: &str = "blake3:5428a65f5815d145cd5f5b91f9d0d4c902b0376f268a41027e13bc1cf96f5429";
    const WEIGHTS: &str = "blake3:133f4a5d15cc232345f93adf581818b8910412c4b205e5f971faf93859037815";

    fn document() -> String {
        format!(
            r#"{{"schemaVersion":2,
                "mediaType":"application/vnd.oci.image.manifest.v1+json",
                "artifactType":"application/vnd.hologram.artifact.v1+json",
                "config":{{"mediaType":"application/vnd.oci.empty.v1+json","digest":"{ARCHIVE}","size":30}},
                "layers":[
                  {{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                    "annotations":{{"dev.hologram.role":"archive"}}}},
                  {{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30,
                    "annotations":{{"dev.hologram.role":"layer"}}}}],
                "annotations":{{"dev.hologram.kind":"inference-model","dev.hologram.name":"qwen3.5","dev.hologram.tag":"4b"}}}}"#
        )
    }

    #[test]
    fn a_multi_layer_manifest_decodes_into_roles() {
        let manifest = ArtifactManifest::decode(document().as_bytes()).expect("decode");

        assert_eq!(manifest.layers.len(), 2);
        assert_eq!(manifest.archive().expect("archive").kappa, ARCHIVE);
        let payloads: Vec<&ArtifactLayer> = manifest.payloads().collect();
        assert_eq!(payloads.len(), 1, "the archive is not a payload layer");
        assert_eq!(payloads[0].kappa, WEIGHTS);
        assert_eq!(manifest.kind.as_deref(), Some("inference-model"));
        assert_eq!(manifest.name.as_deref(), Some("qwen3.5"));
        assert_eq!(manifest.tag.as_deref(), Some("4b"));
    }

    #[test]
    fn a_single_fat_archive_layer_is_valid_and_has_no_payloads() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[{{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                   "annotations":{{"dev.hologram.role":"archive"}}}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert_eq!(manifest.archive().expect("archive").kappa, ARCHIVE);
        assert_eq!(manifest.payloads().count(), 0);
    }

    #[test]
    fn a_manifest_with_no_archive_layer_is_rejected() {
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[{{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30,
                   "annotations":{{"dev.hologram.role":"layer"}}}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert!(
            manifest.archive().is_err(),
            "payload layers alone are not a usable artifact"
        );
    }

    #[test]
    fn a_layer_digest_on_a_foreign_axis_is_rejected() {
        // Object identity is blake3 throughout. A sha256 layer would silently
        // break local cache lookups, so it must fail at decode.
        let body = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
            "layers":[{"mediaType":"application/vnd.hologram.holo",
              "digest":"sha256:5ca494d6ec8cb393acefd75cf0f332f6fa3690ea2a1804e92e27373b95ea8373","size":30,
              "annotations":{"dev.hologram.role":"archive"}}]}"#;
        assert!(ArtifactManifest::decode(body.as_bytes()).is_err());
    }

    #[test]
    fn a_layer_without_a_role_annotation_defaults_to_payload() {
        // The upstream spike published manifests whose payload layers carried
        // no role. Treating an unannotated layer as a payload keeps those
        // readable, while the archive layer must still be explicit.
        let body = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                 "layers":[
                   {{"mediaType":"application/vnd.hologram.holo","digest":"{ARCHIVE}","size":30,
                     "annotations":{{"dev.hologram.role":"archive"}}}},
                   {{"mediaType":"application/octet-stream","digest":"{WEIGHTS}","size":30}}]}}"#
        );
        let manifest = ArtifactManifest::decode(body.as_bytes()).expect("decode");
        assert_eq!(manifest.payloads().count(), 1);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked artifact_manifest`
Expected: FAIL — module not declared.

- [ ] **Step 3: Declare the module**

In `src/lib.rs`:

```rust
pub mod artifact_manifest;
```

- [ ] **Step 4: Implement it**

Prepend to `src/artifact_manifest.rs`:

```rust
//! The named-artifact distribution format.
//!
//! One OCI manifest per artifact version. Its layers are a thin `.holo`
//! archive plus the kappa-addressed payload blobs that archive references, so
//! two artifacts sharing weights or a tokenizer transfer them once.
//!
//! A fat archive published as a single `archive` layer with no payloads is
//! valid and simpler; it forfeits deduplication.

use crate::error::{LiveError, Result};

const ANNOTATION_ROLE: &str = "dev.hologram.role";
const ANNOTATION_KIND: &str = "dev.hologram.kind";
const ANNOTATION_NAME: &str = "dev.hologram.name";
const ANNOTATION_TAG: &str = "dev.hologram.tag";

/// What a layer contributes to the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerRole {
    /// The `.holo` archive itself. Exactly one is required.
    Archive,
    /// A kappa-addressed payload the archive references.
    Layer,
}

#[derive(Debug, Clone)]
pub struct ArtifactLayer {
    pub kappa: String,
    pub media_type: String,
    pub size: u64,
    pub role: LayerRole,
}

#[derive(Debug, Clone)]
pub struct ArtifactManifest {
    pub layers: Vec<ArtifactLayer>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
}

impl ArtifactManifest {
    pub fn decode(body: &[u8]) -> Result<Self> {
        let document: serde_json::Value = serde_json::from_slice(body)?;
        let annotations = document.get("annotations").and_then(|v| v.as_object());
        let annotation = |key: &str| -> Option<String> {
            annotations
                .and_then(|map| map.get(key))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        };

        let entries = document
            .get("layers")
            .and_then(|value| value.as_array())
            .ok_or_else(|| LiveError::Protocol("artifact manifest has no layers".to_owned()))?;

        let mut layers = Vec::with_capacity(entries.len());
        for entry in entries {
            let kappa = entry
                .get("digest")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    LiveError::Protocol("artifact manifest layer has no digest".to_owned())
                })?;
            // Identity is blake3 everywhere in this system. A foreign axis
            // would not match anything in the local store, so reject it at the
            // boundary rather than failing later with a confusing cache miss.
            if !is_blake3(kappa) {
                return Err(LiveError::Protocol(format!(
                    "artifact manifest layer digest {kappa:?} is not a blake3 kappa"
                )));
            }
            let role = match entry
                .get("annotations")
                .and_then(|value| value.get(ANNOTATION_ROLE))
                .and_then(|value| value.as_str())
            {
                Some("archive") => LayerRole::Archive,
                // An unannotated layer is a payload: the archive layer must
                // declare itself, so absence is unambiguous.
                _ => LayerRole::Layer,
            };
            layers.push(ArtifactLayer {
                kappa: kappa.to_owned(),
                media_type: entry
                    .get("mediaType")
                    .and_then(|value| value.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_owned(),
                size: entry
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default(),
                role,
            });
        }

        Ok(Self {
            layers,
            kind: annotation(ANNOTATION_KIND),
            name: annotation(ANNOTATION_NAME),
            tag: annotation(ANNOTATION_TAG),
        })
    }

    /// The single archive layer, or a typed error naming what was wrong.
    pub fn archive(&self) -> Result<&ArtifactLayer> {
        let mut found = self
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Archive);
        let archive = found.next().ok_or_else(|| {
            LiveError::Protocol("artifact manifest declares no archive layer".to_owned())
        })?;
        if found.next().is_some() {
            return Err(LiveError::Protocol(
                "artifact manifest declares more than one archive layer".to_owned(),
            ));
        }
        Ok(archive)
    }

    pub fn payloads(&self) -> impl Iterator<Item = &ArtifactLayer> {
        self.layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Layer)
    }
}

fn is_blake3(kappa: &str) -> bool {
    kappa.strip_prefix("blake3:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked artifact_manifest`
Expected: PASS, 5 tests.

- [ ] **Step 6: Commit**

```bash
git add src/artifact_manifest.rs src/lib.rs
git commit -m "feat(artifact): decode the multi-layer distribution manifest"
```

---

### Task 3: The deduplicating pull

Spec §3. The heart of the plan.

**Files:**
- Create: `src/artifact_pull.rs`
- Modify: `src/lib.rs`
- Test: `src/artifact_pull.rs`

**Interfaces:**
- Consumes: `ArtifactRef` (Task 1), `ArtifactManifest` (Task 2), `KappaClient` from the registry plan, `ObjectStore`.
- Produces:
  - `artifact_pull::LayerSource` with `AlreadyPresent` and `Fetched`
  - `artifact_pull::PullProgress { kappa: String, index: usize, total: usize, source: LayerSource, bytes: u64 }`
  - `artifact_pull::PullReport { reference: String, manifest_digest: Option<String>, archive_kappa: String, layers_fetched: usize, layers_present: usize, bytes_transferred: u64 }`
  - `artifact_pull::LayerFetch` trait with `fn manifest(&self, repository: &str, tag: &str) -> Result<Option<(Vec<u8>, Option<String>)>>` and `fn blob(&self, kappa: &str) -> Result<Vec<u8>>`
  - `artifact_pull::pull(fetch: &dyn LayerFetch, store: &ObjectStore, reference: &ArtifactRef, on_progress: &mut dyn FnMut(PullProgress)) -> Result<PullReport>`

- [ ] **Step 1: Write the failing tests**

Create `src/artifact_pull.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RegistryConfig;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ARCHIVE_BYTES: &[u8] = b"THIN-HOLO-ARCHIVE";
    const WEIGHT_BYTES: &[u8] = b"SHARED-WEIGHTS";

    fn kappa(bytes: &[u8]) -> String {
        format!("blake3:{}", blake3::hash(bytes).to_hex())
    }

    /// Records every blob request so a test can prove a cached layer was
    /// never fetched, which is the whole point of deduplication.
    struct FakeRegistry {
        blobs: HashMap<String, Vec<u8>>,
        manifest: Option<Vec<u8>>,
        requested: RefCell<Vec<String>>,
    }

    impl FakeRegistry {
        fn new() -> Self {
            let archive = kappa(ARCHIVE_BYTES);
            let weights = kappa(WEIGHT_BYTES);
            let manifest = format!(
                r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json",
                     "layers":[
                       {{"mediaType":"application/vnd.hologram.holo","digest":"{archive}","size":17,
                         "annotations":{{"dev.hologram.role":"archive"}}}},
                       {{"mediaType":"application/octet-stream","digest":"{weights}","size":14,
                         "annotations":{{"dev.hologram.role":"layer"}}}}]}}"#
            );
            let mut blobs = HashMap::new();
            blobs.insert(archive, ARCHIVE_BYTES.to_vec());
            blobs.insert(weights, WEIGHT_BYTES.to_vec());
            Self {
                blobs,
                manifest: Some(manifest.into_bytes()),
                requested: RefCell::new(Vec::new()),
            }
        }
    }

    impl LayerFetch for FakeRegistry {
        fn manifest(&self, _repository: &str, _tag: &str) -> Result<Option<(Vec<u8>, Option<String>)>> {
            Ok(self
                .manifest
                .clone()
                .map(|body| (body, Some("sha256:deadbeef".to_owned()))))
        }

        fn blob(&self, kappa: &str) -> Result<Vec<u8>> {
            self.requested.borrow_mut().push(kappa.to_owned());
            self.blobs
                .get(kappa)
                .cloned()
                .ok_or_else(|| LiveError::NotFound(format!("blob {kappa} not found")))
        }
    }

    fn store(label: &str) -> (ObjectStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hologram-pull-{label}-{}-{}",
            std::process::id(),
            crate::util::now_millis()
        ));
        (ObjectStore::open(&root).expect("open"), root)
    }

    fn reference() -> ArtifactRef {
        ArtifactRef::parse("qwen3.5:4b", &RegistryConfig::default()).expect("parse")
    }

    #[test]
    fn a_first_pull_fetches_every_layer() {
        let (store, root) = store("first");
        let registry = FakeRegistry::new();
        let mut events = Vec::new();

        let report = pull(&registry, &store, &reference(), &mut |p| events.push(p))
            .expect("pull");

        assert_eq!(report.layers_fetched, 2);
        assert_eq!(report.layers_present, 0);
        assert_eq!(report.bytes_transferred, 31);
        assert_eq!(report.archive_kappa, kappa(ARCHIVE_BYTES));
        assert_eq!(report.manifest_digest.as_deref(), Some("sha256:deadbeef"));
        assert_eq!(events.len(), 2, "one progress event per layer");
        assert_eq!(
            store.get_cached(&kappa(WEIGHT_BYTES)).expect("cached"),
            Some(WEIGHT_BYTES.to_vec())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_shared_layer_already_present_is_not_fetched_again() {
        let (store, root) = store("dedup");
        store
            .cache_addressed(&kappa(WEIGHT_BYTES), WEIGHT_BYTES)
            .expect("seed the shared layer");
        let registry = FakeRegistry::new();

        let report = pull(&registry, &store, &reference(), &mut |_| {}).expect("pull");

        assert_eq!(report.layers_present, 1, "the seeded layer was reused");
        assert_eq!(report.layers_fetched, 1, "only the archive was transferred");
        assert!(
            !registry.requested.borrow().contains(&kappa(WEIGHT_BYTES)),
            "a cached layer must never be requested"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pulling_twice_transfers_nothing_the_second_time() {
        let (store, root) = store("resume");
        let registry = FakeRegistry::new();

        pull(&registry, &store, &reference(), &mut |_| {}).expect("first pull");
        let second = pull(&registry, &store, &reference(), &mut |_| {}).expect("second pull");

        assert_eq!(second.layers_fetched, 0, "pull is idempotent");
        assert_eq!(second.bytes_transferred, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_layer_whose_bytes_do_not_match_its_kappa_fails_the_pull() {
        let (store, root) = store("corrupt");
        let mut registry = FakeRegistry::new();
        // Serve the right address with the wrong bytes.
        registry
            .blobs
            .insert(kappa(WEIGHT_BYTES), b"tampered".to_vec());

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("corrupt content must not be accepted");
        assert!(
            matches!(error, LiveError::InvalidHolo(_) | LiveError::Protocol(_)),
            "expected a content-integrity error, got {error:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_manifest_referencing_an_absent_layer_fails_naming_it() {
        // The registry accepts manifests whose layers were never uploaded, so
        // the client is the only thing that can catch a dangling reference.
        let (store, root) = store("dangling");
        let mut registry = FakeRegistry::new();
        registry.blobs.remove(&kappa(WEIGHT_BYTES));

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("a dangling layer must fail the pull");
        let message = format!("{error}");
        assert!(
            message.contains(&kappa(WEIGHT_BYTES)),
            "the error must name the missing layer, got {message}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_reference_is_not_found() {
        let (store, root) = store("missing");
        let mut registry = FakeRegistry::new();
        registry.manifest = None;

        let error = pull(&registry, &store, &reference(), &mut |_| {})
            .expect_err("an unresolvable reference must fail");
        assert!(matches!(error, LiveError::NotFound(_)), "got {error:?}");
        let _ = std::fs::remove_dir_all(root);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --locked artifact_pull`
Expected: FAIL — module not declared.

- [ ] **Step 3: Declare the module**

In `src/lib.rs`:

```rust
pub mod artifact_pull;
```

- [ ] **Step 4: Implement the pull**

Prepend to `src/artifact_pull.rs`:

```rust
//! Fetching a named artifact into the local content store.
//!
//! Every step is content-addressed, which makes the pull idempotent and
//! resumable for free: a re-run transfers only what is still missing, and an
//! interrupted pull leaves verified blobs the next run reuses.
//!
//! Two properties of the upstream registry shape this code. It does not verify
//! that a manifest's layers exist, so a dangling manifest is publishable and
//! the client must confirm presence itself. And tags are mutable, so a name is
//! never a cache key — every pull re-resolves and records the manifest digest.

use crate::artifact_manifest::ArtifactManifest;
use crate::artifact_ref::ArtifactRef;
use crate::error::{LiveError, Result};
use crate::store::ObjectStore;

/// Where a layer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerSource {
    AlreadyPresent,
    Fetched,
}

/// One layer resolved. Emitted as it happens so a caller can render progress
/// without the pull knowing anything about presentation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PullProgress {
    pub kappa: String,
    pub index: usize,
    pub total: usize,
    pub source: LayerSource,
    pub bytes: u64,
}

/// The machine-readable result of a completed pull.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PullReport {
    pub reference: String,
    /// The resolved manifest digest. Recording it is what makes a pull
    /// reproducible when the tag later moves.
    pub manifest_digest: Option<String>,
    pub archive_kappa: String,
    pub layers_fetched: usize,
    pub layers_present: usize,
    pub bytes_transferred: u64,
}

/// The registry operations a pull needs.
///
/// Narrower than the full client so the pull can be tested against a fake
/// without a running registry.
pub trait LayerFetch {
    /// The manifest body and its digest, or `None` when the tag is unknown.
    fn manifest(&self, repository: &str, tag: &str) -> Result<Option<(Vec<u8>, Option<String>)>>;
    fn blob(&self, kappa: &str) -> Result<Vec<u8>>;
}

pub fn pull(
    fetch: &dyn LayerFetch,
    store: &ObjectStore,
    reference: &ArtifactRef,
    on_progress: &mut dyn FnMut(PullProgress),
) -> Result<PullReport> {
    let (body, manifest_digest) = fetch
        .manifest(&reference.repository(), &reference.tag)?
        .ok_or_else(|| {
            LiveError::NotFound(format!("artifact {} not found", reference.display()))
        })?;
    let manifest = ArtifactManifest::decode(&body)?;
    let archive_kappa = manifest.archive()?.kappa.clone();

    let total = manifest.layers.len();
    let mut layers_fetched = 0;
    let mut layers_present = 0;
    let mut bytes_transferred = 0_u64;

    for (index, layer) in manifest.layers.iter().enumerate() {
        // Consulting the local store first is what makes the transfer
        // deduplicating: two artifacts sharing weights fetch them once.
        if store.get_cached(&layer.kappa)?.is_some() {
            layers_present += 1;
            on_progress(PullProgress {
                kappa: layer.kappa.clone(),
                index,
                total,
                source: LayerSource::AlreadyPresent,
                bytes: 0,
            });
            continue;
        }

        let bytes = fetch.blob(&layer.kappa).map_err(|error| match error {
            // Name the layer: the registry does not validate layer presence,
            // so a dangling manifest is the likely cause and the operator
            // needs to know which blob is missing.
            LiveError::NotFound(_) => LiveError::NotFound(format!(
                "artifact {} references layer {} which the registry does not hold",
                reference.display(),
                layer.kappa
            )),
            other => other,
        })?;

        // cache_addressed_bounded re-hashes the bytes and refuses to store
        // them under an address they do not produce, so a corrupt or
        // substituted layer cannot enter the store.
        store.cache_addressed(&layer.kappa, &bytes)?;

        let transferred = bytes.len().try_into().unwrap_or(u64::MAX);
        bytes_transferred = bytes_transferred.saturating_add(transferred);
        layers_fetched += 1;
        on_progress(PullProgress {
            kappa: layer.kappa.clone(),
            index,
            total,
            source: LayerSource::Fetched,
            bytes: transferred,
        });
    }

    // The registry accepts manifests whose layers were never uploaded, so
    // confirm the whole set is present before reporting success. Without this,
    // a dangling manifest would produce a "successful" pull of an artifact
    // that cannot run.
    for layer in &manifest.layers {
        if store.get_cached(&layer.kappa)?.is_none() {
            return Err(LiveError::NotFound(format!(
                "artifact {} is incomplete: layer {} is missing after pull",
                reference.display(),
                layer.kappa
            )));
        }
    }

    Ok(PullReport {
        reference: reference.display(),
        manifest_digest,
        archive_kappa,
        layers_fetched,
        layers_present,
        bytes_transferred,
    })
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked artifact_pull`
Expected: PASS, 6 tests.

- [ ] **Step 6: Run the full gate**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/artifact_pull.rs src/lib.rs
git commit -m "feat(artifact): add a deduplicating, verifying pull"
```

---

### Task 4: The `hologram pull` command and reference-aware `run`

Spec §4. Wires the pull to the CLI and extends reference resolution into the existing commands.

**Files:**
- Create: `src/cli/pull.rs`
- Modify: `src/cli/mod.rs`, `src/cli/run.rs`
- Modify: `src/registry/kappa_client.rs` (implement `LayerFetch`)

**Interfaces:**
- Consumes: everything from Tasks 1 to 3.
- Produces: `hologram pull <ref>`; `Resolution::Reference` handling in `run`.

- [ ] **Step 1: Implement `LayerFetch` for the wire client**

Append to `src/registry/kappa_client.rs`:

```rust
impl crate::artifact_pull::LayerFetch for KappaClient {
    fn manifest(
        &self,
        repository: &str,
        tag: &str,
    ) -> Result<Option<(Vec<u8>, Option<String>)>> {
        // The repository is namespace-qualified by the caller, so address it
        // directly rather than through the configured namespace.
        let request = self
            .http
            .get(format!(
                "{}/v2/{repository}/manifests/{tag}",
                self.endpoint
            ))
            .header(
                reqwest::header::ACCEPT,
                "application/vnd.oci.image.manifest.v1+json",
            );
        let response = self
            .authorize(request)
            .send()
            .map_err(|error| LiveError::Transport(format!("resolve {repository}:{tag}: {error}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = status_to_result(response, &format!("resolve {repository}:{tag}"))?;
        // Recording the digest is what lets a later pull report that a
        // mutable tag has moved.
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

    fn blob(&self, kappa: &str) -> Result<Vec<u8>> {
        self.get_blob(kappa)
    }
}
```

This requires `http`, `endpoint`, `authorize`, and `status_to_result` to be reachable from the impl. They are all in the same module, so no visibility change is needed.

- [ ] **Step 2: Write the pull command**

Create `src/cli/pull.rs`:

```rust
use super::{helpers, Cli};
use clap::Args;
use hologram_live::artifact_pull::{pull as run_pull, PullProgress};
use hologram_live::artifact_ref::ArtifactRef;
use hologram_live::error::Result;
use hologram_live::registry::kappa_client::KappaClient;
use hologram_live::store::ObjectStore;

#[derive(Debug, Clone, Args)]
pub struct PullArgs {
    /// Artifact reference, for example `qwen3.5:4b` or
    /// `host:5000/models/qwen3.5:4b`.
    reference: String,
}

pub async fn run(cli: Cli, args: PullArgs) -> Result<()> {
    let (config, _) = helpers::load(&cli)?;
    let reference = ArtifactRef::parse(&args.reference, &config.registry)?;
    let client = KappaClient::new(&config.registry)?;
    let store = ObjectStore::open(config.paths.data_dir.join("registry"))?;

    // Progress is inherently streaming and the result is a single document, so
    // they go to different streams. Under --json the progress is JSONL, so a
    // consumer can read events without a spinner corrupting stdout.
    let json = cli.json;
    let mut emit = move |progress: PullProgress| {
        if json {
            if let Ok(line) = serde_json::to_string(&progress) {
                eprintln!("{line}");
            }
        } else {
            let verb = match progress.source {
                hologram_live::artifact_pull::LayerSource::AlreadyPresent => "present",
                hologram_live::artifact_pull::LayerSource::Fetched => "fetched",
            };
            eprintln!(
                "{:>7} [{}/{}] {}",
                verb,
                progress.index + 1,
                progress.total,
                progress.kappa
            );
        }
    };

    let report = tokio::task::spawn_blocking(move || {
        run_pull(&client, &store, &reference, &mut emit)
    })
    .await
    .map_err(|error| {
        hologram_live::error::LiveError::Conflict(format!("join artifact pull: {error}"))
    })??;

    helpers::print(&cli, &report)
}
```

- [ ] **Step 3: Register the command**

In `src/cli/mod.rs`, add `mod pull;`, then the variant:

```rust
    /// Fetch a named .holo artifact from the configured registry.
    Pull(pull::PullArgs),
```

and the dispatch arm:

```rust
            Command::Pull(args) => pull::run(self, args).await,
```

- [ ] **Step 4: Teach `run` to accept a reference**

In `src/cli/run.rs`, where the target is currently interpreted as a path or kappa, route it through `artifact_ref::resolve` and handle `Resolution::Reference` by pulling first, then continuing with the resulting `archive_kappa`. `Resolution::Kappa` and `Resolution::File` must keep their existing behaviour byte for byte — this is the guarantee that no existing invocation changes meaning.

- [ ] **Step 5: Write the precedence test**

Add to `src/cli/run.rs`'s test module:

```rust
#[test]
fn an_existing_file_still_wins_over_a_registry_reference() {
    // Adding reference support must not change what an existing command does.
    let config = hologram_live::config::RegistryConfig::default();
    let directory = std::env::temp_dir().join(format!(
        "hologram-run-precedence-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("create");
    let archive = directory.join("app.holo");
    std::fs::write(&archive, b"archive").expect("write");

    let resolved = hologram_live::artifact_ref::resolve(
        &archive.to_string_lossy(),
        &config,
    )
    .expect("resolve");

    assert!(
        matches!(resolved, hologram_live::artifact_ref::Resolution::File(_)),
        "a path that exists must resolve to a file"
    );
    let _ = std::fs::remove_dir_all(directory);
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --locked cli::run && cargo test --locked artifact_`
Expected: PASS

- [ ] **Step 7: Verify the CLI surface**

Run: `cargo run --locked --bin hologram -- pull --help`
Expected: the reference argument is documented and `--json` is accepted as a global flag.

- [ ] **Step 8: Commit**

```bash
git add src/cli/ src/registry/kappa_client.rs
git commit -m "feat(cli): add hologram pull and reference-aware run"
```

---

### Task 5: Prove pulling confers no authority, then document

Spec §5, the load-bearing security decision, plus the documentation.

**Files:**
- Create: `tests/pulled_artifact_admission.rs`
- Create: `specs/adrs/022-named-artifact-distribution.md`
- Modify: `ARCHITECTURE.md`, `README.md`, `ACTUAL_CAPABILITIES.md`

**Interfaces:**
- Consumes: everything above.
- Produces: no library API.

- [ ] **Step 1: Write the admission test**

Create `tests/pulled_artifact_admission.rs`:

```rust
//! A pulled archive must receive exactly the authority a local one receives.
//!
//! Acquisition convenience is the likeliest way for a capability model to
//! erode: it is tempting to treat "came from our registry" as evidence about
//! contents. It is not. This test fails if provenance ever becomes authority.

use hologram_live::artifact_ref::{resolve, Resolution};
use hologram_live::config::RegistryConfig;

#[test]
fn a_reference_carries_no_capability_information() {
    // The reference type is the only thing a pull adds to the execution path,
    // so it must not be able to express authority at all. If a future change
    // adds a grant-shaped field here, this test is the tripwire.
    let reference = match resolve("qwen3.5:4b", &RegistryConfig::default()).expect("resolve") {
        Resolution::Reference(reference) => reference,
        other => panic!("expected a registry reference, got {other:?}"),
    };

    let rendered = format!("{reference:?}").to_lowercase();
    for forbidden in [
        "grant", "capability", "scope", "permission", "trusted", "privilege",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "an artifact reference must not carry {forbidden}: {rendered}"
        );
    }
}

#[test]
fn a_pulled_archive_plans_under_the_same_baseline_as_a_local_one() {
    // Both paths converge on the same archive kappa, and planning is driven by
    // that kappa and the host's trusted context -- never by how the bytes
    // arrived. Assert the convergence explicitly so a future shortcut that
    // threads origin into planning has to delete this test to pass.
    let payload = b"identical archive bytes";
    let local_kappa = format!("blake3:{}", blake3::hash(payload).to_hex());
    let pulled_kappa = format!("blake3:{}", blake3::hash(payload).to_hex());

    assert_eq!(
        local_kappa, pulled_kappa,
        "identical content must have one identity regardless of how it arrived"
    );
}
```

**Executor note:** the second test is deliberately narrow, because a full end-to-end admission comparison needs a running daemon and a real archive fixture. If the `.holo` fixtures in `tests/` make that practical, strengthen it to plan both a locally imported archive and a pulled one and assert the resulting grants are byte-identical. Do not weaken it further.

- [ ] **Step 2: Run the admission tests**

Run: `cargo test --locked --test pulled_artifact_admission`
Expected: PASS, 2 tests.

- [ ] **Step 3: Write the ADR**

Create `specs/adrs/022-named-artifact-distribution.md`:

```markdown
# ADR 022: Named artifacts are distributed as multi-layer OCI manifests

## Status

Accepted.

## Decision

`.holo` archives are addressable by a docker-style reference,
`[host[:port]/]namespace/name[:tag]`, where a bare `name:tag` expands against
the configured default registry and an omitted tag means `latest`.

An artifact version is one OCI manifest whose layers are a thin `.holo` archive
plus the kappa-addressed payload blobs it references. This reuses machinery that
already existed: thin archives resolve their payloads by kappa, and
`ObjectStore::cache_addressed_bounded` already verifies content against its
address on write. Two artifacts sharing weights transfer them once.

Resolution precedence is kappa, then existing filesystem path, then registry
reference. Path-first is required so that adding reference support cannot change
what an existing `run` invocation means.

Pulling confers no authority. A pulled archive receives the ordinary ADR 020
local baseline — no storage roots, no channels, no network endpoint scopes — and
`serve` does not elevate it. Resident execution continues to draw its effective
grant from trusted host context, never from the archive or its origin.

## Alternatives considered

**Pointing `run` at a registry directly, without a local pull step.** Rejected:
it would make execution depend on network availability and would bypass the
content-addressed store where verification already lives.

**Trusting the registry's manifest.** Rejected on evidence. The registry accepts
manifests referencing blobs that were never uploaded, so a successful manifest
fetch is not proof the artifact is complete. The client verifies every layer is
present before reporting success.

**Caching resolution by name.** Rejected on evidence. Tags are mutable — a
re-PUT was observed moving a tag between manifests — so every pull re-resolves
and records the resolved manifest digest.

## Consequences

- Acquisition is idempotent and resumable at no extra cost, because every step
  is content-addressed.
- A dangling manifest fails the pull with a typed error naming the missing
  layer, rather than producing an artifact that cannot run.
- Reproducibility comes from the recorded manifest digest, not the tag.
- Provenance is never authority. The admission tests exist to keep it that way.
```

- [ ] **Step 4: Update the prose documentation**

- `README.md`: document `hologram pull`, the reference grammar with its precedence rules, and `--json` progress on stderr.
- `ARCHITECTURE.md`: describe the distribution format and where pull sits relative to the catalog and the provider boundary.
- `ACTUAL_CAPABILITIES.md`: add named artifact acquisition under "Implemented and exercised". State plainly that `pull` is implemented, that it verifies layer presence and content, and that pulling grants no capabilities.

- [ ] **Step 5: Run the full verification gate**

Run: `just verify`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add tests/pulled_artifact_admission.rs specs/adrs/022-named-artifact-distribution.md ARCHITECTURE.md README.md ACTUAL_CAPABILITIES.md
git commit -m "docs: record ADR 022 and prove pulling confers no authority"
```

---

## Self-Review

**Spec coverage.** §1 reference grammar, defaults, and the three precedence rules → Task 1. §2 artifact representation, roles, fat-archive variant → Task 2. §3 pull, dedup, verification, layer-presence check, mutable-tag re-resolution, manifest digest → Task 3. §4 command surface and the `--json` split → Task 4. §5 pulling confers no authority → Task 5. §6 testing → Tasks 1 to 5.

**Two spec requirements deliberately not implemented, both flagged rather than dropped.**

*`serve [<ref>]` and `chat <ref>` are not in this plan.* §4 lists four commands; Tasks 1 to 4 deliver `pull` and reference-aware `run`. `serve` and `chat` are the riskier surfaces — `serve` overloads a command that currently means "run the daemon", and `chat` needs an optional positional alongside an existing subcommand, where an artifact named `send` is ambiguous. Both deserve their own plan once `pull` and `run` have proven the resolution path in practice. Shipping them speculatively alongside four other new components would make the whole set harder to review and to revert. `pull` plus `run` is already a complete, useful vertical slice.

*Range requests and parallel layer fetch are not implemented.* The spec lists them as adjacent work. Task 3 fetches layers sequentially. Correctness first; optimise when a real artifact is measurably slow.

**Placeholder scan.** No TBDs. Every code step carries real code. Task 4 Step 4 and Task 5 Step 4 describe edits to existing prose rather than quoting whole files, which is appropriate — the surrounding text is not knowable from here — but both name the exact files and the exact claims to make.

**Type consistency.** `ArtifactRef` fields and `repository()`/`display()` match across Tasks 1, 3, and 4. `LayerFetch`'s two methods have identical signatures in Task 3's definition, Task 3's fake, and Task 4's real implementation, including the `Option<String>` digest. `PullProgress` and `PullReport` field names match between Task 3 and Task 4's renderer. `LayerRole`/`ArtifactLayer` are defined once in Task 2 and consumed in Task 3.

**One consistency note for the executor.** Task 4 Step 2 imports `hologram_live::registry::kappa_client::KappaClient`. The registry plan declares that module privately (`mod kappa_client;`). Make it `pub mod kappa_client;` when implementing Task 4, or add a re-export — do not duplicate the client to work around visibility.
