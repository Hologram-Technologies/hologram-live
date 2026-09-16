# Design: Kappa Registry provider and object search

- Status: approved, not yet implemented
- Date: 2026-09-15
- Scope: a second `RegistryProvider` implementation backed by an external
  kappa-registry instance, and an object-search contract served identically by
  both providers. A standalone client SDK and dependency-currency work are
  explicitly out of scope; see "Adjacent work" below.

## Context

`src/registry.rs` declares a `RegistryProvider` trait whose documented purpose is
to let "a future adapter speak to the external Kappa Registry service without
changing module routes, native operation IDs, or desktop clients". Only
`LocalRegistryProvider` exists. `DEPENDENCIES.md` records the same intent from
the other side: "Kappa Registry remains an external service/project. Hologram
Live integrates it through the registry provider boundary rather than adding its
workspace crates to this dependency graph."

The seam is therefore declared, documented, and unused. This design fills it.

Separately, neither provider can search. `ObjectStore::list` filters on exactly
one field (`kind`) and returns every match unpaginated, and there is no other
query surface anywhere in the repository. Listing a large object store is
currently an all-or-nothing read.

### The decisive fact

The external registry is `github.com/uoR-Foundation/kappa-registry`: a
single-binary Rust server exposing OCI, Git, S3, Nix, and AT Protocol surfaces
over one content-addressed store.

Its identity format and ours are already the same. `kappa-core/src/kappa/mod.rs`
defines a kappa-label as `<algorithm>:<lowercase-hex-digest>` and lists `blake3`
as one of six first-class axes at 32 digest bytes and 71 label bytes, stating
that "all six axes are structurally equal; no axis receives special policy
treatment". `kappa-module-oci/src/blob.rs` opens with "every digest algorithm is
first-class" and names its path parameter `kappa`.

`ObjectStore::validate_id` accepts exactly `blake3:` followed by 64 lowercase hex
digits — 71 bytes, a valid kappa-label. The two systems converged independently
on one identity format.

This is why this design adds an adapter rather than authoring a wire contract. A
third content-addressed object API in an ecosystem that already has one would
fragment it for no gain.

### What was verified

A throwaway spike built kappa-registry at `2026-09-02` and exercised a live
instance. Confirmed:

- `PUT /v2/{ns}/blobs/blake3:<hex>` returns `201`. Our existing object IDs are
  accepted unchanged.
- Tampered content under the same kappa returns `400 DIGEST_INVALID`. The server
  re-verifies; integrity is enforced remotely, not merely asserted locally.
- `GET` round-trips bytes exactly. `HEAD` returns `content-length`,
  `docker-content-digest`, `x-kappa-label`, `x-kappa-axis`, and an immutable
  `cache-control`. `DELETE` returns `202`.
- `Content-Type` supplied at PUT is preserved and returned on GET.
- `GET /v2/{ns}/blobs/?prefix=` lists and filters correctly.
- A manifest whose `subject` is a blob makes
  `GET /v2/{ns}/referrers/<blob-kappa>` return that manifest's descriptor **with
  its annotations inline**, in one request.
- `PUT /v2/{ns}/manifests/blake3:<hex>` returns `201`, so manifests *can* be
  blake3-addressed. Addressing a manifest by *tag name* instead causes the server
  to assign the manifest its own sha256 kappa. See section 2 for why this design
  writes by tag and treats the manifest's own address as server-owned.
- `GET /v2/{ns}/tags/list?n=2` paginates properly, returning
  `link: </v2/{ns}/tags/list?last=file2.txt>; rel="next"` and honouring `last=`.
- Namespaces auto-create on first write via `namespace_resolve_or_create`.

### What was disproved

`GET /v2/{ns}/blobs/_meta?key=&value=` is **not** a general metadata search.
`blob_put_meta` writes the redb table `BLOB_META` keyed `{kappa}\0{key}`, while
`meta_query` reads `NS_META` keyed `{ns}\0{key}\0{value}`. The two never meet.
`meta_set`, the only writer of `NS_META`, is called with exactly one key across
the entire upstream codebase — `"object-type"`, with values `manifest`,
`composition`, `witness`, `edge`, `pin`, `schema`, and `filter`.

