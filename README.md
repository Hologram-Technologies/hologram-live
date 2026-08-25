# Hologram Live

![Hologram Desktop console showing local store size, module readiness, and system meters](docs/images/hologram-desktop-console.jpg)

Hologram Live is a local-first module host for the Hologram ecosystem. This repository produces two independent products:

- **Hologram Server** — the standalone `hologram` binary, containing the CLI and background service.
- **Hologram Desktop** — a Tauri application that bundles `hologram` as a managed sidecar.

The current desktop experience provides a Console dashboard, multi-thread Chat with archiving, content-addressed Files, watched `.holo` Applications, and module discovery in a responsive dark/light interface. A `Cmd/Ctrl+K` command palette reaches every action, and text size is adjustable with `Cmd/Ctrl` `+`/`-`/`0`. Chat routes through a configurable inference engine: the default `echo` engine repeats your message, while `weightc` (one-shot CLI over imported `.wcpu` artifacts) and Ollama-compatible HTTP endpoints serve real model completions.

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
- watch application source directories, compile/import their `.holo` archives,
  and inspect verified archive metadata;
- inspect the enabled module catalogue;
- follow the system appearance or remember a light/dark choice; and
- remain available from the system menu bar after the main window closes.

Build an installable desktop bundle with:

```bash
just desktop-build
```

### Standalone server

```bash
cargo build --release --locked --package hologram-live --bin hologram
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

## Machine-readable CLI output

Global `--json` is supported by every CLI command and may appear before or after the subcommand. A successful command writes one JSON value to stdout, including lifecycle actions, downloads, generated files, accepted mutations, and `run --output-format text`. Diagnostics remain on stderr, while a runtime failure writes a JSON object with `code` and `message` to stdout and exits nonzero. This makes the complete CLI safe to compose with `jq`:

```bash
hologram status --json | jq -r '.status'
hologram files get blake3:... --output ./asset.bin --json | jq '.byte_length'
hologram run ./my-app --input-text 'hello' --output-format text --json | jq -r '.[]'
hologram run application.holo --output-format text --json | jq -r '.[]'
```

Help and shell-completion text retain Clap's human-readable format.

## Demo workflows

### Chat and conversation history

Create a thread, copy its returned ID, and send a message:

```bash
hologram --json history new "Demo chat"
hologram --json chat send <conversation-id> "Hello, Hologram"
hologram --json history show <conversation-id>
```

`chat send` records the user message and the assistant response as one persisted exchange. Threads retain separate histories and can be resumed from the desktop app.

The response comes from the inference engine selected in `live.toml`:

```toml
[inference]
engine = "echo"            # echo | weightc | ollama
default_model = ""         # blake3:... of an imported model (weightc) or a model tag (ollama)
weightc_path = "weightc"
ollama_endpoint = "http://127.0.0.1:11434"
request_timeout_secs = 300
resident_sessions = false  # weightc only: keep one resident enter session per conversation
max_resident_sessions = 4  # LRU cap on resident sessions
```

The default `echo` engine repeats the user message; it needs no model and no external process. The `weightc` engine shells out to `weightc ask <artifact-dir> <prompt> --json` against an imported `.wcpu` artifact, and the `ollama` engine proxies `POST /api/generate` on the configured endpoint.

With `resident_sessions = true`, the weightc engine instead keeps a supervised `weightc enter --jsonl` process per conversation, so turns reuse the live KV context instead of replaying a transcript, and only the new message crosses the wire each turn. Sessions are LRU-capped by `max_resident_sessions`; a crashed session is reported as a typed error and lazily respawned (starting fresh context) on the next turn. This mode needs a weightc build with `enter --jsonl` support. Models are managed with:

```bash
hologram models import ./tinyllama.wcpu
hologram models list
hologram models remove blake3:...
```

Threads can be archived instead of deleted. Archived threads keep their messages and their `updated_at_millis`, but drop out of the default listing:

```bash
hologram --json history archive <conversation-id>
hologram --json history list          # archived threads are omitted
hologram --json history list --all    # archived threads included
hologram --json history unarchive <conversation-id>
```

In the desktop app, hovering a thread reveals an archive button, and archived threads collapse into an `ARCHIVED` group at the bottom of the thread list.

Existing schema-v1 configurations that used the original default modules are migrated automatically to enable Chat and the inference module. Custom module selections remain unchanged. The desktop also recovers safely when an older background service is still running by restarting onto its bundled sidecar before retrying a `chat.send` capability miss.

### Files

```bash
hologram files put ./notes.txt --media-type text/plain
hologram files list
hologram files rename blake3:... meeting-notes.txt
hologram files get blake3:... --output ./meeting-notes.txt
```

File bytes are addressed by their BLAKE3 content ID. Renaming changes only persisted filename metadata; the ID and bytes remain unchanged.

### `.holo` archives

Generate a validated source manifest interactively:

```bash
mkdir my-app
cd my-app
hologram app init
```

The generator prompts for ordered layers, their kind-specific entrypoint or surface information, the primary layer, and an optional capability file. It writes `hologram.json` atomically and prints the commands needed to compile and run it. For scripts and CI, provide the first layer as flags:

```bash
hologram app init ./my-app \
  --kind wasm \
  --path app.wasm \
  --entry holo_run
