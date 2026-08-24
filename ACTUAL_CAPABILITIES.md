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
- Durable local conversation history.
- Conversation-backed echo chat with independent, switchable threads in the desktop app.
- Minimal control-plane node inventory and heartbeat records.
- Digest-verified update/rollback foundation.
- Built-in browser status page.
- Tauri desktop shell that bundles the daemon as a managed sidecar.
- Responsive Astro documentation website.

## Present as an extension seam, not implemented by the default module set

- `.holo` execution.
- Resident model/inference execution.
- OpenAI-compatible inference.
- Ollama-compatible inference.
- Full enterprise users, OIDC/SAML, organizations, and RBAC policy storage.
- Dynamic third-party modules.
- Fleet scheduling.

Calls requiring a missing runtime return a typed `LIVE_CAPABILITY_MISSING` error rather than pretending execution occurred.
