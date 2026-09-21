# Research: facts the plan stands on

2026-09-21. Every row was checked in this session unless marked otherwise.

Marks: **[read]** opened the file today. **[ran]** ran a command today. **[memory]** recalled from the registry's source and documents, not fetched today; gate B is the authority. **[assumption]** not checked.

Repositories read:
- Hologram Live: `Hologram-Technologies/hologram-live`, main @ `b359ff1`. Paths below are relative to it.
- Kappa Registry: clone @ `2af8656`, paths start with `kappa:`.

Nothing was built. Free disk was 11 GB [ran: `df`], below what a build of either tree needs.

## Hologram Live

| # | Fact | Where | Mark |
|---|---|---|---|
| R1 | `LiveModule` has four methods: `descriptor`, `router`, `openapi`, `start`. No hook for authentication | `src/module.rs:55-66` | read |
| R2 | `ModuleRegistry::router` merges every enabled module's router into one | `src/module.rs:157-161` | read |
| R3 | That one router is wrapped in the bearer `authenticate` layer. The same layer creates the request id and the tracing span | `src/server.rs:48-50`, `168-191` | read |
| R4 | Auth failures answer `ApiError` JSON (`LIVE_*` codes), 401 or 403 | `src/server.rs:181-189` | read |
| R5 | `assemble` merges HTTP and gRPC, sets `.fallback(no_route)`, then layers `DefaultBodyLimit::max(max_http_body_bytes)` over everything | `src/server.rs:85-89` | read |
| R6 | tonic's router carries its own catch-all; `no_route` exists because of it. A module router that sets its own fallback would collide at merge | `src/server.rs:91-113` | read |
| R7 | Default body limit 32 MiB; default listen `127.0.0.1:11435` | `src/config.rs:316-318` | read |
| R8 | `DefaultBodyLimit` binds only extractors built on `Bytes` (`String`, `Json`, `Form`). A handler that takes `Request` and reads the body as a stream is not limited | `~/.cargo/registry/.../axum-core-0.5.6/src/extract/default_body_limit.rs:8-19` | read |
| R9 | `validate()` refuses a non-loopback `server.listen` unless `auth.required = true` | `src/config.rs:574-578` | read |
| R10 | With `auth.required`, HTTP modules and gRPC both demand the bearer token | `src/server.rs:193-215`, `src/grpc.rs:84-107` | read |
| R11 | gRPC is one unary method, `Call(RpcRequest) returns (RpcResponse)`. There is no streaming RPC | `proto/hologram/live/v1/live.proto:5-6` | read |
| R12 | gRPC shares the HTTP port | `src/server.rs:51-61` | read |
| R13 | The server is started with `axum::serve(listener, router)`. No TLS anywhere | `src/server.rs:63-74` | read |
| R14 | `builtin_modules!` generates `builtins()` and `default_builtin_ids()` from one list. Ten modules today. A test pins "default enabled = whole catalogue" | `src/modules/mod.rs:13-41`, `src/config.rs:371`, `:922` | read |
| R15 | Module id `dev.hologram.live.kappa-registry` belongs to `registry::KappaRegistryModule` (the objects API) | `src/modules/mod.rs:32` | read |
| R16 | `hologram registry` is already a CLI subcommand, and `src/registry/` holds the provider code (`kappa.rs`, `kappa_client.rs`, `local.rs`) | `src/cli/mod.rs:86`, `src/registry/` | read |
| R17 | `AppState::build` opens the object store, history, models, inference engine, node directory, audit log and plugins in every mode. There is no slim path | `src/app.rs:51-137` | read |
| R18 | All four path roots default under `$HOME` | `src/config.rs:300-309` | read |
| R19 | Environment overrides that exist: `HOLOGRAM_LISTEN`, `HOLOGRAM_DATA_DIR`, `HOLOGRAM_STATE_DIR`, `HOLOGRAM_CACHE_DIR`, `HOLOGRAM_CONFIG_DIR`, `HOLOGRAM_AUTH_TOKEN`, `HOLOGRAM_LOG`, `HOLOGRAM_MAX_RPC_BYTES`, `HOLOGRAM_REMOTE_ENDPOINT` | `src/config.rs:341-780` | read |
| R20 | `main` parses one clap `Cli`; there is no `argv[0]` dispatch | `src/main.rs:9-23` | read |
| R21 | Toolchain 1.97.1; `rust-version = "1.95"` | `rust-toolchain.toml`, `Cargo.toml:5` | read |
| R22 | Direct dependencies already present and useful here: `axum 0.8` (resolved 0.8.9), `hyper-util 0.1` (feature `tokio` only), `rustls 0.23` with `ring`, `sha2`, `blake3`, `tokio-stream`, `tower`, `tempfile`, `zstd 0.13` | `Cargo.toml:29-66` | read |
| R23 | In `Cargo.lock` transitively, not direct: `tokio-rustls 0.26.4`, `tokio-util 0.7.19`, `http-body-util 0.1.5`, `httpdate 1.0.3`, `base64`. Absent: `redb`, `bcrypt`, `lzma-sys`, `bzip2-sys`, any YAML parser | `Cargo.lock` | read |
| R24 | The tree is not pure Rust today: `cc 1.4.4`, `ring 0.17.14`, `zstd-sys 2.0.16` are locked. `DEPENDENCIES.md:21` still says "pure Rust" | `Cargo.lock`, `DEPENDENCIES.md:21` | read |
| R25 | The rule that no Kappa crate enters the graph is the last paragraph of `DEPENDENCIES.md` | `DEPENDENCIES.md:35` | read |
| R26 | `just verify` = fmt, file-size, product-boundary, check, test (one thread), clippy `-D warnings`, bdd, build, smoke | `justfile:95-96` | read |
| R27 | File-size gate: 1500 lines, counted up to the first `#[cfg(test)]`; `tests/` is exempt; `.md`, `.yml`, `.sh`, `.toml` are counted whole | `scripts/check-file-size.sh:4-26` | read |
| R28 | CI is one job on `ubuntu-24.04`. No macOS, no Windows | `.github/workflows/ci.yml:11-27` | read |
| R29 | `release-server.yml` has five targets (linux gnu x86_64 and aarch64, macOS both, Windows MSVC). No musl. No image. It has never run (no tag exists) | `.github/workflows/release-server.yml:39-55` | read |
| R30 | The two carried Kappa patches live as inline edits in `scripts/check-kappa-registry.sh`, pinned to `2af86560…` | `scripts/check-kappa-registry.sh:7-14` | read |

