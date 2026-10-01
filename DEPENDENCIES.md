# Dependency budget

The project uses one primary dependency per responsibility and keeps desktop and documentation dependencies outside the daemon crate.

| Crate or application                         | Responsibility                                                    |
| -------------------------------------------- | ----------------------------------------------------------------- |
| `tokio`                                      | async runtime, sockets, signals, and process control              |
| `axum`                                       | browser-facing JSON/HTTP routes and shared HTTP serving           |
| `tonic`, `prost`                             | native Protobuf/gRPC API and client                               |
| `reqwest`                                    | outbound HTTP over Rustls: verified update downloads, Ollama/vLLM inference, and mediated Component fetch |
| `serde`, `serde_json`, `toml`                | typed configuration and public JSON                               |
| `clap`                                       | CLI parsing                                                       |
| `async-trait`                                | `dyn`-dispatched async for the pluggable cluster network trait |
| `fs4`                                        | cross-platform daemon ownership lock                              |
| `kameo`                                      | bounded actors, links, and supervision                            |
| `tracing`, `tracing-subscriber`              | structured local diagnostics and runtime filtering                |
| `opentelemetry*`, `tracing-opentelemetry`    | OTLP/gRPC trace and metric export                                 |
| `utoipa`                                     | OpenAPI generation for the JSON API                               |
| `scalar_api_reference`                       | self-hosted interactive OpenAPI reference                         |
| `rustls`                                     | explicit ring crypto provider for reqwest, keeping the build pure Rust |
| `blake3`                                     | content addressing and update integrity                           |
| `ed25519-dalek`                              | per-node cluster identity: the public key is the node id and signs every cluster request |
| `uor-hologram` (`archive`, `space`)          | canonical v2–v4 `.holo` archives and application manifests        |
| `wasmtime`                                   | in-process Wasm execution for resident `.holo` archives           |
| `sha2`                                       | sha256 pinning of third-party plugin executables                  |
| `notify` (`hologram-application-watch`)      | portable source-project filesystem observation and debounce       |
| `hologram-client` (workspace crate)          | standalone typed HTTP client for the object and file API          |
| Tauri (`apps/desktop`)                       | desktop shell and managed `hologram` sidecar                      |
| Cucumber (development only)                  | executable Gherkin public-boundary scenarios                      |

The Rust daemon does not include an ORM, OIDC/SAML provider, dynamic native plugin loader, or multiple native RPC codecs. Kameo is deliberately process-local; gRPC is the network boundary. Third-party plugin modules run as separate subprocesses speaking gRPC over a Unix socket rather than as loaded native code. The explicit inference exceptions are the off-by-default `llamacpp`, `candle`, and `burn` features recorded in ADRs 033 and 034.

`hologram-client` is deliberately standalone: it mirrors the wire shapes rather than importing them from `hologram-live`, because depending on the daemon would pull `wasmtime`, `axum`, and `tonic` into every consumer's build for the sake of a handful of JSON structures. Its only dependencies are `reqwest`, `rustls`, `serde`, and `serde_json`. The daemon takes it as a dev-dependency so a contract test can prove the mirrored types still agree; that keeps it out of the server's normal dependency graph, which the product-boundary gate checks.

Tauri is isolated in `apps/desktop`, and Astro is isolated in `apps/docs`. Neither is part of the server's Cargo dependency graph. The small `hologram-application-watch` workspace crate is Tauri-independent and injected into the desktop adapter; the standalone `hologram-live` server package does not depend on it.

## Optional: llama.cpp (`--features llamacpp`)

| Dependency | Purpose |
| --- | --- |
| `llama-cpp-2` | in-process GGUF model loading, tokenization, sampling, and decode |
| `encoding_rs` | stateful UTF-8 assembly across token-piece boundaries |

The feature requires CMake, Clang, and a C++ compiler. `llamacpp-metal` and `llamacpp-cuda` select the corresponding native GPU backend; the default build resolves neither dependency.

## Optional: Candle (`--features candle`)

| Dependency | Purpose |
| --- | --- |
| `candle-core`, `candle-transformers` | in-process quantized Llama GGUF tensors, sampling, KV cache, and CPU/GPU kernels |
| `tokenizers` | load the model's explicit `tokenizer.json` and encode/decode exact token ids |

`candle-metal` and `candle-cuda` opt into their respective GPU kernels. The base feature is CPU-only; Candle's tokenizer dependency currently enables Oniguruma through Candle's own feature selection, so it is Rust-native inference rather than a strict no-native-code build. This adapter supports only the explicitly documented Llama-family implementation; adding a Candle crate does not make every Candle example or GGUF architecture a server capability.

## Optional: Burn (`--features burn`)