Verified in both directions: `?key=object-type&value=manifest` returns our
manifest kappas; `?key=content-type&value=text/plain` returns `[]` even when
URL-encoded.

Consequence: search cannot be delegated upstream. It is implemented in Hologram
Live over upstream's listing primitives, and the provider seam is the only place
it lives.

## Decision

### 1. Module layout and the synchronous trait

`src/registry.rs` becomes a directory:

```
src/registry/mod.rs     trait, ObjectQuery, ObjectPage, provider selection
src/registry/local.rs   LocalRegistryProvider, moved unchanged
src/registry/kappa.rs   KappaRegistryProvider and its wire client
```

**`RegistryProvider` stays synchronous.** Every call site already wraps provider
calls in `tokio::task::spawn_blocking` (`src/modules/registry.rs`,
`src/modules/files.rs`), and `reqwest`'s `blocking` feature is already enabled in
`Cargo.toml`. `KappaRegistryProvider` therefore uses `reqwest::blocking` and
slots in behind `Arc<dyn RegistryProvider>` with no change to the HTTP modules,
gRPC service, or CLI. An async trait would require boxed futures at every call
site, because `async fn` in traits is not `dyn`-safe, and would buy nothing.

Risk: `reqwest::blocking` panics if it detects an active reactor on the calling
thread. Invoking it from inside `spawn_blocking` is the documented-safe pattern,
but this is verified against a live call in the first implementation step rather
than assumed. If it does not hold, the fallbacks are a dedicated client thread
with a channel, or converting the trait to async with boxed futures.

### 2. Object representation

Each Hologram object is one blob plus one sidecar OCI manifest.

| `ObjectMetadata` field | Carrier |
| ---------------------- | -------------------------------------------- |
| `id`                   | blob kappa, `blake3:<hex>`, unchanged        |
| `size`                 | `content-length` from `HEAD`                 |
| `media_type`           | blob `Content-Type`                          |
| `kind`                 | annotation `dev.hologram.kind`               |
| `filename`             | annotation `dev.hologram.filename`           |
| `created_at_millis`    | annotation `dev.hologram.created-at-millis`  |

The sidecar manifest sets `artifactType` to
`application/vnd.hologram.object.v1+json`, its `subject` to the blob descriptor,
and is tagged `blake3_<hex>` — the object's kappa with `:` replaced by `_`. That
tag is 71 characters and matches OCI's tag grammar
(`[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`), so it is always valid regardless of the
object's filename.

`subject` points at the blob because that is what `subject` means: this artifact
is about that blob. It costs nothing, and any OCI-aware tool can discover our
metadata through the standard referrers path.

An earlier variant pointed every manifest at a single synthetic namespace root so
that one `referrers` call could enumerate every object with annotations inline.
It is rejected. It depends on referrers pagination, which the spike did not
verify; it optimises enumeration at the cost of correct semantics and an
unbounded per-root index; and its failure mode is baked into persisted data,
requiring every manifest to be rewritten. Blob-as-subject fails cheaply instead:
if enumeration proves slow, an index is added behind the search seam without
touching stored data.

**Write** (`put_object`), two idempotent PUTs:

1. `PUT /v2/{ns}/blobs/{kappa}` with `Content-Type: {media_type}`.
2. `PUT /v2/{ns}/manifests/blake3_<hex>` — addressed by the object's *tag*,
   carrying the annotations above.

Writing by tag is what creates the tag, and the tag is what makes point lookup
and enumeration work. The server consequently assigns the manifest its own
sha256 kappa. That address is deliberately not part of our identity contract:
the object's identity is the blob kappa, which stays blake3, and the manifest is
only ever reached through its tag or through referrers. Accepting a server-owned
manifest address costs one axis inconsistency in data we never address directly;
insisting on a blake3 manifest address would mean a second PUT by digest to
create the same content twice under two addresses, for no gain.

**Point read**: `GET /v2/{ns}/manifests/blake3_<hex>` for metadata;
`GET /v2/{ns}/blobs/{kappa}` for bytes. `metadata(id)` needs only the first.