## The hub today

| # | Fact | Where | Mark |
|---|---|---|---|
| H1 | Service `kappa` runs a mounted `kappa-server` binary in `debian:bookworm-slim`, port 5000, store at `./store`, anonymous, writes gated at Caddy, blob cap 1.1 GiB | `apps/model-hub/deploy/docker-compose.yml:6-26` | read |
| H2 | Service `hologram` depends on it and talks to it through the `kappa` provider | same file `:41-60` | read |
| H3 | That provider addresses blobs by **blake3** under `/v2/`: `GET`, `HEAD` and a direct `PUT /v2/<repo>/blobs/blake3:<hex>`. The direct `PUT` is not a registry route | `src/registry/kappa_client.rs:93-135`, `:298-335` | read |
| H4 | Each object is one blob plus one manifest whose `subject` points at the blob, tagged `blake3_<hex>` | `docs/superpowers/plans/2026-09-15-kappa-registry-provider.md:7`, `kappa_client.rs:34-51` | read |
| H5 | So the hub's store holds blake3-primary blobs with tags and no repository links | H3, H4, K6 | read |

## Kappa store

| # | Fact | Where | Mark |
|---|---|---|---|
| K1 | Upload API: `upload_begin(&NamespaceRef, max_size: u64) -> Result<String>`, `upload_put_part(&str, offset: u64, &[u8]) -> Result<u64>`, `upload_complete(&str, Option<&str>) -> Result<IngestResult>`, `upload_abort`, `upload_bytes_received -> Option<u64>`, `upload_evict_expired(secs) -> usize` | `kappa:crates/kappa-core/src/store/mod.rs:248-303` | read |
| K2 | `upload_put_part` demands `offset == bytes so far`; else `Rejected` | `kappa:crates/kappa-store-redb/src/lib.rs:765-772` | read |
| K3 | Each `upload_put_part` call reopens the staging file, computes MD5, CRC32C and CRC64 over the part, and pushes one `PartDigests` record into memory | `kappa:…/lib.rs:779-806` | read |
| K4 | `upload_complete` re-reads the staging file to hash it, then renames it into place and syncs the **parent directory** only. The file's own data is not synced | `kappa:…/lib.rs:843`, `:964-968` | read |
| K5 | Upload sessions are an in-memory `Mutex<HashMap>`. Staging files are deleted at open | `kappa:…/lib.rs:76`, `:200-215` | read |
| K6 | Default `mandatory_axes` is `[Sha256]`. A blake3 upload gets a sha256 hard link. A sha256 upload gets **no** blake3 address | `kappa:…/store/mod.rs:104-106`, `…/lib.rs:971-981` | read |
| K7 | Hard link errors are discarded (`let _ = std::fs::hard_link`) | `kappa:…/lib.rs:400`, `:436`, `:979` | read |
| K8 | `blob_open` returns `Box<dyn BlobReader>`; `BlobReader: Read + Seek + Send`, implemented for `std::fs::File` | `kappa:…/store/mod.rs:16-19`, `:374` | read |
| K9 | `ingest_verified`, `ingest_compute`, `blob_get`, `blob_get_range` hold whole content in a `Vec<u8>` | `kappa:…/store/mod.rs:117-192` | read |
| K10 | Store constructor: `PersistentStore::new(PersistentStoreConfig::new(blob_root, db_path), Arc<dyn Clock>)`. `Clock` has one method, `now_ms`. The only implementation is `NtpLamportClock::new()` | `kappa:…/lib.rs:100-135`, `kappa:crates/kappa-core/src/clock/` | read |
| K11 | `Database::create` (redb 4) takes an exclusive file lock: a second process cannot open the same store | `kappa:…/lib.rs:135` | read; lock behaviour [assumption] from redb documentation |
| K12 | Repositories map to namespaces: `namespace_resolve_or_create(name, owner, protocol)`, `namespace_list(protocol)`. Tags are per namespace: `tag_set`, `tag_get`, `tag_delete`, `tag_list`, `tag_prefix` | `kappa:…/store/mod.rs:207-212`, `:401-490` | read |
| K13 | Per-blob and per-namespace metadata exist: `blob_put_meta`, `meta_set`, `meta_query`, `edge_put`, `edge_query`. Each call is its own transaction | `kappa:…/store/mod.rs:196-229` | read |
| K14 | `kappa-core` dependencies that matter: `dcbor` (git branch of a personal fork), `rekindle-aead` (git branch of a personal fork), `zstd`, `xz2`, `bzip2`, `frost-*`, `ed25519-dalek`, `p256`, `k256`, `chrono`, `uuid`, `dashmap`, `async-trait`. `kappa-store-redb` adds `redb 4`, `base64-simd`, `crc-fast`, `md-5`, `serde_json`. Neither names `topcoat` | `kappa:crates/kappa-core/Cargo.toml`, `kappa:crates/kappa-store-redb/Cargo.toml`, `kappa:Cargo.toml:30`, `:57`, `:72` | read |
| K15 | Both crates inherit `version`, `edition`, `rust-version`, `license`, `publish` and most dependencies from the workspace root. Cargo must parse that root to use them, and the root does not parse as published (the reason for carried patch 1) | same manifests; `scripts/check-kappa-registry.sh:7-12` | read; "cargo refuses" [assumption] until P0 Task 1 step 2 runs it |

