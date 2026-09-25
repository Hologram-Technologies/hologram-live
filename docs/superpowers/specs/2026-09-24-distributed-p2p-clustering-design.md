# Distributed P2P clustering

## Goal

A Hologram server joins a distributed network of other Hologram servers without
requiring a publicly routable address, proves who it is with a key rather than a
shared secret, and keeps converging as members come and go. One reachable seed,
or one relay, is enough to enter the network.

This supersedes the guarantees stated in
`2026-09-21-server-cluster-membership-design.md`. That document describes the
seed-based HTTP mesh as it was intended; the audit below records why it cannot
deliver distributed membership as written.

## Why the current implementation cannot deliver it

The following are defects in the shipped code, not missing features. File
references are against `1b7e610`.

1. **Every stock node shares one identity.** `src/app.rs:129-135` derives
   `server_id = blake3(listen ‖ data_dir ‖ role)`. Two default installations on
   different hosts produce a byte-identical `node_id`. `NodeDirectory`
   (`src/nodes.rs`) is keyed by `node_id`, so the two entries collapse into one
   and each heartbeat overwrites the other. Worse, `src/cluster.rs:167` compares
   `response.node.node_id != self_node.node_id` to decide whether a reply came
   from a peer, so each node concludes the other *is itself* and never records
   it. Out-of-the-box distributed clustering is not merely insecure; it does not
   function.
2. **No path through NAT.** `validate_cluster_endpoint` (`src/config.rs:1092`)
   admits only HTTPS origins and loopback HTTP. A node behind NAT, CGNAT, or a
   dynamic address has no way to be dialled and therefore no way to join.
3. **One shared symmetric secret is the whole trust model.** Any holder of the
   cluster token may claim any `node_id`. There is no per-node key, no
   authorization decision, and no revocation short of rotating the secret across
   every member simultaneously.
4. **The read proof is a replayable bearer token.** `signed_get`
   (`src/cluster.rs:389`) signs only `(timestamp, empty body)`. The resulting
   header pair authenticates no method, path, query, or recipient, so a captured
   proof replays for the full 30-second window against any cluster `GET` on any
   peer in the cluster.
5. **Ownership is grindable.** `ownership::owner` (`src/ownership.rs:23`)
   rendezvous-hashes over `node_id`, and `validate_node_record`
   (`src/cluster.rs:440`) only length-checks that field. Any party that can join
   chooses `node_id` values that hash to the resources it wishes to own,
   including the `capable_owner` placement path that selects where inference and
   Holo execution run.
6. **Ownership is computed from disagreeing inputs.** `AppState::cluster_owner`
   (`src/app.rs:234-245`) derives the owner from the node's own directory. Two
   nodes with different views select different owners for the same resource.
   There is no lease and no fencing token, so nothing detects or resolves it.
7. **The peer set never shrinks and is lexicographically starved.**
   `src/cluster.rs:142` iterates `peers.iter().take(config.max_peers)` over a
   `BTreeSet<String>` that is only ever inserted into. Once more than
   `max_peers` origins are known, the node contacts the alphabetically first
   `max_peers` forever, whether or not they are alive, and never reaches the
   rest.
8. **A restart forgets the network.** `src/cluster.rs:129` seeds the working
   peer set from `config.seeds` alone. The persisted `nodes.json` is reloaded
   into the directory but never back into the peer set, so a restarted node
   configured without seeds is isolated despite knowing its peers on disk.
9. **Replication is O(n²) and fragile.** Every node pulls every peer's full
   object inventory on every heartbeat, buffers each object wholly in memory,
   and aborts the entire round for a peer on the first per-object error.

## Substrate

