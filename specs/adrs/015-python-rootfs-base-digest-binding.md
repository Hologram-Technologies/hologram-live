# ADR 015: Bind Python rootfs builds to registry manifest digests

- Status: accepted and implemented
- Date: 2026-08-26

## Context

Python rootfs recipes accept an OCI base reference. The convenient default,
`python:3.12-slim`, is a mutable tag. Recording the local image ID after a
Docker build, as ADR 014 originally did, could describe the result but could
not prove which registry object Docker selected. Resolving a tag for reporting
after the build would also leave a race: the tag could move between resolution
and the `FROM` instruction.

`compile --check` has a separate constraint. It is a read-only, offline
validation path and must not require Docker credentials, a running engine, or
network access.

## Decision

Before a real rootfs build, resolve every mutable base reference with:

```console
docker buildx imagetools inspect --raw -- <requested-reference>
```

Require a JSON registry manifest with `schemaVersion: 2`, compute SHA-256 over
the exact raw bytes returned by Docker, and construct
`repository@sha256:<digest>`. Preserve registry ports while removing only a tag
separator after the final path slash. Write that resolved reference—not the
mutable request—into the generated Dockerfile's `FROM` instruction. The
existing `--platform` selection then chooses a platform from an immutable
index or consumes an immutable single-platform manifest.

If `source.base` is already a lowercase 64-digit SHA-256 digest reference, use
it unchanged and do not perform a registry lookup. Reject malformed registry
output and unsupported manifest schemas before staging a Docker build.

Extend ADR 014's non-canonical provenance with
`base_image.resolved_reference`. `base_image.reference` remains the source
recipe's requested value. During `compile --check`, a mutable request omits the
resolved field and reports that resolution is deferred to compilation; an
already pinned request reports itself as resolved. A completed compile always
reports the exact reference used by Docker.

The resolver does not mutate `hologram.json` or add registry evidence to the
archive. Docker's raw-manifest command owns registry transport, authentication,
and media negotiation.

## Consequences

- A tag may move before resolution, but it cannot redirect the build after
  Hologram has selected and recorded the manifest digest.
- Requested intent and consumed registry identity are both queryable with
  `jq` without changing canonical application or archive identity.
- `compile --check` remains useful offline and never invents a resolved value.
- Rootfs compilation now requires a Docker CLI with Buildx image-tools support
  for mutable references. Digest-pinned references retain the previous engine
  requirement but do not require registry inspection.
- ADR 017 normalizes Docker image export and proves byte-identical config,
  layer, application, and archive identities on two clean runners for each
  supported Linux target. Completed provenance therefore reports
  `reproducible: true` after this resolver binds the base.

## Follow-up

- Add authenticated private-registry integration coverage without persisting
  credentials in provenance.
- Produce SBOM and signed-attestation material over resolved inputs and output
  identities.
