# Current sprint: M4.2 clean Python rootfs equality

## Sprint status

- State: active
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Decision: [ADR 017](adrs/017-normalized-python-rootfs-archive.md)
- Goal: prove that uncached rootfs compilation is byte-identical on independent
  clean builders for both supported Linux target architectures
- Exit signal: the release gate compares two clean replicas each for
  `linux/amd64` and `linux/arm64`, all target-local identities match, and any
  generated filesystem differences have been eliminated

## Builder contract

- [x] Distinguish the five standalone-server release hosts from the Linux
  container-engine contract required to compile a rootfs.
- [x] Compare independent builder replicas within one target architecture;
  never require an amd64 artifact and an arm64 artifact to share identity.
- [x] Add `compile --no-build-cache` and pass Docker `--no-cache` without
  weakening the normal cached developer path.
- [x] Record `builder.cache_disabled` in non-canonical completed provenance.
- [x] Keep `compile --check` offline and report that no build cache was disabled.

## Reproducibility evidence

- [x] Add `just python-rootfs-repro` with one JSON document on stdout and
  progress on stderr so every result can be queried with `jq`.
- [x] Compare image ID, rootfs layer κ/size, application κ, archive κ/size, and
  footer fingerprint.
- [x] Add two independent clean GitHub runners per architecture and a
  target-aware aggregate comparison artifact.
- [x] Make the clean-builder matrix a prerequisite of every server release.
- [x] Run the local two-build uncached probe and record its identities here.
- [ ] Run the clean GitHub matrix and record the workflow evidence here.
- [ ] If either comparison fails, identify and normalize the differing image
  config or generated filesystem bytes, then rerun both proofs.

Local evidence (2026-08-26): the first uncached comparison exposed unstable
timestamps in every generated Docker layer. A two-stage recipe reduced that
to the local-project `uv_cache.json`, whose nanosecond source timestamp also
changed its `RECORD` hash. The final recipe installs only locked dependencies,
runs the already-staged source through `PYTHONPATH`, normalizes the runtime
tree to epoch zero, and copies only that tree onto the pinned base. Two
uncached macOS-arm64/Linux-arm64-engine builds then matched exactly: image ID
`sha256:a4d4ad759567e43ebec5bcc84d5dae5a52a0a5f3fcce74cd7fe1e756f97e2271`,
rootfs layer κ
`blake3:64f53c4cf1f721a7efa857e3397589034eea565adb89dc93ce3db8799062f538`,
application κ
`blake3:9b20b3cb7f6a9fcabcd9888b54a05bad6b7f9c50a396ecfdf5cbdd4aae30b451`,
archive κ
`blake3:e31387403074e0e7546de124012764c6b389222d881d50d66e48447260ca0048`,
and footer fingerprint
`f4638af9a5d3e5c95d3c1170b558e82796095cec5773e9e2cfb16a5c5f0c9e25`.
The resulting archive executed NumPy/pandas successfully and returned three
rows, mean `20.0`, and sum `60.0` from an isolated current configuration.

## Verification and delivery

- [x] Add focused CLI/provenance and report-comparison tests.
- [x] Pass formatting, workspace tests/checks, Clippy, BDD, release/smoke,
  documentation, and desktop gates.
- [x] Update README, website Python guidance, ADR 017, and the durable runtime
  plan with the rootfs-builder boundary and exact commands.
- [ ] Commit, open and merge the PR, remove only this worktree, and leave the
  primary checkout clean on synchronized `main`.

Repository evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests (including 198
library tests, 23 CLI tests, and four provenance/comparator tests), Clippy with
warnings denied, 12 BDD scenarios with 123 steps, the optimized server build,
and isolated smoke. `just docs` regenerated OpenAPI and built all 13 pages.
The Tauri release gate reported zero npm vulnerabilities and produced the
macOS application and arm64 DMG. The docs audit continues to report the
existing one low and two high findings.

## Next prioritized work

- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
- [ ] Continue M4 deterministic compiler work after both Python profiles have
  clean-build evidence.

---

# Previous sprint: M4.2 normalized Python rootfs archives

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
- [x] Keep build provenance non-canonical and `reproducible: false` until the
  current clean Linux builder matrix passes for both rootfs target architectures.

## Tests and evidence

- [x] Prove differing input member order, JSON key order, timestamps, modes, and
  ownership normalize to identical bytes.
- [x] Cover duplicate members, unexpected tags, and canonical member sets.
- [x] Compile the locked NumPy/pandas application twice and prove equal layer,
  application, archive, and footer identities.
- [x] Remove the generated local image tag, cold-load from the `.holo`, and
  recover three rows, mean `20.0`, and sum `60.0`.
- [x] Hand uncached equality to the current sprint, with release-binary hosts
  separated from the Docker-compatible Linux builder contract.
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
