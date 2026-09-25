# ADR 035: A cluster node is an ed25519 key, and membership is admitted per key

- Status: accepted
- Date: 2026-09-25
- Supersedes: the shared-secret trust model of
  `docs/superpowers/specs/2026-09-21-server-cluster-membership-design.md`

## Context

An audit of the shipped cluster code found nine defects. They are recorded in
full in `docs/superpowers/specs/2026-09-24-distributed-p2p-clustering-design.md`;
in brief: one identity for every stock node; no path through NAT; one shared
symmetric secret as the whole trust model; a read proof that authenticates
neither method, path, query nor recipient, so a captured proof replays for its
whole window; ownership grindable by anyone who can choose a `node_id`;
ownership computed from disagreeing directories; a peer set that never shrinks
and is lexicographically starved; a restart that forgets the network; and
replication that is O(n²) and aborts a peer's round on the first bad object.

The blocking one is the first. `src/app.rs` derived
`server_id = blake3(listen ‖ data_dir ‖ role)`, so two default installations on
different hosts produced a byte-identical `node_id`. `NodeDirectory` is keyed by
`node_id`, so the two collapsed into one entry that each heartbeat overwrote,
and `cluster.rs` compared `response.node.node_id != self_node.node_id` to decide
whether a reply came from a peer — so each node concluded the other *was itself*
and never recorded it. Out-of-the-box clustering was not merely insecure; it did
not function, and no amount of work on the other eight defects could be observed
until identity was per node.

## Decision

A node's identity is an ed25519 keypair at `<state_dir>/node.key`, created on
first start with mode 0600 under the same create-or-load discipline as
`cluster.token`, and its `node_id` is `ed25519:<64 hex>`. The derived `server_id`
is deleted; there is exactly one identity in the process.

Every cluster request carries a proof: an ed25519 signature over the canonical
preimage binding a context string, the method, the path, the canonical query,
the recipient, the timestamp, and `blake3(body)`. Both sides build the preimage
with the same function, and the receiver verifies before it parses or persists
anything. Binding the recipient is what stops a proof captured from one peer
being replayed at another; binding method, path and query stops it being replayed
at a different route. Replay against the *same* endpoint inside the clock window
remains possible and is accepted, because cluster reads are idempotent.

Who may participate is one trait, `Admission`, with two implementations.
*Allowlist* admits the `ed25519:…` identities in `cluster.trusted_keys` and
nothing else. *Token trust-on-first-use* demotes the shared secret from being
the trust model to being an admission ticket: an unknown node presenting a
ticket derived from the token and its own claimed identity is pinned once into
`cluster-pinned.json`, and every byte after that is authenticated per key.
Compromise of the token then permits admitting new nodes, not impersonating
existing ones. A future capability-grant implementation satisfies the same trait,
which is the whole of the open-network seam.

Ownership candidates are the admitted members **plus this node itself**. Self is
required, not a convenience: `Admission` only ever names other parties, so
without self-trust a default single-node install had no owner for anything, and a
joiner answered `404` for an operation only it advertised. Self-trust is applied
where ownership is decided (`AppState::admitted_with_self`), never inside an
`Admission` implementation, which stays about authenticating others.

## Two breaking wire changes, shipped together

There is deliberately no mixed-version window: a node on the old code and a node
on the new one do not form a cluster, and both changes land in the same release
so no intermediate combination has to be reasoned about.

1. `CapabilityManifest.server_id` now carries the ed25519 public key. This is a
   change of *value*, not of shape: the field is still a string, and clients that
   only echo or compare it keep working. Anything that parsed the old
   `blake3:<hex>` form, or persisted it as a join key, does not.
2. `x-hologram-cluster-recipient` is a new **required** request header on every
   cluster route. There is no default and no fallback, because a recipient the
   receiver inferred for itself would defeat the point of binding one; a request
   without it is refused.

## The guarantee, stated exactly

Ownership **converges** under stable membership. A partition may transiently
produce two owners. Nothing in this ADR prevents that, and nothing currently
detects it. Exclusive ownership needs leases and fencing tokens, which are out of
scope and tracked as issue #180. This is stated narrowly on purpose: "converges"
is not "is exclusive", and the difference is the whole of #180.