**Chosen: iroh 1.x.** Its endpoint identity is an ed25519 public key, which is
the address; connections are QUIC with TLS 1.3 and cannot be established under a
forged identity. It hole-punches directly and falls back to relays, which is
precisely the capability defect 2 lacks. It is `MIT OR Apache-2.0`, matching
`deny.toml` without an exception, and its MSRV of 1.91 is below this repository's
1.95. On `default-features = false` with `tls-ring` it pulls neither `aws-lc` nor
`openssl`, so `scripts/check-kappa-pin.sh` stays green. `iroh-blobs` transfers
content-addressed blobs with BLAKE3/bao verified streaming, and the registry
already addresses objects as `blake3:…`.

The honest caveat: `iroh` core is 1.0 with a stable wire protocol, but
`iroh-blobs` (0.103) and `iroh-gossip` (0.101) remain pre-1.0 and will break
API across releases. This design takes a dependency on `iroh-blobs` and
deliberately does not take one on `iroh-gossip`.

**Not the default, but not excluded: Veilid.** An earlier draft of this document
called Veilid rejected on the grounds that `scripts/check-kappa-pin.sh:50`
already fails the build when `veilid` appears in the dependency graph. That gate
is narrower than the claim: it runs `cargo tree --package hologram-live
--features oci`, so it forbids Veilid from the *default and oci* graph, not from
the crate's optional feature set. Behind an off-by-default feature, Veilid would
clear that gate as written.

Two real reservations remain. `veilid-core` is MPL-2.0, and whether
`cargo-deny-action` flags an optional, off-by-default dependency depends on its
feature resolution — unverified here, and to be settled before any Veilid
feature merges. On technical merit its anonymity routing adds latency this
workload does not need, and its DHT stores small records rather than blobs, so
it is a poor fit for object replication even when it is a fine fit for reaching
a peer.

Veilid is therefore a *candidate implementation of the network trait below*,
not the default transport and not a rejected option.

**Not primary: Reticulum.** It targets high-latency, low-bandwidth austere
links such as LoRa and packet radio. The Rust ecosystem is fragmented across
several incomplete implementations with an open reference-parity effort as of
2026-09-20. It remains a plausible future implementation of the network trait for
disconnected or edge deployments, and no implementation of it is in scope here.

## The network is a trait

No transport is privileged. The cluster speaks to peers through one trait, and
HTTP, iroh, Veilid, Reticulum, or anything later are implementations of it that
can coexist in one running cluster.

```rust
pub trait ClusterNetwork: Send + Sync {
    /// URL scheme or address prefix this network claims, e.g. "https", "iroh".
    fn scheme(&self) -> &'static str;
    /// This node's address on this network, as peers should record it.
    fn local_address(&self) -> Option<String>;
    /// Whether this network can reach the given address.
    fn accepts(&self, address: &str) -> bool;
    /// One request/response exchange with a peer.
    async fn send(&self, peer: &str, request: ClusterRequest)
        -> Result<ClusterResponse>;
    /// Serve inbound exchanges into the shared cluster service.
    async fn listen(&self, service: ClusterService) -> Result<()>;
    /// Addresses learned without configuration. Empty is a valid answer.
    async fn discover(&self, limit: usize) -> Result<Vec<String>> { Ok(Vec::new()) }
}
```

Three properties make this a thin seam rather than a second protocol.

**Identity is not the network's business.** A node's `node_id` is its ed25519
public key on every network. The proof in the section above authenticates the
*request*, not the connection, so it is carried unchanged over any transport. A
network that happens to authenticate its own connections — iroh does, because the
EndpointId *is* the key — is a belt-and-braces bonus, not a prerequisite. A
network that authenticates nothing, such as plain HTTP, is equally safe here
because the proof does not depend on it.

**The service is not the network's business either.** `ClusterService` is the
existing axum cluster router. Every implementation serves the same handlers over
whatever byte stream it has, so adding a network adds addressing, dialling, and
discovery — never a parallel set of handlers, and never a second wire format to
keep in sync.

