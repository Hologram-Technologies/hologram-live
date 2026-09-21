# ADR 030: blake3 beside sha256, by an alias table and one pass of hashing

- Status: proposed
- Date: 2026-09-21

## Context

Docker clients address blobs by sha256. Hologram addresses objects by blake3, and the hub's store already holds
blobs under blake3. The same bytes must be readable by either name. The Kappa store does not make a blake3 address
for a sha256 upload (its mandatory axis is sha256 only; spike test q5), and where it links two addresses it uses
hard links and discards their errors, so nothing can rely on them, least of all on Windows or across volumes.

## Decision

- While a blob streams in, the adapter feeds each frame to a `blake3::Hasher` as well. The bytes are already in
  memory, so this is CPU only: no second read.
- At finish, one `links.redb` transaction writes the link and both directions of the pair in `aliases`
  (`sha256:… → blake3:…` and back), and removes the session row.
- For a blob pushed by blake3, the pair's other side is the sha256 the store computes as its mandatory axis.
- To read: link check first, then ask the store by the requested digest; on a miss, ask by its alias. A link under
  one name makes the blob reachable under the other name in the same repository, and in no other.
- After a restart the running hash is gone (the `blake3` crate cannot serialise a hasher). Then the alias is found
  by one streamed read of the finished blob.
- A `/v2/` response names the digest the client asked by. A sha256 client never sees blake3.

No hard link is needed anywhere.

## Consequences

- One more hash per byte on the push path. If the performance gate shows push speed outside 20% of the reference,
  the hasher becomes optional and aliases are computed lazily; the design already handles a missing hasher.
- `aliases` is part of garbage collection's mark set: both sides of a marked pair are marked.
