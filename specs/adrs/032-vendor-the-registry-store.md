# ADR 032: The registry's store crates are vendored in this repository

- Status: accepted
- Date: 2026-09-21
- Supersedes: the git pin in ADR 025 (the rest of ADR 025 stands)

## Context

ADR 025 let two Kappa crates into the graph behind the off-by-default feature `oci`, pinned by revision to a git
repository. In practice that meant three repositories outside this one: `Hologram-Technologies/kappa-registry`
(a fork carrying six fixes that upstream has not merged), `UOR-Foundation/kappa-registry` (upstream), and
`usrbinkat/bc-dcbor-rust` (a personal fork, named by branch, that `kappa-core` needs for `dcbor`).

The registry should build, be reviewed and be released from this repository alone. A branch on a personal fork
can move or vanish; a fork of our own still has to be kept in step by hand; and a reviewer here could not see the
code the registry stores its bytes with.

## Decision

`kappa-core`, `kappa-store-redb`, `dcbor` and `dcbor-derive` are copied into `third_party/`: sources byte for byte,
manifests rewritten to stand alone. They are path dependencies of `hologram-live`, still optional, still behind
`oci`, and excluded from the workspace: they are built as dependencies and never formatted, linted or tested as our
code.

`third_party/kappa/VENDORED.sha256` records every vendored file. `scripts/check-kappa-pin.sh` fails if a file
differs, a store crate has any source but this repository, a carried patch lacks its upstream link, or `aws-lc`,
`rekindle`, `openssl-sys`, `topcoat` or `veilid` enters the registry graph. An edit to vendored code is therefore a
deliberate, reviewed change with a patch file beside it.

## Consequences

- A stock build is unchanged: the crates are optional, and `check-product-boundaries.sh` still keeps them out.
- About 30,000 lines of other people's code live in this repository. They are not ours to restyle; the file-size
  guard skips `third_party/`.
- The format check is `cargo fmt --check`, not `cargo fmt --all --check`: `--all` also formats local path
  dependencies, and the vendored code does not follow our style. Without `--all` it still checks every workspace
  member.
- The upstream pull requests (UOR-Foundation/kappa-registry#13, #14, #15) stay open. When they merge, the copy is
  refreshed from upstream and the patch files go.
- `kappa-core`'s optional `encryption` feature still names `rekindle-aead` by git branch. It is never resolved with
  `default-features = false`, and the guard would catch it.
- The vendored tests are not copied or run here. The store is tested through the adapter's own tests
  (`src/oci_store`, `tests/oci_store.rs`) on three systems.

## Alternatives considered

**Keep the git pin.** Rejected: the registry would still depend on three outside repositories, one a branch on a
personal fork.

**`cargo vendor` for the whole graph.** Rejected: it would vendor about 600 crates.io packages that crates.io already
serves immutably. Only the git-sourced crates were at risk.

**Wait for upstream to merge the fixes and publish to crates.io.** Not rejected, only not blocking: this decision is
reversible the day upstream publishes.
