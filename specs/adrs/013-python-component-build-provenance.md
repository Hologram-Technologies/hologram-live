# ADR 013: Python Component build provenance is versioned and non-canonical

- Status: accepted and implemented
- Date: 2026-08-26

## Context

The portable Python compiler has three identities with different meanings:
source and toolchain inputs, the generated Wasm layer, and the canonical
Hologram application assembled around that layer. Operators need enough
evidence to explain a build and audit locked dependency selection, but adding
host-specific tool details to `AppManifest` would make those observations part
of application identity. It would also require a `.holo` format change before
the report has proven stable.

Pinned `componentize-py 0.25.0` remains nondeterministic. Its pre-initializer
creates a private WASI context and obtains build-time randomness without a seed
control. Controlled dependency-free builds showed that this is not limited to
the `--stub-wasi` adapter: two non-stubbed outputs from identical inputs were
18,308,535 and 18,320,373 bytes. Their SHA-256 values were respectively
`5c3ecc2d0c4b526ae1cd4ab2014db386c9137bc5c32f8e79e966265c8a1eb873`
and `2f2c40263039a6f22dcaa24cd81ee9ff135198c6507ea9fd3c51b2805c323841`.
Rewriting the finished component is unsafe because snapshot layout and length
both change.
Caching one random output would make repeated local builds look stable without
making a clean build reproducible.

## Decision

Every `hologram compile` and `hologram compile --check` result contains a
`build_provenance` report with `schema_version: 1` and `canonical: false`.
Reports contain one entry for each source-compiled Python Component layer and
identify its manifest layer index. Prebuilt layers and the experimental Python
rootfs path are outside this first schema slice.

A Python Component entry records:

- the `wasi-component` profile, exact Component v1 guest contract, and
  `wasm32-wasip2-component` target ABI;
- build host OS and architecture;
- Hologram compiler version, CPython `3.14.0`, componentize-py `0.25.0`, its
  release source revision, and the exact host-specific distribution URL and
  SHA-256;
- normalized logical paths plus SHA-256 for `pyproject.toml`, `uv.lock`, and a
  versioned source-tree digest;
- every selected dependency name, version, HTTPS wheel URL, and SHA-256 from
  the runtime lock closure;
- for a completed build, the observed uvx version, the uv dependency-installer
  version when dependencies exist, and the generated layer κ and byte length;
- `reproducible: false` plus the current deterministic-seed blocker.

The source-tree digest uses domain `hologram-python-source-tree-v1`, lexical
UTF-8 `/`-separated paths, file lengths, and file bytes. It deliberately ignores
directory creation order, timestamps, and permissions. Component compilation
copies those regular files to a private tree in the same lexical order. Nested
symlinks and special files fail before componentization.

`compile --check` never executes or downloads build tools. Its report therefore
omits `componentizer_runner`, `dependency_installer`, and `output`; the declared
toolchain pins and complete locked dependency inventory remain available.

The componentizer distribution is selected from a closed mapping for the five
server release hosts: macOS arm64/x86_64, Linux arm64/x86_64, and Windows
x86_64. Each entry is an upstream wheel URL and PyPI-published SHA-256. The
compiler passes that direct reference to uvx with registry lookup and source
builds disabled. A host outside the mapping returns
`LIVE_CAPABILITY_MISSING`; it never falls back to version-only resolution.

The report is CLI result data. It is not embedded in archive metadata, the
application directory, a content blob, or the canonical `AppManifest`. It does
not affect layer, application, or archive κ values. A durable copy can be made
explicitly:

```console
hologram --json compile hologram.json --output application.holo \
  | jq '.build_provenance' > application.provenance.json
```

## Consequences

- Build validation is inspectable with `jq` before downloading the toolchain.
- A completed report ties the exact selected lock artifacts to the observed
  tool runners and emitted layer without conflating evidence with identity.
- A mutable package index cannot select a different componentizer distribution
  behind the same version; changing or adding a release host requires an
  explicit reviewed URL/hash pin.
- Reports may differ across hosts or tool runners; that is evidence, not an
  application-identity change.
- Provenance schema evolution can proceed independently and may later become a
  signed attestation. Embedding it would require a separate format and identity
  decision.
- Byte-identical Python Component output remains unclaimed. The report exposes
  this as machine-readable state instead of hiding it in documentation.

## Follow-up

- Obtain or maintain a componentizer that accepts an explicit deterministic
  pre-initialization random source, then verify byte-identical components and
  `.holo` archives across clean builds and supported hosts.
- Extend the schema to rootfs inputs only after base-image digest resolution,
  reproducible OCI construction, and the microVM execution boundary land.
- Define signing and retention if provenance becomes a supply-chain
  attestation rather than ephemeral compiler output.