**Addresses are scheme-tagged and a cluster may be mixed.** A `NetworkRegistry`
holds the active implementations and dispatches an address to whichever one
`accepts` it. `https://node.example:11435` goes to the HTTP network,
`iroh:ed25519:…` to the iroh network. This is what lets a cluster migrate one
node at a time instead of in a flag day, and it is why `cluster.seeds` is a list
of addresses rather than a list of URLs.

Phase 1 defines the trait and lands exactly one implementation, HTTP, wrapping
the reqwest client and axum router the daemon already has. That is deliberate:
the extraction is verifiable against a working, tested cluster, and Phase 2 then
adds iroh as a second implementation rather than as a refactor of the first.

## Can kappa-registry handle the servers?

Not the membership, and it already handles the objects.

kappa-registry is the object *data plane* behind `RegistryProvider` (ADR 021):
content-addressed blobs, sidecar OCI manifests carrying kind and filename as
annotations, and paginated tag listing. Object identity is already
`blake3:<64 hex>` on both sides.

It is the wrong substrate for membership for four reasons, each measurable
rather than aesthetic. Heartbeats are frequent small mutable writes, and a
kappa record is addressed by the hash of its bytes, so every heartbeat rewrites
a tag. Listing members means a tag walk that reads one manifest per tag — ADR
021 measured 0.78 s at 459 objects and 6.96 s at 5,000 — which a 15-second
heartbeat cannot absorb. There is no TTL, so pruning becomes a walk-and-delete.
And there is no compare-and-swap, so it cannot provide the ownership fencing
that the epoch check only detects.

The decisive objection is architectural: routing membership through one registry
endpoint makes a "distributed P2P network" depend on a central service, which is
the property this design exists to remove. `registry.provider` also defaults to
`local`, so most installations have no kappa at all.

There is one genuinely good use, and it is recorded here as a future option
rather than built now: a kappa-backed **rendezvous directory**, where a node
publishes its address as a small object under a well-known tag so a new node can
bootstrap without a hand-configured seed. That keeps kappa out of the heartbeat
path while using exactly what it is good at — durable, shared, content-addressed
storage. It would be an implementation of `ClusterNetwork::discover`, not a
replacement for membership.

## Phase 1 — a cluster that works, over HTTP

Phase 1 changes no transport. It makes membership correct and authenticated so
that Phase 2 is a transport swap rather than a rewrite.

`src/cluster.rs` splits into a `src/cluster/` directory — `identity.rs`,
`admission.rs`, `membership.rs`, `replication.rs`, and `mod.rs` — so each unit is
separately testable and each file stays well inside the 1500-line gate.

### Identity

A node holds an ed25519 keypair at `<state_dir>/node.key`, created on first
start with mode 0600, using the same create-or-load discipline as today's
`cluster.token`. Its identity is `node_id = "ed25519:<64 hex characters>"`.

The derived `server_id` of `src/app.rs:129-135` is removed.
`CapabilityManifest.server_id` is populated from the same key, so the system
carries exactly one node identity. This changes the *value* of a public wire
field, not its shape.

In Phase 2 these same 32 bytes construct the iroh `SecretKey`. The EndpointId a
peer dials is therefore identical to the `node_id` it already admitted: no
second namespace and no mapping table.

### Admission

Admission is one trait, and it is the seam that lets a closed cluster become an
open network later without touching transport or membership.

```rust
trait Admission {
    fn authorize(&self, node: &NodeRecord, proof: &JoinProof) -> Decision;
}
```

Two implementations ship in Phase 1.

*Allowlist* — `cluster.trusted_keys` enumerates the admitted `ed25519:…`
identities. This is the ground truth for a closed cluster.

*Token trust-on-first-use* — the existing shared secret is demoted from being
the entire trust model to being an admission ticket. A node not yet known may
present a token-signed join, which pins its public key into the local allowlist
once. Every subsequent byte is authenticated per-key. Compromise of the token
then permits admitting new nodes but no longer permits impersonating an existing
one, which is a strict improvement on the current behaviour.

