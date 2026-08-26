# ADR 007: `.holo` compilation, resolution, and execution are separate layers

## Status

Accepted.

## Context

The archive format separates a canonical application manifest from its physical packaging. Live writes v4 archives and retains v2/v3 read compatibility; ADR 009 records the additive v4 inference-model layer. A fat archive embeds the κ-addressed content referenced by the manifest; a thin archive carries the same manifest but expects a content store to resolve those references. Treating archive parsing, content resolution, and execution as one operation would erase that distinction and make new execution providers difficult to add.

## Decision

Hologram Live uses three explicit product layers:

1. The compiler reads `hologram.json`, canonicalizes its `AppManifest`, and emits either a fat or thin v4 archive. Packaging does not change the canonical manifest. Readers continue accepting v2/v3 archives.
2. The runtime verifies the archive, decodes and validates its manifest, resolves and re-hashes the capability object plus every non-child layer from embedded content and then the local content cache, and produces a strict `ApplicationPlan` before provider work.
3. A closed registry keyed by `LayerKind` selects async providers. Providers prepare and start in manifest order, invoke the declared primary layer where applicable, and stop or roll back in reverse order. Wasmtime is the first provider and implements the import-free `core-wasm-v1` guest contract in `src/holo_wasm.rs`. Its provider resolves the callable export from the canonical layer `entry`; `holo_run` is only the source compiler and compatibility default. The direct-only Python OCI adapter remains explicitly experimental.

Importing a fat archive caches its verified `ContentBlob` payloads without creating user-facing registry metadata. A later thin archive can therefore resolve the same κ locally. Direct file execution intentionally accepts only self-contained archives because it has no configured external resolver; catalog-backed resident execution supports thin archives when their content is cached.

`hologram run` selects the direct executor when its reference is a local `.holo` path and the catalog-backed RPC when the reference is a κ. The two paths return the same `HoloRunResult` shape.

ADR 010 refines the runtime side of this boundary with an explicit three-part identity model, a runtime-owned `ApplicationPlan`, complete pre-start resolution, and transactional provider lifecycle phases.

## Consequences

- A compiled fat `.holo` can run without a service, import, or load step.
- Fat and thin archives preserve one application identity while having different physical archive fingerprints.
- Adding tensor or rootfs execution requires a real provider behind the execution boundary, not a format or CLI redesign.
- Missing thin content is a typed `LIVE_NOT_FOUND`; unsupported layer kinds remain typed `LIVE_CAPABILITY_MISSING` errors.
- Cached layer bytes are content-only objects and do not overwrite filenames or kinds in the user-facing registry.
