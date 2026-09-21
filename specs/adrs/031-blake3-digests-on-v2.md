# ADR 031: `/v2/` accepts `blake3:` digests

- Status: proposed
- Date: 2026-09-21

## Context

The reference registry accepts `sha256:` (and `sha512:`) digests. Hologram addresses objects by `blake3:`, and the
hub's provider (ADR 021) reads and writes `blake3:` blobs and manifests through `/v2/` today. v1 succeeds when the
hub serves its registry from a release of this product; with sha256 only, that cutover cannot pass.

## Decision

`Digest::parse` (`src/oci_store/types.rs`) accepts `sha256`, `sha512` and `blake3`, lowercase hex of exactly the
algorithm's length. A blob or manifest may be pushed and read by any of them. The store keeps the pair for the same
bytes in an alias table (ADR 030), so content pushed by one name is served by the other in the same repository.

A response names the digest the client asked by. A sha256 client never sees a blake3 digest.

This is a kept difference from the reference and is listed in `apps/registry/DIFFERENCES.md`: the reference answers
`DIGEST_INVALID` to `blake3:`; this product serves it.

## Consequences

- The hub's provider can move from its direct blob `PUT`, which is not a registry route, to the standard upload
  flow with a blake3 digest.
- Tools that only know sha256 are unaffected: nothing they send or receive changes.
