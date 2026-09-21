# P3. Registry module: self-authentication, routing, errors, read path

> Task level. **Steps to be written before the phase starts** (day 8), in the style of `p1-store-adapter.md`, using golden transcripts 1 to 6 from P2 as fixtures.
> Entry: P1 exit. Engineer A. 4 days (days 9 to 12).

**Goal:** `/v2/` exists inside the server, outside the bearer layer, in the registry's own shape. Blobs and manifests can be read. Exit: the conformance suite's pull category is green; the parser table is green.

**Files in this phase:**

| File | Holds | Lines |
|---|---|---|
| `src/modules/oci/mod.rs` | `OciRegistryModule`, descriptor, router, layers | 250 |
| `src/modules/oci/path.rs` | `parse(method, rest) -> Route` | 300 |
| `src/modules/oci/error.rs` | `ErrorCode`, `OciError`, `From<OciStoreError>` | 300 |
| `src/modules/oci/respond.rs` | header helpers: digest, range, location, link | 200 |
| `src/modules/oci/blobs.rs` | routes 8, 9 | 300 |
| `src/modules/oci/manifests.rs` | routes 4, 5 (P4 adds 6) | 250 |

---

## Task 1: A module that authenticates itself (ADR-026)

**Requirements:** FR-015, FR-021 (first half). **Files:** modify `src/module.rs`, `src/modules/mod.rs`, `src/server.rs`, `src/app.rs`; create `src/modules/oci/mod.rs`, `specs/adrs/026-self-authenticating-modules.md`; test in `src/server.rs` and `src/module.rs`.

**Interfaces:**
- `LiveModule` gains `fn authenticates_itself(&self) -> bool { false }` (R1).
- `ModuleRegistry::routers(&self) -> ModuleRouters` where `ModuleRouters { protected: Router<AppState>, open: Router<AppState> }`. `router()` stays, returning `protected` merged with `open`, so existing callers compile.
- `serve_with_ready` (R3): `protected` gets the bearer layer as today; `open` is merged beside it, without it.
- `src/modules/mod.rs`: a second macro arm so the catalogue and the default set differ (R14):

```rust
builtin_modules! {
    default: [ system::SystemModule, registry::KappaRegistryModule, /* … the ten … */ ],
    opt_in:  [ oci::OciRegistryModule ],
}
// builtins() = default + opt_in;  default_builtin_ids() = default only.
```