```

Use `--yes` for a minimal `app.wasm`/`_start` manifest. Existing manifests are preserved unless `--force` is explicit. Packaging remains a compiler choice: use `hologram compile` for a fat archive or add `--thin` for a manifest-only archive.

```bash
hologram holo fixture ./fixture.holo
hologram holo import ./fixture.holo
hologram holo list
hologram holo inspect ./application.holo
hologram holo plan ./application.holo
hologram holo verify ./application.holo
hologram holo inspect blake3:...
hologram holo plan blake3:...
hologram holo verify blake3:...
hologram holo remove blake3:...
```

#### Desktop watch loop

Open **Applications** in Hologram Desktop and choose **Add directory**, then
select a project containing `hologram.json`. The desktop compiles it immediately,
imports the successful archive into the normal local catalog, and recursively
watches the project for later changes. Builds are debounced and written to the
desktop cache rather than into the source directory.

Choose **Run** on a ready watched project, enter a text input in its inspector,
and the desktop loads the latest successful archive and displays its output.
Choose **Add .holo** to import an existing archive through the native file picker;
catalog archives use the same Run panel. Unsupported providers and denied
capabilities remain explicit runtime errors.

The Applications list is backed by the same `holo list` and `holo inspect`
operations shown above, so its archive κ, application κ, layers, capabilities,
physical sections, and verification state are not reconstructed by the web
frontend. A failed rebuild is shown on the watched project while the last good
immutable archive stays available. **Stop watching** removes only the persisted
watch registration; it does not delete the last cataloged `.holo` archive.

The stable build creates and validates real v4 `.holo` archives while retaining v2/v3 read compatibility. The physical file starts with `HOLO`, a version and section count, then fixed 24-byte section-table entries containing each section's kind, offset, and length. Logical layers do not have separate physical headers: their ordered descriptors live in the canonical `AppManifest` section and refer to payloads by κ.

```text
.holo v4
├─ header + section table
├─ AppManifest       primary · requires · ordered layers · children
├─ Extension         verified, queryable application directory
├─ ContentBlob × N   κ71 · content bytes
├─ other sections    plans · weights · ports · certificates · metadata
└─ BLAKE3 footer
```

The closed layer kinds are `wasm`, `tensor`, `rootfs`, `view`, and v4's non-exit-bearing `inference-model`. A layer records its content κ and entrypoint plus an architecture for rootfs, surface for views, or engine identifier for model services. Fat archives embed referenced blobs; thin archives retain the same application identity while resolving content through a store. Live emits fat archives by default, supports thin output with `--thin`, executes Wasm primary layers through wasmtime, and can directly execute a Python OCI bundle carried by a rootfs layer through the experimental local container provider.

Applications request authority with an optional `capabilities.json`. This is a
human-authored compiler input, not the object embedded in the archive:

```json
{
  "schema_version": 1,
  "storage_roots": [],
  "storage_quota_bytes": 0,
  "network_fetch": false,
  "network_announce": false,
  "publish_channels": [],
  "subscribe_channels": [],
  "memory_max_bytes": 0,
  "cpu_time_per_event_ms": 0,
  "priority_weight": 0
}
```

Set `"requires": "capabilities.json"` in `hologram.json`. `compile --check`
validates the version, fields, and canonical sorted κ lists; `compile` converts
the JSON into the upstream canonical `CapabilitySet` bytes and stores that
object's κ in `AppManifest.requires`. Omitting `requires` produces the canonical
empty request. A request describes what an application needs—it is never itself
a grant. In the upstream capability contract, a scalar budget of `0` means
unbounded; use a nonzero value to request a finite ceiling. Runtime grant
enforcement now decodes this canonical request and authorizes it before any
provider prepares. Child attenuation remains the next M2 slice.

Ordinary local execution uses the built-in baseline grant: no storage roots,
publish/subscribe channels, or network flags. A non-empty request therefore
fails with `LIVE_AUTHORIZATION_DENIED`. For an explicit local demo, provide a
separate trusted capability source as the effective grant:

```bash
hologram --json run ./application.holo \
  --development-grant ./development-grant.json \
  --input ./payload.bin | jq