**Enumeration**: `GET /v2/{ns}/tags/list?n=&last=`, then one manifest fetch per
tag, issued with bounded concurrency. This is N+1 and accepted as such; see
section 3 for the bound and the truncation contract.

**`rename_file` is not atomic.** The blob is immutable, so a rename writes a new
manifest and deletes the previous one. A crash between the two leaves two
manifests for one object. Reads therefore resolve ties deterministically: the
manifest with the greatest `dev.hologram.created-at-millis` wins. A crash
degrades to garbage, never to incorrectness.

### 3. Search contract

```rust
pub struct ObjectQuery {
    pub kind: Option<String>,
    pub media_type: Option<String>,
    pub filename_contains: Option<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub created_after_millis: Option<u64>,
    pub created_before_millis: Option<u64>,
    pub limit: u32,
    pub cursor: Option<String>,
}

pub struct ObjectPage {
    pub objects: Vec<ObjectMetadata>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}
```

`limit` defaults to 100 and is clamped to 1000.

**Cursors are opaque strings, valid only for the provider that issued them.**
The local provider encodes its scan position; the kappa provider encodes
upstream's `last=` tag cursor. A structured cursor would leak provider internals
into a public API and prevent either side from changing independently. A cursor
presented to a provider that did not issue it is rejected with
`LiveError::Protocol`.

**Ordering is ascending by kappa, identically for both providers.** Upstream's
`tags/list` returns lexical order; imposing time ordering on it would require
enumerating every object before returning the first page, which defeats
pagination. Kappa ordering is stable, deterministic, and content-addressed, so
both providers honour it cheaply.

`list_objects` keeps its existing newest-first contract and its bare-array
response. Only the new `search` surface is kappa-ordered. No shipped response
shape changes.

**Filtering for the kappa provider happens in Hologram Live**, because upstream
cannot filter on `kind` or `filename`. A selective query therefore walks upstream
pages until `limit` is satisfied or the walk is exhausted. That walk is bounded
by `registry.max_scan_pages` (default 20). When the bound is reached before
`limit` is satisfied, the response sets `truncated: true`, returns a usable
`next_cursor`, and logs the number of objects examined. Truncation is always
explicit; a capped result is never presented as a complete one.

New surfaces, all additive:

- `GET /api/v1/objects/search` and `GET /api/v1/files/search`, returning
  `ObjectPage`.
- Native operations `registry.search` and `files.search`, both
  `OperationKind::Read` and `fallback_safe_before_dispatch: true`.
- `hologram registry search` and `hologram files search`, honouring `--json`.

### 4. Error mapping and retry

Upstream returns the OCI error envelope,
`{"errors":[{"code":"DIGEST_INVALID","message":"..."}]}`. It maps onto existing
`LiveError` variants, so remote failures surface through the existing
`HttpError` conversion in `src/modules/mod.rs` with no new plumbing:

| Upstream                     | `LiveError`                     | HTTP |
| ---------------------------- | ------------------------------- | ---- |
| 400 `DIGEST_INVALID`         | `Protocol`                      | 400  |
| 404                          | `NotFound`                      | 404  |
| 401 / 403                    | `Authentication` / `Authorization` | 401 / 403 |
| 413 `SIZE_EXCEEDED`          | `Protocol`                      | 400  |
| 507 `INSUFFICIENT_STORAGE`   | `Io`                            | 500  |
| 429                          | `Transport`                     | 500  |
| connect failure or timeout   | `Transport`                     | 500  |

Content addressing gives a genuine safety property. `LiveClient` returns
`UnknownCommitState` for mutations whose outcome is unknown. A blob or manifest
PUT is idempotent — the content determines the address — so an ambiguous PUT is
retried with bounded backoff rather than reported as unknown.

`rename_file` does not get that treatment. It is a manifest PUT followed by a
DELETE, so an ambiguous failure re-reads state and resolves by the
greatest-`created-at-millis` rule before acting. It is never blindly retried.

### 5. Configuration

```toml
[registry]
provider = "local"                     # "local" | "kappa"
endpoint = "http://127.0.0.1:5000"
namespace = "hologram"
token = ""
request_timeout_secs = 30
max_scan_pages = 20
```

