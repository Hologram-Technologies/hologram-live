# Hologram Live

`hologram-live` is an extensible local/remote host for the Hologram ecosystem. The Cargo package is named `hologram-live`; the user-facing executable is **`hologram`**.

This source archive contains the Rust daemon/CLI, a Tauri desktop application, and the documentation site. The daemon's declared minimum Rust version is 1.88.

## Build

```bash
cargo build --release --locked
./target/release/hologram --version
```

## Run locally

```bash
./target/release/hologram init
./target/release/hologram start
./target/release/hologram status
./target/release/hologram modules list
```

The default configuration is always:

```text
~/.config/hologram/live.toml
```

The local HTTP endpoint defaults to:

```text
http://127.0.0.1:11435
```

Open that URL for the built-in status page, or use:

```text
http://127.0.0.1:11435/docs
http://127.0.0.1:11435/openapi.json
```

`/docs` is the self-hosted Scalar API reference. `/openapi.json` is the raw
Utoipa-generated document.

The CLI communicates with the daemon through the versioned Protobuf/gRPC service on the same endpoint. Browser routes remain JSON/HTTP.

## Telemetry

Local structured tracing is always available. OTLP export is enabled only when a collector endpoint is configured:

```toml
[telemetry]
enabled = true
endpoint = "http://127.0.0.1:4317"
service_name = "hologram-live"
export_timeout_secs = 5
```

`OTEL_EXPORTER_OTLP_ENDPOINT` and `OTEL_SERVICE_NAME` override the corresponding configuration values.

## Verify the end-to-end starter

```bash
./scripts/smoke.sh ./target/release/hologram
```

The smoke test uses an isolated temporary home directory and exercises daemon startup, module discovery, `.holo` fixture creation/import/inspection/verification, history, OpenAPI generation, and shutdown.

## Install from this checkout

```bash
./install.sh
```

This builds with `cargo build --release --locked` and installs `hologram` into `~/.local/bin` unless `HOLOGRAM_INSTALL_PREFIX` is set.

## Current modules

The first release statically registers trusted modules:

- `dev.hologram.live.system`
- `dev.hologram.live.kappa-registry`
- `dev.hologram.live.files`
- `dev.hologram.live.holo`
- `dev.hologram.live.history`
- `dev.hologram.live.control-plane`

Kappa Registry is an ordinary first module, not the host kernel.

## `.holo` support

Implemented:

```bash
hologram holo fixture ./fixture.holo
hologram holo import ./fixture.holo
hologram holo list
hologram holo inspect blake3:...
hologram holo verify blake3:...
hologram holo remove blake3:...
```

The default stable build intentionally does not advertise `.holo` execution. It depends on the pinned upstream Hologram archive surface only; execution is a typed provider seam for the next engine module. See [`ACTUAL_CAPABILITIES.md`](ACTUAL_CAPABILITIES.md).

## Development

```bash
cargo fmt --all --check
cargo check --all-targets --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

For a foreground development server that recompiles and restarts whenever the
Rust sources, Protobuf schema, build script, or Cargo manifests change:

```bash
cargo install cargo-watch --locked # one-time prerequisite
just dev
```

The Astro documentation site is isolated from the Rust runtime:

```bash
just docs-dev
```

Astro serves the site at `http://127.0.0.1:4321` and reloads the browser when
documentation layouts, pages, or styles change.

The Tauri desktop application is also isolated and bundles `hologram` as a sidecar:

```bash
cd apps/desktop
npm install
npm run dev
```

## Architecture

See:

- [`ARCHITECTURE.md`](ARCHITECTURE.md)
- [`SECURITY.md`](SECURITY.md)
- [`DEPENDENCIES.md`](DEPENDENCIES.md)
- [`VERIFICATION.md`](VERIFICATION.md)