```

Resident execution reads its development grant only from the service's trusted
configuration, never from a remote request. Set
`holo.development_grant = "development-grant.json"`; relative paths resolve
from `paths.config_dir`, and configuration validation rejects this mode on a
non-loopback listener. Both direct and service modes emit a warning and trace
the request κ, effective-grant κ, source, and allow/deny decision without
logging the capability document. Successful raw run results expose the same
non-secret decision metadata as `requested_capabilities_kappa`,
`effective_grant_kappa`, `grant_source`, and `authorization`, so automated
checks can retain the authority evidence:

```bash
hologram --json run ./application.holo \
  --development-grant ./development-grant.json |
  jq '{authorization, grant_source, requested_capabilities_kappa, effective_grant_kappa}'
```

See the [complete `.holo` format guide](https://hologram-technologies.github.io/hologram-live/docs/holo-files) for the byte layout, section kinds, manifest schema, identity model, application directory, verification rules, and current runtime support.

Archives whose primary layer is Wasm execute in-process through wasmtime. A self-contained archive can run directly without starting the service:

```bash
# Compile a source directory in memory and run it immediately
hologram run ./my-app --input-text 'hello' --output-format text

# Or compile once and run the resulting immutable archive
hologram compile ./my-app/hologram.json -o ./my-app.holo
hologram run ./my-app.holo --input ./payload.bin
```

`hologram run` accepts a project directory, its `hologram.json`, a local
self-contained `.holo` file, or a catalog κ. Project references are compiled as
fat archives in memory and are not written or imported. Repeat `--input` for
binary file inputs or `--input-text` for UTF-8 values.

For warm, repeated execution, import and load the archive into a resident session:

```bash
hologram compile ./my-app/hologram.json -o ./my-app.holo
hologram holo import ./my-app.holo
hologram holo load blake3:...
hologram run blake3:... --input ./payload.bin
hologram holo resident
hologram holo unload blake3:...
```

The compiler emits fat archives by default. `--thin` emits the same canonical application manifest without its κ-addressed payloads:

```bash
hologram compile ./my-app/hologram.json -o ./my-app.holo
hologram compile ./my-app/hologram.json --thin -o ./my-app.thin.holo
```

With `--json`, compilation reports the three distinct identities directly:

```bash
hologram --json compile ./my-app/hologram.json -o ./my-app.holo |
  jq '{archive_kappa, archive_fingerprint, application_kappa, capabilities_kappa}'
