# Phase 2b: object replication over iroh-blobs

## Status

Designed, not implemented. Completes #179 (Phase 2 of #177). Depends on the
Phase 2a transport (#221), whose endpoint, identity and admission seams this
reuses. The parent design (`2026-09-24-distributed-p2p-clustering-design.md`)
named this work — "object replication moves to `iroh-blobs`: BLAKE3/bao
verified streaming, resumption, and range requests, with the registry's
existing `blake3:` ids mapping onto blob hashes" — and left it undesigned;
the 2a design repeated that it is "Phase 2b and is not designed here". This
document designs it.

Scope is deliberately the **transfer mechanism**. Replica sets (`r = 3`
holders chosen by rendezvous hash) are the recorded follow-on and are not
designed here.

## Goal

Replication between key-addressed peers moves whole, verified objects without
buffering them in memory, resumes an interrupted transfer instead of
restarting it, and serves bytes only to admitted cluster members — while the
registry stays the authority for objects, exactly as ADR 021 and the parent
design require.

Phase 2a made a NAT'd server reachable. It changed nothing about *what*
replication does once connected, and what it does has three recorded
weaknesses. Defect 9 of the parent design: every object is buffered wholly in
memory on both the wire and the store side. Phase 1 left the transfer loop
alone precisely because this phase replaces it. And an interrupted 512 MB
object restarts from byte zero on the next round.

## What carries over unchanged

- **The inventory stays on the cluster router.** `GET /api/v1/cluster/objects`
  and its pagination, the request proof, admission, and the per-round cadence
  (`replication_interval_secs`, fanout, `replication_max_objects_per_round`)
  are untouched. The inventory is how a node learns *which* `blake3:` ids a
  peer holds, along with kind, media type, filename and size; it is small,
  authenticated, and already transport-agnostic. Only the bulk bytes move to a
  second protocol.
- **HTTP peers keep the existing per-object GET.** A mixed cluster replicates
  each direction over whichever mechanism the peer's network supports; nothing
  changes for an HTTP-only peer, including `max_response_bytes` enforced
  mid-read.
- **The registry is the only object authority.** iroh-blobs gets a staging and
  serving cache; it never decides what an object is, never holds metadata, and
  nothing is served from it that the registry does not hold.

## Substrate facts, verified against the crates

Claimed here only after reading the published sources, not estimated:

- `iroh-blobs 0.103.0` (the current release) requires `iroh ^1.0.0`, so it
  sits on the `iroh 1.3.0` already locked for Phase 2a. MSRV 1.91; this
  repository is on 1.95. `MIT OR Apache-2.0`, as the parent design recorded.
- **A blob's id is the plain BLAKE3 hash of its bytes.**
  `iroh_blobs::Hash` wraps `blake3::Hash` (`Hash::new` is `blake3::hash`), and
  verified streaming uses bao outboards whose root is that same hash. The
  registry's `blake3:<64 hex>` object ids therefore map onto blob hashes with
  no translation table. A golden test pins this (see Testing), because the
  whole phase rests on it.
- **`default-features = false, features = ["fs-store"]`** is sufficient: the
  `fs-store` feature pulls `redb` and `reflink-copy`; the default `rpc`
  feature (`noq`, irpc endpoint setup) is not needed and is excluded.
- The fetch side resumes by construction: `Remote::fetch` computes the
  locally present ranges (`local.missing()`) and requests only those, so a
  round that dies mid-object continues where it stopped on the next round.
  `GetStreamPair` is implemented for `iroh::endpoint::Connection`, so the
  existing endpoint dials the blobs protocol directly.
- The provider side is one public function,
  `provider::handle_connection(conn, store, events)`, which is exactly what
  `BlobsProtocol`'s `ProtocolHandler` calls. Serving can therefore be gated in
  front of it (see Authorization).
- Import from a file can avoid a copy: `ImportMode::TryReference`. Registry
  blob files are content-addressed, written atomically and never modified in
  place (a flipped bit is detected on read, per
  `registry::local::tests::a_blob_corrupted_on_disk_is_refused_not_served`),
  so referencing them is safe; a store that ignores the hint falls back to
  copying, which is correct at twice the disk.
- The protocol's own ALPN is `/iroh-bytes/4` (`iroh_blobs::ALPN`), distinct
  from Phase 2a's `hologram/cluster/1`, so one endpoint can speak both.

Pre-1.0 status is inherited from the parent design, which already accepted it:
`iroh-blobs` will break API across releases, and the dependency is taken with
that known.

## One store per node, shared by both directions

A single `FsStore` (under `paths.data_dir`, location fixed in implementation)
serves both directions of replication:

- **Outbound (this node as provider):** a mirror loop imports the registry's
  objects into the store — `add_path` with `ImportMode::TryReference`, so a
  same-filesystem import links rather than copies — under a named tag
  (`hologram:<digest>`) that pins the blob against the store's garbage
  collector. The loop reconciles tag set against registry inventory on the
  replication cadence: new objects are imported and tagged, tags whose digest
  the registry no longer holds are deleted so the collector can reclaim the
  bytes. First start imports the existing inventory; steady state is a diff.
- **Inbound (this node as requester):** the fetch streams verified ranges into
  the same store under a batch temp tag (a download that outlives a GC
  interval is not collected mid-flight). On completion the object is imported
  into the registry and the blob takes the same named tag — the bytes are
  already local, so this node can serve what it just replicated without a
  second transfer.

The named-tag reconcile is what keeps the store a *cache* rather than a second
source of truth: whatever the registry holds is tagged and served; whatever it
no longer holds becomes collectable.

### The store side stops buffering whole objects

Today `ObjectStore::put` and `RegistryProvider::put_object` take `&[u8]`, so
even a perfectly streamed download ends as up to
`replication_max_object_bytes` (default 512 MB) held in memory at once —
the store half of defect 9. This phase adds a streaming put to `ObjectStore`
(hash-while-writing to the atomic temp file, then rename; the id is known only
at EOF, so the temp file is named randomly and renamed to the digest, matching
the existing atomic-write discipline of #204) and uses it for the local
provider. The kappa provider keeps the whole-bytes path — its put crosses the
network to the registry service regardless — and the trait's object-safe
surface is unchanged. The requester path then never holds a whole object:
verified ranges land in the store's own data file, and the registry import
reads them back incrementally.

## Authorization: the connection is checked, the bytes check themselves

`iroh-blobs` has no request-level authorization of its own; its security model
is hash-as-capability. That model is not sufficient here on its own: an
object's hash is not secret — it appears in every inventory answer, in OCI
manifests, in logs — and the QUIC handshake does not require admission to
dial. So the phase adds the gate the cluster already has, at the one place it
can be total:

**On accepting a blobs-protocol connection, the dialler's `remote_id()` must
be in this node's admitted set** (the same `Admission` the cluster router
consults, read at accept time). A dialler that is not admitted has the
connection closed before any request is read. The admitted set is keyed by the
same ed25519 key the EndpointId encodes, so the check cannot be passed by
forging an identity. This is stronger than the HTTP object route, which
authorizes per request: here one check covers every exchange on the
connection, and there is no handler surface behind it other than "serve bytes
whose hash the dialler already knows, from a set the registry already
publishes to admitted peers".

What does not appear here is a request proof. The proof machinery exists to
authenticate *claims* — method, path, query, body — to a parser that will act
on them. A blobs request carries no claim: the hash is self-authenticating,
and bao verified streaming means the requester detects any byte that is not
the hash's content, so a peer cannot feed bad data to a requester that knows
the hash it wants. The residual is availability, not integrity: an
admitted-but-hostile peer can waste its own connection's bandwidth. That is
the same standing such a peer already has on the HTTP object route, bounded by
the same admission decision that let it in.

## The endpoint speaks two protocols

The Phase 2a endpoint gains a second registered ALPN, `iroh_blobs::ALPN`,
alongside `hologram/cluster/1`. The existing accept loop dispatches on
`Incoming::alpn()` before awaiting the handshake (verified present in iroh
1.3's connection API): the cluster ALPN goes to the existing HTTP-over-stream
serving untouched, the blobs ALPN goes through the authorization gate above to
`provider::handle_connection`. A connection offering any other ALPN fails the
handshake, as it does today.

The dial side mirrors this: cluster exchanges open connections with the
cluster ALPN as today; a blob fetch opens a connection with the blobs ALPN and
hands it to `store.remote().fetch(conn, HashAndFormat::raw(hash))`. iroh
multiplexes both over one QUIC connection to the same peer, so a replication
round costs no extra handshakes for running the inventory and the fetches to
the same endpoint.

The "one handler set" rule from Phase 2a is restated for the new protocol and
enforced by test: the blobs ALPN reaches the blob provider **only** — not the
cluster router, not the application router — and the cluster ALPN still
reaches only the three cluster routes.

## Replication, per peer network

`replicate_peer` keeps its round structure — paged inventory, per-object
outcomes, `ends_replication_round` classification — and gains a branch chosen
by capability, not by string matching: the `ClusterNetwork` trait gets a
defaulted `fetch_blob` seam returning `None` ("this network has no blob
channel; use the per-object GET"), which `IrohNetwork` implements and
`HttpNetwork` inherits. Membership and the round loop still never branch on
transport; the seam is the same kind Phase 1 used for `recipient_for`, where
each network answers for the address it is about to dial.

For an iroh peer, a missing object becomes: skip if `metadata.size` exceeds
`replication_max_object_bytes` (unchanged policy, now a pure local resource
bound — a peer that lies about size cannot smuggle extra bytes, because the
verified stream contains exactly the bytes the hash names), then fetch, then
streaming import, then the same digest equality check the HTTP path performs
(stored id vs advertised id), which a bao-verified transfer makes nearly
always redundant and which stays as the backstop. Transport and auth failures
still end the round; a failed or interrupted fetch is per-object, and its
partial ranges persist so the next round resumes them.

## Dependencies and gates

`iroh-blobs = { version = "0.103", optional = true, default-features = false,
features = ["fs-store"] }`, added to the existing `p2p` feature. The real
net-new count is measured against this repository's lockfile during
implementation and recorded in `DEPENDENCIES.md` next to the Phase 2a figures
(the 2a scratch measurement missed the blake3 pin conflict; this phase's
numbers are quoted from the real graph or not at all). `redb` and
`reflink-copy` are pure-Rust and `MIT/Apache-2.0`-family licensed; the
forbidden-crate gate (`scripts/check-kappa-pin.sh`) already checks the
`oci,p2p` tree, so nothing further is extended — but the gate is run, and the
outcome recorded in the implementation PR, because a new store stack is
exactly where a surprise would arrive.

No new configuration keys. The store directory derives from `paths.data_dir`;
the existing `replication_*` bounds keep their meanings. `DEPENDENCIES.md`
gains the feature's new entries.

## Testing

Unit:

- **The golden vector the phase rests on:** bytes imported into the
  iroh-blobs store hash to exactly the `blake3:<hex>` id `ObjectStore::put`
  assigns them.
- Address parsing: a `blake3:` object id converts to a blobs `Hash` and back
  losslessly; malformed ids are refused.
- The tag reconcile: inventory added, kept and removed produces tag set,
  kept and removed, with no tag created for an object the registry does not
  hold.
- The streaming put: hash-while-writing yields the same id as the whole-bytes
  put for the same content, including the empty input and a multi-chunk input.
- The authorization gate: an admitted `remote_id()` passes, an unadmitted one
  is closed, and a set that changes between connections is honoured per
  connection.

Feature-gated integration, extending Phase 2a's two-endpoint loopback harness:

- Two in-process endpoints; the provider's registry holds an object the
  requester's does not. After a replication round the requester's registry
  serves the identical bytes under the identical id, and the requester's blob
  store can serve it onward (the named tag exists).
- Resume: a fetch interrupted mid-object leaves partial ranges, and the next
  fetch completes the object — asserted by the second fetch transferring fewer
  bytes than the object's size (progress accounting) rather than merely by
  eventual success.
- A dialler outside the admitted set gets no bytes even for a hash it knows.
- The blobs ALPN reaches no HTTP route; the cluster ALPN is unaffected.
- Feature off: nothing changes; the stock build is what ships (the Phase 2a
  precedent — the compile-time matrix stays two builds, not three).

The honest limitation, restated from Phase 2a: loopback proves the mechanism,
not NAT behaviour; and the resume test proves range resumption inside one
process, not across a crash of the store itself.

## Out of scope

Replica sets (`r = 3` holders by rendezvous hash over the admitted set) — the
recorded follow-on this makes tractable. The multi-provider `Downloader`.
HashSeqs and collections. Push-mode transfer. Any change to the HTTP object
route or to what HTTP peers do. Replacing the inventory with set
reconciliation (the bloom/counting-sketch direction); the paged inventory
stays. A kappa-provider streaming put. Deleting objects. `iroh-gossip`,
still.

## Decisions recorded

- The inventory stays on the proof-authenticated cluster router; only bulk
  bytes move to the blobs protocol. Metadata authentication and byte
  verification are then each solved by the layer that already solves them.
- One `FsStore` per node serves both directions; named tags reconciled against
  the registry inventory keep it a cache, and the registry remains the sole
  object authority (ADR 021 untouched).
- `ImportMode::TryReference` for the provider mirror is safe because registry
  blobs are content-addressed and immutable; a store ignoring the hint copies,
  at twice the disk, and the trade is recorded rather than hidden.
- Authorization is a connection-level gate on `remote_id()` membership in the
  admitted set, in front of `provider::handle_connection` — total over every
  exchange on the connection, and no request proof, because a blobs request
  carries no claim to authenticate and bao makes the bytes self-verifying.
- The `replication_max_object_bytes` skip stays, reclassified from a safety
  bound to a local resource bound: verified streaming makes an over-size
  transfer detectable rather than dangerous.
- The store side of defect 9 is fixed here (streaming `ObjectStore` put), not
  deferred again; the kappa provider keeps whole-bytes put because its bytes
  cross the network regardless.
- Pre-1.0 `iroh-blobs` is accepted, as the parent design already decided;
  `default-features = false` plus `fs-store` only, so `rpc` and its `noq`
  subtree stay out of the graph.
- The resume claim is scoped to what the substrate guarantees: an interrupted
  fetch resumes from its verified partial ranges on a later round. Mid-chunk
  progress and crash-of-store recovery are not claimed.
