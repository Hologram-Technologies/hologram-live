# ADR 001: Hologram Live is a module host

## Status

Accepted.

## Decision

`hologram-live` owns product concerns—configuration, lifecycle, local/remote clients, routing, APIs, tracing, audit, updates, and future control-plane integration. Kappa Registry, `.holo`, files, history, and inference are modules rather than kernel responsibilities.

V1 modules are trusted Rust code statically linked into the binary. The public executable is named `hologram`.

## Consequences

- Kappa Registry can evolve or be replaced without redefining the host.
- New modules use stable IDs, operation IDs, dependencies, and typed request dispatch.
- Native dynamic Rust plugins are excluded.
- Future untrusted modules require a capability-limited WASM or process boundary.
