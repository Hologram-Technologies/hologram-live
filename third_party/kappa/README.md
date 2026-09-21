# Kappa store, vendored

The registry (`apps/registry/`, cargo feature `oci`, off by default) stores its bytes in the Kappa store. Its two
crates are vendored here, so the registry builds from this repository alone (ADR 032, which supersedes the git pin of
ADR 025). `scripts/check-kappa-pin.sh` audits this directory.

| | |
|---|---|
| Upstream | https://github.com/UOR-Foundation/kappa-registry |
| Upstream revision the copy is based on | 2af86560a177fc9651b6c0e92e7974140ed77dd5 |
| Copied from | https://github.com/Hologram-Technologies/kappa-registry @ c7b2ee722cfad39bfe486af0f5f37646bccc68f5 = upstream + the six patches below |
| Crates | `crates/kappa-core`, `crates/kappa-store-redb`: `src/` and `README.md` only; tests, benches and the rest of the workspace are not copied |
| Licence | `MIT OR Apache-2.0`, as declared in the upstream workspace manifest. Upstream ships no licence file yet: https://github.com/UOR-Foundation/kappa-registry/issues/16 |
| Record | `VENDORED.sha256`: one line per vendored file. The guard fails if a file differs, appears or disappears |

**What was changed in copying, and only this.** Each `Cargo.toml` was rewritten to stand alone: fields and
dependencies that said `workspace = true` now carry the workspace's own values, the crate-to-crate links are paths,
`[dev-dependencies]` is dropped because the tests are not copied, and `dcbor` points at `../../../dcbor/dcbor`.
The Rust sources are byte for byte those of the fork revision.

**Not built, but named.** `kappa-core`'s optional `encryption` feature still names `rekindle-aead` by git branch. The
registry depends on `kappa-core` with `default-features = false`, so that dependency is never resolved, fetched or
built, and the guard fails if `rekindle` or `aws-lc` enters the registry graph.

**To change vendored code:** edit it here, add the change as a patch file below with its upstream link, run
`scripts/check-kappa-pin.sh --record`, and say why in the pull request. **To re-vendor:** replace `crates/` from a new
revision, keep the manifest rewrite, re-record, and update this table.

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

## The one other vendored dependency

`kappa-core` needs `dcbor`, which upstream names by branch on a personal fork. It is vendored too, in
`../dcbor/` (BSD-2-Clause-Patent, licence file included), so no branch can move or vanish under the build.

| Crate | From | Commit |
|---|---|---|
| dcbor, dcbor-derive | https://github.com/usrbinkat/bc-dcbor-rust, branch `feat/dcbor-derive` (a fork of BlockchainCommons/bc-dcbor-rust) | 2e5b901e8c9946794c5491cf92eeec2540b84261 |