- `AppState::oci_store(&self) -> Option<&Arc<OciStore>>`: opened in `AppState::build` only when `dev.hologram.live.oci` is enabled, inside `spawn_blocking` (the pattern PR #81 needed for the Kappa provider), at `config.paths.data_dir` (registry mode in P5 points that at `<root>`).
- `OciRegistryModule`: id `dev.hologram.live.oci`, dependency `dev.hologram.live.system`, `authenticates_itself() = true`, router with three routes: `/v2/`, `/v2`, `/v2/{*rest}`, all `any(handler)`; method dispatch is the parser's job. Two layers on the router: the version header on every response, and a span layer that does what `authenticate` does for the others (request id from the same `REQUEST_IDS` counter, `live.server.request` span, R3).

**Done when:** (1) `src/config.rs` test `default_config_enables_the_builtin_module_catalogue` still passes unchanged and a new test asserts `oci` is in `builtins()` and not in the default ids; (2) with the module enabled and `auth.required = true`, `GET /v2/` without a token answers 200 `{}` with the version header while `GET /api/v1/modules` answers 401 `LIVE_AUTHENTICATION_FAILED`; (3) with the module disabled, `GET /v2/` answers 404 `LIVE_NOT_FOUND` as today; (4) the four existing `server.rs` tests pass; (5) BDD suite green.

**Traps:**
- Do not give the module router a `.fallback(…)`. tonic's router has one and the merge collides (R5, R6). The catch-all route makes it unnecessary.
- `REQUEST_IDS` is private to `server.rs`. Expose a `pub(crate) fn next_request_id()`, not the static.
- `AppState::build` runs on the async runtime; `PersistentStore::new` and `redb` block. `spawn_blocking`, then `?`.
- This task touches five shared files. Keep each diff minimal and offer the trait hook and the two-list macro upstream as their own pull request (`plan.md` section 10).

## Task 2: Path parsing from the right

**Requirements:** FR-004. **Files:** create `src/modules/oci/path.rs`; tests in file.

**Interfaces:** `pub fn parse(method: &Method, rest: &str) -> Result<Route, OciError>`:

```rust
pub enum Route {
    Base,
    Catalog,
    TagsList { repo: RepoName },
    Manifest { repo: RepoName, reference: Reference, verb: ManifestVerb },   // Get, Head, Put, Delete
    Blob { repo: RepoName, digest: Digest, verb: BlobVerb },                 // Get, Head, Delete
    UploadStart { repo: RepoName },                                         // POST
    Upload { repo: RepoName, id: UploadId, verb: UploadVerb },              // Get, Patch, Put, Delete
    Referrers { repo: RepoName, digest: Digest },
}
```

Rule and table: `contracts/registry-api.md`, "Path parsing". A known path with a wrong method is 405 with the `Allow` header golden transcript 2 recorded.

**Done when:** the contract's 9-row table passes as a test, plus every path in golden transcripts 1 to 3 parses to the route and status the reference gave, plus every repository name the OCI conformance suite uses (read from its source: `OCI_NAMESPACE`, `OCI_CROSSMOUNT_NAMESPACE` defaults).

**Traps:**
- Test for `/blobs/uploads/` before `/blobs/`. Otherwise `foo/blobs/uploads/<uuid>` parses as a blob named `uploads/<uuid>` and fails as `DIGEST_INVALID` instead of routing.
- The path arrives percent-encoded. Decode once; refuse `%2F` inside a component.
- A digest contains `:`, a tag cannot. `Reference::parse` (P1 T2) already refuses a malformed digest instead of calling it a tag.

## Task 3: Error envelope and store error mapping

**Requirements:** FR-003. **Files:** create `src/modules/oci/error.rs`, `respond.rs`; tests in file.

**Interfaces:**
- `pub enum ErrorCode` with the 18 documented codes plus `Unknown`; `ErrorCode::status()`, `ErrorCode::default_message()`: table in `contracts/registry-api.md`, corrected by golden transcript 4.
- `pub struct OciError { code: ErrorCode, message: Cow<'static, str>, detail: serde_json::Value, headers: HeaderMap }`; `impl IntoResponse`.
- `impl From<(OciStoreError, Context)> for OciError` where `Context` says which route asked, because one store error maps differently by route:

| `OciStoreError` | Blob route | Manifest route | Upload route |
|---|---|---|---|
| `NotInRepository` | `BLOB_UNKNOWN` 404 | `MANIFEST_UNKNOWN` 404 | |
| `UnknownRepository` | `NAME_UNKNOWN` 404 (tags list); elsewhere as the row above | | |
| `Invalid{what:"digest"}` | `DIGEST_INVALID` 400 | same | same |
| `Invalid{what:"repository name"}` | `NAME_INVALID` 400 | same | same |
| `Invalid{what:"tag"}` | | `TAG_INVALID` 400 | |
| `UnknownUpload`, `Invalid{what:"upload id"}` | | | `BLOB_UPLOAD_UNKNOWN` 404 |
| `DigestMismatch` | | `DIGEST_INVALID` 400 | `DIGEST_INVALID` 400 |
| `OffsetMismatch` | | | `RANGE_INVALID` 416, with `Range: 0-<expected-1>` |
| `MissingReferences` | | `MANIFEST_BLOB_UNKNOWN` 400, digests in `detail` | |
| `Io`, `Locked`, `Layout` | `UNKNOWN` 500; the cause is logged, never sent | | |

**Done when:** one test per code asserts status, body shape, `Content-Type` and the version header; every error in golden transcript 4 is reproduced byte for byte after normalisation.

**Trap:** the reference's messages are what scripts grep. Copy them from the transcript, not from memory.

## Task 4: Blob `GET` and `HEAD`, streamed, with `Range`

**Requirements:** FR-002; streaming rule (`plan.md` 9.1). **Files:** create `src/modules/oci/blobs.rs`; modify `Cargo.toml` (`tokio-util` with `io`, already locked, R23); tests in `tests/oci_http.rs`.

**Interfaces:** `async fn get_blob(state: &AppState, repo: RepoName, digest: Digest, headers: &HeaderMap, head: bool) -> Result<Response, OciError>`. Body: `Body::from_stream(ReaderStream::with_capacity(AsyncBlob::new(reader, start, len), 1 << 20))`. `blob_open` runs in `spawn_blocking`.

`Range` handling, exactly as golden transcript 5 recorded for: one range, open-ended, suffix, multiple ranges, past the end (416 `RANGE_INVALID` with `Content-Range: bytes */<size>`). `ETag` and `If-None-Match` as recorded.

**Done when:** golden transcript 5 reproduces; `tests/oci_http.rs::a_range_reads_only_what_it_serves` (counting reader, 64 MiB blob, 1 MiB range, at most 2 MiB read); `check-oci-streaming.sh` passes.

**Traps:**
- `HEAD` must send `Content-Length` of the blob with an empty body. axum strips the body of a `HEAD` response but only if the route is registered for `HEAD`; with `any()` the handler must build it explicitly.
- A client that disconnects mid-body drops the stream; `AsyncBlob`'s in-flight `spawn_blocking` read finishes and is discarded. No leak, but do not hold a session lock in it.

## Task 5: Manifest `GET` and `HEAD`

**Requirements:** FR-002. **Files:** create `src/modules/oci/manifests.rs`; tests in `tests/oci_http.rs`.

**Interfaces:** `async fn get_manifest(state, repo, reference, headers, head) -> Result<Response, OciError>`. Resolve a tag to a digest once, then read by digest (concurrency table). `Accept` handling as golden transcript 6 recorded; until it is recorded, serve what is stored.

**Done when:** golden transcript 6 reproduces; `tests/oci_concurrency.rs::tag_moved_during_get`.

## Task 6: Gate A job, pull category

**Requirements:** SC-002. **Files:** modify `.github/workflows/gates.yml`.

Add job `gate-a`: build the server, start it with the module enabled on loopback, seed it through the adapter with the conformance suite's expected image (`OCI_TAG_NAME`, `OCI_MANIFEST_DIGEST`, `OCI_BLOB_DIGEST` environment), run `ghcr.io/opencontainers/distribution-spec/conformance` pinned by digest with `OCI_TEST_PULL=1` and the other three `0`, `OCI_VERSION=1.1`, upload `report.html` as an artifact. `continue-on-error: true` until day 12; then pull blocks.

**Done when:** the pull category reports zero failures in CI.

**Trap:** the suite's environment variable names and the image location are from memory. Read `conformance/README.md` at the pinned commit in step 1 and correct this task before writing anything else.

---

## Phase exit

Conformance pull green and blocking. Parser table green. Closes FR-004. Unblocks P4 and P5 T3. On day 12 gate B turns on against the first image (red allowed until day 24).
