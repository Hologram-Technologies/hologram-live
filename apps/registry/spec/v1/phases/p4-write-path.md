# P4. Write path: uploads, manifest `PUT`, concurrency, crash safety

> Task level. **Steps to be written before the phase starts** (day 12), using golden transcripts 7 to 11.
> Entry: P3 exit. Engineer A. 4.5 days (days 13 to 17).

**Goal:** `docker push` works. No layer is ever in memory. A kill at any point leaves a store a restart can use. Exit: conformance push category green; `docker push` then `pull` round trip in CI; crash suite green.

**Files:**

| File | Holds | Lines |
|---|---|---|
| `src/modules/oci/uploads.rs` | routes 11 to 16 | 500 |
| `src/modules/oci/body.rs` | request body → exact 4 MiB frames → store | 200 |
| `src/modules/oci/manifests.rs` | adds route 6 | +250 |
| `src/modules/oci/media.rs` | media types, manifest parsing, `ManifestPlan` | 350 |
| `tests/oci_http.rs`, `tests/oci_memory.rs`, `tests/oci_concurrency.rs`, `tests/oci_crash.rs` | | exempt |

---

## Task 1: Chunked uploads, streamed

**Requirements:** FR-006, FR-020, SC-007. **Files:** create `uploads.rs`, `body.rs`; modify `Cargo.toml` (`http-body-util`, `bytes` direct; both locked).

**Interfaces:**
- `async fn stream_into(store: Arc<OciStore>, id: UploadId, mut offset: u64, body: Body) -> Result<u64, OciError>`:

```rust
let mut framer = Framer::default();
let mut stream = body.into_data_stream();
while let Some(piece) = stream.next().await {
    let piece = piece.map_err(OciError::client_gone)?;
    for frame in framer.push(&piece) {                     // zero or more exact 4 MiB frames
        let store = store.clone(); let id = id.clone();
        offset = tokio::task::spawn_blocking(move || store.upload_append(&id, offset, &frame)).await??;
    }
}
if let Some(tail) = framer.finish() { /* same, once */ }
Ok(offset)
```

One frame in memory per upload; back-pressure is natural because the next body piece is not polled while a frame is being written.

- Handlers take `Request` (not `Bytes`), so the server-wide `DefaultBodyLimit` does not bind them (R8). No `DefaultBodyLimit::disable()` anywhere.
- Route 14 `PATCH`: if `Content-Range` is present its start must equal the session's offset, else 416 with the true `Range`. Without `Content-Range`, append at the current offset (what `docker` does).
- Route 15 `PUT`: optional final body through `stream_into`, then `spawn_blocking(upload_finish)`. This call re-reads the whole blob (K4): no timeout on this route.
- Response headers per `contracts/registry-api.md` rows 11 to 16, corrected by golden transcripts 8 and 9.

**Done when:**
1. golden transcripts 8 and 9 reproduce;
2. `tests/oci_memory.rs::two_gib_push_through_the_router_stays_under_200_mb`: a real listener on loopback, a client streaming 2 GiB from a generator, RSS sampled every 100 ms from `/proc/self/statm`, fail over 200 MB (Linux only, `#[ignore]`, run by `ci.yml` in release);
3. `tests/oci_http.rs::the_body_limit_still_binds_the_object_api`: with the module on, `POST /api/v1/objects` with 33 MiB answers 413 as today, and a 33 MiB `PATCH` under `/v2/` answers 202.

**Traps:**
- hyper delivers about 16 KiB pieces. Without `Framer` a 20 GB layer is 1.3 million store calls, each a file open (K3).
- `spawn_blocking(...).await??`: the first `?` is the join error. A panic in the store must become `UNKNOWN` 500, not a hung connection.
- A client that disconnects mid-`PATCH` leaves a valid prefix. Do not abort the session; `docker` resumes from `GET` status.
- `Content-Length` may be absent (chunked transfer encoding). Never require it.

## Task 2: Monolithic upload and cross-repository mount

**Requirements:** FR-002, FR-009. **Files:** `uploads.rs`.

