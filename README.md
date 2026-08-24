# Hologram Live

![Hologram Desktop chat interface](docs/images/hologram-desktop.jpg)

Hologram Live is a local-first module host for the Hologram ecosystem. This repository produces two independent products:

- **Hologram Server** — the standalone `hologram` binary, containing the CLI and background service.
- **Hologram Desktop** — a Tauri application that bundles `hologram` as a managed sidecar.

The current desktop experience provides an Overview, multi-thread Echo Chat, content-addressed Files, and module discovery in a responsive light/dark interface. Echo Chat is deliberately a demo module: it saves each user message and repeats it as the assistant response. Model inference is not implemented yet.

## Quick start

### Desktop application

```bash
cd apps/desktop
npm ci
npm run dev
```

The preparation step builds the server sidecar before Tauri opens. The desktop app can:

- start, restart, and stop the local Hologram service;
- create and switch between chat threads with independent, durable histories;
- upload, rename, list, and download local files;
- inspect the enabled module catalogue;
- follow the system appearance or remember a light/dark choice; and
- remain available from the system menu bar after the main window closes.

Build an installable desktop bundle with:

```bash
just desktop-build
```

### Standalone server

```bash
cargo build --release --locked
./target/release/hologram init
./target/release/hologram start
./target/release/hologram status
./target/release/hologram modules list
```

Run the service in the foreground instead with:

```bash
./target/release/hologram serve
```

The default configuration and local endpoint are:

```text
~/.config/hologram/live.toml
http://127.0.0.1:11435
```

Open the endpoint for the built-in status page. API documentation is available at:

```text
http://127.0.0.1:11435/docs
http://127.0.0.1:11435/openapi.json
```

`/docs` is the self-hosted Scalar reference; `/openapi.json` is the generated OpenAPI document. Native clients use the versioned Protobuf/gRPC service on the same endpoint.

## Demo workflows

### Chat and conversation history

Create a thread, copy its returned ID, and send a message:

```bash
hologram --json history new "Demo chat"
hologram --json chat send <conversation-id> "Hello, Hologram"
hologram --json history show <conversation-id>
```

`chat send` records the user message and echoed assistant response as one persisted exchange. Threads retain separate histories and can be resumed from the desktop app.

Existing schema-v1 configurations that used the original default modules are migrated automatically to enable Echo Chat. Custom module selections remain unchanged. The desktop also recovers safely when an older background service is still running by restarting onto its bundled sidecar before retrying a `chat.send` capability miss.

### Files

```bash
hologram files put ./notes.txt --media-type text/plain
hologram files list
hologram files rename blake3:... meeting-notes.txt
hologram files get blake3:... --output ./meeting-notes.txt
```

File bytes are addressed by their BLAKE3 content ID. Renaming changes only persisted filename metadata; the ID and bytes remain unchanged.

### `.holo` archives

```bash
hologram holo fixture ./fixture.holo
hologram holo import ./fixture.holo
hologram holo list
hologram holo inspect blake3:...
hologram holo verify blake3:...
hologram holo remove blake3:...
```

The stable build creates and validates real v3 `.holo` archives. It does not advertise `.holo` execution because no compute backend is enabled.

## Built-in modules

Trusted modules are statically linked and registered in the `builtin_modules!` catalogue in `src/modules/mod.rs`:

| Module | Current responsibility |
| --- | --- |
| `dev.hologram.live.system` | Health, capabilities, and module discovery |
| `dev.hologram.live.kappa-registry` | Local content-addressed registry provider |
| `dev.hologram.live.files` | File upload, listing, renaming, and download |
| `dev.hologram.live.holo` | `.holo` import, inspection, verification, and cataloguing |
| `dev.hologram.live.history` | Durable conversations and messages |
| `dev.hologram.live.chat` | Conversation-backed echo demo |
| `dev.hologram.live.control-plane` | Minimal node inventory and heartbeats |