```

`archive_kappa` is the BLAKE3 address of the complete physical file,
`archive_fingerprint` is the footer integrity value, and
`application_kappa` addresses the canonical `AppManifest`. Fat and thin files
have different archive κ values and footer fingerprints but the same
application κ and `capabilities_kappa`. The existing `kappa` returned by `holo inspect`, catalog,
resident, and run operations continues to mean the physical archive object;
inspection adds `application_kappa` when an application manifest is present.

Importing the fat archive verifies and caches its content blobs. A subsequently imported thin archive can resolve those payloads from the local κ store and use the same resident load/run commands. Direct file execution requires a fat archive because it deliberately has no external content resolver.

Compiled archives include a versioned application directory over their canonical manifest. `hologram --json holo inspect ./application.holo` inspects a local archive without importing it or starting the service; the command also accepts an imported `blake3:...` object ID. It exposes the physical `kappa`, canonical `application_kappa`, footer fingerprint, ordered layers, child applications, required capability set, model engine tags, and embedded κ-addressed blobs. The directory is verified against the manifest and blob contents on import; older archives without it are still inspected by deriving the same view, and pre-application structural archives report no application κ.

Use `holo plan` to explain whether an application can run before starting any provider. Local paths plan without the service; imported κ values plan against the local content cache. The payload-free report includes all three identities, fat/thin/hybrid packaging, the capability object, ordered layers, resolution sources, provider availability, planning limits, and stable typed blockers:

```bash
hologram --json holo plan ./application.holo |
  jq '{application_kappa, packaging, runnable, layers, blockers}'

hologram --json holo plan blake3:... |
  jq '{execution_target, resolved_object_count, resolved_bytes, runnable}'
```

An unsupported layer is still a successful plan: `runnable` is `false` and `blockers` explains the unavailable provider with a `kind`, `error_code`, and message. Malformed archives, missing catalog objects, and transport failures retain the normal typed JSON error contract.

#### AI model applications

Hologram Live now reads and writes `.holo` v4 `InferenceModel` layers. A complete model archive produced by `hologram-ai` can be imported and inspected without initializing its engine:

```bash
hologram --json ai inspect model.holo
hologram holo import model.holo
```

The inspection lists each callable service entry, its engine identifier, content κ, and whether the bundle is embedded. Model-source acquisition and R4G1 compilation remain owned by `hologram-ai`; this binary does not yet expose `hologram ai compile` or connect a model session to `hologram ai infer`. Attempting to execute a model-only archive returns `LIVE_CAPABILITY_MISSING` and names the unconnected service rather than simulating inference.

For low-level archive assembly, a source manifest can package an already-built provider bundle:

```json
{
  "schema_version": 1,
  "layers": [{
    "kind": "inference-model",
    "path": "model.bundle",
    "entry": "ai.default",
    "engine": "uor-r4"
  }]
}
```

`weightc` remains a chat execution provider over imported `.wcpu` directories. Those directories are not placed into `.holo` files until a deterministic single-blob bundle and validation contract is defined. See the [AI model application guide](https://hologram-technologies.github.io/hologram-live/docs/model-apps) and [ADR 009](specs/adrs/009-inference-model-holo-v4.md).

Before direct execution or `holo load` starts a provider, Live builds a runtime-owned application plan from the canonical manifest. It resolves and re-hashes the required capability object and every non-child layer from embedded content or the local κ store, deduplicates shared objects, applies layer/object/byte limits, and rejects missing secondary layers before compiling anything. Child references remain visible blockers until M2 defines capability attenuation. A closed `LayerKind` registry then prepares and starts every supported layer in manifest order, invokes the declared primary layer, and stops or rolls back in reverse order. Wasm layers use Wasmtime behind this boundary and may have a primary position other than zero; resident status reports `state`, aggregate resident bytes, queued calls, and processed calls. Repeated load and unload are idempotent. Python rootfs archives use the same lifecycle through an explicitly experimental, direct-only OCI adapter. Inference-model services can be inspected but not invoked through Live yet. Tensors, inference models without a provider, resident Python rootfs archives, and unknown rootfs payloads return a typed `LIVE_CAPABILITY_MISSING` error. The compiler/runtime/executor boundary is recorded in `specs/adrs/007-holo-compiler-runtime-execution.md` and the planning/provider contract in `specs/adrs/010-holo-application-plan-and-provider-lifecycle.md`; the Wasm guest contract is documented in `src/holo_wasm.rs` and demonstrated by `features/fixtures/wasm-app/`.

#### Python applications

Python is a compiler input, not a fifth `.holo` layer kind. Source-manifest schema v2 can now turn a locked Python project into an architecture-specific `rootfs` layer containing CPython, the application, dependencies, and required Linux libraries. The entrypoint contract is `module:function` where the function accepts and returns `bytes`.

The repository includes a working NumPy + pandas project in `examples/python-numpy-pandas/`. A running Docker-compatible engine is required for this experimental compiler and direct executor:

```console
$ hologram compile --check examples/python-numpy-pandas/hologram.json
$ hologram compile examples/python-numpy-pandas/hologram.json \
    --output numpy-pandas.holo