## Registry reference

All rows are **[memory]**. The plan never relies on one without naming the gate B scenario that pins it.

| # | Fact |
|---|---|
| D1 | Image `registry:3`: `ENTRYPOINT ["registry"]`, `CMD ["serve", "/etc/distribution/config.yml"]`, `EXPOSE 5000`, `VOLUME /var/lib/registry` |
| D2 | Its default `config.yml` sets `log.fields.service`, `storage.cache.blobdescriptor: inmemory`, `storage.filesystem.rootdirectory`, `http.addr: :5000`, `http.headers.X-Content-Type-Options`, `health.storagedriver` |
| D3 | Environment rule: `REGISTRY_` + the key path in upper case, levels joined by `_` |
| D4 | 18 documented error codes and their statuses: see `contracts/registry-api.md` |
| D5 | Manifest body cap 4 MiB. Delete is off unless `storage.delete.enabled`. `garbage-collect [--dry-run] [--delete-untagged] <config>` is documented as an offline or read-only operation, but nothing stops an operator running it live |
| D6 | htpasswd accepts bcrypt entries only. In v3, a missing htpasswd file is created with a random password that is logged once (to be confirmed: scenario `auth-missing-htpasswd`) |

## What would change the phase order if false

| Assumption | If false |
|---|---|
| The two Kappa crates build inside this binary on three systems (P0) | No-go. Everything after P0 is re-planned around `kappa-server` as a second process |
| `upload_put_part` writes are safe to call from `spawn_blocking` at 4 MiB frames with flat memory (P0 Task 2) | Patch K3 moves from "later" to the first task of P1 |
| redb's lock makes a second opener fail fast, not hang (K11) | P8 garbage collection needs its own lock file before anything else |
| `registry:3` answers as its documents say | Gate B finds it in week 1, because P2 runs before the code it judges |
