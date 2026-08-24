# Architecture

## Product boundary

`hologram-live` is a module host. It is not a Kappa Registry wrapper.

```text
Tauri desktop ──managed sidecar──┐
hologram CLI ───────gRPC─────────┼── hologram daemon
browser ─────────JSON/HTTP───────┘   config · auth · telemetry · audit
                                            │
                                ┌───────────┼────────────┐
                                ▼           ▼            ▼
                           registry       .holo        history
                            module        module        module
```

## Kernel responsibilities

The kernel owns configuration, module dependency resolution, lifecycle, request dispatch, actor supervision primitives, routing, authentication/authorization seams, tracing, audit, server startup, graceful shutdown, and update coordination.

Business capabilities live in modules.

## Modules

Modules are trusted Rust implementations compiled into the binary for v1. Each module declares:

- a stable module ID and version;
- module dependencies;
- supported operation IDs;
- request dispatch behavior.

The registry is deterministic and starts dependencies before dependents.

Rust dynamic libraries are intentionally not supported because Rust has no stable plugin ABI and native plugins would inherit all daemon authority. Future untrusted extensions should be WASM components or separate processes with explicit capabilities.

## Native protocol

Native clients use the versioned `hologram.live.v1.HologramLive` Protobuf/gRPC service. The schema lives in `proto/hologram/live/v1/live.proto`, and Rust client/server types are generated during the build with a vendored `protoc`. Public browser routes remain JSON and are described by OpenAPI. Transport bytes are not used as Hologram canonical identity bytes.

The daemon serves the raw OpenAPI document at `/openapi.json` and a self-hosted Scalar reference at `/docs`. Product and operator guides are built separately as the static Astro website in `apps/docs`.

## Local and remote routing

`LiveClient` can target local and remote endpoints. It discovers capabilities before selecting a target. Authorization, authentication, and TLS failures are not safe fallback reasons. Resources created on one authority remain attached to that authority.

## Actors

Kameo provides process-local actors with bounded mailboxes, actor links, and supervision. Actors own long-lived mutable state and apply backpressure through fixed mailbox capacity. Durable state remains in module storage rather than actor memory, and gRPC—not actor remoting—is the cross-process boundary.

## Telemetry

`tracing` remains the local diagnostic API. When an OTLP endpoint is configured, OpenTelemetry exports traces and RPC metrics over OTLP/gRPC. Export is off until an endpoint is supplied, so the default configuration makes no telemetry network connection.

## Desktop

The Tauri application in `apps/desktop` bundles and controls the `hologram` executable as a sidecar. Its commands expose a narrow lifecycle/status interface rather than arbitrary shell execution. The desktop build is isolated from the daemon crate.

## Storage

The current content store is a simple content-addressed file store suitable for the starter. The provider boundary allows Kappa Registry or another store implementation to replace it without changing `.holo`, history, or client APIs.

## `.holo`

The pinned upstream Hologram archive reader/writer is used to create and validate real `.holo` archives. The stable build deliberately limits the dependency feature to `archive`; a future engine module will supply persistent execution.
