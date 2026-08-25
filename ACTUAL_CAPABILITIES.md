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
- A separate audit-event boundary.
- Bearer-token authentication seam for protected routes.
- First-class `.holo` fixture creation, import, list, inspect, payload-free plan, verify, and remove through CLI, native gRPC, JSON/HTTP, and OpenAPI surfaces.
- `.holo` compiler/runtime/execution path: v4 writes with v2/v3 reads, fat or thin packaging, explicit archive object κ / footer fingerprint / canonical application κ reporting, complete pre-provider resolution and re-hashing of capabilities plus all non-child layers with deterministic limits/blockers, explanatory local or catalog-backed plans (including unsupported providers), a closed `LayerKind` provider registry with transactional ordered prepare/start and reverse stop/rollback, multi-layer Wasm execution with nonzero primary positions, direct service-free execution, κ-backed thin payload resolution, and idempotent resident load/unload sessions over supervised Wasmtime actors with lifecycle status.
- `.holo` capability admission: canonical requests are distinct from trusted effective grants, the default local baseline has no storage/channel/network authority, explicit development grants are restricted to direct files or loopback service configuration, denial occurs before provider preparation, and successful run results report non-secret request/grant identities and their trusted source across JSON and gRPC.
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
  the last good archive after a failed build.
- Responsive Astro documentation website.

## Present as an extension seam, not implemented by the default module set

- Child-application capability delegation and attenuation; child references remain planning blockers.
- Engine enforcement of scalar CPU, memory, deadline, and concurrency budgets carried by effective grants.
- `.holo` execution for `tensor`, inference-model, and non-Python/resident `rootfs` layers.
- Token streaming on the compatibility APIs.
- Full enterprise users, OIDC/SAML, organizations, and RBAC policy storage.
- Fleet scheduling.
- Plugin host-resource capabilities, plugin HTTP routes, and microVM-isolated plugin execution.

Calls requiring a missing runtime return a typed `LIVE_CAPABILITY_MISSING` error rather than pretending execution occurred.
