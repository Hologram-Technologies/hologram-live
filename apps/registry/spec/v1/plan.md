# Hologram Registry v1: master implementation plan (revision 3)

## 0. Revision 3 (2026-09-21): built in the parent repository, as the app `registry`

Decided by the maintainer. Revision 2 is kept beside this folder as `001-hologram-registry-v1.rev2-backup`. Everything below this section is revision 2 with paths and targets changed; where the two disagree, this section wins.

**Where it is built.** In `Hologram-Technologies/hologram-live`, not in a long-lived fork. Branches are pushed to a development fork (a true GitHub fork of the parent) and every pull request targets the parent's `main`. There is no weekly merge of `upstream/main`: a branch is rebased on the parent's `main` before its pull request, and lives days, not weeks.

**The split.** `apps/` holds products built around the `hologram` binary (`apps/desktop`, `apps/model-hub`, `apps/docs`); the server lives in `src/`. The registry follows that line.

| Part | Where | Why |
|---|---|---|
| The `/v2/` module, the store adapter, settings compatibility, TLS, the `hologram oci` commands | `src/modules/oci/`, `src/oci_store/`, `src/registry_compat/`, `src/tls.rs`, `src/cli/oci.rs` | It is a server module like the other ten and reuses the module host, config, tracing, gRPC and CLI. The step after v1 (one store for images, models and objects) needs it in the same process |
| The product | `apps/registry/` | Same place as the other products |

`apps/registry/` layout:

```
apps/registry/
  README.md                 what it is, the three commands, status
  Dockerfile                built from the repository root:  docker build -f apps/registry/Dockerfile .
  Dockerfile.dockerignore   BuildKit reads this beside the Dockerfile; the root needs no .dockerignore
  config.yml                the image's default, equal to the reference's
  DIFFERENCES.md            every kept difference from registry:3 (gate B's allowlist)
  chart/                    Helm chart
  docs/                     "Coming from Docker Registry", migration, settings table (generated)
  gates/                    differential/ (its own crate) · clients/ · corpus/ · compose/ · release/
```

Workflows stay in `.github/workflows/` (GitHub reads them nowhere else): `gates.yml`, `gates-nightly.yml`, `registry-os.yml`.

**Rules that exist because this is the parent repository.**

1. **The feature is off by default.** `oci = ["dep:kappa-core", "dep:kappa-store-redb", "dep:redb"]`, and `default` does not name it. A stock `cargo build` of Hologram Live pulls no Kappa crate, no new C library, and behaves as today. The registry image, the release binaries, and every `oci` test build with `--features oci`. This replaces revision 2's "feature on by default" and its `no-oci` recipe; the recipe becomes `oci-check: cargo check --locked --features oci`, added to `just verify`.
2. **The product boundary gate learns the rule.** `scripts/check-product-boundaries.sh` gains a second check: the default server graph (`cargo tree -p hologram-live -e normal`) must not contain `kappa-core`, `kappa-store-redb`, `lzma-sys` or `bzip2-sys`. Written in P0 T1, with the pin gate.
3. **The registry's CI does not tax the rest of the repository.** `gates.yml` and `registry-os.yml` run only when a pull request touches `src/modules/oci/**`, `src/oci_store/**`, `src/registry_compat/**`, `src/tls.rs`, `apps/registry/**`, `Cargo.toml`, `Cargo.lock`, or the workflow files. `ci.yml` gains one job, `oci-check`, and nothing else.
4. **Shared files change by small pull requests of their own, first.** The `LiveModule::authenticates_itself()` hook, the second module list, the `argv[0]` rewrite and the TLS accept loop each go to the parent as a standalone pull request with its own test, before the registry code that needs it. About 170 lines in 12 shared files (section 10); none of them changes default behaviour.
5. **Nothing merges to the parent before the P0 verdict.** The spike stays on `registry/p0-spike` in the fork. ADR-025 (Kappa crates in the dependency graph, reversing the last paragraph of `DEPENDENCIES.md`) is the first pull request to the parent, and the other maintainer's agreement to it is the entry condition for P1.
6. **Names.** Image `ghcr.io/hologram-technologies/registry`. **One version, one tag: `server-v<version>`** (decided by the maintainer, 21 September, recorded on the Server roadmap): the same tag publishes the binaries, the registry image, the server image and the chart. The release candidate is `server-v1.0.0-rc.1`, the release `server-v1.0.0`. There is no `registry-v*` tag. Module id `dev.hologram.live.oci`. Command group `hologram oci`; the image answers to `registry` through `argv[0]`.
7. **Where the plan lives.** On acceptance: `plan.md` and `phases/` go to `docs/superpowers/plans/2026-09-21-registry-*.md` and `spec.md`, the contracts and `data-model.md` to `docs/superpowers/specs/`, by pull request to the parent. `apps/registry/README.md` links to them.

**P0 outcome (2026-09-21): GO on one condition.** Verdict: `p0-verdict.md` (and `docs/superpowers/specs/2026-09-21-registry-p0-verdict.md` on `registry/p0-spike`). The plain upstream git dependency builds as published, so P0 T1 steps 3 and 4 were not needed and the Kappa fork moves to P1 T6. Linux, macOS and Windows pass the store tests; 2 GiB streams in 8.8 MB. The condition: `rekindle-aead` pulls `aws-lc`, which broke a clean Windows runner; carried patch A removes it and gates every merge into the parent. Dropped from the plan: carried patches 0001 (manifest `[[test]]`) and 0005 (S3 part digests), and the own-lock-file risk (a second opener fails fast on all three systems). ADR 025 is draft pull request #90 on the parent.

8. **Eight gates, not five.** A release is blocked on A to E (this plan: conformance, comparison with the reference, unchanged clients, peer round trips, peers pulling from us) **and** F, G, H from the Server specification (`apps/registry/spec/operability`: form factor, operations, OpenAPI). Where this plan or `spec.md` says "five gates", read eight.

