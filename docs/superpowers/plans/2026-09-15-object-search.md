# Object Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Hologram Live a bounded, paginated object-search contract served by the local provider, and prepare the provider seam for a second implementation.

**Architecture:** `src/registry.rs` becomes a module directory so a second provider can join it without any file approaching the source-size gate. `search` joins the `RegistryProvider` trait, returning a page of metadata ordered ascending by object id with an opaque cursor. The local provider satisfies it from the existing metadata scan; no index is introduced. New HTTP, gRPC, and CLI surfaces are additive, so no shipped response shape changes.

**Tech Stack:** Rust 1.94 (toolchain pins 1.97.1), axum 0.8, tonic 0.14, utoipa 5, clap 4.6, cucumber 0.23.

**Spec:** `docs/superpowers/specs/2026-09-15-kappa-registry-provider-design.md` (§1, §3, §5, §7)

**Plan 1 of 2.** Plan 2, `2026-09-15-kappa-registry-provider.md`, adds the remote provider and the conformance suite that proves the two are substitutable. This plan stands alone: at its end, search works end to end against local storage.

## Global Constraints

- **No new dependencies.** Search is built on the existing store, never an index.
- **1500 production lines per file**, enforced by `scripts/check-file-size.sh`. Lines from the first `#[cfg(test)]` onward do not count for `.rs`; `.md` files are counted whole. This is why `registry.rs` becomes a directory, and why this plan is split in two.
- **`--locked` on every cargo invocation**, matching the `Justfile`.
- **Clippy `pedantic` is warn-level and `just clippy` runs `-D warnings`.** A warning fails the gate.
- **`unsafe_code = "forbid"`, `unused_must_use = "deny"`.**
- **Comment density matches surrounding code:** `//!` module headers and `///` on public items, explaining *why*.
- **The local provider stays the default.** `hologram` must remain a single binary needing no external service.
- **Object identity is `blake3:` + 64 lowercase hex.** Never mix digest axes for object identity.
- Full gate: `just verify`.

---

### Task 1: Split `registry.rs` into a module directory

Pure refactor. No behaviour change, so the existing suite is the test.

**Files:**
- Create: `src/registry/mod.rs`
- Create: `src/registry/local.rs`
- Delete: `src/registry.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `registry::RegistryProvider` (unchanged trait), `registry::LocalRegistryProvider` (moved). Both must remain importable at their current paths so `src/app.rs` and the modules need no edit.

- [ ] **Step 1: Confirm the current suite is green before touching anything**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS. Record the total count; Task 1 must not change it.

- [ ] **Step 2: Create `src/registry/mod.rs`**

```rust
//! Storage-facing seam for the Kappa Registry module.
//!
//! The trait is deliberately synchronous. Every caller already runs provider
//! work inside `tokio::task::spawn_blocking` (see `src/modules/registry.rs`
//! and `src/modules/files.rs`), and an async trait would force boxed futures
//! at every call site because `async fn` in traits is not `dyn`-safe.

mod local;

pub use local::LocalRegistryProvider;

use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata};

/// Storage-facing seam for the Kappa Registry module.
///
/// `LocalRegistryProvider` keeps development entirely local.
/// `KappaRegistryProvider` speaks to an external kappa-registry instance
/// without changing module routes, native operation IDs, or desktop clients.
pub trait RegistryProvider: Send + Sync {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>>;
    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata>;
    fn get_object(&self, id: &str) -> Result<ObjectContent>;
    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata>;
}
```

- [ ] **Step 3: Create `src/registry/local.rs` with the moved implementation**

```rust
//! The built-in provider. Content lives in the local `ObjectStore`, so this
//! path needs no network and no external service.

use super::RegistryProvider;
use crate::error::Result;
use crate::protocol::{ObjectContent, ObjectMetadata};
use crate::store::ObjectStore;
use std::sync::Arc;

pub struct LocalRegistryProvider {
    store: Arc<ObjectStore>,
}

impl LocalRegistryProvider {
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }
}

impl RegistryProvider for LocalRegistryProvider {
    fn list_objects(&self, kind: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        self.store.list(kind)
    }

    fn put_object(
        &self,
        kind: String,
        media_type: String,
        filename: Option<String>,
        bytes: &[u8],
    ) -> Result<ObjectMetadata> {
        self.store.put(kind, media_type, filename, bytes)
    }