$ hologram run numpy-pandas.holo \
    --input examples/python-numpy-pandas/request.json \
    --output-format json
{
  "columns": [
    "label",
    "value"
  ],
  "mean": 20.0,
  "rows": 3,
  "sum": 60.0
}
```

`hologram run` preserves the binary-safe `HoloRunResult` envelope by default. Add `--output-format text` for UTF-8 application output or `--output-format json` for JSON application output. One decoded result prints directly; results from multiple `--input` arguments print in order, with JSON results collected into an array. Invalid text or JSON returns a typed protocol error instead of changing the bytes.

Generate the same schema without hand-writing JSON:

```bash
hologram app init ./my-python-app \
  --template python --profile rootfs \
  --project . --entry my_package:main --lock uv.lock --arch arm64
```

Compilation stages only `pyproject.toml`, the declared `uv.lock`, and `src/`, then runs `uv sync --locked` in a clean Linux image. It does not read or copy the host `.venv`. Direct execution validates the archive and target architecture, disables networking, mounts the container filesystem read-only, drops Linux capabilities, enables `no-new-privileges`, and applies CPU, memory, PID, temporary-storage, input/output, and 30-second wall-clock limits.

This is an intentionally explicit demo provider, not the final untrusted-workload boundary: it requires a local Docker-compatible engine, supports direct fat archives only, and leaves cached OCI images behind for repeat runs. New archives record the exact image ID, so a warm local run skips decompression and `docker image load` only when that trusted ID is already present; a cold machine still restores the image from the archive. Compile once with an optimized release binary and reuse the resulting `.holo`; debug builds spend substantially longer hashing the roughly 100 MiB archive. `just python-holo-demo` builds and uses the release CLI, with a one-time optimized link on the first invocation. Use a digest-pinned value for `source.base` when reproducible builds matter. Portable Python/WASI and hardware-backed microVM rootfs execution remain planned. See the [Python application guide](https://hologram-technologies.github.io/hologram-live/docs/python-apps) and [ADR 008](specs/adrs/008-python-rootfs-oci-provider.md).

The demo writes one JSON document to stdout; progress and failures use stderr:

```bash
just python-holo-demo | jq .
just python-holo-demo | jq '.output'

# Retain the verified archive instead of deleting the temporary artifact
just python-holo-package target/numpy-pandas.holo | jq '{archive, archive_bytes}'
target/release/hologram --json run target/numpy-pandas.holo \
  --input examples/python-numpy-pandas/request.json
```

### Inference compatibility APIs

The daemon exposes non-streaming OpenAI- and Ollama-compatible HTTP surfaces over the configured inference engine:

```bash
curl http://127.0.0.1:11435/v1/models
curl -X POST http://127.0.0.1:11435/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"echo","messages":[{"role":"user","content":"Hello"}]}'
curl -X POST http://127.0.0.1:11435/api/generate \
  -H 'content-type: application/json' \
  -d '{"model":"echo","prompt":"Hello","stream":false}'
curl http://127.0.0.1:11435/api/tags
```

Requests with `stream: true` are rejected with a typed error; token streaming requires engine support and is future work. Both surfaces sit behind the same bearer-token seam as every other module route.

### Third-party plugin modules

Beyond the built-in catalogue, the daemon hosts dynamic third-party modules as supervised subprocesses. Plugins are an explicit, sha256-pinned allowlist in `live.toml` — nothing is scanned or loaded implicitly:

```toml
[plugins]
enabled = true