**New decision for the maintainer (8).** The other maintainer must agree to three things before P1: two optional git dependencies behind an off-by-default feature; two path-filtered workflows; four small hooks in shared files. Recommend: open ADR-025 as a draft pull request on day 1, in parallel with the spike, so the answer and the verdict arrive together.

---

# Revision 2 (paths and targets updated)

2026-09-21. Replaces draft 1 (`docs/superpowers/plans/2026-09-21-hologram-registry-v1.md` in the clone, left untouched). Made with Spec Kit: `spec.md` → clarify → checklists → this plan → `phases/` → `tasks.md`.

**Read in this order:** this file, then `phases/p0-spike.md`. Supporting: `research.md` (every fact, with file and line), `contracts/registry-api.md`, `contracts/config-table.md`, `data-model.md`.

**Depth:** P0, P1 and P2 are at step level (test code, code, gate). P3 to P10 give every task with files, interfaces, done-when and traps, and are marked "steps to be written before the phase starts".

Fact marks: R, H, K, D numbers point into `research.md`. **[assumption]** means not checked.

---

## 1. Audit of draft 1

Draft 1 got the shape right: one module, an adapter that hides Kappa, parsing from the right, gates as work, no cut list. It is kept. What follows is what was wrong, missing, out of order, would not compile, or could not fail.

### Wrong

| # | Draft 1 said | The code says | Fix |
|---|---|---|---|
| 1 | Task 7.2: `verify` runs "over gRPC, streaming progress" | gRPC is one unary method (R11). There is no stream | Two operations on the existing `Call`: `oci.verify.start` and `oci.verify.status`, polled by the CLI (P8 T3) |
| 2 | Task 2.3: "a module fallback turns any unclaimed `/v2/*` path into a registry 404" | A router with its own fallback collides with tonic's at merge (R5, R6). And `/v2/{*rest}` already claims every path | No fallback. The catch-all's parser answers the 404. `/v2` without the slash gets its own route (P3 T2) |
| 3 | Task 2.1: "`default_builtin_ids()` must filter it out" | One macro list feeds both functions, and a test pins "default = whole catalogue" (R14) | A second macro list, `opt_in_modules!`, read by `builtins()` only (P3 T1) |
| 4 | Global constraint: "never return a blake3 digest on `/v2/`" | The hub's own server reads and writes `blake3:` blobs through `/v2/`, with a direct blob `PUT` that is not a registry route (H3, H4). With draft 1's rule the hub cutover, which is the success test, cannot pass | `/v2/` accepts `blake3:` digests (new FR-022, a listed difference). The provider moves to the standard upload flow (P8 T5). Decision 2 for the maintainer |
| 5 | Task 1.4: the store makes a blake3 address next to sha256 by hard link | It does not. `mandatory_axes` is `[Sha256]`: a sha256 upload gets no blake3 address (K6). Hard link errors are thrown away (K7) | The adapter hashes blake3 in the same pass and keeps an `aliases` table. No hard links needed anywhere (`data-model.md`, ADR-030) |
| 6 | Task 0.1 step 2: try a plain `git`, `rev` dependency first | Both crates inherit from a workspace root that cargo cannot parse as published (K15). That attempt fails every time | First try is a fork that carries the patches as commits, pinned by `rev` (P0 T1) |
| 7 | Task 3.1: "these routes carry `DefaultBodyLimit::disable()`" | There is one catch-all route for uploads and manifests, so a per-route layer cannot tell them apart. And the limit binds only `Bytes`-based extractors (R8) | The handler takes `Request`, streams the body, and enforces 4 MiB on manifests itself. A test proves the 32 MiB limit still binds `/api/v1/objects` (P4 T1) |
| 8 | Task 6.3: registry mode listens on `:5000`, anonymous, like the reference | `validate()` refuses a non-loopback listen without `auth.required` (R9). With it on, system routes and gRPC want a bearer token (R10), and gRPC sits on the public port (R12) | Registry mode generates a token at start, keeps the system module behind it, and serves gRPC on loopback only (new FR-021, ADR-028, P5 T3) |
| 9 | Task 7.1: garbage collection "takes the store's process lock" | There is no such lock to take. redb locks the database file, so a second opener fails (K11) | Use that: translate the open failure into "the registry is running" (P8 T1). P0 checks it fails fast and does not hang |
| 10 | Task 5.2: TLS by adding `tokio-rustls` | `axum::serve` has no TLS (R13). Its `Listener` hook would run each handshake inside `accept`, one at a time | Own accept loop on `hyper-util`, handshakes in their own tasks (P6 T2). `tokio-rustls` and `hyper-util` are already locked (R22, R23) |
| 11 | Name grammar separator `[._-]` | The reference allows `.`, `_`, `__`, or a run of `-` | Fixed in `contracts/registry-api.md`, with four table cases |
| 12 | Spec: "one engineer", a cut order, "first to be cut" | Settled: two engineers, no cut list | `spec.md` updated |

### Would not compile as written

| Draft 1 interface | Real signature | 
|---|---|
| `upload_begin(repo)` | `upload_begin(&NamespaceRef, max_size: u64) -> Result<String, StoreError>` (K1). A repository must be resolved to a namespace first (K12) |
| `upload_finish(id, &Digest) -> Result<Digest>` over `upload_complete(id, digest)` | `upload_complete(&str, Option<&str>) -> Result<IngestResult, StoreError>` (K1) |
| `OciStore::open(data_dir)` | The store needs `PersistentStore::new(PersistentStoreConfig, Arc<dyn Clock>)` (K10). The adapter must own a clock |
| `BlobStream` "with a known length" from `blob_open` | `blob_open` returns `Box<dyn BlobReader>` with no length (K8). Length is a second call, `blob_size` |
| `repos() -> impl Iterator` stored through `meta_set` | `meta_query` returns `Vec<String>` of kappas, keyed by one namespace (K13). It cannot list repositories. Links move to our own database (ADR-027) |
| `src/cli/registry_compat.rs`, flag `--registry` | `hologram registry` is already a subcommand (R16). New commands go under `hologram oci`; the compose entrypoint is handled by `argv[0]` before clap (R20) |

