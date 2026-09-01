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

ADR 011 assigns Wasm contracts through the canonical, identity-bearing layer
`aux` tag. Source schema v4 exposes the required tag as `contract`; core-Wasm
v1 is `hologram:guest/core-wasm@1`. The runtime validates the selector before
exact `(kind, contract)` provider lookup. An empty or unknown contract fails closed without
reaching core Wasm or ambient WASI. Component v1 executes through a dedicated import-free
provider with fixed memory, fuel, input/output, and wall-time ceilings. It uses
a fresh store per input and an isolated epoch-interruptible engine so timeout,
stop, or dropped-future cancellation terminates synchronous guest work without
interrupting another application or core Wasm. The first Component Model world
imports nothing. The separate `component-store-read@1` world imports only the
mediated object-store read interface. It requires a nonempty admitted
`storage_roots` request before linker construction, retains only those
contained roots, and checks the exact target before touching the store;
direct, resident, and delegated-child paths share the same rule. Future write,
channel, and mediated network interfaces require their corresponding admitted canonical fields;
clocks, random, environment, process control, secrets, inference, and raw
sockets remain unavailable while no sufficiently scoped capability exists.
Under-granted imports must fail before linker construction.

Python Component compilation selects `componentize-py 0.25.0` from a closed
set of exact upstream wheel URLs and SHA-256 hashes covering the server release
matrix. uvx runs that direct reference with indexes and source builds disabled;
unsupported hosts fail with `LIVE_CAPABILITY_MISSING` rather than resolving a
different artifact. The non-canonical compile report records the selected
distribution. This is a supply-chain pin, not a reproducibility claim: the
componentizer's uncontrolled pre-initialization randomness still changes clean
build output bytes.

Python rootfs compilation resolves mutable base tags to a registry manifest
digest before Docker consumes `FROM`. Bundle schema 3 then rejects unsafe,
duplicate, missing, non-file, oversized, or image-ID-mismatched Docker archive
content and rewrites the exact config and ordered layer bytes into ADR 017's
canonical SHA-256 blob layout with fixed tar metadata. This removes export
metadata as an identity input, but does not make the experimental direct Docker
provider an isolation boundary or prove uncached clean-build equality across
hosts. Provenance therefore remains non-canonical and reports
`reproducible: false`.

## Desktop sidecar

The Tauri application invokes a bundled `hologram` sidecar through fixed lifecycle and status commands. It does not expose a general-purpose shell command to the webview.

## Reporting

Do not publish vulnerability details in a public issue. Use the repository's private security-reporting channel once the production repository is available.
