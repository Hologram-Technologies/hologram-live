# ADR 017: Normalize the Python rootfs Docker archive

- Status: accepted and implemented
- Date: 2026-08-26

## Context

The Python rootfs compiler previously wrapped the byte stream emitted by
`docker image save`. Docker archive member order, tar ownership, permissions,
timestamps, directory records, JSON field order, and storage-driver path
layout could therefore change the rootfs layer κ without changing the image
config or layer blobs. ADR 015 bound mutable bases before the build, but did
not make the exported representation stable.

The application has not shipped, so this decision replaces the experimental
rootfs bundle contract instead of adding a compatibility reader.

## Decision

Python rootfs layers use bundle schema 3, magic `HOLOPYR2`, and provider
`normalized-docker-archive-zstd-v1`. Every other rootfs bundle schema, magic,
or provider is rejected.

The compiler asks Docker to build one tagged image with
`SOURCE_DATE_EPOCH=0` supplied both to the process and as a build argument. It
disables Buildx provenance injection, saves the image, and then constructs a
new Docker archive under Hologram's control:

1. Accept exactly one `manifest.json` image containing the expected
   content-derived tag.
2. Reject unsafe paths, duplicate members, non-file members, missing config or
   layers, and expanded content over the two-GiB limit.
3. Hash the exact config and ordered layer bytes with SHA-256 and place them at
   `blobs/sha256/<digest>`, independent of Docker's storage-driver paths.
4. Emit only the canonical manifest, config, and referenced layer blobs. Sort
   tags, deduplicate tags and equal blobs, but preserve semantic layer order.
5. Write members in lexical path order as GNU tar regular files with mode
   `0644`, uid/gid zero, mtime zero, and no host names or directory records.
6. Zstandard-compress the canonical tar at the fixed level 3 and wrap it in
   the schema-3 envelope.

The config hash is the image ID. Compilation compares it with Docker's
inspected image ID before emitting the layer. Runtime cold loads the canonical
archive with `docker image load` and verifies the loaded tag has that exact ID.

Build provenance names `normalized-docker-archive-v1`, source epoch zero,
bundle schema 3, and the provider. It remains `canonical: false` and outside
application identity. `compile --no-build-cache` adds Docker's `--no-cache`
build option and records `builder.cache_disabled: true`; `compile --check`
remains offline and records `false` because no build occurred.

The rootfs build contract requires a Docker-compatible Linux engine. That is
separate from the five operating-system/architecture combinations on which the
standalone server binary is released. The reproducibility release gate uses
two independent clean Linux runners for each output architecture
(`linux/amd64` and `linux/arm64`) and compares replicas within the target.
Cross-architecture κ equality is neither expected nor required.

The application image uses separate builder and runtime builds. The disposable
builder installs the pinned `uv` tool and locked third-party dependencies. It
does not install the local project as a wheel: `uv_cache.json` embeds the
source directory's nanosecond timestamp and `RECORD` then hashes that unstable
value. Instead the runtime imports the compiler-staged source through
`PYTHONPATH=/app/src`.

The builder normalizes every application and launcher filesystem timestamp to
epoch zero, then writes `/app` and `/hologram` into one lexically sorted GNU tar
with epoch-zero headers, numeric root ownership, and stable GNU format. The
compiler creates but does not start a container from that image, copies
`/runtime.tar` to the host staging directory, and removes the disposable
container and builder tag. The final image starts from the same digest-bound
base and consumes the local tar with `ADD runtime.tar /`. This excludes `uv`,
its cache, and transient build layers while preventing BuildKit from choosing
the traversal order of a cross-stage directory copy. Creating and copying from
the stopped container does not execute a foreign-architecture process.

## Consequences

- Repeated exports of the same config and layer bytes produce identical
  rootfs layer, application, and physical archive κ values.
- Docker engines using classic save paths and engines using
  `blobs/sha256/...` paths converge on the same Hologram representation.
- A cold machine can restore and execute the normalized archive through the
  existing direct Docker provider.
- Rootfs bundle schema 2 is deliberately unsupported under ADR 016.
- Package installation, generated filesystem contents, Docker/BuildKit
  behavior, and platform output are covered by two uncached clean replicas for
  each supported Linux target. Completed provenance reports
  `reproducible: true`; an offline mutable-base check remains false only until
  compilation binds the base digest.

## Verification

Focused tests construct equivalent Docker archives with different member
orders, JSON key order, timestamps, modes, and owners and require identical
normalized bytes. Tests also reject duplicate members and unexpected tags.

On macOS arm64 with Docker client 29.2.1/server 29.4.0, two exports of the
locked NumPy/pandas image produced the same rootfs layer κ
`blake3:6ac835129125e3f997a211611c96094e606fdbf332073c02fe2a9f906a7c07f7`,
application κ
`blake3:104da1166bf688727352e966097e1d0ce837c4ad3873199e4d6038d5ac0b24b0`,
and archive κ
`blake3:3e302dff5f62ed341d5ce9b65296167bffb93d948330947db366c17d9726aff0`.
After removing the local tag, direct execution restored the embedded image and
returned three rows, mean `20.0`, and sum `60.0`.

The first uncached comparison found that all generated layers differed. The
two-stage build isolated the remaining difference to the local project's
timestamped `uv_cache.json` and its `RECORD` row. After removing that
installation from the runtime image, two uncached arm64 builds produced image
ID `sha256:a4d4ad759567e43ebec5bcc84d5dae5a52a0a5f3fcce74cd7fe1e756f97e2271`
and equal rootfs layer κ
`blake3:64f53c4cf1f721a7efa857e3397589034eea565adb89dc93ce3db8799062f538`.
The complete application and archive identities also matched, and the archive
executed successfully. This local proof preceded the independent runner gate.

The first independent matrix, workflow run `33031626335`, then completed four
clean builds with the same Docker 28.0.4 client/server and the same selected
base digest but found different target-local identities. The amd64 replicas
emitted rootfs layer κ values beginning `blake3:164e` and `blake3:49f9`; the
arm64 replicas emitted `blake3:ad32` and `blake3:fece`. The only remaining
engine-owned construction boundary was `COPY --from=builder` of the normalized
directories, so the compiler replaced that copy with the canonical runtime tar
described above. Two local uncached arm64 builds now agree on image ID
`sha256:1f55f44f41af891e3464b056f6b0beefbf6be9d736611de1505d17fb9a8cd754`
and rootfs layer κ
`blake3:7466d21d435ec4d2a7da0efdd9e974f26ff58a471d579abfa04b3f6df4077b8b`.

Workflow run `33035209550` then repeated two uncached builds on independent
clean runners for each target and passed the aggregate comparison. Linux/amd64
matched at image ID
`sha256:778bc5f5e4c66392b798ff8b6ad6178e42c80efff6a8ff44b18fbfc44573d31f`
and rootfs layer κ
`blake3:cae3233e0f062c839b0517de631e4d77774cdda3df341fc152b8776b919bb6c9`.
Linux/arm64 matched the local identities above. This closes the clean-builder
proof for the current rootfs contract.

## Follow-up

- Pin every build-time acquisition, including the uv installer artifact, and
  eliminate or normalize any differing generated filesystem content.
- Add signed SBOM/attestation material without moving observational evidence
  into canonical identity. Authenticated private-registry coverage is complete
  under ADR 015.
