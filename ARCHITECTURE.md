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
- an optional startup hook for module-owned actor trees.

The registry is deterministic and starts dependencies before dependents.

Rust dynamic libraries are intentionally not supported because Rust has no stable plugin ABI and native plugins would inherit all daemon authority. Future untrusted extensions should be WASM components or separate processes with explicit capabilities.

Modules are not actors by default. A module uses its startup context to create Kameo actors only for long-lived state, reconciliation, subscriptions, streams, or bounded background queues. Plain request/response behavior remains ordinary Rust code.

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

The Tauri application in `apps/desktop` bundles and controls the `hologram` executable as a sidecar. Its commands expose a narrow lifecycle/status/module-discovery interface rather than arbitrary shell execution. Module discovery follows `LiveClient` routing, so the configured authority may be the local daemon or a future cloud endpoint. The desktop build is isolated from the daemon crate.

## Storage

The current content store is a simple content-addressed file store suitable for the starter. The provider boundary allows Kappa Registry or another store implementation to replace it without changing `.holo`, history, or client APIs.

## `.holo`

The pinned upstream Hologram archive reader/writer and space manifest types create and validate real v4 `.holo` archives while retaining v2/v3 reads. `hologram compile` builds fat archives containing a canonical application manifest and κ-addressed layer blobs by default; `--thin` preserves that manifest while omitting the payloads. Identity is explicit: the complete file has an archive object κ, its footer has an archive fingerprint, and the canonical `AppManifest` has an application κ that is stable across fat/thin packaging. New archives also carry a versioned application-directory extension: a deterministic, normalized projection of requirements, ordered layers, children, embedded blobs, and inference-model engine tags. Inspection re-derives this directory from the canonical manifest and content bytes, so it is queryable metadata rather than a second source of truth; older archives remain readable. Importing a fat archive verifies and caches its layer content by κ, which lets the catalog-backed runtime resolve a later thin archive. Archives whose primary layer is Wasm execute in-process through wasmtime: `hologram run application.holo` is a service-free one-shot executor, while `holo load` keeps an imported module resident under a supervised actor and `holo run` invokes it per input. Direct Python OCI rootfs archives execute through the experimental local provider. Inference-model archives can be packaged and inspected through `hologram ai inspect`, but invocation waits for a typed `hologram-ai` adapter; unsupported providers return capability errors. See `specs/adrs/004-holo-wasm-runtime.md`, `specs/adrs/006-holo-application-directory.md`, `specs/adrs/007-holo-compiler-runtime-execution.md`, `specs/adrs/009-inference-model-holo-v4.md`, and `specs/adrs/010-holo-application-plan-and-provider-lifecycle.md`.

## Inference engine boundary

The daemon never executes model weights in-process. Chat and model management call an `InferenceEngine` selected by `[inference].engine` in `live.toml`: `echo` (local fallback that repeats the user message), `weightc` (spawns `weightc ask <artifact-dir> <prompt> --json` against an imported `.wcpu` artifact directory), or `ollama` (proxies `POST /api/generate` on an Ollama-compatible endpoint). Imported artifacts are copied under `data_dir/models/<digest>/` and recorded in the content-addressed object store with `kind = "model"`. The daemon renders conversation history as a plain `role: content` transcript; engines apply their own chat templates. An unconfigured engine or model returns `LIVE_CAPABILITY_MISSING` rather than simulating a response.