Each module declares its stable ID, dependencies, operation IDs, HTTP routes, and OpenAPI contribution. Operators can enable a subset with `modules.enabled` in `live.toml`. Executable module behavior remains compiled Rust rather than configuration-defined code.

## Configuration

The current configuration schema is version 2. Generate a complete file with `hologram init`, inspect it with `hologram config show`, and validate the installation with `hologram doctor`.

Local structured tracing is always available. OTLP export is enabled only when a collector endpoint is configured:

```toml
[telemetry]
enabled = true
endpoint = "http://127.0.0.1:4317"
service_name = "hologram-live"
export_timeout_secs = 5
```

`OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME` override those values. No telemetry network connection is made when the endpoint is unset.

The client can route to local or remote authorities after a capability handshake. Non-loopback remote endpoints require HTTPS, and authentication, authorization, TLS, and integrity errors never trigger fallback to another authority.

## Architecture

```text
Tauri desktop ── managed sidecar ──┐
hologram CLI ─────── gRPC ─────────┼── Hologram service
browser/API ───── JSON/HTTP ───────┘    config · auth · telemetry · audit
                                               │
                              registry · files · .holo · history · chat
```

The kernel owns configuration, lifecycle, capability-aware routing, authenticated dispatch, tracing, audit, update coordination, and actor supervision primitives. Product behavior belongs to modules. Kameo actors are used only for long-lived mutable state or bounded background work; ordinary request handlers remain ordinary Rust code.

## Releases

Server and desktop versions advance independently and create separate GitHub Releases:

- `server-v<version>` publishes standalone Linux, macOS, and Windows server archives plus `SHA256SUMS`.
- `desktop-v<version>` publishes Linux, macOS, and Windows Tauri installers.
- `docs-v<version>` deploys the versioned Astro documentation site to GitHub Pages.

The desktop installer includes a server sidecar for local operation, but it remains a desktop release. See [RELEASING.md](RELEASING.md) for version checks, tags, and the artifact matrix.

Build either product locally with:

```bash
just server-build
just desktop-build
just docs-release     # after committing the documentation version
```

## Install the server from this checkout

```bash
./install.sh
```

This builds the release binary and installs `hologram` into `~/.local/bin`. Set `HOLOGRAM_INSTALL_PREFIX` to choose a different prefix.

## Development

The crate declares Rust 1.88 as its minimum supported version. `rust-toolchain.toml` currently pins Rust 1.97.1 for repository development. The desktop and documentation projects also require Node.js and npm; `just` is used for the common project recipes.

Run the complete server verification path with:

```bash
just verify
```

That includes formatting, source-size policy, checks, unit tests, Clippy, public-boundary BDD scenarios, a release build, and an isolated end-to-end smoke test. The smoke test covers legacy configuration migration, module discovery, file storage and renaming, `.holo` operations, Echo Chat persistence, OpenAPI generation, and shutdown.

Useful development commands:

```bash
just dev          # foreground server with automatic Rust rebuild/restart
just tauri        # Tauri development application
just docs-dev     # Astro documentation at http://127.0.0.1:54321
```

To use `just dev`, install `cargo-watch` once:

```bash
cargo install cargo-watch --locked
```

## Current limitations

The default build does not yet provide:

- model or inference execution behind Chat;
- `.holo` execution or resident compute sessions;
- OpenAI- or Ollama-compatible inference APIs;
- dynamic third-party modules;
- enterprise identity, organizations, or RBAC storage; or
- fleet scheduling.

Missing runtime capabilities return a typed `LIVE_CAPABILITY_MISSING` error rather than simulating success.

## Further documentation

- [Actual capabilities](ACTUAL_CAPABILITIES.md)
- [Architecture](ARCHITECTURE.md)
- [Security](SECURITY.md)
- [Dependencies](DEPENDENCIES.md)
- [Release process](RELEASING.md)
