# Actual capabilities

This document is deliberately strict about what the current stable build does and does not do.

## Implemented and exercised

- One executable named `hologram`.
- Typed configuration rooted at `~/.config/hologram/` on every platform.
- Foreground and background daemon lifecycle.
- Statically registered, dependency-ordered modules.
- Kappa Registry represented as the first ordinary module.
- File listing, durable renaming, and retrieval over the content-addressed object store.
- Versioned Protobuf/gRPC native API and client.
- JSON REST endpoints and Utoipa-generated OpenAPI.
- A global `--json` CLI contract covering every command result, action acknowledgement, download report, decoded run mode, and typed runtime error so stdout can be consumed consistently with `jq`.
- Self-hosted Scalar interactive API reference.
- Local and remote client targets with capability-aware route planning.
- Bounded Kameo actors with links and supervision.
- Configurable `tracing` and runtime trace-filter updates.
- Optional OTLP/gRPC trace and RPC-metric export through OpenTelemetry.
- An awaited JSONL audit-event boundary with typed, non-secret allow/deny
  records for root requests, child delegations, and child requests.
- Bearer-token authentication seam for protected routes.
- First-class `.holo` fixture creation, import, list, inspect, payload-free plan, verify, and remove through CLI, native gRPC, JSON/HTTP, and OpenAPI surfaces.
- `.holo` compiler/runtime/execution path: v4 writes with v2/v3 reads, fat or thin packaging, explicit archive object κ / footer fingerprint / canonical application κ reporting, source-schema-v4 Wasm guest-contract tags with legacy identity preservation, normalized contract inspection and planning, exact `(LayerKind, contract)` provider selection, complete pre-provider resolution and re-hashing of root and child closures with deterministic limits/blockers, explanatory local or catalog-backed plans (including unsupported providers), transactional depth-first manifest-order prepare/start and exact reverse stop/rollback, root-primary-only invocation, aggregate tree status, multi-layer core-Wasm execution with nonzero primary positions, manifest-declared callable exports, direct service-free execution, κ-backed thin payload resolution, and idempotent resident load/unload sessions over supervised Wasmtime actors with lifecycle status.
- `.holo` capability admission: canonical requests are distinct from trusted effective grants, the default local baseline has no storage/channel/network authority, explicit development grants are restricted to direct files or loopback service configuration, denial occurs before provider preparation, and durable audit rows plus run/resident results report non-secret request/grant identities, relation, principal, trusted source, and outcome across CLI, JSON/HTTP, and Protobuf/gRPC.
- Typed `.holo` completion across CLI, JSON/HTTP, and Protobuf/gRPC: byte
  outputs remain separate from `returned` callable completion and real
  `exited { code }` process status; legacy peers decode as `unknown` without
  fabricating an exit code.
- `.holo` v4 inference-model packaging, import, verified application-directory metadata, and metadata-only `hologram ai inspect`.
- Direct execution of locked Python OCI rootfs archives through the experimental local container provider.
- Durable local conversation history.
- Conversation-backed chat over a configurable inference engine (`echo` by default; `weightc` one-shot CLI or an Ollama-compatible HTTP endpoint via `live.toml`), with independent, switchable threads in the desktop app.
- Optional resident per-conversation weightc sessions (`resident_sessions = true`): a supervised `weightc enter --jsonl` process per conversation with KV continuity, LRU-capped and lazily respawned on failure.
- Import, listing, and removal of `weightc` `.wcpu` model artifact directories.
- Non-streaming OpenAI-compatible (`/v1/chat/completions`, `/v1/models`) and Ollama-compatible (`/api/generate`, `/api/chat`, `/api/tags`, `/api/show`) HTTP inference APIs.
- Minimal control-plane node inventory and heartbeat records.
- Dynamic third-party modules as sha256-pinned, supervised subprocess plugins speaking gRPC over a Unix socket (`plugins list` / `plugins call`); plugins receive no host resource access in v1.
- Digest-verified update/rollback foundation.
- Built-in browser status page.
- Tauri desktop shell that bundles the server as a managed sidecar, persists
  user-selected application-directory watches, debounces recursive changes,
  compiles/imports outside the source tree, and lists/inspects the resulting
  verified `.holo` archives through the real catalog boundary while preserving
  the last good archive after a failed build. The persistence, filtering,
  debounce, and build-state engine is a Tauri-independent workspace crate;
  `src-tauri` retains only native path authority, fixed sidecar calls, and UI
  event delivery.
- Responsive Astro documentation website.
- Import-free Component Model v1 execution for exact-contract Wasm layers,
  directly and resident. Compiled components stay warm while every input uses
  a fresh store. Runtime-owned 64 MiB memory, 100 million fuel, 1 MiB
  input/output, and two-second deadline ceilings apply by default; admitted
  memory and CPU-time scalars can only tighten them. Timeout and cancellation
  use a component-local epoch-interruptible engine. No WASI or ambient host
  interface is linked.

## Present as an extension seam, not implemented by the default module set

- WASI or capability-gated Component host imports beyond the import-free v1
  world.
- Independently addressable or explicitly invokable child applications; current children share their parent's lifecycle and only the root primary is invoked.
- Uniform engine enforcement of scalar CPU, memory, deadline, priority, and
  concurrency budgets across providers; Component v1 currently enforces its
  memory/time subset plus host-owned ceilings.
- `.holo` execution for `tensor`, inference-model, and non-Python/resident `rootfs` layers.
- Token streaming on the compatibility APIs.
- Full enterprise users, OIDC/SAML, organizations, and RBAC policy storage.
- Fleet scheduling.
- Plugin host-resource capabilities, plugin HTTP routes, and microVM-isolated plugin execution.

Calls requiring a missing runtime return a typed `LIVE_CAPABILITY_MISSING` error rather than pretending execution occurred.
