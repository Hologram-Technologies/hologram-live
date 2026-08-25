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
- Self-hosted Scalar interactive API reference.
- Local and remote client targets with capability-aware route planning.
- Bounded Kameo actors with links and supervision.
- Configurable `tracing` and runtime trace-filter updates.
- Optional OTLP/gRPC trace and RPC-metric export through OpenTelemetry.
- A separate audit-event boundary.
- Bearer-token authentication seam for protected routes.
- First-class `.holo` fixture creation, import, list, inspect, verify, and remove.
- `.holo` compiler/runtime/execution path: fat or thin v3 packaging, direct service-free execution of self-contained Wasm archives, κ-backed thin payload resolution, and resident load/run/unload sessions over wasmtime.
- Durable local conversation history.
- Conversation-backed chat over a configurable inference engine (`echo` by default; `weightc` one-shot CLI or an Ollama-compatible HTTP endpoint via `live.toml`), with independent, switchable threads in the desktop app.
- Optional resident per-conversation weightc sessions (`resident_sessions = true`): a supervised `weightc enter --jsonl` process per conversation with KV continuity, LRU-capped and lazily respawned on failure.
- Import, listing, and removal of `weightc` `.wcpu` model artifact directories.
- Non-streaming OpenAI-compatible (`/v1/chat/completions`, `/v1/models`) and Ollama-compatible (`/api/generate`, `/api/chat`, `/api/tags`, `/api/show`) HTTP inference APIs.
- Minimal control-plane node inventory and heartbeat records.
- Dynamic third-party modules as sha256-pinned, supervised subprocess plugins speaking gRPC over a Unix socket (`plugins list` / `plugins call`); plugins receive no host resource access in v1.
- Digest-verified update/rollback foundation.
- Built-in browser status page.
- Tauri desktop shell that bundles the daemon as a managed sidecar.
- Responsive Astro documentation website.

## Present as an extension seam, not implemented by the default module set

- `.holo` execution for `tensor` and `rootfs` layers (Wasm layers execute today).
- Token streaming on the compatibility APIs.
- Full enterprise users, OIDC/SAML, organizations, and RBAC policy storage.
- Fleet scheduling.
- Plugin host-resource capabilities, plugin HTTP routes, and microVM-isolated plugin execution.

Calls requiring a missing runtime return a typed `LIVE_CAPABILITY_MISSING` error rather than pretending execution occurred.
