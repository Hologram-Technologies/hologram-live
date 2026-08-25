# Security model

## Defaults

- The daemon binds to loopback by default.
- Configuration lives under `~/.config/hologram/`.
- Unknown configuration fields are rejected with `serde(deny_unknown_fields)`.
- The crate forbids unsafe Rust in project code.
- gRPC messages and HTTP bodies are size-bounded.
- Actor mailboxes are bounded.
- Protected routes can require a bearer token supplied through an environment variable.
- Secret values are not intentionally included in trace fields.

## Remote endpoints

Non-loopback remote endpoints must use HTTPS. Authentication, authorization, integrity, and TLS failures must never fall through to a different authority. Read fallback is permitted only when route planning determines that an operation is absent or a target is unavailable before dispatch.

## Content integrity

Stored objects are addressed with BLAKE3. `.holo` archives are parsed by the pinned upstream Hologram archive implementation and their native footer fingerprint is verified before catalog admission.

## Updates

Update manifests identify target-specific binary size and BLAKE3 digest. Downloads are staged and verified before atomic replacement. The previous executable is retained for rollback on Unix platforms.

A production release should additionally sign the update manifest and publish provenance/SBOM artifacts.

## Tracing and audit

Tracing is configurable and may be filtered. OpenTelemetry does not export unless an OTLP endpoint is explicitly configured. Security-relevant state changes use a separate audit boundary so audit behavior is not coupled to log verbosity.

Raw prompts, response bodies, authorization headers, bearer tokens, private keys, and uploaded file bytes should not be added to tracing fields.

Every evaluated `.holo` root request, child delegation, and child request is
written to `audit.jsonl` through an awaited audit boundary before provider
preparation. The typed row contains the principal, application and optional
parent application κ, relation, requested or delegated capability κ, effective
grant κ, trusted grant-source label, and `allowed` or `denied` outcome. It does
not contain capability source documents, storage roots, channels, tokens,
authorization headers, or application payloads. Successful admission fails
closed if the audit record cannot be persisted; authorization denial remains
the primary error if denial auditing also fails.

Core-Wasm guest contract v1 links no imports and no WASI functions. A layer's
manifest entry selects only a typed export inside the already selected module;
it cannot select a provider or host function. Future WASI or Hologram imports
must be introduced by a versioned contract and linked only from the admitted
effective grant.

## Desktop sidecar

The Tauri application invokes a bundled `hologram` sidecar through fixed lifecycle and status commands. It does not expose a general-purpose shell command to the webview.

## Reporting

Do not publish vulnerability details in a public issue. Use the repository's private security-reporting channel once the production repository is available.