## What this deliberately does not solve

- **Cluster responses are unauthenticated.** A request is proven; a reply is not.
  A peer list in a join response can therefore be forged by whatever answers on
  a dialled origin, which is why nothing is admitted on the strength of a
  response — admission needs a ticket-bearing, signed request in the other
  direction. Issue #183.
- **The membership epoch is computed and sent, and not enforced.** The digest
  rides every request in `x-hologram-cluster-epoch` and no receiver compares it.
  Enforcement was implemented and reverted: the epoch digests the *admitted* set,
  which is local trust rather than a converging membership view, so under token
  admission the two sides disagree for at least one round after any membership
  change — and refusing those requests refuses exactly the ones that would have
  made them agree, which 409'd a working cluster permanently rather than
  transiently. Safe enforcement needs sender-side refresh-and-retry. Issue #184.
- **There is no in-band revocation of a ticket-pinned identity.** Removing a key
  from `cluster.trusted_keys` and restarting revokes it, because configured keys
  are rebuilt from configuration and never persisted. A pinned key has no such
  entry: revoking one means editing `cluster-pinned.json` on each node.

## Operator notes

Both of these cost real debugging time when they are not written down.

**`admission = "allowlist"` must be configured symmetrically.** Because ownership
includes self, a node whose `trusted_keys` is narrower than its peers' does not
fail closed — it names an owner from among the candidates its own list admits,
for keys a wider-listed peer assigns elsewhere. The cluster then disagrees about
ownership while every node believes it has answered correctly, and no node can
detect the disagreement locally.

**A cluster that will not form should be run with `HOLOGRAM_LOG=debug`, looking
for the misaddressed-request line.** It names the recipient the caller signed
against and the node id and endpoint this node accepts, which is what identifies
a seed spelled differently from the peer's `cluster.advertise_endpoint`. Under
the default `admission = "token"`, a *first*-contact mismatch logs at `debug`,
not `warn`: a proof that verifies says nothing about who sent it, so warning
unconditionally would let an unadmitted caller drive unbounded warn-level
writes. Only a mismatch between two nodes that already admit each other is a
`warn`.

## Consequences

- One new dependency, `ed25519-dalek` (BSD-3-Clause, already an allowed licence
  in `deny.toml`), recorded in `DEPENDENCIES.md`.
- Node identity changes exactly once. Entries under the old derived identifiers
  age out through `node_ttl_secs`.
- The same 32 secret bytes construct the iroh `SecretKey` in Phase 2, so the
  EndpointId a peer dials is the `node_id` it already admitted: no second
  namespace and no mapping table.
- `<state_dir>` gains two files to back up and to keep unreadable by others:
  `node.key`, which *is* the node, and `cluster-pinned.json`.
- `src/cluster.rs` became `src/cluster/` (`identity`, `admission`, `proof`,
  `membership`, `replication`, `mod`), so each unit is separately testable and
  every file stays inside the 1500-line gate.

## Alternatives considered

**Veilid as the substrate.** Not rejected on the gate that an earlier draft
claimed: `scripts/check-kappa-pin.sh` runs `cargo tree --package hologram-live
--features oci`, so it forbids `veilid` from the default and `oci` graphs only,
and an off-by-default feature would clear it as written. Two real reservations
stand. `veilid-core` is MPL-2.0, and whether `cargo-deny` flags an optional,
off-by-default dependency depends on its feature resolution — unresolved, and to
be settled before any such feature merges. On technical merit its anonymity
routing adds latency this workload does not need, and its DHT stores small
records rather than blobs, so it is a poor fit for object replication even where
it is a fine fit for reaching a peer.

**Reticulum as the substrate.** It targets high-latency, low-bandwidth links such
as LoRa and packet radio, which is not this workload, and its Rust ecosystem is
fragmented across several incomplete implementations with an open
reference-parity effort.

Neither is a rejection of the library. Both are candidate implementations of the
`ClusterNetwork` trait — the transport seam this phase extracts next, alongside
HTTP and, in Phase 2, iroh — and that trait is what keeps such a choice from
being a rewrite.
