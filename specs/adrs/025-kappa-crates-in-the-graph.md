# ADR 025: The Kappa store crates enter the dependency graph, behind an off-by-default feature

- Status: proposed
- Date: 2026-09-21

## Context

Hologram Registry v1 (`apps/registry/`) is a drop-in replacement for the Docker Registry image, built on the Kappa
Registry store. Its `/v2/` API is a server module, `dev.hologram.live.oci`, and it needs a store that addresses
blobs by `sha256:` and `blake3:`, stages uploads in parts, and reads with `Read + Seek`.

`DEPENDENCIES.md` records the opposite position: "Kappa Registry remains an external service/project … None of its
workspace crates enter this dependency graph". That was right for the provider added in ADR 021, which speaks to an
external `kappa-server` over HTTP. It does not fit a registry that must start from one `docker run` and stream a
20 GB layer with flat memory: `kappa-server` reads every upload body whole into memory, and most of the gaps against
the reference registry are in its HTTP layer, not in its store.

## Decision

Two crates from `UOR-Foundation/kappa-registry`, and only two, become optional dependencies of `hologram-live`:
`kappa-core` (the `KappaStore` trait) and `kappa-store-redb` (the store). `kappa-server`, `kappa-module-oci` and the
other workspace members are not used.

```toml
[features]
oci = ["dep:kappa-core", "dep:kappa-store-redb"]    # not in `default`
```

**The default build does not change.** A stock `cargo build` pulls no Kappa crate and no new native library.
`scripts/check-product-boundaries.sh` fails if `kappa-core`, `kappa-store-redb`, `lzma-sys`, `bzip2-sys` or
`aws-lc-sys` appears in the default server graph. The registry image, the release binaries and the registry tests
build with `--features oci`; `just verify` gains `oci-check` so that build cannot rot.

**The pin is auditable.** The two crates are pinned by revision to the organisation's fork,
`Hologram-Technologies/kappa-registry`, which carries the fixes below until upstream merges them; after that the pin
moves back to upstream. `third_party/kappa/README.md` records the revision, every carried patch with its upstream
link, and the two dependencies upstream names by branch on personal forks. `scripts/check-kappa-pin.sh` fails if the
locked revision differs from the documented one, if a carried patch has no upstream link, if `topcoat`, `veilid`,
`openssl-sys` or `aws-lc` is in the registry graph, or if any git dependency is not locked to a commit.

**No Kappa type leaves `src/oci_store/`.** The registry module sees an adapter. A later store swap touches one
directory.

## What the spike found (2026-09-21)

Full record: `docs/superpowers/specs/2026-09-21-registry-p0-verdict.md`. Branch `registry/p0-spike` on the fork
`humuhumu33/hologram-registry`; nothing from it is proposed here.

- The plain upstream git dependency, pinned by `rev`, builds as published. Upstream's workspace root declares
  `[[test]]` in a virtual manifest, which breaks a build of that workspace, but cargo does not read it when the
  workspace is a git dependency. No fork was needed to compile.
- Linux, macOS and Windows build the crates and pass the store round trip tests. A 2 GiB upload streams in
  8.8 MB of memory on Linux. The release binary is the same size with and without the feature until code uses it.
- 94 crates are new in the registry graph (318 → 412). New native code: `lzma-sys`, `bzip2-sys`, and **`aws-lc-sys`**.
- `aws-lc-rs` arrives through `rekindle-aead` (blob encryption at rest), which the registry does not use. This
  repository keeps `aws-lc` out on purpose (`rustls` with the `ring` provider). It is used in two upstream files
  only, so the fix is a feature gate upstream, carried as a patch until merged. It is not cosmetic: a clean
  `windows-2022` runner failed in `aws-lc-sys` for want of NASM. **The registry code does not merge
  here while `aws-lc` is in its graph.**

## Alternatives considered

**Run `kappa-server` as a second process** (what the hub does today behind Caddy). Rejected: the equivalence work
would land in handlers written for a personal-fork web framework, in a repository with one author and no release;
the image would carry two processes.

**Write a store of our own.** Rejected for v1: the Kappa store already gives multi-algorithm addressing, staged
uploads and deduplication, and the step after v1 is one store for images, models and Hologram objects.

**Feature on by default.** Rejected: it would add three native libraries and two git dependencies to every build of
Hologram Live for a module most installs do not enable.

## Consequences

- `DEPENDENCIES.md` loses its last paragraph's claim and gains rows for the two crates, in the pull request that
  adds them. Its "pure Rust" wording for `rustls` stays true for the default build only.
- Supply risk moves in-house: one upstream author, no release, two branch-named dependencies. Mitigation is the pin
  gate, carried patches from day one, and mirrors of the two fork repositories before a release.
- Builds with `--features oci` need a C compiler on every system (already true of the default build: `ring`,
  `zstd-sys`).
