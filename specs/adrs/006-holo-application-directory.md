# ADR 006: `.holo` archives carry a verified application directory

## Status

Accepted.

## Context

The canonical v3 `.holo` format already has the right execution model: an application manifest names ordered, κ-addressed layers and child applications, while fat packaging embeds content blobs and thin packaging resolves them externally. Its section table makes physical payloads addressable, but generic tools should not need to decode canonical realization bytes merely to answer structural questions such as which layers an application contains or which blobs are embedded.

A database-backed executable demonstrates the value of a self-describing schema and relational inspection. Replacing `.holo` with a mutable database would work against canonical byte identity, the verified BLAKE3 footer, zero-copy section access, and the upstream v3 reader/writer contract. The useful property is queryability, not SQLite itself.

## Decision

New application archives carry one extension named:

```text
https://hologram.foundation/extension/application-directory/v1
```

Its deterministic JSON document is a normalized projection with six fields:

- `schema_version`;
- `primary_layer`;
- `requires_kappa`;
- ordered `layers` referring to content by kappa;
- ordered child applications and their delegated capability sets;
- a kappa-sorted table of physically embedded blobs and byte lengths.

The canonical `AppManifest` remains the source of application identity and execution truth. On inspection or import, Live decodes and validates that manifest, re-derives every embedded blob's kappa from its bytes, derives the directory, and compares an embedded directory byte-for-byte at the typed value level. Duplicate blob labels, forged content addresses, duplicate directories, unknown directory schema versions, and disagreement with the manifest are rejected as `LIVE_HOLO_INVALID`.

The extension is optional when reading. A legacy v3 application without it receives the same derived directory in inspection results with `directory_embedded = false`. Bare tensor archives without an application manifest have no application directory.

## Consequences

- `holo inspect` and the HTTP/gRPC APIs expose a stable table-like view without embedding a database engine.
- Layer order and κ identity remain canonical; the directory cannot override either.
- Existing v3 archives and upstream fat/thin conversion remain compatible.
- New directory schemas require a new extension key or supported schema version rather than reinterpretation.
- A future catalog can index these rows across installed applications, and a thin-archive resolver can join layer references against the content store without changing the archive identity model.
