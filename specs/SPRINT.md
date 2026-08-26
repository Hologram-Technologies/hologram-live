# Current sprint: M3.1e Python rootfs build provenance

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Goal: make Python rootfs validation and compilation produce useful,
  machine-readable build evidence without overstating reproducibility or
  changing canonical `.holo` identity
- Exit signal: the NumPy/pandas `compile --check` report is selectable with
  `jq`, a real Docker build adds observed builder/image/output identities, the
  end-to-end archive still executes, docs explain planned versus completed
  evidence, and all repository/release gates pass

Completed Component dependency, provenance, and exact-tool-artifact work is
retained in Git history and ADRs 012–013. Durable runtime requirements remain in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md).
Rootfs evidence and its identity boundary are recorded in ADR 014.

## Acceptance boundary

- Keep `build_provenance.schema_version: 1` additive, non-canonical, and
  outside every archive, manifest, application directory, and content blob.
- `compile --check` may read and hash declared project files but must not
  require, contact, pull from, or execute Docker.
- Distinguish the requested base reference, a digest-pinned request, a locally
  observed image identity, and registry digest resolution. Do not conflate
  these identities.
- Completed evidence identifies the emitted bytes; it does not claim another
  clean build will produce the same bytes.
- Continue reporting `reproducible: false` until OCI normalization and clean
  supported-host equality are proven.
- Preserve the experimental, trusted-local, direct-fat-only rootfs support
  boundary from ADR 008.

## Compiler and provenance

- [x] Include source-compiled rootfs layers in the existing versioned
  `build_provenance` envelope.
- [x] Report normalized Linux target, build host, compiler version, requested
  base, digest-pin status, pinned uv, and Docker as the planned builder.
- [x] Reuse the versioned Component source-input hashing contract for
  `pyproject.toml`, `uv.lock`, and the normalized source tree.
- [x] Keep check provenance independent from Docker and omit observations and
  output until a build actually occurs.
- [x] Record observed Docker client/server versions after a successful build.
- [x] Record the locally observed requested-base image ID when Docker exposes
  one without claiming it is a registry-resolved digest.
- [x] Record exact output layer κ, rootfs-envelope length, final image ID, and
  uncompressed image archive size.
- [x] Report a mutable-base blocker only for tag-based base requests and retain
  the unnormalized-OCI blocker for digest-pinned requests.
- [x] Preserve Component provenance JSON while admitting both Python profiles
  through one untagged per-layer report boundary.

## Tests and execution evidence

- [x] Add an integration test proving rootfs `compile --check` returns planned
  provenance without Docker observations or output.
- [x] Cover strict lowercase `repository@sha256:<64 hex>` pin detection.
- [x] Keep existing Component provenance assertions intact after the shared
  report accepts both profiles.
- [x] Verify the user's exact NumPy/pandas `--check | jq
  '.build_provenance'` workflow locally.
- [x] Run the real Docker-backed NumPy/pandas compile and execution proof and
  assert completed provenance has builder versions and output identity.
- [x] Run formatting, workspace checks/tests, Clippy with warnings denied,
  BDD/release/smoke verification, and the Astro documentation build.

## Documentation and delivery

- [x] Add ADR 014 for observational rootfs provenance and amend ADRs 008/013
  so their follow-up boundaries agree.
- [x] Update README, Python website guide, architecture, security, and actual
  capabilities with planned/completed examples and remaining limitations.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with proven
  and still-open acceptance.
- [ ] Commit the reviewable milestone, open and merge its PR, remove only this
  feature worktree, and return the primary repository to clean synchronized
  `main`.

## Deferred discoveries

- [ ] `DISC-017d` — Patch or replace the Component compiler so its build-time
  WASI context receives deterministic randomness, then gate clean supported-
  host output equality.
- [ ] `DISC-017f` — Decide whether uvx and host Python become an independently
  distributed, digest-pinned toolchain bundle before Component provenance can
  graduate to a signed attestation.
- [ ] `DISC-019a` — Resolve mutable rootfs base tags through a registry and
  bind the selected manifest digest into build execution and evidence.
- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-019c` — Generate dependency inventory/SBOM material for the actual
  installed rootfs closure and define signed-attestation retention.
- [ ] `DISC-018` — Define capability-gated WASI only when an application needs
  real host interfaces; do not broaden the import-free base world.