### Missing

- A request id and tracing span for `/v2/`: today the bearer layer makes them (R3), and the module leaves that layer.
- Registry mode paths. `AppState::build` opens eight services in every mode (R17) and every path defaults under `$HOME` (R18). A container with no home directory fails at start.
- Concurrency rules and their tests. Crash table. Both now in `contracts/registry-api.md` and `data-model.md`.
- Blob data is not synced before rename in the store (K4). An upstream fix v1 needs; draft 1 did not list it.
- Per-part overhead in the store (K3): a file open, MD5 and two CRCs per call, one record kept per call. Frames must be re-cut to a fixed 4 MiB, not passed through as hyper delivers them (about 16 KiB).
- The hub store has no links (H5). `adopt` was discovered in draft 1's last milestone; it constrains the link design, so its design moves to P1.
- CI is Linux only (R28). "Three systems" needs new jobs, and says which block.
- ADR paths. They live in `specs/adrs/NNN-*.md`; next free number is 025.
- Success criteria were not traced. Section 8 traces all 22 FR and 10 SC.
- Schedule by week, CI timeline, runtime budget, secrets, weekly upstream merge, rollback. Sections 4, 5, 10, 11.

### Out of order

- Name, tag and digest types were owned by M2 but consumed by M1. They move to P1 T2.
- The harness was "M8, starts day 3". A phase that starts on day 3 is phase 2. It is now P2, and its exit test is "zero differences when the reference is compared with itself", which can fail and needs no product code.
- Login and TLS (M5) came before the config that names their settings (M6). Config is now P5, login and TLS P6.

### "Done when" that could not fail

| Draft 1 | Replaced by |
|---|---|
| M0: "the verdict is written" | The CI job is green on each system the verdict commits to, **and** `scripts/check-kappa-pin.sh` passes |
| 1.5: "pull requests open" | `scripts/check-kappa-pin.sh`: every file in `third_party/kappa/patches/` has an upstream link in `README.md`; the `rev` in `Cargo.lock` equals the one in `README.md` |
| 3.1: "a 5 GiB `docker push` succeeds with flat memory" | `tests/oci_memory.rs`: push 2 GiB through the router, sample RSS every 100 ms, fail over 200 MB. The 20 GB figure runs nightly in P9 T5 |
| M5: "on a second machine with the CA installed" | `apps/registry/gates/clients/tls.sh`: two containers on one Docker network, a throwaway CA, no `insecure-registries` |
| FR-019 proved by "`third_party/kappa/README.md`" | The pin script above, in `just verify` |
| 9.3: docs | `scripts/check-docs-settings.sh`: every key in the config table appears in "Coming from Docker Registry" with the same class |

---

## 2. Goal, architecture, stack

**Goal.** Ship Hologram Registry v1: a drop-in replacement for the Docker Registry image (`registry:3`), built on the Kappa Registry store. Take the compose file from the registry's documentation, change only the image name, and clients, commands, port 5000, `REGISTRY_*` settings, TLS, htpasswd login, delete-off-by-default and `garbage-collect` behave the same. Existing data migrates with one command.

**Architecture.**

```
            clients (docker, containerd, oras, skopeo, helm, cosign, ollama)
                                  │  HTTP or TLS, port 5000
┌─────────────────────────────────▼──────────────────────────────────────┐
│ src/server.rs   accept loop (plain or rustls) → one axum Router         │
│   ├─ system routes  ── bearer layer (unchanged)                         │
│   ├─ /v2/…  src/modules/oci/   own auth (htpasswd), own span,           │
│   │          own error shape, streams bodies     id dev.hologram.live.oci│
│   └─ gRPC  (registry mode: loopback listener only)                      │
├──────────────────────────────────────────────────────────────────────────┤
│ src/oci_store/   the only code that names a Kappa type                  │
│   types · layout · links (links.redb) · blobs · uploads · manifests     │
│   gc · verify · import · adopt                                          │
├──────────────────────────────────────────────────────────────────────────┤
│ kappa-core + kappa-store-redb   pinned by rev, patches carried          │
└──────────────────────────────────────────────────────────────────────────┘
```

`kappa-server` and `kappa-module-oci` are not used. The store's own garbage collection is never called.

**Stack.** Rust, toolchain 1.97.1 (R21). axum 0.8.9, tonic 0.14 (R22). Direct dependencies added, each with a row in `DEPENDENCIES.md` in the task that adds it:

| Crate | Why | Already locked | Phase |
|---|---|---|---|
| `kappa-core`, `kappa-store-redb` | the store | no | P0 |
| `redb` 4 | `links.redb` | comes with the store | P1 |
| `tokio-util` (`io`) | `ReaderStream` for blob bodies | yes, 0.7.19 (R23) | P3 |
| `http-body-util` | frame-by-frame body reads | yes (R23) | P4 |
| `bcrypt` | htpasswd | no. Pure Rust **[assumption]**, checked in P6 T1 step 1 | P6 |
| `tokio-rustls`, `rustls-pemfile` | TLS listener | `tokio-rustls` yes (R23) | P6 |
| one YAML parser, pure Rust | `config.yml` | no. Chosen in P5 T1 step 2 by three tests: maintained in the last 6 months, no C, parses the reference's default file | P5 |
| `hyper-util` features `server-auto`, `service` | accept loop | crate yes (R22) | P6 |

**Specs.** `spec.md` (4 stories, FR-001 to FR-022, SC-001 to SC-010), the brief (`Product/HOLOGRAM-REGISTRY-V1-BRIEF.pdf`), the two contracts, `data-model.md`.

---

## 3. Settled decisions and global constraints

Settled, not reopened: native `/v2/` module on the two Kappa crates; no in-place open of a reference volume; upstream pull requests plus carried patches from day one; v1 is a foundation (eight gates green, the hub runs on it); image first, binaries second, Helm third; gates A to E here, F to H from the Server specification; no cut list; garbage collection refuses beside a live server.

