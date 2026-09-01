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

## Source and deployment boundary

The root `hologram-live` package is the standalone server and CLI. Desktop
source does not contain or link a second server implementation:

```text
src/                                  server, CLI, protocol, and runtime
crates/hologram-application-watch/    reusable watch/debounce engine
apps/desktop/src-tauri/               native permissions and sidecar adapter
apps/desktop/src/                     webview frontend
```

`cargo build --release --locked --package hologram-live --bin hologram` is the
server release and future cloud-image boundary. It does not build Tauri,
WebKit, Node.js, or the frontend. The desktop preparation step copies that root
binary into an ignored Tauri packaging directory; the copy is an artifact, not
server source.

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

The Tauri application in `apps/desktop` bundles and controls the `hologram` executable as a sidecar. Its commands expose narrow lifecycle, workspace, and archive operations rather than arbitrary shell execution. Module discovery follows `LiveClient` routing, so the configured authority may be the local service or a future cloud endpoint. Local application path authorization is deliberately desktop-owned: the native picker grants the adapter access to a selected directory containing `hologram.json`. The Tauri-independent `hologram-application-watch` crate owns persistent registrations, recursive event filtering, debounce, and build-state transitions; the adapter supplies desktop configuration/cache paths, fixed compile/import calls, and UI events. The webview lists and inspects resulting archives only through `holo list` and `holo inspect`, while the service never receives ambient host-filesystem authority. Successful changed builds replace the prior watched catalog variant, failed builds preserve the last good archive, and removing a watch leaves its final immutable archive. The desktop build remains isolated from the server crate.

## Storage

The current content store is a simple content-addressed file store suitable for the starter. The provider boundary allows Kappa Registry or another store implementation to replace it without changing `.holo`, history, or client APIs.

## `.holo`

The pinned upstream Hologram archive reader/writer and space manifest types create and validate v4 `.holo` archives; Live rejects every other physical version. `hologram compile` builds fat archives containing a canonical application manifest and κ-addressed layer blobs by default; `--thin` preserves that manifest while omitting the payloads. Identity is explicit: the complete file has an archive object κ, its footer has an archive fingerprint, and the canonical `AppManifest` has an application κ that is stable across fat/thin packaging. Every application archive carries exactly one versioned application-directory extension: a deterministic, normalized projection of requirements, ordered layers, children, embedded blobs, and inference-model engine tags. Inspection re-derives this directory from the canonical manifest and content bytes, so it is queryable metadata rather than a second source of truth. Execution planning validates that directory, then a runtime-owned `ApplicationPlan` decodes and validates the canonical manifest, resolves and re-hashes every root and child manifest, capability object, and layer, deduplicates equal κ payloads without collapsing logical applications, records embedded/local-store sources, and applies tree-wide depth/application/layer/object/byte limits before provider work. Archive-controlled requests and delegations are decoded into typed values but never become authority. Direct and resident execution receive a separately constructed effective grant from trusted host context; admission proves every parent grant → child delegation → child request chain before provider preparation. Ordinary execution uses a local baseline with no storage roots, channels, or network endpoint scopes; explicit development files are accepted only for direct local archives or loopback service configuration. ADR 020's canonical HTTPS origin/path scopes attenuate on exact origin and path-segment boundaries, preserve legacy no-network identities, and grant no raw socket or mediated-fetch interface by themselves. Run results and structured traces identify the request κ, grant κ, trusted grant source, and decision without copying source documents or secrets. The payload-free explanatory form is exposed consistently as `hologram holo plan <PATH|KAPPA>`, native gRPC, `GET /api/v1/holo/{kappa}/plan`, and OpenAPI; unavailable providers are successful non-runnable reports with typed blockers. A closed registry keyed by upstream `LayerKind` turns blocker-free plans into provider instances through async `Send + Sync` prepare/start/invoke/stop contracts. The complete admitted tree prepares and starts depth-first in manifest order, with each child receiving only its delegated grant; normal stop and failure rollback use the exact reverse while preserving the original typed error. Only the root primary is invoked, child primaries share the parent's transactional lifetime, and shared status aggregates root and child resource counters without Wasmtime, container, or actor types. Importing a fat archive verifies and caches its layer content by κ, which lets the catalog-backed runtime produce the same logical plan for a later thin archive. Wasm layers execute in-process through Wasmtime behind the provider boundary, including ordered multi-layer applications whose primary is not position zero: `hologram run application.holo` is service-free, while `holo load` keeps compiled layers resident under supervised actors and `holo run` invokes the root primary per input. Direct Python OCI rootfs archives use an experimental container provider. Inference-model archives can be packaged and inspected through `hologram ai inspect`, but invocation waits for a typed `hologram-ai` adapter; unsupported providers return capability errors. See `specs/adrs/004-holo-wasm-runtime.md`, `specs/adrs/006-holo-application-directory.md`, `specs/adrs/007-holo-compiler-runtime-execution.md`, `specs/adrs/009-inference-model-holo-v4.md`, `specs/adrs/010-holo-application-plan-and-provider-lifecycle.md`, `specs/adrs/016-strict-pre-release-contract.md`, and `specs/adrs/020-endpoint-scoped-network-capabilities.md`.

The current Python rootfs bundle is schema 3 under ADR 017. It SHA-256-addresses
the exact image config and ordered layers, canonicalizes the manifest and tar
encoding, fixes source epoch and compression, and rejects every other bundle
schema. Clean uncached equality across the supported host matrix remains
unproven.

Capability admission also crosses an awaited JSONL audit boundary. It records
the real principal, relation, application identities, request or delegation κ,
effective-grant κ, trusted source, and allow/deny outcome before provider
preparation without copying source documents or secrets. Run and resident
records expose the non-secret authorization evidence through CLI, JSON/HTTP,
and Protobuf/gRPC. HTTP resident list/load/unload/run operations are available
under `/api/v1/holo` alongside the planning route.

