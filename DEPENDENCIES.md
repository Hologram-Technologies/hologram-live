# Dependency budget

The project uses one primary dependency per responsibility and keeps desktop and documentation dependencies outside the daemon crate.

| Crate or application                         | Responsibility                                                    |
| -------------------------------------------- | ----------------------------------------------------------------- |
| `tokio`                                      | async runtime, sockets, signals, and process control              |
| `axum`                                       | browser-facing JSON/HTTP routes and shared HTTP serving           |
| `tonic`, `prost`                             | native Protobuf/gRPC API and client                               |
| `reqwest`                                    | outbound HTTP over Rustls: verified update downloads, the Ollama inference engine, and mediated Component fetch |
| `serde`, `serde_json`, `toml`                | typed configuration and public JSON                               |
| `clap`                                       | CLI parsing                                                       |
| `fs4`                                        | cross-platform daemon ownership lock                              |
| `kameo`                                      | bounded actors, links, and supervision                            |
| `tracing`, `tracing-subscriber`              | structured local diagnostics and runtime filtering                |
| `opentelemetry*`, `tracing-opentelemetry`    | OTLP/gRPC trace and metric export                                 |
| `utoipa`                                     | OpenAPI generation for the JSON API                               |
| `scalar_api_reference`                       | self-hosted interactive OpenAPI reference                         |
| `rustls`                                     | explicit ring crypto provider for reqwest, keeping the build pure Rust |
| `blake3`                                     | content addressing and update integrity                           |
| `uor-hologram` (`archive`, `space`)          | canonical v2–v4 `.holo` archives and application manifests        |
| `wasmtime`                                   | in-process Wasm execution for resident `.holo` archives           |
| `sha2`                                       | sha256 pinning of third-party plugin executables                  |
| `notify` (`hologram-application-watch`)      | portable source-project filesystem observation and debounce       |
| `hologram-client` (workspace crate)          | standalone typed HTTP client for the object and file API          |
| Tauri (`apps/desktop`)                       | desktop shell and managed `hologram` sidecar                      |
| Cucumber (development only)                  | executable Gherkin public-boundary scenarios                      |

The Rust daemon does not include an ORM, OIDC/SAML provider, dynamic native plugin loader, or multiple native RPC codecs. Kameo is deliberately process-local; gRPC is the network boundary. Third-party plugin modules run as separate subprocesses speaking gRPC over a Unix socket rather than as loaded native code.

`hologram-client` is deliberately standalone: it mirrors the wire shapes rather than importing them from `hologram-live`, because depending on the daemon would pull `wasmtime`, `axum`, and `tonic` into every consumer's build for the sake of a handful of JSON structures. Its only dependencies are `reqwest`, `rustls`, `serde`, and `serde_json`. The daemon takes it as a dev-dependency so a contract test can prove the mirrored types still agree; that keeps it out of the server's normal dependency graph, which the product-boundary gate checks.

Tauri is isolated in `apps/desktop`, and Astro is isolated in `apps/docs`. Neither is part of the server's Cargo dependency graph. The small `hologram-application-watch` workspace crate is Tauri-independent and injected into the desktop adapter; the standalone `hologram-live` server package does not depend on it.

Kappa Registry remains an external service/project. Hologram Live now ships an adapter for it behind the registry provider boundary, speaking its OCI blob and manifest surface over HTTP with the `reqwest` client already in this graph. None of its workspace crates enter this dependency graph, and selecting the adapter is opt-in configuration: the default install stays self-contained.