Still open: whether the Kappa crates can live inside this binary on three systems. P0 decides. Everything after P0 is written for "go".

Constraints on every commit:

- `just verify` stays green (R26). 1500 production lines per file (R27): `src/modules/oci/` and `src/oci_store/` are directories; each phase file says where a module splits.
- `--locked` everywhere. Clippy pedantic at `-D warnings`. `unsafe_code = "forbid"`.
- **No layer in memory.** The only request body read whole is a manifest, capped at 4 MiB. The only store calls that take a whole buffer (`ingest_verified`, `blob_get`, K9) are used for manifests only. A grep gate enforces it: `scripts/check-oci-streaming.sh` fails if `blob_get(`, `blob_get_range(` or `to_bytes(` appears under `src/modules/oci/` or `src/oci_store/` outside `manifests.rs`.
- **Registry shape under `/v2/`.** Never a `LIVE_*` body. Always `Docker-Distribution-API-Version: registry/2.0`.
- **The digest a client asked by is the digest it gets back.** sha256 clients never see blake3.
- **Hash on write (FR-020).** A mismatch is `DIGEST_INVALID` and leaves nothing reachable.
- **An unsupported setting stops the start and names the key (FR-011).**
- **Every kept difference is a line in `apps/registry/DIFFERENCES.md` in the commit that makes it (FR-013).** Gate B fails on an unlisted difference and on a listed one that no longer happens.
- **The default install does not change.** Outside registry mode the ten modules behave as today; existing routes, keys and tests stay.
- **No Kappa type outside `src/oci_store/`.** `scripts/check-oci-streaming.sh` also fails on `kappa_core` or `kappa_store_redb` anywhere else under `src/`.
- Branches `registry/p<N>-<topic>`, pushed to the fork a development fork; one pull request per task, squash merge, against **`Hologram-Technologies/hologram-live` main** (section 0). Rebase on the parent's `main` before opening it.
- **The `oci` feature is off by default** (section 0, rule 1). Every command in the phase files that builds or tests registry code carries `--features oci`.

---

## 4. Phase map

Days are engineer days **[assumption]**. "Closes" means the last task a requirement needs is in that phase.

| P | Name | Purpose in one line | Entry | Exit test (can fail) | Days | Closes | Unblocks |
|---|---|---|---|---|---|---|---|
| 0 | Spike | Decide go or no-go on the Kappa crates inside this binary | none | `registry-spike` CI job green on the three systems; `check-kappa-pin.sh` passes; verdict filed | 4 (2 × 2) | decision | everything |
| 1 | Store adapter | Hide Kappa behind `OciStore`: types, layout, links, streaming, durable sessions | P0 go | `cargo test --features oci --test oci_store` green on three systems, including kill-and-resume and 2 GiB under 200 MB | 6 | FR-019, FR-020 | P3, P8 |
| 2 | Gate harness | Build gates B, C, D tooling against the reference alone | P0 done (needs no product code) | reference vs reference: zero differences over 12 scenarios; every client script green against the reference | 5 | FR-013 (mechanism) | P9; tells P3, P4, P7 what "same" means |
| 3 | Module, read path | `/v2/` exists: auth outside the bearer layer, parser, errors, blob and manifest reads | P1 | conformance "pull" category green; parser table green | 4 | FR-004 | P4, P5 T3 |
| 4 | Write path | Uploads (chunked, monolithic, mount), manifest `PUT`, concurrency, crash tests | P3 | conformance "push" green; `docker push` then `pull` round trip in CI; crash suite green | 4.5 | FR-005, FR-006 | P7, P9 |
| 5 | Drop-in config, registry mode, image | The reference's compose file runs with one line changed | P0; T3 needs P3 T1 | `apps/registry/gates/clients/compose-swap.sh` green; key walk test green | 4.5 | FR-001, FR-011, FR-015, FR-021, SC-001 | P6, P9 |
| 6 | Login and TLS | htpasswd and TLS through the reference's setting names | P5 T1 | `apps/registry/gates/clients/tls.sh` and `login.sh` green | 3.5 | FR-007, FR-008 | P9 |
| 7 | Discovery and management | Tags, catalogue, referrers, delete, read-only, upload purge | P4 | **gate A: every category green**, now blocking | 3 | FR-002, FR-009, SC-002 | P8, P9 |
| 8 | Operator commands | `garbage-collect`, `import`, `verify`, `adopt`; provider moves to standard uploads | P7 (gc, adopt), P1 (verify, import) | property test 256 cases green; import twice = zero new objects; 100 of 100 flipped bytes named | 5.5 | FR-010, FR-012, FR-014, FR-022, SC-008, SC-009 | P9, P10 |
| 9 | Gates on the product | Burn gate B down to zero unlisted; C, D, E and performance on the image | P7, P5, P6 | `gates.yml` blocks merges on A, B, C, D-commit; nightly D, E, performance green three nights running | 6 | FR-003, FR-017, FR-018, SC-003 to SC-007 | P10 |
| 10 | Release, hub rehearsal, docs | Publish; rehearse the hub swap; write the operator docs; Helm chart | P9 | release job refuses a tag whose gates are not green (tested with a red dry run); rehearsal script green with rollback | 5 | FR-016, SC-010 | v1 |
| | | | | **Total** | **51** | | |

Dependency graph:

```
P0 ─┬─► P1 ───► P3 ───► P4 ───► P7 ─┬─► P8(adopt, gc) ─┐
    │            │                   │                  ├─► P9 ───► P10
    │            └─► P5.T3           └──────────────────┤
    ├─► P2 ─────────────────────────────────────────────┤
    └─► P5(T1,T2,T4) ───► P6 ───► P8(import, verify) ───┘
```

**Critical path:** P0 → P1 → P3 → P4 → P7 → P8 adopt → P9 burn-down → P10 rehearsal → tag. 27 working days. Engineer B's track has about 1.5 days of slack; A's has none. Any slip in P1, P3 or P4 moves the date.