| Dependency | Purpose |
| --- | --- |
| `burn` | CPU tensor backend used by the initial adapter |
| `burn-lm-llama`, `burn-lm-inference` | Tracel's Llama 3 model, tokenizer, sampling, and generated-text collector |

The initial backend is CPU-only and consumes Burn named-MPK checkpoints rather than GGUF or raw Safetensors. Burn-LM currently pins Burn 0.18, so this optional graph remains isolated from the default binary and should be upgraded with Burn-LM rather than mixing model/runtime versions locally. Burn-LM's published inference module requires its `pretrained` feature and therefore resolves its download client and cache-directory dependencies; Hologram calls only the local `load_*` APIs and performs no model download.

The optional Burn graph includes unmodified `colored` and `option-ext` source files under MPL-2.0. `deny.toml` records exceptions for exactly those two crates; MPL-2.0 is not admitted globally, and the default build resolves neither dependency.

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
| `tokio-rustls` | TLS on the registry's listener (`http.tls`, `src/tls.rs`), over the `rustls` this server already uses, with ring only (`default-features = false`), so aws-lc stays out of the graph. Already in the graph through `reqwest`. PEM is read with `rustls::pki_types`, so `rustls-pemfile` (deprecated) is not needed |

The Kappa Registry provider (ADR 021) still speaks to an external `kappa-server` over HTTP with `reqwest`; it uses none
of these crates. With `oci` on, the build gains two bundled C libraries (`lzma-sys`, `bzip2-sys`) through `kappa-core`.
`kappa-core` needs `dcbor`, vendored in `third_party/dcbor` (BSD-2-Clause-Patent). `aws-lc` stays out: the copy
carries a patch that turns the store's encryption backend off, and `scripts/check-kappa-pin.sh` fails if it returns.

## Optional: the iroh cluster transport (`--features p2p`)

Off by default (ADRs 033/034); a stock build resolves none of these. Phase 2a of #179: a second
`ClusterNetwork` dialled by public key, so a node behind NAT joins without a routable address. The
design is `docs/superpowers/specs/2026-09-25-iroh-transport-design.md`.

| Dependency | Purpose |
| --- | --- |
| `iroh` (`default-features = false`, `tls-ring`) | QUIC connections dialled by public key; hole punching and optional relays. ring only: aws-lc and openssl stay out of the graph, and `scripts/check-kappa-pin.sh` runs its forbidden-crate tree check over `--features oci,p2p` |
| `hyper` | HTTP/1 over the iroh duplex, server and client; already in the graph through axum |
| `http-body-util` | request/response bodies over the iroh duplex; already locked through axum |

`uor-prism-crypto` is patched to `Hologram-Technologies/prism` branch `relax-blake3-pin`: the
published 0.4.0 pins blake3 to `>=1.5, <1.6` for its own MSRV, iroh needs `^1.8.3`, and Cargo will
not hold two semver-compatible blake3 versions. The patch changes the requirement only, no code.
Offered upstream as UOR-Foundation/prism#3; drop the patch when that lands in a release.

`dlopen2` appears in the lockfile through this tree and is never compiled — it is not on a normal
build edge, and the daemon still has no dynamic native plugin loader.

Phase 2b of #179 adds object replication over `iroh-blobs` (design:
`docs/superpowers/specs/2026-10-01-iroh-blobs-replication-design.md`): BLAKE3/bao verified,
resumable streaming between key-addressed peers, with the registry's `blake3:` ids doubling as
blob hashes, so no translation table sits between them.

| Dependency | Purpose |
| --- | --- |
| `iroh-blobs` (`default-features = false`, `fs-store`) | verified, resumable object transfer, the per-node staging store, and its provider protocol; excluding the default `rpc` feature keeps the `noq` endpoint-setup subtree out of the graph |
| `tokio-util` | `SyncIoBridge` drives the store's async reader from the registry's synchronous streaming put; already locked through the `oci` feature, so it adds no crate |

Measured from the real graph, as the design requires: name+version pairs in `Cargo.lock` after
resolving `--features p2p` versus `git show main:Cargo.lock` — 22 net-new packages (985 → 1007),
the whole cost of this phase, since Phase 2a's tree was already merged. `redb` comes in through
`fs-store` but was already locked for the `oci` feature and adds no package. `redb` and
`reflink-copy` are pure Rust; their licenses were read from the resolved crates (`redb` 4.1.0 is
`MIT OR Apache-2.0`, `reflink-copy` 0.1.30 is `MIT/Apache-2.0`), as the design recorded.
`scripts/check-kappa-pin.sh` runs its forbidden-crate tree check over `--features oci,p2p`, and
with the new store stack in that tree it still finds no aws-lc, openssl, veilid, topcoat or
rekindle.
