# Current sprint: M4.1 rootfs base-digest binding

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

Completed legacy empty-capability compatibility remains in Git history and the
durable runtime plan. Rootfs provenance remains governed by ADR 014; this
sprint adds the binding decision in ADR 015.

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
- [x] Prove malformed, empty, and legacy registry manifests fail closed.
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
- [ ] Commit, open and merge the PR, remove only this worktree, and return the
  primary checkout to clean synchronized `main`.

## Next prioritized work

- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