---

## 5. Two engineers, by week

A owns the protocol track. B owns gates, configuration, operations. Day numbers are working days from the start.

| Week | Days | Engineer A | Engineer B | They meet |
|---|---|---|---|---|
| 1 | 1 to 5 | P0 T1 (pin, build), T3 (CI); then P1 T1 to T3 | P0 T2 (round trip test), T4 (verdict); then P2 T1 to T3 | Day 2: verdict, go or no-go, sent to the maintainer. Day 3: B shows A the scenario format so A names scenarios in P3 tests |
| 2 | 6 to 10 | P1 T4 to T8; P3 T1, T2 | P2 T4 to T6; P5 T1, T2 | Day 7: reference-vs-reference is zero; the first 12 golden transcripts become A's fixtures. Day 9: P3 T1 lands, which P5 T3 needs |
| 3 | 11 to 15 | P3 T3 to T6; P4 T1, T2 | P5 T3 to T5; P6 T1, T2 | Day 12: first image; gate A "pull" and gate B turn on against it, red allowed |
| 4 | 16 to 20 | P4 T3 to T5; P7 T1 to T6 | P6 T3, T4; P8 T1 (gc), T2 (import), T3 (verify) | Day 17: `docker push` works; B points the client scripts at the product. Day 20: gate A fully green, blocking |
| 5 | 21 to 25 | P8 T4 (adopt); P9 T1 (burn-down), T5 (performance), T6 | P8 T5 (provider); P9 T2 to T4 (C, D, E); P10 T1, T2 (release workflow, dry run tag) | Day 23: nightly gates start their three-night run. Day 24: every gate B difference is fixed or listed |
| 6 | 26 to 27 | P10 T3 (hub rehearsal, swap script) | P10 T4 (docs), T5 (Helm) | Day 27: both cut `server-v1.0.0`; the maintainer runs the hub swap |

Gate B's harness (P2, days 3 to 7) is finished six days before the first route it judges exists (P3 T4, day 11).

---

## 6. CI timeline

`ci.yml` stays the merge gate it is. New work goes in two new workflows so the shared file changes once.

| From day | Job (workflow) | Runs | May be red until | Blocks a merge from |
|---|---|---|---|---|
| 2 | `registry-spike` (`ci.yml`, branch `registry/p0-spike` only) | 3 systems | it is the experiment | never; deleted when P1 T1 lands |
| 8 | `oci-store` (`registry-os.yml`): `cargo test --locked --features oci --test oci_store` | macOS, Windows | day 8 | day 8, for the systems the verdict commits to |
| 7 | `b-self` (`gates.yml`): reference vs reference | per commit | day 7 | day 7 |
| 12 | `image` (`gates.yml`): build amd64, load, smoke | per commit | day 12 | day 12 |
| 12 | `gate-a` (`gates.yml`): conformance, categories by env | per commit | pull: day 12. push: day 17. all: day 20 | each category on its date; all from day 20 |
| 12 | `gate-b` (`gates.yml`): reference vs product | per commit | day 24 | day 24 |
| 21 | `gate-c`, `gate-d-commit` (`gates.yml`) | per commit | day 23 | day 23 |
| 23 | `gate-d-nightly`, `gate-e`, `performance` (`gates-nightly.yml`) | 02:00 UTC | day 26 | never a merge; always a release |
| 24 | `release-server.yml` `server-v*` tags | on tag | | refuses unless the tagged commit has green `gates.yml` and a green nightly no older than 36 hours |

Until a gate's blocking date, its job has `continue-on-error: true` and posts a summary. On the date, one commit removes that line. Nothing is "red but fine" without a date.

Runtime budget per commit **[assumption, measured in P2 T6 and P9 T6]**: `ci.yml` as today plus about 3 minutes; `gates.yml` at most 20 minutes wall clock with jobs in parallel (A 3, B 5, C 10, D-commit 5, image 6). C is the long one because of the kind cluster. If C passes 12 minutes it splits into two jobs.

Secrets: per commit needs none beyond `GITHUB_TOKEN` (GHCR). Nightly D needs `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`. Harbor and Zot run inside the job from their own compose files, so they need none. AWS, Google and Azure are run by hand before a release with the operator's own login and recorded in the release checklist; no cloud secret lives in the repository.

---

## 7. Architecture decisions to record

ADRs live in `specs/adrs/`. Next free number is 025. Each is written in the task named, in the same pull request as the code that needs it.

| ADR | Decision | Settled in | Recommendation |
|---|---|---|---|
| 025 | Kappa crates enter the dependency graph. Reverses the last paragraph of `DEPENDENCIES.md` (R25); also corrects "pure Rust" (R24) | P0 T4, written P1 T8 | Yes, two crates, pinned by `rev` to a fork that carries the patches as commits |
| 026 | Modules may authenticate themselves: `LiveModule::authenticates_itself()`; such routers merge outside the bearer layer and make their own span | P3 T1 | As stated. Default `false`; only `oci` returns `true` |
| 027 | Repository links, referrers, aliases and upload records live in our own `links.redb`. Garbage collection is ours. The store's sweep is never called | P1 T3 | As stated (`data-model.md`) |
| 028 | Registry mode network surface and the admin transport. Public listener: `/v2/`, `/`, `/healthz`, `/docs`, `/openapi.json`, TLS from `http.tls.*` (P6 T2). Administration on a Unix socket at `<root>/live/state/admin.sock`, mode 0600 in a 0700 directory, bound before the public port, no token, never TLS (errata E5, supersedes the token file and the `127.0.0.1:5001` listener). Windows: registry mode serves no administration in v1.0 — **accepted limitation (errata E6 decision, 2026-09-22)**; the start logs a warning, and the socket is the only admin transport until a named pipe arrives in a later release | P5 T3, extended P6 T3 | As stated. Closes FR-021 |
| 029 | On-disk layout version 1 and its marker file; refusal of a reference volume | P1 T1 | `data-model.md` |
| 030 | blake3 beside sha256 by an alias table and one-pass double hashing. No dependence on hard links | P1 T5 | `data-model.md` |
| 031 | `/v2/` accepts `blake3:` digests. Listed difference. The hub's provider moves from direct blob `PUT` to the standard monolithic upload | P1 T2, P8 T5 | Yes. Without it the hub cutover fails (audit 4) |
| 032 | Garbage collection refuses beside a live server. The reference allows it; operators do run `docker exec … garbage-collect`. Listed difference, with the documented recipe: `docker compose stop registry && docker compose run --rm registry garbage-collect /etc/distribution/config.yml && docker compose start registry` | P8 T1 | Refuse in v1 (settled). Note for the next release: an online mode that flips the server read-only through the admin listener |

