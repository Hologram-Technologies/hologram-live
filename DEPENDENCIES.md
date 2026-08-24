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
| `kameo`                                      | bounded actors, links, and supervision                            |
| `tracing`, `tracing-subscriber`              | structured local diagnostics and runtime filtering                |
| `opentelemetry*`, `tracing-opentelemetry`    | OTLP/gRPC trace and metric export                                 |
| `utoipa`                                     | OpenAPI generation for the JSON API                               |
| `scalar_api_reference`                       | self-hosted interactive OpenAPI reference                         |
| `blake3`                                     | content addressing and update integrity                           |
| `uor-hologram` (`archive` only)              | canonical `.holo` archive parsing/writing                         |
| Tauri (`apps/desktop`)                       | desktop shell and managed `hologram` sidecar                      |

The Rust daemon does not include an ORM, OIDC/SAML provider, dynamic native plugin loader, or multiple native RPC codecs. Kameo is deliberately process-local; gRPC is the network boundary.

Tauri is isolated in `apps/desktop`, and Astro is isolated in `apps/docs`. Neither is part of the daemon's Cargo dependency graph.