- `POST ?digest=<d>` with a body: begin, `stream_into`, finish, 201. One request, still streamed.
- `POST ?mount=<d>&from=<repo>`: `link_get(from, d)`; if present, `link_add(name, d)` and 201 with `Location: /v2/<name>/blobs/<d>`; otherwise 202 as a fresh session. The no-`from` and unknown-`from` cases as golden transcript 10 recorded. Kappa's server ignores `from`; this one must not: a mount without the source check is a way to read any blob by guessing its digest.

**Done when:** golden transcripts 7 and 10 reproduce; `tests/oci_http.rs::a_mount_cannot_reach_a_blob_the_source_does_not_link`.

## Task 3: Manifest `PUT` with validation

**Requirements:** FR-005, FR-020, FR-022. **Files:** `manifests.rs`, create `media.rs`.

**Interfaces:**
- Body: `http_body_util::Limited::new(body, 4 * 1024 * 1024)` then `collect()`. Over the cap: the status golden transcript 11 recorded (**B:`manifest-too-large`**).
- `media::plan(content_type: &str, bytes: &[u8]) -> Result<ManifestPlan, OciError>`: parses with `serde_json` into a permissive struct (unknown fields kept out of the way; bytes are stored verbatim, never re-serialised), and returns `ManifestPlan { kind, must_exist, subject }` for `OciStore::manifest_put` (P1 T7).
- Validation rules 1 to 7 in `contracts/registry-api.md`, "Manifest validation".
- Accepts: Docker manifest v2 and list, OCI manifest and index, any `artifactType`, any layer media type (Helm, cosign, `application/vnd.ollama.image.*`). Descriptors may carry `blake3:` digests (FR-022).
- `subject` is never required to exist and never required to be a manifest (the hub points it at a blob, H4). Response carries `OCI-Subject`.

**Done when:** golden transcript 11 reproduces; conformance push category green; `apps/registry/gates/clients/docker.sh` green against a local build, including the two-platform buildx push.

**Traps:**
- Never re-serialise a manifest. The digest is over the bytes the client sent.
- A manifest `PUT` by digest must check the digest against the body (`DIGEST_INVALID`), not trust the path.
- Foreign layers (`urls` set) are not checked for existence.

## Task 4: Concurrency tests

**Requirements:** FR-006, FR-009. **Files:** create `tests/oci_concurrency.rs`.

The six rows of the concurrency table in `contracts/registry-api.md`, one test each, against a real listener on loopback with 8 client threads: `same_blob_twice`, `delete_during_pull`, `tag_moved_during_get`, `concurrent_patch_one_session`, `tag_last_writer_wins`; `refuses_beside_live_server` is written here as `#[ignore]` and enabled in P8 T1.

**Done when:** each passes 50 consecutive runs locally (`for i in $(seq 50)`) and in CI once per commit. A test that is flaky is a finding about the rule, not about the test.

## Task 5: Crash tests

**Requirements:** FR-006; `data-model.md` crash tables. **Files:** create `tests/oci_crash.rs`; extend `tests/support/oci_child.rs` with kill points.

The child binary takes `--die-at <point>`: `after-post`, `mid-patch`, `before-complete`, `after-rename-before-link` (needs a test-only hook: `OciStore` calls `crate::oci_store::testhook::reached("after-rename")`, a no-op unless the `OCI_TEST_DIE_AT` environment variable is set, in which case it calls `std::process::abort()`), `after-manifest-bytes`, `after-manifest-link`. For each, the parent reopens the store and asserts the row of the crash table: what is reachable, what a retry does, that `resume_uploads` reports what the table says.

**Done when:** seven kill points, seven passing tests, on three systems.

**Traps:**
- `abort()` does not flush; that is the point. Do not use `exit()`.
- The hook reads the environment once into a `OnceLock`. It must cost nothing in production and must not be a cargo feature (a feature would mean the tested binary is not the shipped one).

---

## Phase exit

Conformance push green and blocking from day 17. `docker push` and `pull` round trip in CI. Crash and concurrency suites green. Closes FR-005, FR-006. B points gate C scripts at the product the same day.