---

## 8. Requirement trace

Every FR and SC has a task and a proof. Every task carries at least one requirement (checked in `tasks.md`; tasks that only enable others carry the requirement they enable).

| Requirement | Tasks | Proved by |
|---|---|---|
| FR-001 compose file, one line changed | P5 T1, T2, T4, T5 | `apps/registry/gates/clients/compose-swap.sh` (gate C) |
| FR-002 every route, referrers, catalogue, paging | P3 T4, T5; P4 T1 to T3; P7 T1 to T4 | gate A, all categories |
| FR-003 same statuses, headers, 18 codes | P3 T3; P9 T1 | gate B; `oci::error::tests::every_code` |
| FR-004 name and tag grammar | P1 T2; P3 T2 | `oci_store::types::tests`; parser table; gate A |
| FR-005 manifests validated | P4 T3 | gate A push; gate B `manifest-*` scenarios |
| FR-006 any size, resumable across restart | P1 T5, T6; P4 T1, T5 | `tests/oci_store.rs::resume_after_kill`; `tests/oci_memory.rs`; SC-007 nightly |
| FR-007 htpasswd, anonymous when unset | P6 T1 | `apps/registry/gates/clients/login.sh`; gate B `auth-*` |
| FR-008 TLS by the reference's names | P5 T1; P6 T2 | `apps/registry/gates/clients/tls.sh` |
| FR-009 delete off by default, scoped | P1 T3; P7 T4 | gate A delete category; gate B `delete-*`; `tests/oci_concurrency.rs::delete_during_pull` |
| FR-010 garbage collection never breaks an image | P8 T1 | `tests/oci_gc_property.rs` |
| FR-011 settings read, unsupported refused by name | P5 T1 | `registry_compat::tests::walks_every_documented_key` |
| FR-012 one command import, re-runnable | P8 T2 | `tests/oci_import.rs::second_run_adds_nothing`, `::interrupted_run_finishes` |
| FR-013 differences file | P2 T3; P9 T1 | gate B parses it; stale entries fail |
| FR-014 verify | P8 T3 | `tests/oci_verify.rs::names_every_flipped_byte` |
| FR-015 registry mode exposes only system and registry | P3 T1; P5 T3 | `features/suites/s5_registry/registry_mode.feature` |
| FR-016 image, binaries, checksums | P5 T4; P10 T1 | release job; `apps/registry/gates/release/check-artifacts.sh` |
| FR-017 release blocked without eight gates | P9 T6; P10 T1, T2 | dry run tag on a commit with a red gate must fail to publish |
| FR-018 the client list | P2 T4; P9 T2 | gate C |
| FR-019 upstream requests and carried patches | P0 T1; P1 T6, T8 | `scripts/check-kappa-pin.sh` in `just verify` |
| FR-020 hash on write | P1 T5, T7 | `oci_store::uploads::tests::wrong_digest_leaves_nothing`; gate B `digest-mismatch` |
| FR-021 nothing else reachable from the network | P5 T3; P6 T3 | `features/…/registry_mode.feature` (gRPC refused on the public port); `apps/registry/gates/clients/surface.sh` |
| FR-022 the hub's blake3 objects keep working | P1 T2, T4; P8 T4, T5; P10 T3 | `tests/registry_conformance.rs` run against our `/v2/`; rehearsal script |
| SC-001 one line, under 5 minutes | P5 T5 | `compose-swap.sh` times it; image already pulled; fails over 300 s |
| SC-002 conformance 100% on every change | P7 T6 | gate A blocking |
| SC-003 zero unlisted differences | P9 T1 | gate B blocking |
| SC-004 every client completes | P9 T2 | gate C blocking |
| SC-005 identical digests through every peer | P2 T5; P9 T3 | gate D |
| SC-006 5 GB within 20% | P9 T5 | `performance` nightly |
| SC-007 under 200 MB at 20 GB | P4 T1; P9 T5 | `tests/oci_memory.rs` (2 GiB, per commit); `performance` (20 GB, nightly) |
| SC-008 zero unpullable images | P8 T1 | property test |
| SC-009 100% of flipped bytes | P8 T3 | `names_every_flipped_byte` |
| SC-010 gates green on a release and the hub runs on it | P10 T2, T3, T6 | release job; rehearsal; the maintainer's swap |

Open items from `checklists/equivalence.md` that this plan closes: CHK001 (version pinned by digest in P2 T1), CHK002, CHK003 (`contracts/config-table.md`), CHK004, CHK005 (P2 T5), CHK008 (normalisation list in P2 T1), CHK009 (section 3), CHK011 (section 10), CHK013 (runner named in P9 T5), CHK015 (FR-015 and FR-016 now have proofs), CHK017, CHK020, CHK021 (`data-model.md`, P8 T3), CHK023 (FR-021), CHK027 (P0 no-go path), CHK028 (no cut list). Still open, for the maintainer or the phase that meets them: CHK006 logging parity, CHK022 upgrades between our own versions (layout marker exists; procedure does not), CHK024 htpasswd reload (done in P6 T1, not yet a spec line), CHK025 minimum TLS (config table says 1.2).

---

## 9. What the plan must get right, and where it is