    fn get_object(&self, id: &str) -> Result<ObjectContent> {
        Ok(ObjectContent {
            metadata: self.store.metadata(id)?,
            bytes: self.store.get(id)?,
        })
    }

    fn rename_file(&self, id: &str, filename: String) -> Result<ObjectMetadata> {
        self.store.rename_file(id, filename)
    }
}
```

- [ ] **Step 4: Delete the old file**

```bash
git rm src/registry.rs
```

- [ ] **Step 5: Verify nothing else needed changing**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS with the same total as Step 1. `src/lib.rs` already says `pub mod registry;`, which now resolves to the directory. If any import broke, the split changed a public path — fix the split, not the caller.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(registry): split the provider seam into a module directory"
```

---
### Task 2: Stop `put` from rewriting `created_at_millis`

Spec §7.1. Content-addressed objects are immutable, so their creation time must not move. This is also the rename tie-break of §2, which makes it load-bearing rather than cosmetic.

**Files:**
- Modify: `src/store.rs` (the `put` method)
- Test: `src/store.rs` (the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Task 1's module layout.
- Produces: `ObjectStore::put` preserving an existing `created_at_millis`. No signature change.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/store.rs`:

```rust
#[test]
fn reputting_identical_content_preserves_the_original_creation_time() {
    let root = std::env::temp_dir().join(format!("hologram-store-created-{}", now_millis()));
    let store = ObjectStore::open(&root).expect("open");
    let first = store
        .put("file", "text/plain", Some("a.txt".to_owned()), b"stable")
        .expect("first put");

    // now_millis() has millisecond resolution, so without a deliberate gap a
    // regression could pass by coincidence.
    std::thread::sleep(std::time::Duration::from_millis(5));

    let second = store
        .put("file", "text/plain", Some("a.txt".to_owned()), b"stable")
        .expect("second put");

    assert_eq!(first.id, second.id, "content addressing must be stable");
    assert_eq!(
        first.created_at_millis, second.created_at_millis,
        "creation time of immutable content must not move on re-put"
    );
    assert_eq!(
        store.metadata(&first.id).expect("metadata").created_at_millis,
        first.created_at_millis,
        "the persisted record must agree with the returned one"
    );
    let _ = std::fs::remove_dir_all(root);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --locked store::tests::reputting_identical_content_preserves_the_original_creation_time -- --exact`
Expected: FAIL — the two timestamps differ, because `put` always calls `now_millis()`.

- [ ] **Step 3: Preserve the earlier timestamp**

In `src/store.rs`, replace the `created_at_millis: now_millis(),` line in `put` by computing it before building the struct. Insert after `let blob = self.blob_path(&digest_hex);` and its `atomic_write` block:

```rust
        // Content addressing makes an object immutable, so its creation time is
        // a property of the content's first appearance. Re-putting the same
        // bytes must not move it: the rename tie-break in the Kappa provider
        // resolves duplicate metadata by greatest creation time, so a drifting
        // timestamp would silently change which record wins.
        let created_at_millis = match self.read_metadata(&digest_hex) {
            Some(existing) => existing.created_at_millis,
            None => now_millis(),
        };
```

Then use it in the struct literal:

```rust
        let metadata = ObjectMetadata {
            id,
            kind: kind.into(),
            media_type: media_type.into(),
            filename,
            size: bytes.len().try_into().unwrap_or(u64::MAX),
            created_at_millis,
        };
```

Add the private helper next to `metadata_path`:

```rust
    /// Best-effort read of an existing record. A missing or unreadable file is
    /// treated as absent: this only chooses a creation timestamp, and failing
    /// the whole write because a stale record will not parse would be worse.
    fn read_metadata(&self, digest: &str) -> Option<ObjectMetadata> {
        let bytes = std::fs::read(self.metadata_path(digest)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --locked store::tests::reputting_identical_content_preserves_the_original_creation_time -- --exact`
Expected: PASS

- [ ] **Step 5: Verify no existing test depended on the old behaviour**

Run: `cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/store.rs
git commit -m "fix(store): keep creation time stable when re-putting identical content"
```

---
### Task 3: Add the search types and implement search for the local provider

Spec §3.

**Files:**
- Modify: `src/protocol.rs` (add `ObjectQuery`, `ObjectPage`)
- Modify: `src/registry/mod.rs` (add `search` to the trait)
- Modify: `src/registry/local.rs` (implement it)
- Test: `src/registry/local.rs` (new `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Task 1's layout, Task 2's stable timestamps.
- Produces:
  - `protocol::ObjectQuery { kind: Option<String>, media_type: Option<String>, filename_contains: Option<String>, min_size: Option<u64>, max_size: Option<u64>, created_after_millis: Option<u64>, created_before_millis: Option<u64>, limit: u32, cursor: Option<String> }`
  - `protocol::ObjectPage { objects: Vec<ObjectMetadata>, next_cursor: Option<String>, truncated: bool }`
  - `ObjectQuery::DEFAULT_LIMIT: u32 = 100`, `ObjectQuery::MAX_LIMIT: u32 = 1000`, `ObjectQuery::effective_limit(&self) -> usize`, `ObjectQuery::matches(&self, &ObjectMetadata) -> bool`
  - `RegistryProvider::search(&self, query: &ObjectQuery) -> Result<ObjectPage>`

- [ ] **Step 1: Add the types to `src/protocol.rs`**

Place directly after the `ObjectContent` struct:

```rust
/// A search over stored objects.
///
/// Every field is an independent conjunctive filter; `None` means "do not
/// constrain". Ordering is ascending by object id, identically for every
/// provider: upstream tag listings arrive in lexical order, and imposing a
/// time ordering on a remote provider would mean enumerating everything
/// before returning the first page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ObjectQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub filename_contains: Option<String>,
    #[serde(default)]
    pub min_size: Option<u64>,
    #[serde(default)]
    pub max_size: Option<u64>,
    #[serde(default)]
    pub created_after_millis: Option<u64>,
    #[serde(default)]
    pub created_before_millis: Option<u64>,
    #[serde(default)]
    pub limit: u32,
    /// Opaque, and valid only for the provider that issued it. A structured
    /// cursor would leak provider internals into a public API and stop either
    /// side changing independently.
    #[serde(default)]
    pub cursor: Option<String>,
}

impl ObjectQuery {
    pub const DEFAULT_LIMIT: u32 = 100;
    pub const MAX_LIMIT: u32 = 1000;

    /// Page size after defaulting and clamping. A zero `limit` means the
    /// caller did not choose, not "return nothing".
    pub fn effective_limit(&self) -> usize {
        let requested = if self.limit == 0 {
            Self::DEFAULT_LIMIT
        } else {
            self.limit
        };
        requested.min(Self::MAX_LIMIT) as usize
    }

    /// Whether one record satisfies every constraint. Shared by both
    /// providers so filtering cannot drift between them.
    pub fn matches(&self, metadata: &ObjectMetadata) -> bool {
        if self.kind.as_ref().is_some_and(|k| *k != metadata.kind) {
            return false;
        }
        if self
            .media_type
            .as_ref()
            .is_some_and(|m| *m != metadata.media_type)
        {
            return false;
        }
        if let Some(needle) = self.filename_contains.as_ref() {
            match metadata.filename.as_deref() {
                Some(name) if name.contains(needle.as_str()) => {}
                _ => return false,
            }
        }
        if self.min_size.is_some_and(|min| metadata.size < min) {
            return false;
        }
        if self.max_size.is_some_and(|max| metadata.size > max) {
            return false;
        }
        if self
            .created_after_millis
            .is_some_and(|after| metadata.created_at_millis <= after)
        {
            return false;
        }
        if self
            .created_before_millis
            .is_some_and(|before| metadata.created_at_millis >= before)
        {
            return false;
        }
        true
    }
}

/// One page of search results.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ObjectPage {
    pub objects: Vec<ObjectMetadata>,
    /// Absent when the walk reached the end of the result set.
    pub next_cursor: Option<String>,
    /// True when a provider-side scan bound stopped the walk before `limit`
    /// was satisfied. A capped result is never presented as a complete one.
    pub truncated: bool,
}
```

- [ ] **Step 2: Write the failing tests in `src/registry/local.rs`**

Append to `src/registry/local.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ObjectQuery;

    fn provider(label: &str) -> (LocalRegistryProvider, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hologram-registry-{label}-{}",
            crate::util::now_millis()
        ));
        let store = Arc::new(ObjectStore::open(&root).expect("open"));
        (LocalRegistryProvider::new(store), root)
    }

    #[test]
    fn search_filters_on_kind_and_orders_by_id() {
        let (registry, root) = provider("filter");
        registry
            .put_object("file".into(), "text/plain".into(), Some("a.txt".into()), b"a")
            .expect("put a");
        registry
            .put_object("file".into(), "text/plain".into(), Some("b.txt".into()), b"b")
            .expect("put b");
        registry
            .put_object("holo".into(), "application/octet-stream".into(), None, b"c")
            .expect("put c");

        let page = registry
            .search(&ObjectQuery {
                kind: Some("file".to_owned()),
                ..ObjectQuery::default()
            })
            .expect("search");

        assert_eq!(page.objects.len(), 2, "only file-kind objects match");
        assert!(!page.truncated);
        let ids: Vec<&str> = page.objects.iter().map(|o| o.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "results are ascending by id");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_paginates_with_an_opaque_cursor_and_no_overlap() {
        let (registry, root) = provider("paginate");
        for index in 0..5u8 {
            registry
                .put_object(
                    "file".into(),
                    "text/plain".into(),
                    Some(format!("f{index}.txt")),
                    &[index],
                )
                .expect("put");
        }

        let first = registry
            .search(&ObjectQuery {
                limit: 2,
                ..ObjectQuery::default()
            })
            .expect("first page");
        assert_eq!(first.objects.len(), 2);
        let cursor = first.next_cursor.clone().expect("more results remain");

        let second = registry
            .search(&ObjectQuery {
                limit: 2,
                cursor: Some(cursor),
                ..ObjectQuery::default()
            })
            .expect("second page");
        assert_eq!(second.objects.len(), 2);

        let first_ids: Vec<&String> = first.objects.iter().map(|o| &o.id).collect();
        for object in &second.objects {
            assert!(
                !first_ids.contains(&&object.id),
                "pages must not overlap: {} repeated",
                object.id
            );
        }
        assert!(
            second.objects[0].id > first.objects[1].id,
            "the second page continues strictly after the first"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_filters_on_filename_substring_and_size() {
        let (registry, root) = provider("substring");
        registry
            .put_object(
                "file".into(),
                "text/plain".into(),
                Some("notes-2026.txt".into()),
                b"hello world",
            )
            .expect("put long");
        registry
            .put_object("file".into(), "text/plain".into(), Some("other.txt".into()), b"x")
            .expect("put short");

        let by_name = registry
            .search(&ObjectQuery {
                filename_contains: Some("notes".to_owned()),
                ..ObjectQuery::default()
            })
            .expect("search by name");
        assert_eq!(by_name.objects.len(), 1);
        assert_eq!(by_name.objects[0].filename.as_deref(), Some("notes-2026.txt"));

        let by_size = registry
            .search(&ObjectQuery {
                min_size: Some(5),
                ..ObjectQuery::default()
            })
            .expect("search by size");
        assert_eq!(by_size.objects.len(), 1);
        assert_eq!(by_size.objects[0].size, 11);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_full_final_page_reports_no_further_cursor() {
        let (registry, root) = provider("exact");
        for index in 0..2u8 {
            registry
                .put_object("file".into(), "text/plain".into(), None, &[index])
                .expect("put");
        }

        let page = registry
            .search(&ObjectQuery {
                limit: 2,
                ..ObjectQuery::default()
            })
            .expect("search");

        assert_eq!(page.objects.len(), 2);
        assert!(
            page.next_cursor.is_none(),
            "a page that exhausts the result set must not advertise more"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test --locked registry::local::tests`
Expected: FAIL to compile — no `search` method on the trait.

- [ ] **Step 4: Add `search` to the trait in `src/registry/mod.rs`**

Add inside `pub trait RegistryProvider`:

```rust
    /// Page through objects matching `query`, ascending by id.
    ///
    /// `list_objects` keeps its newest-first contract and bare-array response
    /// for existing callers; only this surface is id-ordered.
    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage>;
```

And extend the import:

```rust
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};
```

- [ ] **Step 5: Implement it in `src/registry/local.rs`**

Add to `impl RegistryProvider for LocalRegistryProvider`:

```rust
    fn search(&self, query: &ObjectQuery) -> Result<ObjectPage> {
        // The store scan is the only enumeration available locally. It stays a
        // scan deliberately: an index would be a new dependency for a need
        // that has not been demonstrated. The seam is here, so an index can
        // replace this without touching callers.
        let mut all = self.store.list(None)?;
        all.sort_unstable_by(|left, right| left.id.cmp(&right.id));

        let limit = query.effective_limit();
        let after = query.cursor.as_deref();
        let mut objects = Vec::with_capacity(limit.min(all.len()));
        let mut exhausted = true;

        for metadata in all {
            if after.is_some_and(|cursor| metadata.id.as_str() <= cursor) {
                continue;
            }
            if !query.matches(&metadata) {
                continue;
            }
            if objects.len() == limit {
                // A further match exists, so the caller can page again.
                exhausted = false;
                break;
            }
            objects.push(metadata);
        }

        let next_cursor = if exhausted {
            None
        } else {
            objects.last().map(|object| object.id.clone())
        };

        Ok(ObjectPage {
            objects,
            next_cursor,
            // The local scan reads everything, so it never stops early.
            truncated: false,
        })
    }
```

Extend the import at the top of the file:

```rust
use crate::protocol::{ObjectContent, ObjectMetadata, ObjectPage, ObjectQuery};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --locked registry::local::tests`
Expected: PASS, 4 tests.

- [ ] **Step 7: Run the whole gate**

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src/protocol.rs src/registry/
git commit -m "feat(registry): add a paginated object search served by the local provider"
```

---
### Task 4: Add the `[registry]` configuration section

Spec §5, with one deliberate divergence recorded below.

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `config::RegistryConfig { provider: String, endpoint: String, namespace: String, token: String, request_timeout_secs: u64, max_scan_pages: u32 }`, reachable as `AppConfig::registry`. `RegistryConfig::default()` yields `provider: "local"`.

**Divergence from the spec:** §5 says this bumps `CURRENT_SCHEMA_VERSION`. It does not need to. Every section on `AppConfig` carries `#[serde(default)]`, and the version machinery with `RETIRED_KEYS` exists for *removals* — an added defaulted section loads cleanly against older files without it. Step 3 proves that with a test rather than asserting it. Do not bump the version.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/config.rs`:

```rust
#[test]
fn registry_defaults_to_the_local_provider() {
    let config = AppConfig::default();
    assert_eq!(
        config.registry.provider, "local",
        "hologram must stay a single binary needing no external service"
    );
}

#[test]
fn a_config_written_before_the_registry_section_still_loads() {
    // Adding a defaulted section must not require a schema bump, because the
    // version machinery exists for retired keys, not for additions.
    let document = r#"
schema_version = 2
[server]
listen = "127.0.0.1:4455"
"#;
    let config: AppConfig = toml::from_str(document).expect("older config must load");
    assert_eq!(config.registry.provider, "local");
    assert_eq!(config.registry.max_scan_pages, 20);
}

#[test]
fn selecting_the_kappa_provider_without_an_endpoint_is_a_config_error() {
    let mut config = AppConfig::default();
    config.registry.provider = "kappa".to_owned();
    config.registry.endpoint = String::new();

    let error = config.validate().expect_err("an unreachable provider must fail early");
    assert!(
        matches!(error, LiveError::Config(_)),
        "expected a config error, got {error:?}"
    );
}

#[test]
fn an_unknown_registry_provider_is_rejected() {
    let mut config = AppConfig::default();
    config.registry.provider = "postgres".to_owned();

    let error = config.validate().expect_err("unknown providers must fail");
    assert!(matches!(error, LiveError::Config(_)));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --locked config::tests::registry config::tests::a_config_written config::tests::selecting_the_kappa config::tests::an_unknown_registry`
Expected: FAIL to compile — no `registry` field.

- [ ] **Step 3: Add the config type**

In `src/config.rs`, next to the other section structs:

```rust
/// Selects which `RegistryProvider` backs object storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegistryConfig {
    /// `"local"` or `"kappa"`. Local is the default so a stock install needs
    /// no external service.
    pub provider: String,
    /// Base URL of the kappa-registry instance. Required when
    /// `provider = "kappa"`.
    pub endpoint: String,
    /// Namespace that objects are written under.
    pub namespace: String,
    /// Bearer token. Upstream's `KAPPA_AUTH_REQUIRED` defaults to false, so an
    /// empty token is valid against a development instance.
    pub token: String,
    pub request_timeout_secs: u64,
    /// Upper bound on upstream pages walked to satisfy one selective query.
    /// Reaching it sets `ObjectPage::truncated` rather than silently capping.
    pub max_scan_pages: u32,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            provider: "local".to_owned(),
            endpoint: "http://127.0.0.1:5000".to_owned(),
            namespace: "hologram".to_owned(),
            token: String::new(),
            request_timeout_secs: 30,
            max_scan_pages: 20,
        }
    }
}
```

Add the field to `AppConfig`, after `plugins`:

```rust
    #[serde(default)]
    pub registry: RegistryConfig,
```

Add to `AppConfig::default()` construction if it is written out field-by-field:

```rust
            registry: RegistryConfig::default(),
```

- [ ] **Step 4: Validate the section**

Inside `AppConfig::validate`, before the final `Ok(())`:

```rust
        match self.registry.provider.as_str() {
            "local" => {}
            "kappa" => {
                // Fail at startup rather than on the first request, so a
                // misconfigured daemon never reports itself ready.
                if self.registry.endpoint.trim().is_empty() {
                    return Err(LiveError::Config(
                        "registry.endpoint is required when registry.provider is \"kappa\""
                            .to_owned(),
                    ));
                }
                if self.registry.namespace.trim().is_empty() {
                    return Err(LiveError::Config(
                        "registry.namespace is required when registry.provider is \"kappa\""
                            .to_owned(),
                    ));
                }
            }
            other => {
                return Err(LiveError::Config(format!(
                    "unsupported registry.provider {other:?}; expected local or kappa"
                )))
            }
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked config::tests`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add the [registry] provider section"
```

---
### Task 5: Expose search over HTTP, gRPC, and the CLI

Spec §3's surfaces. All additive; no shipped response shape changes.

**Files:**
- Modify: `src/protocol.rs` (operation ids, `RpcRequest`/`RpcResponse` variants)
- Modify: `src/modules/registry.rs` (route, handler, OpenAPI)
- Modify: `src/modules/files.rs` (route, handler, OpenAPI)
- Modify: `src/grpc.rs` (dispatch)
- Modify: `src/cli/registry.rs` and `src/cli/files.rs` (subcommands)
- Test: `src/modules/registry.rs`

**Interfaces:**
- Consumes: `ObjectQuery`/`ObjectPage` and `RegistryProvider::search` from Task 3.
- Produces: `operation::REGISTRY_SEARCH = "registry.search"`, `operation::FILES_SEARCH = "files.search"`, `RpcRequest::RegistrySearch { query: ObjectQuery }`, `RpcRequest::FilesSearch { query: ObjectQuery }`, `RpcResponse::ObjectPage(ObjectPage)`.

- [ ] **Step 1: Add the operation ids and protocol variants**

In `src/protocol.rs`, beside the existing registry constants:

```rust
    pub const REGISTRY_SEARCH: &str = "registry.search";
    pub const FILES_SEARCH: &str = "files.search";
```

Add to `RpcRequest`:

```rust
    RegistrySearch {
        query: ObjectQuery,
    },
    FilesSearch {
        query: ObjectQuery,
    },
```

Add to `RpcResponse`:

```rust
    ObjectPage(ObjectPage),
```

Then extend the `operation()` and `kind()` match arms: both new requests map to their ids and to `OperationKind::Read`.

- [ ] **Step 2: Write the failing handler test**

Add to `src/modules/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::protocol::ObjectQuery;

    #[test]
    fn query_parameters_default_to_a_bounded_page() {
        // The HTTP surface deserializes ObjectQuery straight from the query
        // string, so an empty query must be a safe bounded request rather than
        // an unbounded scan.
        let query: ObjectQuery =
            serde_urlencoded::from_str("").expect("an empty query string is valid");
        assert_eq!(query.effective_limit(), 100);
        assert!(query.cursor.is_none());
        assert!(query.kind.is_none());
    }

    #[test]
    fn an_oversized_limit_is_clamped() {
        let query: ObjectQuery =
            serde_urlencoded::from_str("limit=100000").expect("parse");
        assert_eq!(
            query.effective_limit(),
            1000,
            "a caller must not be able to request an unbounded page"
        );
    }

    #[test]
    fn filters_parse_from_the_query_string() {
        let query: ObjectQuery =
            serde_urlencoded::from_str("kind=file&filename_contains=notes&min_size=10")
                .expect("parse");
        assert_eq!(query.kind.as_deref(), Some("file"));
        assert_eq!(query.filename_contains.as_deref(), Some("notes"));
        assert_eq!(query.min_size, Some(10));
    }
}
```

**Note:** `serde_urlencoded` arrives transitively through axum. If it is not directly accessible, use `axum::extract::Query::<ObjectQuery>::try_from_uri` against a built `Uri` instead — do not add a dependency.

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test --locked modules::registry::tests`
Expected: FAIL to compile — `effective_limit` exists, but the test module does not yet.

- [ ] **Step 4: Add the HTTP handler and route**

In `src/modules/registry.rs`, add the operation descriptor:

```rust
    OperationDescriptor {
        id: operation::REGISTRY_SEARCH,
        kind: OperationKind::Read,
        fallback_safe_before_dispatch: true,
    },
```

Add the route inside `router()`:

```rust
            .route("/api/v1/objects/search", get(search_objects))
```

And the handler:

```rust
#[utoipa::path(
    get,
    path = "/api/v1/objects/search",
    params(
        ("kind" = Option<String>, Query, description = "Exact object kind"),
        ("media_type" = Option<String>, Query, description = "Exact media type"),
        ("filename_contains" = Option<String>, Query, description = "Filename substring"),
        ("min_size" = Option<u64>, Query, description = "Minimum size in bytes"),
        ("max_size" = Option<u64>, Query, description = "Maximum size in bytes"),
        ("created_after_millis" = Option<u64>, Query, description = "Exclusive lower bound"),
        ("created_before_millis" = Option<u64>, Query, description = "Exclusive upper bound"),
        ("limit" = Option<u32>, Query, description = "Page size; defaults to 100, clamped to 1000"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor from a previous page")
    ),
    responses((status = 200, body = ObjectPage))
)]
pub async fn search_objects(
    State(state): State<AppState>,
    Query(query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    let registry = state.registry().clone();
    let page = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join object search: {error}"))
        })??;
    Ok(Json(page))
}
```

Update the imports to include `axum::extract::Query` and `crate::protocol::{ObjectPage, ObjectQuery}`, and register both new schemas in the `RegistryApiDoc` `components(schemas(...))` list along with the new path.

- [ ] **Step 5: Mirror it for files**

In `src/modules/files.rs`, add `operation::FILES_SEARCH` to `OPERATIONS`, add the route `/api/v1/files/search`, and add a handler that forces `kind` to `file` before delegating:

```rust
#[utoipa::path(
    get,
    path = "/api/v1/files/search",
    responses((status = 200, body = ObjectPage))
)]
pub async fn search_files(
    State(state): State<AppState>,
    Query(mut query): Query<ObjectQuery>,
) -> Result<Json<ObjectPage>, HttpError> {
    // The files surface is the file-kind projection of the object surface, so
    // the kind is fixed here rather than trusted from the caller.
    query.kind = Some("file".to_owned());
    let registry = state.registry().clone();
    let page = tokio::task::spawn_blocking(move || registry.search(&query))
        .await
        .map_err(|error| {
            crate::error::LiveError::Conflict(format!("join file search: {error}"))
        })??;
    Ok(Json(page))
}
```

- [ ] **Step 6: Add the gRPC dispatch**

In `src/grpc.rs`, follow the existing `RegistryList` arm and add arms for `RpcRequest::RegistrySearch { query }` and `RpcRequest::FilesSearch { query }` that call `registry.search` inside `spawn_blocking` and return `RpcResponse::ObjectPage`. Mirror the surrounding error mapping exactly.

- [ ] **Step 7: Add the CLI subcommands**

In `src/cli/registry.rs`, extend `RegistryCommand`:

```rust
    /// Search stored objects with filters and pagination.
    Search {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        media_type: Option<String>,
        #[arg(long)]
        filename_contains: Option<String>,
        #[arg(long)]
        min_size: Option<u64>,
        #[arg(long)]
        max_size: Option<u64>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
    },
```

And the arm:

```rust
        RegistryCommand::Search {
            kind,
            media_type,
            filename_contains,
            min_size,
            max_size,
            limit,
            cursor,
        } => {
            let query = hologram_live::protocol::ObjectQuery {
                kind,
                media_type,
                filename_contains,
                min_size,
                max_size,
                created_after_millis: None,
                created_before_millis: None,
                limit,
                cursor,
            };
            match helpers::call(&cli, RpcRequest::RegistrySearch { query }).await? {
                RpcResponse::ObjectPage(value) => helpers::print(&cli, &value),
                other => helpers::unexpected(other),
            }
        }
```

Mirror this in `src/cli/files.rs` as `FilesCommand::Search`, sending `RpcRequest::FilesSearch`.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --locked modules::registry::tests`
Expected: PASS, 3 tests.

- [ ] **Step 9: Regenerate the OpenAPI document and run the full gate**

Run: `just docs && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --all-targets --locked -- --test-threads=1`
Expected: PASS, with `apps/docs/public/openapi.json` gaining both search paths.

- [ ] **Step 10: Commit**

```bash
git add src/protocol.rs src/modules/ src/grpc.rs src/cli/ apps/docs/public/openapi.json
git commit -m "feat(registry): expose object search over HTTP, gRPC, and the CLI"
```

---
### Task 6: Document the search surface

Folded into one task because these files change together and share a single review. The ADR recording the provider boundary belongs to Plan 2, which is where that decision is made.

**Files:**
- Modify: `README.md`, `ACTUAL_CAPABILITIES.md`
- Create: `features/suites/s0_cli/registry_search.feature`
- Modify: `tests/bdd.rs`

**Interfaces:**
- Consumes: the surfaces from Task 5.
- Produces: no library API.

- [ ] **Step 1: Update the prose documentation**

- `README.md`: document `hologram registry search` and `hologram files search` with their flags in the CLI section, and add `[registry]` to the configuration section.
- `ACTUAL_CAPABILITIES.md`: add object search under "Implemented and exercised". Be precise about what it is, because that document is deliberately strict: search filters on stored metadata — kind, media type, filename substring, size range, creation-time range — and is bounded and paginated. It is not full-text and not semantic.

- [ ] **Step 2: Add the public-boundary scenario**

Create `features/suites/s0_cli/registry_search.feature`:

```gherkin
Feature: Object search over the registry provider

  Objects are searchable by their stored metadata through a bounded,
  paginated surface that behaves the same whichever provider backs it.

  Scenario: Filtering objects by kind
    Given a running hologram daemon
    When I store a file object named "alpha.txt"
    And I store a file object named "beta.txt"
    And I search objects with kind "file"
    Then the search result contains 2 objects
    And every returned object has kind "file"

  Scenario: Paging through results without overlap
    Given a running hologram daemon
    When I store 3 file objects
    And I search objects with limit 2
    Then the search result contains 2 objects
    And the search result carries a cursor
    When I search objects with limit 2 from that cursor
    Then the search result contains 1 object
    And no object appears on both pages
```

- [ ] **Step 3: Implement the step definitions**

Add the steps to `tests/bdd.rs`, following the patterns already there: reuse the existing daemon fixture, drive the CLI with `--json`, and parse stdout rather than asserting on human-readable text.

- [ ] **Step 4: Run the scenarios**

Run: `cargo test --package hologram-live --features bdd --test bdd --locked`
Expected: PASS, including both new scenarios.

- [ ] **Step 5: Run the full verification gate**

Run: `just verify`
Expected: PASS — fmt, file-size, product-boundary, check, test, clippy, bdd, build, and smoke.

- [ ] **Step 6: Commit**

```bash
git add README.md ACTUAL_CAPABILITIES.md features/ tests/bdd.rs
git commit -m "docs: document the bounded object search surface"
```

---

## Self-Review

**Spec coverage for this plan's scope.** §1 module layout and the synchronous trait → Task 1. §3 search contract, opaque cursors, id ordering, surfaces → Tasks 3 and 5. §5 configuration → Task 4. §7.1 the creation-time bug → Task 2. §7.2 the `list` scan stays a scan and moves behind the search seam → Task 3. §6 testing, for the local half → Tasks 2, 3, 6. Everything in §2 and §4, and the conformance suite, belongs to Plan 2.

**One deliberate divergence, recorded at its task.** Task 4 does not bump the config schema version. Every section on `AppConfig` carries `#[serde(default)]`, and the version machinery with `RETIRED_KEYS` exists for removals, so an added defaulted section loads against older files unchanged. Task 4 Step 1 proves this with a test rather than asserting it.

**Placeholder scan.** No TBDs. Every code step carries real code. No step defers to another task's content.

**Type consistency.** `ObjectQuery` and `ObjectPage` field names match between Task 3, where they are defined, and Task 5, where they are serialized. `effective_limit` and `matches` are defined once in Task 3 and used in Tasks 3 and 5. `RegistryProvider::search` has one signature across Tasks 3 and 5.

**One verification note for the executor.** Task 5's handler tests assume `serde_urlencoded` is reachable transitively through axum. If it is not, use `axum::extract::Query::<ObjectQuery>::try_from_uri` against a constructed `Uri` — do not add a dependency to make a test compile.