A later capability-grant implementation — signed grants, quotas, per-operation
scopes — satisfies the same trait. That is the whole of the open-network seam.

### Request authentication

Every cluster request carries `x-hologram-cluster-node`,
`x-hologram-cluster-timestamp`, `x-hologram-cluster-signature`, and
`x-hologram-cluster-recipient` — the last of these **required**, with no default
and no fallback, since a recipient a receiver inferred for itself would defeat
the point of binding one. It is the second breaking wire change in this phase,
and it ships with the first. `x-hologram-cluster-ticket` carries the admission
ticket where one is presented. The signature is ed25519 over the canonical
preimage

```
"dev.hologram.live.cluster.v2" ‖ method ‖ path ‖ canonical_query
                               ‖ recipient_node_id ‖ timestamp ‖ blake3(body)
```

`canonical_query` is the request's query parameters sorted by key, then by
value, each percent-encoded and joined with `&`; an absent query signs as the
empty string. `blake3(body)` covers the empty body for requests that carry none.
`JoinProof` is a token signature under the trust-on-first-use implementation and
is absent under the allowlist implementation.

Binding the method, path, query, and recipient is what closes defect 4: a
captured proof no longer replays against a different route or a different peer.
The receiver verifies the signature against the claimed identity, consults
`Admission`, checks the timestamp window, and only then parses or persists
anything.

Accepted residual risk: replay against the *same* endpoint within the clock
window remains possible. Cluster reads are idempotent, so this design accepts it
rather than carrying a per-node seen-set. It is recorded here so a future
reviewer does not mistake it for an oversight.

### Membership

The working peer set is initialised from the persisted `nodes.json` union
`config.seeds`, which closes defect 8.

Each peer carries `PeerState { endpoint, consecutive_failures,
next_attempt_millis }` with exponential backoff capped at `node_ttl_secs`. A
peer is evicted when the directory prunes it, unless it is a configured seed:
seeds are the recovery path and are never dropped.

Each round contacts `min(cluster.fanout, eligible)` peers selected by a rotating
cursor over the sorted set, always including peers never yet contacted and any
seed whose backoff has elapsed. This replaces the lexicographic truncation of
defect 7 and reduces the round from O(n²) to O(n·fanout). `cluster.fanout`
defaults to 8; `max_peers` remains the directory bound.

### Ownership: the guarantee, stated exactly

The candidate set narrows to *admitted members plus this node itself*, which
ends outsider grinding of placement decisions. Self-trust is not a convenience:
`Admission` only ever names *other* parties — `cluster.trusted_keys` is
configured about peers, and a pin records a peer that presented a ticket — so
without it a node's own identity is in nobody's admitted set as far as that node
can see. As implemented, a default single-node install then owned nothing at
all, and a joiner could not even place an operation that only it advertises
(`/api/v1/nodes/placement` answered `404` on the joiner while answering `200` on
the seed). Nothing authenticates a node to itself, so self-trust is added where
ownership is decided — `AppState::admitted_with_self` — and not inside an
`Admission` implementation, which stays solely about authenticating others.

What this does not fix, and what rendezvous hashing over self-chosen identifiers
cannot fix, is an already-admitted member generating keypairs until one hashes
to a resource it wants to own. Within a closed cluster that member is trusted by
construction. This is a recorded property of the design, not an open defect.

Self-trust also makes an *asymmetric* `admission = "allowlist"` configuration
fail open rather than closed: a node whose `trusted_keys` is narrower than its
peers' still names an owner — some candidate its own list admits — for keys a
wider-listed peer assigns elsewhere, and no node can detect the disagreement
locally. The allowlist must be configured symmetrically.

A `membership_epoch` — a digest over the sorted admitted set, self included —
rides every outbound cluster request in `x-hologram-cluster-epoch`. A digest
carries no ordering, so no receiver could decide which of two epochs is newer
anyway.

