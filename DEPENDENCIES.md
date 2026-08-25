# Dependency budget

The project uses one primary dependency per responsibility and keeps desktop and documentation dependencies outside the daemon crate.

| Crate or application                         | Responsibility                                                    |
| -------------------------------------------- | ----------------------------------------------------------------- |
| `tokio`                                      | async runtime, sockets, signals, and process control              |
| `axum`                                       | browser-facing JSON/HTTP routes and shared HTTP serving           |
| `tonic`, `prost`                             | native Protobuf/gRPC API and client                               |
| `reqwest`                                    | verified update downloads using Rustls                            |
| `serde`, `serde_json`, `toml`                | typed configuration and public JSON                               |
| `clap`                                       | CLI parsing                                                       |
| `fs4`                                        | cross-platform daemon ownership lock                              |
| `kameo`                                      | bounded actors, links, and supervision                            |
| `tracing`, `tracing-subscriber`              | structured local diagnostics and runtime filtering                |
| `opentelemetry*`, `tracing-opentelemetry`    | OTLP/gRPC trace and metric export                                 |
| `utoipa`                                     | OpenAPI generation for the JSON API                               |
| `scalar_api_reference`                       | self-hosted interactive OpenAPI reference                         |
| `blake3`                                     | content addressing and update integrity                           |
| `uor-hologram` (`archive`, `space`)          | canonical `.holo` archives and application manifests              |
| `wasmtime`                                   | in-process Wasm execution for resident `.holo` archives           |
| `sha2`                                       | sha256 pinning of third-party plugin executables                  |
| Tauri (`apps/desktop`)                       | desktop shell and managed `hologram` sidecar                      |
| Cucumber (development only)                  | executable Gherkin public-boundary scenarios                      |

The Rust daemon does not include an ORM, OIDC/SAML provider, dynamic native plugin loader, or multiple native RPC codecs. Kameo is deliberately process-local; gRPC is the network boundary. Third-party plugin modules run as separate subprocesses speaking gRPC over a Unix socket rather than as loaded native code.

Tauri is isolated in `apps/desktop`, and Astro is isolated in `apps/docs`. Neither is part of the daemon's Cargo dependency graph.

Kappa Registry remains an external service/project. Hologram Live integrates it through the registry provider boundary rather than adding its workspace crates to this dependency graph.
