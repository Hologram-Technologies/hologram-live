# Contract: the registry API under `/v2/`

What the module must answer. Source: the registry's API document and the OCI Distribution specification 1.1, recalled, not fetched today **[memory]**. Where a cell says **B:`name`**, the behaviour is not guessed: the gate B scenario of that name records what `registry:3` does, and that becomes the contract.

Every response under `/v2/`, errors included, carries `Docker-Distribution-API-Version: registry/2.0`. Every error body is `{"errors":[{"code":…,"message":…,"detail":…}]}` with `Content-Type: application/json`.

`<name>` may contain slashes. `<ref>` is a tag or a digest.

## Routes

| # | Method and path | Success | Headers on success | Errors | Body in, body out |
|---|---|---|---|---|---|
| 1 | `GET /v2/` | 200 `{}` | | 401 `UNAUTHORIZED` | none, JSON |
| 2 | `GET /v2/_catalog?n=&last=` | 200 `{"repositories":[…]}` | `Link: </v2/_catalog?last=<x>&n=<n>>; rel="next"` when more | 400 `PAGINATION_NUMBER_INVALID` | none, JSON |
| 3 | `GET /v2/<name>/tags/list?n=&last=` | 200 `{"name":…,"tags":[…]}` | `Link` as above | 404 `NAME_UNKNOWN`, 400 `PAGINATION_NUMBER_INVALID` | none, JSON |
| 4 | `GET /v2/<name>/manifests/<ref>` | 200 | `Content-Type` = stored media type, `Docker-Content-Digest`, `ETag: "<digest>"`, `Content-Length` | 404 `MANIFEST_UNKNOWN`, 404 `NAME_UNKNOWN`, 400 `NAME_INVALID`, `TAG_INVALID`, `DIGEST_INVALID`. `Accept` mismatch: **B:`manifest-accept`** | none, bytes (≤ 4 MiB, from memory) |
| 5 | `HEAD /v2/<name>/manifests/<ref>` | 200, no body | as 4 | as 4 | none |
| 6 | `PUT /v2/<name>/manifests/<ref>` | 201 | `Location: /v2/<name>/manifests/<digest>`, `Docker-Content-Digest`, `OCI-Subject: <digest>` when the body has `subject` | 400 `MANIFEST_INVALID`, `MANIFEST_BLOB_UNKNOWN`, `DIGEST_INVALID`, `TAG_INVALID`, `NAME_INVALID`; 413 over 4 MiB: **B:`manifest-too-large`** | bytes ≤ 4 MiB, read whole; none |
| 7 | `DELETE /v2/<name>/manifests/<ref>` | 202 | | 405 `UNSUPPORTED` when delete is off; 404 `MANIFEST_UNKNOWN`; delete by tag: **B:`delete-by-tag`** | none |
| 8 | `GET /v2/<name>/blobs/<digest>` | 200, or 206 with `Range` | `Content-Length`, `Docker-Content-Digest`, `Content-Type: application/octet-stream`, `Accept-Ranges: bytes`, `ETag`, `Content-Range` on 206 | 404 `BLOB_UNKNOWN`, 400 `DIGEST_INVALID`, 416 `RANGE_INVALID`. Multi-range and suffix range: **B:`blob-range-forms`** | none, **stream** |
| 9 | `HEAD /v2/<name>/blobs/<digest>` | 200 | as 8 | as 8 | none |
| 10 | `DELETE /v2/<name>/blobs/<digest>` | 202 | | 405 `UNSUPPORTED` when off; 404 `BLOB_UNKNOWN` | none |
| 11 | `POST /v2/<name>/blobs/uploads/` | 202 | `Location: /v2/<name>/blobs/uploads/<uuid>`, `Range: 0-0`, `Docker-Upload-UUID`, `Content-Length: 0` | 400 `NAME_INVALID` | none |
| 11b | `POST …/uploads/?digest=<d>` with a body (monolithic) | 201 | `Location: /v2/<name>/blobs/<d>`, `Docker-Content-Digest` | 400 `DIGEST_INVALID`, `SIZE_INVALID` | **stream** in |
| 12 | `POST …/uploads/?mount=<d>&from=<repo>` | 201 when `<repo>` holds `<d>`; otherwise 202 as route 11 | as 11b or 11 | mount without `from`: **B:`mount-no-from`** | none |
| 13 | `GET /v2/<name>/blobs/uploads/<uuid>` | 204 | `Range: 0-<n-1>`, `Docker-Upload-UUID`, `Location` | 404 `BLOB_UPLOAD_UNKNOWN` | none |
| 14 | `PATCH /v2/<name>/blobs/uploads/<uuid>` | 202 | `Location`, `Range: 0-<n-1>`, `Docker-Upload-UUID` | 404 `BLOB_UPLOAD_UNKNOWN`; 416 `RANGE_INVALID` when `Content-Range` does not start at the current offset | **stream** in, with or without `Content-Range` and `Content-Length` |
| 15 | `PUT /v2/<name>/blobs/uploads/<uuid>?digest=<d>` | 201 | `Location: /v2/<name>/blobs/<d>`, `Docker-Content-Digest` | 400 `DIGEST_INVALID` (missing, malformed, or mismatch), 404 `BLOB_UPLOAD_UNKNOWN` | optional final chunk, **stream** in |
| 16 | `DELETE /v2/<name>/blobs/uploads/<uuid>` | 204 | | 404 `BLOB_UPLOAD_UNKNOWN` | none |
| R | `GET /v2/<name>/referrers/<digest>?artifactType=` | 200 OCI index, empty index when none | `Content-Type: application/vnd.oci.image.index.v1+json`, `OCI-Filters-Applied: artifactType` when filtered | 400 `DIGEST_INVALID`. Whether `registry:3` serves this at all: **B:`referrers-present`** | none, JSON |

