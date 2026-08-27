# Current sprint: M4.2 normalized Python rootfs archives

## Sprint status

- State: complete
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Decision: [ADR 017](adrs/017-normalized-python-rootfs-archive.md)
- Goal: remove Docker-export metadata and storage-layout variation from Python
  rootfs layer identity
- Exit signal: the current schema-3 rootfs bundle has one canonical archive
  representation, repeated exports are byte-identical, cold-load execution
  works, and the remaining uncached cross-host proof is precisely tracked

## Contract boundary

- [x] Replace the experimental rootfs envelope with bundle schema 3, magic
  `HOLOPYR2`, and provider `normalized-docker-archive-zstd-v1`.
- [x] Accept exactly one Docker image with the expected content-derived tag.
- [x] Re-address config and layer bytes as `blobs/sha256/<digest>` while
  preserving semantic layer order.
- [x] Emit only the canonical manifest and referenced blobs with lexical member
  order and fixed tar headers.
- [x] Set `SOURCE_DATE_EPOCH=0` for the build and disable injected provenance.
- [x] Reject unsafe paths, duplicate or non-file members, missing references,
  oversized archives, and image-ID mismatches.
- [x] Keep build provenance non-canonical and `reproducible: false` until clean
  builds match across every supported release host.

## Tests and evidence

- [x] Prove differing input member order, JSON key order, timestamps, modes, and
  ownership normalize to identical bytes.
- [x] Cover duplicate members, unexpected tags, and canonical member sets.
- [x] Compile the locked NumPy/pandas application twice and prove equal layer,
  application, archive, and footer identities.
- [x] Remove the generated local image tag, cold-load from the `.holo`, and
  recover three rows, mean `20.0`, and sum `60.0`.
- [ ] Run uncached clean builds on macOS arm64/x86_64, Linux arm64/x86_64, and
  Windows x86_64 and compare config, layer, application, and archive identities.
- [x] Pass formatting, workspace tests/checks, Clippy, BDD, release/smoke,
  documentation, and desktop gates.

Local evidence (2026-08-26): Docker client 29.2.1/server 29.4.0 emitted two
identical normalized exports with rootfs layer κ
`blake3:6ac835129125e3f997a211611c96094e606fdbf332073c02fe2a9f906a7c07f7`,
application κ
`blake3:104da1166bf688727352e966097e1d0ce837c4ad3873199e4d6038d5ac0b24b0`,
archive κ
`blake3:3e302dff5f62ed341d5ce9b65296167bffb93d948330947db366c17d9726aff0`,
and fingerprint
`d01c6246d6efb6909262eea1df0489a575086dab67da236174f2f520b932db2c`.
The cold-load direct run completed successfully after removing the local tag.

Repository evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests, Clippy with warnings
denied, 12 BDD scenarios with 123 steps, the optimized server build, and the
isolated smoke test. `just docs` regenerated OpenAPI and built all 13 static
pages. `npm --prefix apps/desktop ci` reported zero vulnerabilities and the
release build produced the sidecar, frontend bundle, macOS application, and
arm64 DMG. The docs dependency audit continues to report the existing one low
and two high findings.

## Documentation and delivery

- [x] Record the representation and remaining proof boundary in ADR 017.
- [x] Update README, architecture, security, actual-capability, and website
  documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Commit, open and merge PR #20, remove only its worktree, and leave the
  primary checkout clean on synchronized `main`.

## Next prioritized work

- [ ] Complete `DISC-019b` with an uncached supported-host equality matrix and
  eliminate any differing generated filesystem content.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.

---

# Previous sprint: strict pre-release contract

## Sprint status

