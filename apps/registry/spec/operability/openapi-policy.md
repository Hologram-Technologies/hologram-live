# OpenAPI policy

2026-09-21. Requirement 5 of the brief, as ten rules. Each rule has a check that can fail. Together they are **gate H**.

## Where we start

The Server already generates its document from the code with `utoipa` 5.5.0, which emits **OpenAPI 3.1.0** [read: `Cargo.lock`; `utoipa-5.5.0/src/openapi.rs:1,384`]. Each module contributes its paths through `LiveModule::openapi()` and the host merges them [read: `src/module.rs:59-61`, `:172-178`; `src/server.rs:31-35`]. It is served at `/openapi.json`, rendered by Scalar at `/docs`, and written without a server by `hologram openapi --output` [read: `src/server.rs:56-57`; `justfile`, recipe `docs`]. A unit test asserts a dozen paths are present [read: `src/module.rs:229-263`].

What is missing: nothing checks that the document is valid, complete, true, stable, or usable by a generator. And the registry's `/v2/` routes are served by one catch-all route (`001` P3 T2), which `utoipa` cannot describe from the router.

Distribution publishes **no** OpenAPI document; its API is prose [read: `dist:docs/content/spec/`]. The OCI Distribution specification has none either [memory]. An accurate, tested OpenAPI description of `/v2/` is something no registry in this comparison offers.

## Rules

| # | Rule | Check | When |
|---|---|---|---|
| O1 | **Version.** The document is OpenAPI **3.1.x**, JSON, and validates against the official schema: `github.com/OAI/OpenAPI-Specification`, `schemas/v3.1/schema.json`, pinned by commit | a JSON Schema validator in CI, fed by `hologram openapi --output` (no server needed) | commit |
| O2 | **Lint.** Zero errors and zero warnings under Spectral with `spectral:oas` plus our ruleset `apps/docs/openapi/.spectral.yaml`: every operation has `operationId`, `summary`, at least one tag, at least one 4xx response; no inline schema used twice; every schema has a `description`. A second implementation, `vacuum`, agrees | both linters | commit |
| O3 | **Complete.** Every HTTP route the Server serves **in the running configuration** is in the document, and nothing else. A registry-mode container's document lists `/v2/…` and the system routes, not the ten modules that are off | route completeness test: walk the axum router's registered paths and the `/v2/` parser's route table; compare both ways with the document | commit |
| O4 | **`/v2/` is described by hand, once.** `src/modules/oci/openapi.rs` declares the templated paths the catch-all really serves: `/v2/`, `/v2/_catalog`, `/v2/{name}/tags/list`, `/v2/{name}/manifests/{reference}`, `/v2/{name}/blobs/{digest}`, `/v2/{name}/blobs/uploads/`, `/v2/{name}/blobs/uploads/{uuid}`, `/v2/{name}/referrers/{digest}`; each method; every header in `contracts/registry-api.md` as a response header; `{name}` documented as "may contain `/`", with its pattern. It is generated from the **same route table** the parser uses, so the two cannot drift | O3's test covers it; a unit test builds the document from the table | commit |
| O5 | **Stable operation ids.** Form `<surface>.<noun>.<verb>`: `registry.blob.get`, `registry.upload.patch`, `objects.object.create`. Ids are an API: `oasdiff` treats a changed id as breaking | `oasdiff` | commit |
| O6 | **One error schema per surface, and security declared.** `/v2/` errors are `RegistryErrors` (`{"errors":[{"code","message","detail"}]}`, `code` an enum of the 18 codes plus `UNKNOWN`). Server API errors are `ApiError` (`LIVE_*`). Security schemes: `registryBasic` (HTTP Basic) on `/v2/`; `serverBearer` (HTTP Bearer) on the Server API; `registryBearer` reserved for v1.1 token login. Every operation names its scheme or `security: []`; none is left to a global default | Spectral rules | commit |
| O7 | **Versioned and published.** `info.version` equals the Server version. Each release attaches `openapi.json`, and one per mode (`openapi-registry.json`). The docs site serves the latest and every past version. A change that breaks a client (removed operation, removed response, narrowed schema, changed id) is allowed only in a major; `oasdiff breaking` against the last release's document enforces it | `oasdiff` in CI; release job uploads | commit, release |
| O8 | **True.** Every documented operation, called with generated valid input against a running server, answers with a documented status and a body that matches its schema; zero undocumented 5xx | Schemathesis against the built image, both modes, seeded, 10 minutes | nightly |
| O9 | **Usable.** OpenAPI Generator produces a `python` and a `go` client from the document; both compile; one call each (`registry.base.get`, `registry.tags.list`) succeeds against a running server. Two languages because one generator bug looks like our bug; two rarely agree by accident | CI job | commit |
| O10 | **Deprecation.** An operation or field is marked `deprecated: true` with `x-sunset: <version>` for at least one minor release before a major removes it; the changelog names it; the Server logs one warning per process when a deprecated operation is first called | Spectral rule: `deprecated` requires `x-sunset`; test for the log line | commit |

## What OpenAPI cannot say, stated in the document

A reader who trusts only the document should still be told where it stops. `info.description` and `x-` notes say:

- `{name}` contains slashes. OpenAPI path parameters do not allow that. The document marks `{name}` with `x-multi-segment: true` and gives the pattern; generated clients must not percent-encode `/` in it. O9's generated-client call uses a two-segment name to prove the clients cope, with the documented workaround if one does not.
- Upload `Location` headers are opaque URLs to follow verbatim; they are documented as `format: uri-reference`, not as a path template.
- Blob bodies are streams: `application/octet-stream`, no schema, no size limit.
- Behaviour beyond shape (ordering of chunks, digest verification, paging rules) lives in `contracts/registry-api.md` and is proven by gates A and B, not by the document. The document links to both.

## Rendering

`/docs` stays Scalar. Swagger UI and Redoc must also render the document with no error (`interop-matrix.md` section 3 row 7, per release): the document is for the ecosystem's tools, not for ours alone.

## Cost

5.5 engineer days (parity row B7): `/v2/` description from the route table 2; linters and schema check 0.5; generated clients 1; completeness and Schemathesis 1.5; publishing, `oasdiff` and versions page 0.5.
