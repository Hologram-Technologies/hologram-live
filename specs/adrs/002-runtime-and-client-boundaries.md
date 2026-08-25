# ADR 002: Runtime and client boundaries

## Status

Accepted.

## Decision

- Protobuf/gRPC is the versioned native client/server boundary.
- Kameo supplies process-local actors, bounded mailboxes, links, and supervision; actor remoting is not a network API.
- `tracing` supplies application instrumentation, while OpenTelemetry optionally exports traces and metrics over OTLP/gRPC.
- Tauri supplies the desktop shell and controls the daemon through a bundled sidecar with a narrow command surface.
- The root `hologram-live` package is the only server/CLI implementation. Tauri
  must not contain or link a second server implementation.
- Reusable local-development orchestration lives in workspace crates outside
  `apps/desktop/src-tauri`; Tauri supplies only native permissions, paths,
  events, and fixed client/sidecar adapters.

The public JSON/HTTP API remains available for browsers and is documented with OpenAPI.

## Consequences

- Native API evolution is explicit in the checked-in `.proto` schema.
- Actor lifecycle semantics are delegated to a maintained framework without coupling remote clients to that framework.
- The default daemon makes no telemetry connection until a collector endpoint is configured.
- Desktop dependencies remain isolated from the daemon crate.
- `cargo build --release --package hologram-live --bin hologram` remains a
  desktop-independent deployment boundary suitable for server releases and
  future cloud images.
- `crates/hologram-application-watch` can be reused by another trusted host,
  but selecting a workstation directory remains explicit desktop authority.
  Extraction of the engine does not grant the local or remote server ambient
  filesystem access.
- Tauri's ignored `src-tauri/binaries/` directory is packaging staging only;
  it contains a copy of the root server artifact, never server source.