**Correction, against the implementation as shipped: the epoch is computed and
sent, and no receiver enforces it.** An earlier draft of this section said the
receiver compares it for equality and refuses a mismatch with `409`, returning
its own epoch so the sender refreshes membership and retries. That was
implemented and then reverted, because it broke a working two-node cluster
deterministically — every request was answered `409` and membership never formed.
The reason is that the epoch is a digest of the *admitted* set, which is local
trust, not a converging membership view. Admission is reached one direction at a
time: a joiner proves itself to a seed, and the seed only admits the joiner back
after it has noticed the joiner in its own directory and dialled it, a round
later. Under `admission = "token"` the two sides' digests are therefore unequal
for at least one round after any membership change — and a hard equality check
refuses exactly the requests that would have made them converge, so it 409s
permanently rather than transiently. Enforcing it safely needs sender-side
refresh-and-retry on a mismatch, which is tracked as issue #184. Until then the
header is observability only.

The guarantee Phase 1 ships is therefore, stated exactly: **ownership converges
under stable membership, and a partition may transiently produce two owners.**
Nothing currently detects or resolves that; the epoch would be the detection
mechanism once #184 lands. Exclusive mutable ownership needs leases and fencing
tokens, which are out of scope here and tracked as issue #180.

### Replication

Phase 1 fixes only what Phase 2 will not delete. `iroh-blobs` replaces the
transfer loop wholesale, so the in-memory buffering is left alone rather than
being rewritten twice; `replication_max_object_bytes` already bounds it.

Per-object failures log and continue instead of aborting the peer's round; only
an inventory-level transport failure ends a round. Anti-entropy moves to its own
cadence, `cluster.replication_interval_secs` (default 60), decoupled from the
15-second heartbeat, and runs only against the fanout subset.

Sharding is not introduced in Phase 1: every admitted node still converges on
every object.

## Phase 2 — the iroh transport

Phase 2 is gated behind a `p2p` Cargo feature that is **off by default**,
matching the precedent of the `llamacpp`, `candle`, and `burn` engine features in
ADRs 033 and 034. The stock daemon graph and the `DEPENDENCIES.md` budget are
unchanged for anyone who does not opt in. It carries its own ADR.

The endpoint is built from the Phase 1 key, so identity is continuous across the
transport change. `iroh` is taken with `default-features = false` plus
`tls-ring`, dropping `portmapper`, `fast-apple-datapath`, and `metrics`, which
keeps the graph lean and keeps the forbidden-crate gate green.

**One handler set, not two.** Under ALPN `hologram/cluster/1`, an accepted iroh
bidirectional stream is handed to `hyper::server::conn::http1` serving the same
axum cluster router that the HTTP listener serves. Handlers, extractors, OpenAPI
annotations, and the existing tests carry over unchanged. Outbound requests go through the
`ClusterNetwork` trait defined above — iroh becomes a second implementation
beside HTTP in the `NetworkRegistry` — so `membership.rs` and `replication.rs`
never branch on transport.

A peer address becomes `PeerAddress::{ Origin(Url), Endpoint(EndpointId) }`, and
`advertise_endpoint` becomes optional when `p2p` is active. A node with no
routable address advertises only its EndpointId and remains fully reachable.
That is the substantive answer to the goal.

Object replication moves to `iroh-blobs`: BLAKE3/bao verified streaming,
resumption, and range requests, with the registry's existing `blake3:` ids
mapping onto blob hashes. Replication becomes "fetch these hashes", which is
also what makes replica sets — `r = 3` holders chosen by rendezvous hash over
the admitted set — tractable as a follow-on.

**Discovery is a deployment decision, not a default.** iroh's stock relays and
pkarr/DNS discovery publish an EndpointId and its candidate addresses to
n0-operated infrastructure, which a closed or air-gapped deployment may not
accept. `cluster.discovery = "none" | "mdns" | "dns"` and `cluster.relays`
for self-hosted relays are therefore part of this design. `discovery = "mdns"`
with no relays yields a fully self-contained LAN cluster.