- State: complete
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M8 — Conformance and release hardening](plans/holo-application-runtime.md#m8--conformance-and-release-hardening)
- Decision: [ADR 016](adrs/016-strict-pre-release-contract.md)
- Goal: remove speculative compatibility paths before the first public release
- Exit signal: one explicit current format is enforced across compiler,
  runtime, configuration, persistence, RPC, fixtures, and documentation; all
  verification gates pass and the change is merged

## Contract boundary

- [x] Accept physical `.holo` version 4 only.
- [x] Require exactly one verified application directory for every application
  archive.
- [x] Accept source-manifest schema version 4 only.
- [x] Require explicit Wasm entry and canonical guest contract.
- [x] Require canonical capability objects; reject the zero-byte sentinel.
- [x] Accept exactly one current Python rootfs bundle schema; ADR 017 now sets
  that contract to schema 3.
- [x] Accept configuration schema version 2 only without automatic rewriting.
- [x] Require complete history, resident, and run records.
- [x] Keep OpenAI and Ollama compatibility APIs as supported integrations.

## Implementation

- [x] Add a Live-owned physical-version gate at inspect, import/cache, compile-
  child, and planning boundaries.
- [x] Replace optional application-directory derivation with required
  verification.
- [x] Remove source-schema feature gates and Wasm contract normalization.
- [x] Remove capability, rootfs, configuration, history, and RPC decode
  fallbacks.
- [x] Update generated examples and fixtures to the current manifest schema.
- [x] Finish strict current-archive test helpers and remove stale assertions.
- [x] Confirm public archive, manifest, configuration, persistence, and RPC
  boundaries return typed errors for noncurrent or incomplete input.

## Tests and evidence

- [x] Add focused rejection tests for physical version, source schema,
  configuration schema, missing application directory, missing Wasm contract,
  malformed capability objects, and incomplete result records.
- [x] Pass Rust formatting, unit tests, checks, and Clippy.
- [x] Pass public-boundary BDD and isolated smoke tests.
- [x] Pass desktop and documentation builds.
- [x] Record the exact verification commands and outcomes here.

Verification evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests, Clippy with warnings
denied, 12 BDD scenarios with 123 steps, the optimized server build, and the
isolated smoke test. `just docs` regenerated OpenAPI and built all 13 static
pages. `npm --prefix apps/desktop ci && npm --prefix apps/desktop run build`
produced the release sidecar, frontend bundle, macOS application, and arm64 DMG.

## Documentation and delivery

- [x] Record the decision in ADR 016 and supersede conflicting ADR clauses.
- [x] Update README, architecture, security, actual-capability, and website
  documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Commit, open and merge the PR, remove only this worktree, and leave the
  primary checkout clean on synchronized `main`.

## Next prioritized work

- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.

---

# Previous sprint: M4.1 rootfs base-digest binding

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Goal: prevent a mutable Python rootfs base tag from moving between selection
  and Docker execution while preserving offline `compile --check`
- Exit signal: a real NumPy/pandas compile resolves the requested tag to the
  registry manifest digest, uses that exact reference in `FROM`, records both
  identities in provenance, executes successfully, passes all gates, and is
  merged

Rootfs provenance remains governed by ADR 014 and the binding decision in ADR
015. The later strict-contract decision in ADR 016 supersedes experimental
format compatibility from this period.

## Acceptance boundary

- Keep `compile --check` offline. It may report a digest-pinned request as
  already resolved, but it must not contact Docker or invent a digest for a
  mutable tag.
- Resolve mutable bases from Docker's original raw registry manifest and
  accept only schema 2.
- Compute the SHA-256 registry identity from the exact manifest bytes and
  preserve registry host/port and repository path when constructing
  `repository@sha256:digest`.
- Put the resolved reference into the Dockerfile before the image build so tag
  movement after resolution cannot redirect `FROM`.
- Preserve the requested source value and add the resolved build value to the
  non-canonical provenance report; do not rewrite `hologram.json` or `.holo`
  identity.
- Keep `reproducible: false` until the emitted OCI representation is normalized
  and equal layer κ values are proven across clean hosts.

## Runtime implementation

- [x] Add registry-manifest resolution for mutable rootfs base references.
- [x] Bypass registry resolution for valid lowercase SHA-256 digest references.
- [x] Generate Docker's `FROM` from the resolved immutable reference.
- [x] Add `base_image.resolved_reference` while retaining the requested
  `base_image.reference` and optional local `observed_image_id`.
- [x] Report mutable resolution as deferred during offline checks and remove
  that blocker after a completed digest-bound build.
- [x] Reject base strings that could be parsed as Docker command options.

## Tests and evidence

- [x] Prove raw schema-2 manifest bytes produce the expected SHA-256 reference.
- [x] Prove repository parsing handles a registry port and a tag.
- [x] Prove malformed, empty, and unsupported registry manifests fail closed.
- [x] Prove an already pinned reference returns unchanged without registry
  access.
- [x] Prove the generated Dockerfile uses the resolved reference.
- [x] Prove `compile --check` reports the request but omits a resolution for the
  mutable NumPy/pandas example.
- [x] Compare the resolver's
  `sha256:7a8b475003c4fe15a2cd4e55e5cfc2f3560bdc9333d624f24cdd6d4340fd7a17`
  with Docker's reported `python:3.12-slim` registry digest.
- [x] Compile the NumPy/pandas example with the digest-bound Dockerfile and
  confirm completed provenance reports both requested and resolved references.
- [x] Run the resulting fat archive through the direct rootfs provider and
  recover three rows, mean `20.0`, and sum `60.0`.
- [x] Run formatting, workspace tests/checks, Clippy, BDD, release/smoke, and
  documentation gates.

## Documentation and delivery

- [x] Record the digest-binding decision and threat boundary in ADR 015.
- [x] Update README and website guidance for offline checks, automatic real-
  build resolution, and the remaining OCI normalization blocker.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with this
  milestone and its evidence.
- [x] Commit, open and merge the PR, remove only this worktree, and return the
  primary checkout to clean synchronized `main`.

## Next prioritized work

- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