[[plugins.modules]]
id = "com.example.weather"
path = "/usr/local/bin/weather-plugin"
sha256 = "<64 hex chars>"
```

Each plugin speaks a small gRPC contract (`hologram.live.plugin.v1.PluginHost`: describe/invoke/ping) over a Unix socket the daemon provides, runs under a supervised actor with a bounded mailbox and bounded restart, and receives a scrubbed environment. Its operations join the capability manifest and are reachable through the native client:

```bash
hologram plugins list
hologram plugins call com.example.weather weather.current '{"city":"Berlin"}'
```

Plugins have no host resource access in v1 — no store, config, or network mediation — and no HTTP routes; they are pure compute over JSON payloads. Wrapping plugin executables in microVMs is the documented hardening path. See `specs/adrs/005-subprocess-plugin-boundary.md`.

## Built-in modules

Trusted modules are statically linked and registered in the `builtin_modules!` catalogue in `src/modules/mod.rs`:

| Module | Current responsibility |
| --- | --- |
| `dev.hologram.live.system` | Health, capabilities, and module discovery |
| `dev.hologram.live.kappa-registry` | Local content-addressed registry provider |
| `dev.hologram.live.files` | File upload, listing, renaming, and download |
| `dev.hologram.live.holo` | `.holo` import, inspection, verification, cataloguing, and resident Wasm execution |
| `dev.hologram.live.history` | Durable conversations and messages |
| `dev.hologram.live.chat` | Conversation-backed chat over the configured inference engine |
| `dev.hologram.live.inference` | Model import, listing, and removal for the engine boundary |
| `dev.hologram.live.openai-compat` | Non-streaming OpenAI-compatible `/v1` inference API |
| `dev.hologram.live.ollama-compat` | Non-streaming Ollama-compatible `/api` inference API |
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

The source tree enforces the product boundary:

```text
src/                                  standalone server, CLI, and shared protocol/runtime
crates/hologram-application-watch/    Tauri-independent local development engine
apps/desktop/src-tauri/               thin native shell and sidecar adapter only
apps/desktop/src/                     webview frontend
```

The desktop never owns a second server implementation. Its preparation script
builds the root `hologram-live` package explicitly and copies the resulting
`hologram` executable into Tauri's ignored sidecar staging directory. The
application-watch crate owns reusable persistence and debounce behavior outside
`src-tauri`; the desktop adapter supplies user-approved paths, fixed
compile/import commands, and UI events. A cloud server build therefore needs
only Rust and the root package—no Tauri, WebKit, Node.js, or desktop source.
The `product-boundary` verification gate inspects the resolved Cargo graph and
fails if Tauri or the application-watch crate enters the standalone server.

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

The server crate declares Rust 1.94 as its minimum supported version, matching Wasmtime 46's compiler floor. `rust-toolchain.toml` currently pins Rust 1.97.1 for repository development and release CI. The desktop and documentation projects also require Node.js and npm; `just` is used for the common project recipes.

Run the complete server verification path with:

```bash
just verify
```

That includes formatting, source-size policy, checks, unit tests, Clippy, public-boundary BDD scenarios, a release build, and an isolated end-to-end smoke test. The smoke test covers legacy configuration migration, module discovery, file storage and renaming, `.holo` operations including resident Wasm execution, chat persistence through the default echo engine, model listing, the OpenAI/Ollama compatibility endpoints, OpenAPI generation, and shutdown.

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

- enterprise identity, organizations, or RBAC storage; or
- fleet scheduling.

Chat runs against the configured inference engine (`echo` remains the default), Wasm-layer `.holo` archives execute resident, direct Python OCI rootfs archives execute through the experimental local container provider, and the weightc engine can keep resident per-conversation sessions. Token streaming, tensor execution, inference-model provider invocation, resident rootfs execution, portable Python/WASI, and the production microVM provider remain future work. Missing runtime capabilities return a typed `LIVE_CAPABILITY_MISSING` error rather than simulating success.

## Further documentation

- [Actual capabilities](ACTUAL_CAPABILITIES.md)
- [Architecture](ARCHITECTURE.md)
- [Security](SECURITY.md)
- [Dependencies](DEPENDENCIES.md)
- [Release process](RELEASING.md)