`iroh-gossip` is deliberately not adopted. Epidemic membership earns its keep
well beyond the node counts this system targets, and it would add a second
pre-1.0 crate and a parallel non-HTTP handler surface. It is revisited if and
when measured round cost justifies it.

## Configuration

All additions; `deny_unknown_fields` makes additive change safe, and existing
`[cluster]` blocks keep working unchanged.

```toml
[cluster]
seeds = ["https://seed.example:11435", "ed25519:…"]  # origins and EndpointIds
trusted_keys = ["ed25519:…"]
admission = "token"          # token (TOFU, default, back-compatible) | allowlist
fanout = 8
replication_interval_secs = 60
transport = "http"           # p2p feature only: http | iroh | both
discovery = "none"           # p2p feature only: none | mdns | dns
relays = []                  # p2p feature only
```

`token_env` retains its current meaning, now scoped to admission.

## Testing

Unit coverage: identity round-trip and file mode; golden vectors for the
canonical signing preimage; rejection of a signature whose method, path, query,
recipient, or body was altered; each `Admission` decision; the backoff and
eviction state machine; that the fanout cursor reaches every peer within
⌈n / fanout⌉ rounds; that the epoch digest is stable over the sorted admitted
set (there is no supersession to test — see the correction above).

Integration coverage in `tests/`: a three-node in-process loopback cluster that
converges, prunes stale members, **recovers from restart with no configured
seeds**, refuses an unadmitted node, and refuses a proof replayed from one peer
to another. Cucumber features cover the public boundary per repository
convention.

Phase 2 adds a two-node iroh test in which neither node has a routable address,
asserting membership convergence and object digest equality after replication.

`just verify` must pass, and `scripts/check-kappa-pin.sh` is extended to run its
forbidden-crate tree check with `--features p2p` so nothing prohibited enters
through iroh.

## Rollout

Phase 1 lands on HTTP and running clusters keep running. Node identity changes
exactly once; entries under the old derived identifiers age out through
`node_ttl_secs`, and the migration is documented in the release notes.

Phase 2 ships behind `--features p2p` with `transport = "both"`, so an operator
migrates a cluster node by node rather than in a flag day.

## Out of scope

Any `ClusterNetwork` implementation other than HTTP (iroh is Phase 2; Veilid and
Reticulum are later and optional). A kappa-backed rendezvous directory. Leases
and fencing tokens for exclusive mutable ownership (Phase 3). Replica-set
sharding of objects (follows Phase 2). `iroh-gossip` membership. Reticulum as a
transport. Any change to the OCI feature's Kappa-backed data plane, which
remains the authority for blobs, manifests, and tags.

## Decisions recorded for review

- `CapabilityManifest.server_id` changes value to the node's public key, and
  `x-hologram-cluster-recipient` becomes a required request header. Both are
  breaking wire changes and ship together, so there is no mixed-version window.
- The membership epoch is computed and sent but not enforced (issue #184).
- Ownership candidates are admitted members *plus self*; an asymmetric
  `allowlist` therefore fails open, and must be configured symmetrically.
- Streaming object transfer is deferred to Phase 2 rather than fixed twice.
- `p2p` is off by default, so P2P is opt-in at compile time, not in a stock
  binary.
- Same-endpoint replay within the clock window is accepted for idempotent reads.
- The network is a trait from Phase 1, with HTTP as its only implementation
  there, so Phase 2 adds a transport instead of refactoring one.
- Veilid is reclassified from rejected to an optional future implementation; the
  MPL-2.0 question under `cargo-deny` must be settled before such a feature
  merges.
- kappa-registry stays the object data plane and does not become the membership
  store; a rendezvous directory is left as a future `discover` implementation.