`provider` defaults to `local`, so `hologram` remains a single binary requiring
no external service. `token` is sent as `Authorization: Bearer`; upstream's
`KAPPA_AUTH_REQUIRED` defaults to false, so an empty token is valid against a
development instance.

Configuration is schema-versioned and upgrades rather than rejects older files.
This change bumps the schema version, adds the corresponding upgrade step, and is
covered by a test proving a configuration written before `[registry]` existed
still loads.

Selecting `provider = "kappa"` without a reachable `endpoint` fails at startup
with `LiveError::Config`, not on first request.

### 6. Testing

**The provider conformance suite is the primary asset**: one test body executed
against both `LocalRegistryProvider` and `KappaRegistryProvider`, asserting
identical observable behaviour for put, get, metadata, rename, list, search,
pagination, ordering, and the truncation contract. If the two providers are not
substitutable, that is precisely the defect this seam exists to prevent, and this
suite is the only thing that catches it.

Supporting layers:

- Unit tests, network-free: wire encode and decode, kappa-to-tag conversion,
  cursor round-tripping and cross-provider rejection, error mapping, and the
  `created-at-millis` tie-break.
- Integration tests against a live kappa-registry, skipped cleanly when the
  binary is absent, following the precedent of
  `scripts/check-python-private-registry.sh`.
- A cucumber scenario in `features/` covering the public search boundary.

### 7. Corrections to existing code

Two defects in `src/store.rs` are fixed as part of this work rather than left:

1. **`put` rewrites `created_at_millis` on content that already exists.** For
   content-addressed, immutable objects the creation time must not move. It is
   now load-bearing — it is the rename tie-break of section 2 — so this is a
   blocker, not a nicety. `put` preserves the existing timestamp when metadata is
   already present.
2. **`list` reads and deserializes every metadata file on every call.** It stays
   a scan, since adding an index would take a new dependency for no proven need,
   but it moves behind the search seam and gains the bounds and truncation
   reporting of section 3.

## Consequences

`DEPENDENCIES.md` currently states that kappa-registry is integrated "through the
registry provider boundary rather than adding its workspace crates"; that
sentence becomes true rather than aspirational and needs rewording to describe a
shipped adapter.

**No new dependencies.** `reqwest` with `blocking` is already in the tree, and
search is built on upstream primitives rather than an index, so the budget in
`DEPENDENCIES.md` is untouched.

Documentation updated alongside the implementation: a new ADR 021 recording the
provider decision and the rejected shared-root variant; `ARCHITECTURE.md`, whose
content-store paragraph anticipates exactly this replacement; the README module
table; `ACTUAL_CAPABILITIES.md`; and the `apps/docs` API pages.

## Adjacent work

Deliberately out of scope, each its own cycle:

- **A. Client SDK.** A reusable typed client for third-party callers. It follows
  naturally once the search contract is stable, and should not shape it.
- **B. Dependency currency.** Audited on 2026-09-15. `clap` 4.6.6 to 4.6.7 and
  `@tauri-apps/plugin-dialog` 2.7.2 to 2.7.3 are safe. `wasmtime` 46 to 48,
  `reqwest` 0.12 to 0.13 (whose `rustls-tls` feature is obsolete in 0.13),
  `sysinfo` 0.32 to 0.39, `typescript` 5 to 7, `vite` 7 to 8, and `astro` 5 to 7
  are migrations deserving isolated, revertable changes. `toml` and `sha2` each
  appear twice in the tree.
- **C. `blake3` is pinned below current.** The tree resolves 1.5.5 against a
  latest of 1.8.7 because `uor-prism-crypto 0.4.0`, reached through the pinned
  `uor-hologram` revision, requires `>=1.5, <1.6`. It cannot be fixed in this
  repository and needs an upstream bump. It matters here because blake3 is the
  addressing primitive both systems share.
- **D. Upstream defects found while building the spike.** `kappa-registry`'s
  `Cargo.toml` declares a `[[test]]` section in a virtual workspace manifest,
  which cargo refuses to parse, and its `Cargo.lock` lists the package
  `inventory` twice. Both block a clean clone-and-build and warrant an upstream
  issue. Minor third: the `Link: rel="next"` header on `tags/list` omits the
  `n=` page size, so a client following it naively loses its page size.