Wasmtime implements the import-free `core-wasm-v1` boundary. The canonical
layer entry selects a typed `(i32, i32) -> i64` export inside the resolved
module; fixed `memory` and `holo_alloc` exports move bytes across the boundary.
Direct and resident providers validate that same declared entry before start
and create a fresh guest instance per input. V1 returns bytes but carries no
numeric process exit status, and it links no WASI or ambient host interface.

Guest-contract selection is canonical but remains separate from the callable
entry. Source schema v4 maps a Wasm `contract` field to the identity-bearing
layer `aux`. Empty `aux` normalizes to `hologram:guest/core-wasm@1`; the explicit
`hologram:guest/core-wasm@1`, `hologram:guest/component@1`,
`hologram:guest/component-store-read@1`,
`hologram:guest/component-store-graph-read@1`,
`hologram:guest/component-store-write@1`,
`hologram:guest/component-channel-publish@1`,
`hologram:guest/component-channel-subscribe@1`, and
`hologram:guest/component-network-fetch@1` tags are accepted. Inspection and
planning expose the normalized selector, and provider lookup is keyed by both
layer kind and exact contract. Component archives therefore reach only the
dedicated direct or resident Component provider and never the core provider.
The Component v1 WIT world is an import-free, stateless
`list<u8> -> result<list<u8>, guest-error>` boundary. ADR 011 maps each
proposed host interface to admitted capability fields and keeps interfaces
with no canonical authority unavailable. Each prepared component owns an
epoch-interruptible Wasmtime engine and serial execution boundary; compiled
code stays warm, while every input receives a fresh store with memory and fuel
limits. The provider also bounds input/output bytes and wall time. Cancellation
increments only that component engine's epoch, isolating it from core Wasm and
other applications. Nonzero admitted memory and CPU-time capability scalars
can tighten, never expand, the runtime-owned ceilings.

The store-read profile is a separate fixed world, not an optional import added
to base Component v1. After application and child-delegation admission, its
provider retains only the requested storage roots contained by the effective
grant, links `hologram:host/store@1.0.0`, and checks every target before calling
the runtime object store. Direct execution supplies the configured registry;
resident execution supplies the catalog store. No other Hologram or WASI
interface enters that linker.

The graph-read profile is a distinct canonical contract even though it reuses
the same narrow store-read WIT ABI. During preparation, a bounded resolver walks
only registered canonical UOR realization edges from the admitted root set.
Each object is fetched locally and re-hashed; unknown types terminate traversal
as opaque leaves, while malformed typed frames, missing descendants, or depth,
object, edge, and aggregate-byte limit violations fail before linker
construction. The resulting first-seen closure becomes the host's read set for
the prepared lifetime. Direct and resident execution use the same resolver,
and child delegation remains an exact subset relation over graph roots. The
exact-root read and write profiles do not inherit these semantics.

The store-write profile is another separate fixed world. Its linker contains
only `hologram:host/store-write@1.0.0` after admission proves a nonempty exact
root set and nonzero quota. The host verifies each supplied κ against the bytes
and performs an atomic bounded cache insertion. A quota counter shared by every
fresh store in the prepared application charges newly materialized blobs for
the application's lifetime; an existing identical blob costs nothing. Root,
hash, quota, and backend failures are checked before or within the atomic store
operation and do not leave partial content. Direct and resident providers use
the same object-store boundary, and child providers retain only attenuated
roots and quota.

Channel publish and subscribe are two further fixed worlds, never optional
imports on the base world. After exact `publish_channels` or
`subscribe_channels` admission, the provider links only its corresponding
host interface and retains only that exact set. Direct executions made through
one executor and resident applications in one runtime share a host-neutral
in-memory broker. Each channel is a 64-message FIFO of messages up to 64 KiB;
publish and receive never wait, a full mailbox rejects without overwrite, and
one receive removes one message. Broker lifetime bounds message lifetime. V1
has no durable, replay, broadcast, acknowledgement, cross-process, or network
semantics, so cancellation cannot leave a registered channel waiter.

Python `wasi-component` is a compiler adapter over this provider, not a new
runtime layer. It chooses an exact `componentize-py 0.25.0` wheel URL/SHA-256
from the five server-release host pairs, invokes it through isolated uvx with
indexes and source builds disabled, and records that distribution alongside
source, dependency, runner, target, and output evidence in non-canonical build
provenance. Unsupported hosts fail closed. The resulting component remains
nondeterministic until the componentizer exposes controlled pre-initialization
randomness; provenance therefore does not become `.holo` identity.

Provider invocation returns outputs and completion as separate values. The
root primary alone supplies `returned` or a provider-observed `exited { code }`
completion; child primaries and non-primary dependencies never compete with it.
View, Tensor, and InferenceModel layers are non-exit-bearing. Every result must
report an actual typed outcome. Direct completion is published only after reverse-order
cleanup succeeds; resident completion ends one call without unloading the app.

## Inference engine boundary

The daemon never executes model weights in-process. Chat and model management call an `InferenceEngine` selected by `[inference].engine` in `live.toml`: `echo` (local fallback that repeats the user message), `weightc` (spawns `weightc ask <artifact-dir> <prompt> --json` against an imported `.wcpu` artifact directory), or `ollama` (proxies `POST /api/generate` on an Ollama-compatible endpoint). Imported artifacts are copied under `data_dir/models/<digest>/` and recorded in the content-addressed object store with `kind = "model"`. The daemon renders conversation history as a plain `role: content` transcript; engines apply their own chat templates. An unconfigured engine or model returns `LIVE_CAPABILITY_MISSING` rather than simulating a response.