| Topic | Where |
|---|---|
| Streaming end to end | Section 9.1 below; P1 T5; P3 T4; P4 T1 |
| Exact protocol behaviour | `contracts/registry-api.md`; gate B scenario names |
| Concurrency | `contracts/registry-api.md`, last table; P4 T4 |
| Crash safety | `data-model.md`; P4 T5 |
| Drop-in configuration | `contracts/config-table.md`; P5 |
| Garbage collection | `data-model.md`; P8 T1 |
| Gates as engineering | P2; P9; section 6 |
| Release and supply chain | Section 10; P10 |
| Hub cutover | Section 11; `data-model.md` (adopt); P10 T3 |

### 9.1 Streaming: extractor, body type and blocking boundary per route

The store is synchronous. Every store call runs inside `tokio::task::spawn_blocking`. Nothing under `/v2/` awaits while holding a store lock.

| Route | Extractor | In | Out | Blocking boundary |
|---|---|---|---|---|
| 8 blob `GET` | `Request` (for `Range`) | none | `Body::from_stream(ReaderStream::with_capacity(reader, 1 MiB))`, where `reader` wraps the `BlobReader` in a type whose `poll_read` hands each `read` to `spawn_blocking` (`src/oci_store/blobs.rs::AsyncBlob`). For `Range`, `seek` then `take(len)` | one `spawn_blocking` per 1 MiB read |
| 9 blob `HEAD` | `Request` | none | empty | one call: `blob_size` |
| 4, 5 manifest `GET`, `HEAD` | `Request` | none | `Body::from(Vec<u8>)`, at most 4 MiB | one call |
| 6 manifest `PUT` | `Request` | `http_body_util::Limited::new(body, 4 MiB)` then `collect()` | empty | one call after the body is whole |
| 11b, 14, 15 upload bodies | `Request` | `body.into_data_stream()`, re-cut into **exactly 4 MiB** frames in a `BytesMut` (last frame shorter). Each frame: update `blake3::Hasher`, then `spawn_blocking(upload_put_part)` | empty | one `spawn_blocking` per 4 MiB frame; at most one frame in memory per upload |
| 15 finish | | | | one `spawn_blocking(upload_complete)`: the store re-reads the staging file to hash it (K4), so this call takes as long as reading the blob once. The HTTP timeout on this route is off |
| everything else | `Request` | none | small JSON | one call |

Memory bound per upload: 4 MiB frame + hyper's buffer. At 16 parallel uploads: about 70 MB. SC-007 has room.

Why 4 MiB and not pass-through: each `upload_put_part` call opens the file and keeps a record (K3). A 20 GB layer in 16 KiB frames would be 1.3 million opens and records; in 4 MiB frames it is 5,120.

---

## 10. Release and supply chain

| Question | Answer |
|---|---|
| Image base | `gcr.io/distroless/cc-debian12:nonroot` with gnu targets. musl is tried once in P10 T1 step 2; it is adopted only if `wasmtime`, `ring`, `zstd-sys`, `lzma-sys`, `bzip2-sys` all link static with no patch **[assumption: not checked]**. A static musl binary would let the image be `scratch`; it is an improvement, not a requirement |
| Architectures | `linux/amd64`, `linux/arm64`, one manifest list, built with `docker buildx` on native runners (`ubuntu-24.04`, `ubuntu-24.04-arm`), not emulation |
| Standalone binaries | Linux x86_64 and aarch64 (gnu), macOS aarch64, Windows x86_64. macOS x86_64 stays in the matrix as upstream has it (R29). A system drops out only if the P0 verdict says the store cannot build there, and that is a listed limitation, not a cut |
| Checksums | `SHA256SUMS` over every artefact, plus the image digest, in the release notes. `apps/registry/DIFFERENCES.md` attached |
| Kappa pin | `Cargo.toml` names the fork and `rev`. `third_party/kappa/README.md` records: upstream `rev` the fork is based on, each carried commit with its upstream pull request link and state, and the two fork-branch crates with the `rev` they are pinned to. `third_party/kappa/patches/*.patch` hold the same commits as files, made by `git format-patch`, so the pin can be rebuilt from upstream alone |
| Audit | `scripts/check-kappa-pin.sh` in `just verify`: (1) `Cargo.lock` `rev` equals the README's; (2) every patch file has an upstream link; (3) `cargo tree -e normal` prints no `topcoat`, `veilid`, `openssl`, `aws-lc`; (4) no dependency resolves to a git **branch** without a locked `rev` |
| Kappa fork location | Spike: the engineer's own fork. Before P1 ends: a fork under the organisation that will own it. FR-019 bars a release on a personal fork branch. Decision 3 for the maintainer |
| Tags | `server-v0.0.1-red` on a scratch branch (must not publish), `server-v1.0.0-rc.1` (the release candidate, P10 T2), then `server-v1.0.0`. One tag publishes binaries, both images and the chart |

**Shared files in the parent.** There is no long-lived fork to merge (section 0). Registry code lives in new files. The shared files this plan touches, and how much:

| File | Change | Lines **[assumption]** |
|---|---|---|
| `Cargo.toml` | two dependencies, four crates made direct, one feature line | ~12 |
| `src/lib.rs` | `pub mod oci_store; pub mod registry_compat; pub mod tls;` | 3 |
| `src/module.rs` | one trait method with a default; `ModuleRegistry::routers()` returns the pair | ~25 |
| `src/modules/mod.rs` | second macro list | ~12 |
| `src/server.rs` | merge the second router outside the bearer layer; call `crate::tls::serve` | ~20 |
| `src/app.rs` | `Option<Arc<OciStore>>` in `AppInner`, built when the module is on | ~15 |
| `src/config.rs` | `registry_mode: bool` (serde skip), relaxed `validate` branch, `[oci]` section | ~40 |
| `src/main.rs` | `argv[0]` rewrite before `Cli::parse` | ~10 |
| `src/cli/mod.rs` | one `Oci` subcommand line | 3 |
| `DEPENDENCIES.md`, `justfile`, `.github/workflows/ci.yml` | rows, two recipes, one job | ~30 |

