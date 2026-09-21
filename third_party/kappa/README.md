# Kappa store pin

The registry (`apps/registry/`, cargo feature `oci`, off by default) stores its bytes in the Kappa store. This file
is the record `scripts/check-kappa-pin.sh` audits. ADR 025 is the decision.

| | |
|---|---|
| Upstream | https://github.com/UOR-Foundation/kappa-registry |
| Upstream revision the pin is based on | 2af86560a177fc9651b6c0e92e7974140ed77dd5 |
| Fork | https://github.com/Hologram-Technologies/kappa-registry, branch `hologram-registry-pin`. Development copy: https://github.com/humuhumu33/kappa-registry (the upstream pull requests come from there) |
| Pin revision (must equal Cargo.lock) | c7b2ee722cfad39bfe486af0f5f37646bccc68f5 |
| Crates used | kappa-core, kappa-store-redb, both with `default-features = false`. Nothing else from the workspace |

The plain upstream dependency builds as published (P0 verdict). The fork exists for the patches below, not to make
the workspace parse.

## Carried patches

`patches/` holds the fork's commits as files (`git format-patch`), so the pin can be rebuilt from upstream alone:
check out the upstream revision, then `git am patches/*.patch`.

| File | What | Upstream | State |
|---|---|---|---|
| patches/0001-feat-crypto-put-the-AEAD-backend-behind-a-default-on.patch | `rekindle-aead` behind a default-on `encryption` feature. Off, no key can be constructed, encryption fails closed, and `aws-lc-rs` and `aws-lc-sys` leave the graph. This repository keeps `aws-lc` out on purpose, and `aws-lc-sys` broke a clean Windows runner for want of NASM | https://github.com/UOR-Foundation/kappa-registry/pull/13 | open; carried |
| patches/0002-fix-store-redb-depend-on-kappa-core-by-path-so-defau.patch | part of 0001: a member cannot turn off defaults the workspace entry leaves on | https://github.com/UOR-Foundation/kappa-registry/pull/13 | open; carried |
| patches/0003-test-store-redb-gate-the-encrypted-upload-tests-on-t.patch | part of 0001: the encrypted upload tests run only with the feature | https://github.com/UOR-Foundation/kappa-registry/pull/13 | open; carried |
| patches/0004-feat-store-resume-an-upload-whose-staging-file-survi.patch | `PersistentStoreConfig::preserve_staging` (default off) and `KappaStore::upload_resume`: an embedder can re-attach to an upload after a restart (FR-006). Default behaviour unchanged | https://github.com/UOR-Foundation/kappa-registry/pull/14 | open; carried |
| patches/0005-fix-store-sync-blob-data-before-the-rename-that-publ.patch | `upload_complete` syncs the staged file before the rename that publishes it, so a power loss cannot leave a named blob with bytes that never reached disk | https://github.com/UOR-Foundation/kappa-registry/pull/15 | open; carried |
| patches/0006-fix-store-flush-through-a-write-handle-and-test-an-u.patch | part of 0005: Windows refuses to flush a read-only handle; adds the first upload test that runs with fsync on | https://github.com/UOR-Foundation/kappa-registry/pull/15 | open; carried |

Verified on the fork (Windows): `kappa-store-redb` tests pass with the feature on (67) and off (56); `kappa-core`
`crypto::aead` tests pass with it on (16). Upstream behaviour is unchanged: the feature is on by default.

Upstream pull requests opened 2026-09-21: #13 (optional encryption), #14 (resumable uploads), #15 (sync before
rename). Each was tested on its own branch from upstream `main`.

Planned, not written yet:

| Patch | Why | Blocks |
|---|---|---|
| `dcbor` named by `rev` on a mirror | a deleted branch must not break the build | a release |
| LICENSE file | the crates declare `MIT OR Apache-2.0` and ship no file. Asked upstream: https://github.com/UOR-Foundation/kappa-registry/issues/16 | a release |

## Fork-branch dependencies

Upstream names `dcbor` by branch on a personal fork. `Cargo.lock` fixes the commit, so a `--locked` build
reproduces while that commit stays fetchable. With `encryption` off, `rekindle-aead` (the other branch-named
dependency) is no longer in the graph at all.

| Crate | Upstream | Locked commit | Our mirror |
|---|---|---|---|
| dcbor, dcbor-derive | https://github.com/usrbinkat/bc-dcbor-rust (branch `feat/dcbor-derive`) | 2e5b901e8c9946794c5491cf92eeec2540b84261 | not yet |
