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

## Optional: the registry (`--features oci`)

Off by default. A stock build pulls none of these; `scripts/check-product-boundaries.sh` holds that line. ADR 025 is
the decision to use them and ADR 032 the decision to vendor them; `third_party/kappa/README.md` is the audited record
of the copy and its carried patches.

| Dependency | Purpose |
| --- | --- |
| `kappa-core` (vendored in `third_party/kappa`, `default-features = false`) | the `KappaStore` trait: blobs addressed by `sha256:` and `blake3:`, staged uploads, tags |
| `kappa-store-redb` (vendored beside it) | the store: blob files on disk, an index in one redb file |
| `redb` | `links.redb`, the registry's own database: repository links, referrers, aliases, upload records (ADR 027) |
| `yaml-rust2` | reads the reference registry's `config.yml` into a flat key table (`src/registry_compat`). Chosen by three checks: a release in the last six months (0.13.0, September 2026), no `-sys` crate, and it parses the reference image's own default file. Rejected: `serde_norway` and `serde_yaml_ng` (no release in six months), `serde-saphyr` (typed deserialisation, which a key table does not need) |
| `bcrypt` | checks and writes `auth.htpasswd` entries (`src/modules/oci/auth.rs`), as the reference's `golang.org/x/crypto/bcrypt` does. Pure Rust (`blowfish`, `cipher`, `inout` come with it, no `-sys` crate); reads `$2y$`, what `htpasswd -B` writes, and `$2a$`, `$2b$` |
| `getrandom` | the random password of a provisioned password file, and the per-process key of the login cache. Already in the graph through `bcrypt` |
| `base64` | HTTP Basic credentials, and the provisioned password in the reference's URL-safe form. Already in the graph through `bcrypt` |

The Kappa Registry provider (ADR 021) still speaks to an external `kappa-server` over HTTP with `reqwest`; it uses none
of these crates. With `oci` on, the build gains two bundled C libraries (`lzma-sys`, `bzip2-sys`) through `kappa-core`.
`kappa-core` needs `dcbor`, vendored in `third_party/dcbor` (BSD-2-Clause-Patent). `aws-lc` stays out: the copy
carries a patch that turns the store's encryption backend off, and `scripts/check-kappa-pin.sh` fails if it returns.