Other shapes:
- `GET /v2` without the slash: **B:`v2-no-slash`**.
- Any other path under `/v2/`: 404. Body shape: **B:`unknown-route`**.
- A known path with a wrong method: 405. Body and `Allow` header: **B:`wrong-method`**.
- `OPTIONS`: **B:`options`**.
- `Location` is relative or absolute: **B:`location-form`** (it depends on `http.relativeurls` and `http.host`).
- The `Range` header on upload responses has no `bytes=` prefix. On blob `GET` requests it has.
- `Location` of an upload carries opaque query state in `registry:3` (`?_state=…`). Clients must follow it verbatim; ours has none. Listed in `apps/registry/DIFFERENCES.md` as a normalised value, not a difference in behaviour.

## Error codes

| Code | Status | Default message | First raised in |
|---|---|---|---|
| `BLOB_UNKNOWN` | 404 | blob unknown to registry | P3 |
| `BLOB_UPLOAD_INVALID` | 404 | blob upload invalid | P4 |
| `BLOB_UPLOAD_UNKNOWN` | 404 | blob upload unknown to registry | P4 |
| `DIGEST_INVALID` | 400 | provided digest did not match uploaded content | P3 |
| `MANIFEST_BLOB_UNKNOWN` | 400 | blob unknown to registry | P4 |
| `MANIFEST_INVALID` | 400 | manifest invalid | P4 |
| `MANIFEST_UNKNOWN` | 404 | manifest unknown | P3 |
| `MANIFEST_UNVERIFIED` | 400 | manifest failed signature verification | never by us (schema 1 only); the code exists so the table is whole |
| `NAME_INVALID` | 400 | invalid repository name | P3 |
| `NAME_UNKNOWN` | 404 | repository name not known to registry | P3 |
| `PAGINATION_NUMBER_INVALID` | 400 | invalid number of results requested | P7 |
| `RANGE_INVALID` | 416 | invalid content range | P3 |
| `SIZE_INVALID` | 400 | provided length did not match content length | P4 |
| `TAG_INVALID` | 400 | manifest tag did not match URI | P4 |
| `UNAUTHORIZED` | 401 | authentication required | P6 |
| `DENIED` | 403 | requested access to the resource is denied | P6 (read-only mode is out of scope; raised only if htpasswd grows roles, which it does not in v1). Exists for the table |
| `UNSUPPORTED` | 405 | The operation is unsupported. | P7 |
| `TOOMANYREQUESTS` | 429 | too many requests | never by us in v1. Exists for the table |

The registry's source also defines `UNKNOWN` (500) and `UNAVAILABLE` (503), outside the documented 18. We raise `UNKNOWN` for an internal failure. Shape: **B:`internal-error`** cannot be provoked; taken from the source.

Exact messages and `detail` payloads: **B:`errors-<code>`**, one scenario per code that `registry:3` can be made to emit.

## Grammars