About 170 lines in 12 shared files. Rule (section 0, rule 4): each shared-file change goes to the parent as a small pull request of its own, with its own test, before the registry code that needs it. The trait hook, the two-list macro, the `argv[0]` rewrite and the TLS accept loop are the four. When a registry change needs more than the lines above in a shared file, stop and ask the other maintainer first.

---

## 11. Rollback and migration

**A user whose v1 misbehaves.**
1. They did not lose the old registry: v1 never opens a reference volume in place, so the old volume is untouched. Rollback is changing the image line back.
2. Data pushed to v1 since the switch moves back with `skopeo sync --src docker --dest docker <v1> <old>` or `crane copy`. Gate D proves this direction on every commit.
3. `hologram oci verify` tells them whether the misbehaviour is damaged data. `apps/registry/DIFFERENCES.md` tells them whether it is a known difference.

**The hub.**
- Rehearsed on a local mirror of `/root/hub` (P10 T3). The swap script, in the style of `deploy/swap-hologram.sh`, does: stop `kappa`; snapshot `./store` (hard link copy); run `adopt`; start the v1 image on the same paths; run `health.sh` plus three checks (pull `model-hub/index:<date>`; read one object through the `kappa` provider; publish one test object); on any failure, stop v1, delete `oci/` and the marker, start `kappa-server` again. `adopt` never modifies the Kappa paths (`data-model.md`), so rollback is exact.
- Order of change on the hub: first ship the provider change (P8 T5) while still on `kappa-server`, which must accept the standard upload flow too (the rehearsal checks this; if `kappa-server` cannot, the provider keeps both flows behind one config key until the swap).
- the maintainer runs the deploy. Nothing in this plan writes to the hub's host.

---

## 12. Risks

| Risk | Bites in | Early signal | Fallback |
|---|---|---|---|
| Kappa crates do not build on Windows, macOS or musl (`lzma-sys`, `bzip2-sys`, fork-branch crates) | P0 | Day 1, the first `cargo build` | Ask upstream to put codecs behind a feature and carry that patch (the registry path does not use them). If a system still fails: ship without it, as a listed limitation. If Linux fails: no-go, re-plan around `kappa-server` as a second process |
| One upstream author, silent since 16 August; two dependencies live on personal fork branches that can be deleted | P0 onward | A `cargo fetch` that cannot find a `rev` | Mirror `bc-dcbor-rust` and `rekindle` forks into the owning organisation on day 1, pin by `rev` (P0 T1 step 3). Carried patches from day one |
| The reference behaves differently from its documents | P3, P4, P7 | P2 golden transcripts, week 1 | The transcript wins. The contract file is corrected in the same pull request |
| `aws-lc` in the `oci` graph through `rekindle-aead` (found in P0; broke a clean Windows runner) | every merge into the parent | `check-kappa-pin.sh` strict mode | Carried patch A in P1 T6; until then the gate runs with `KAPPA_PIN_ALLOW_AWS_LC=1` on fork branches only |
| Blob data not synced before rename (K4) | any power loss | P0 reads the code; cannot be seen in tests | Carried patch `0004`. `verify` is the net |
| bcrypt cost makes `docker push` slow (40 layers, 40 checks) | P6 T1 | `apps/registry/gates/clients/login.sh` timing | Cache of verified `(user, keyed hash of password)` for 60 s; invalidated when the file changes |
| A registry-mode container cannot write its state (no `$HOME`) | P5 T3 | First image smoke test | All Live paths forced under `<root>/live/` (ADR-028, R18) |
| Gate C's kind cluster makes CI slow or flaky | P9 T2 | Runtime over 12 minutes, or two flakes in a week | Split the job; move the Kubernetes pull to nightly and keep containerd per commit (same pull code path) |
| `ollama push` to plain HTTP changes flags by version | P2 T4 | Script red against the reference itself | Pin the Ollama version in the script; test over TLS only |
| Upstream Hologram Live moves under us | weekly | Merge conflicts on Monday | Section 10: 12 shared files, about 170 lines, upstream the hooks |
| The 27 days are an estimate with no slack on A's track | P1, P3, P4 | P1 exit later than day 8 | The date moves. There is no cut list |

---

## 13. Decisions for the maintainer

Recommendation first. The plan proceeds on each recommendation.

1. **Go or no-go after P0.** Recommend: go if Linux and one desktop system are green and the upstream fix list is under 5 patches and 300 lines. The verdict template asks exactly this.
2. **`/v2/` accepts blake3 digests (FR-022, ADR-031).** Recommend yes. The hub stores its objects by blake3 through `/v2/` today; without this the success test cannot pass. Cost: one line in `apps/registry/DIFFERENCES.md`.
3. **Where the Kappa fork lives.** Recommend a fork under the organisation that ships the image (Hologram Technologies), with the UOR Foundation given write access, created in week 1. A personal fork is fine for the two-day spike only.
4. **Registry mode hides the admin API (FR-021, ADR-028).** Recommend yes: token generated at start, gRPC on loopback only. The alternative exposes `shutdown` on port 5000 of an anonymous registry.
5. **Unknown `REGISTRY_*` variables stop the start.** Recommend yes (the reference ignores them). A typo in `REGISTRY_AUTH_HTPASSWD_PATH` would otherwise start an open registry. Listed difference.
6. **Image base.** Recommend distroless with gnu targets now; musl and `scratch` only if it links clean in P10.
7. **Outputs.** These files sit in `Product/1. Hologram Registry/Specs`. Recommend (section 0, rule 7): on acceptance they go to the parent by pull request, plans under `docs/superpowers/plans/`, specs under `docs/superpowers/specs/`, with `apps/registry/README.md` linking to them. Draft 1 in the clone is superseded and is not proposed upstream.
8. **The other maintainer's agreement** (section 0). Recommend: ADR-025 as a draft pull request to the parent on day 1, in parallel with the spike.
