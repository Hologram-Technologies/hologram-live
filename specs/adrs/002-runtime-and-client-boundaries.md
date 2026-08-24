# ADR 002: Runtime and client boundaries

## Status

Accepted.

## Decision

- Protobuf/gRPC is the versioned native client/server boundary.
- Kameo supplies process-local actors, bounded mailboxes, links, and supervision; actor remoting is not a network API.
- `tracing` supplies application instrumentation, while OpenTelemetry optionally exports traces and metrics over OTLP/gRPC.
- Tauri supplies the desktop shell and controls the daemon through a bundled sidecar with a narrow command surface.

The public JSON/HTTP API remains available for browsers and is documented with OpenAPI.

## Consequences

- Native API evolution is explicit in the checked-in `.proto` schema.
- Actor lifecycle semantics are delegated to a maintained framework without coupling remote clients to that framework.
- The default daemon makes no telemetry connection until a collector endpoint is configured.
- Desktop dependencies remain isolated from the daemon crate.