| Thing | Rule |
|---|---|
| Repository name | components `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*` joined by `/`; whole name at most 255 characters |
| Tag | `[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}` |
| Digest | `algorithm:hex`. `sha256` with 64 lowercase hex. `sha512` with 128: **B:`digest-sha512`**. `blake3` with 64 lowercase hex: accepted by us (FR-022), refused by the reference, listed in `apps/registry/DIFFERENCES.md` |
| Upload id | a UUID, 36 characters. Anything else is `BLOB_UPLOAD_UNKNOWN`, never a path |

Draft 1 gave the name separator as `[._-]`. The reference allows one `.`, one or two `_`, or any run of `-`. `a__b` and `a---b` are valid; `a___b` and `a..b` are not. These four are in the parser's table test.

## Path parsing

axum cannot put a wildcard in the middle of a path. The module registers three routes: `/v2/`, `/v2/_catalog`, `/v2/{*rest}`. `rest` is parsed by hand.

Rule: find the **last** occurrence of, in this order of test: `/blobs/uploads/`, `/manifests/`, `/blobs/`, `/referrers/`, and a trailing `/tags/list`. What is left of it must be a valid repository name. What is right of it must be a valid reference, digest or upload id, with no further slash.

| `rest` | Repository | Route |
|---|---|---|
| `team/tags/app/manifests/latest` | `team/tags/app` | 4 |
| `a/blobs/b/blobs/sha256:<64>` | `a/blobs/b` | 8 |
| `x/manifests/y/tags/list` | `x/manifests/y` | 3 |
| `foo/blobs/uploads/` | `foo` | 11 |
| `foo/blobs/uploads/<uuid>` | `foo` | 13 to 16 |
| `blobs/uploads/blobs/uploads/` | `blobs/uploads` | 11 |
| `Foo/manifests/latest` | none | plain 404: the reference's router has the name grammar in its patterns (measured, gate B `names`) |
| `foo/manifests/a/b` | none | 404 (reference has a slash): **B:`unknown-route`** |
| `_catalog/manifests/x` | none | plain 404, as above (measured) |

## Manifest validation (route 6)

1. Body at most 4 MiB, read whole into memory. This is the only body the module ever holds whole.
2. `Content-Type` is one of: Docker manifest v2, Docker manifest list v2, OCI manifest v1, OCI index v1. Schema 1 types: 400 `MANIFEST_INVALID`. Missing `Content-Type`: **B:`manifest-no-content-type`**.
3. JSON parses; `schemaVersion` is 2; a `mediaType` field, if present, equals `Content-Type`.
4. If `<ref>` is a digest, it equals the digest of the body. Else 400 `DIGEST_INVALID`.
5. Every descriptor in `config`, `layers` and `manifests` is linked in **this** repository. Else 400 `MANIFEST_BLOB_UNKNOWN`, with the missing digests in `detail`. Exceptions: layers with `urls` (foreign layers) are not checked; `subject` is never checked (it may not exist yet).
6. Children of an index that are absent: **B:`index-missing-child`** (the reference has been lenient here).
7. Store bytes, then link, then referrer link when `subject` is set, then tag. Order is the crash rule in `data-model.md`.

## Concurrency rules

| Case | Rule | Held by |
|---|---|---|
| Two pushes of the same blob | Both sessions are independent. Both finish 201. The store keeps one file (rename is skipped when the path exists, K4). Each repository gets its own link | `tests/oci_concurrency.rs::same_blob_twice` |
| Delete racing a pull | A blob `GET` holds an open file handle. Delete removes the link only; the file goes at the next garbage collection, which cannot run beside the server. The pull completes | `…::delete_during_pull` |
| Tag moved while it is read | A manifest `GET` by tag resolves tag to digest once, then serves that digest. The response is always one consistent manifest, old or new | `…::tag_moved_during_get` |
| Garbage collection beside a live server | Refused: the store is locked by the server process (K11). Exit code 1, message names the running server, nothing is changed | `tests/oci_gc.rs::refuses_beside_live_server` |
| Two `PATCH` on one session at once | One proceeds; the other gets 416 `RANGE_INVALID` because its offset is stale. A per-session mutex orders them | `…::concurrent_patch_one_session` |
| `PUT` manifest twice with the same tag | Last writer wins; each write is one `tag_set`. Both digests stay linked | `…::tag_last_writer_wins` |
