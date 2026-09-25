# Authenticated cluster responses

## Goal

A requester can tell who actually answered a cluster request, and that the
answer belongs to that request. Today a requester authenticates itself to a
peer, but the reply is unauthenticated: a peer list can be forged, and nothing
ties a reply to the request it claims to answer.

Implements #183. Depends on the Phase 1 work in #178, whose request-proof
machinery this mirrors.

## Why it matters

Phase 1 compensated for unauthenticated replies by binding every outbound proof
to the **origin** being dialled and refusing to trust any peer's claim about a
third party. That closed an escalation path, but it left two things unprotected
and one thing unavailable.

Unprotected: the **peer list** in a join reply, and the **inventory** of object
ids a peer claims to hold. A rogue or intercepted endpoint can shape either.

Unavailable: a verified `(endpoint, node_id)` pairing. Phase 2 (#179) needs one,
because an iroh address *is* a node id — building that phase without this one
means building on the same unverified pairing that produced Phase 1's escalation
path.

Object **bodies** are deliberately excluded: replication already verifies a
stored object's digest against the advertised `blake3:` id, so a forged body is
caught today. Signing them would duplicate that check and add a signature to the
bulk transfer path.

## The response proof

```
"dev.hologram.live.cluster-response.v1" ‖ responder_node_id ‖ request_signature
                                        ‖ status ‖ timestamp ‖ blake3(body)
```

`request_signature` is the requester's own signature from the request being
answered. It is unpredictable, unique per request, and already held by both
sides. Binding it means a reply can only answer *that* request: a genuine older
reply cannot be replayed, a reply minted for a different requester cannot be
substituted, and a join reply cannot be served as an inventory reply, because
the request each names differs.

That choice is why this needs **no new nonce, no server-side state, and no clock
dependency for freshness** — the requester compares against a signature it is
holding. `timestamp` is retained for diagnostics and for symmetry with the
request path, not as the replay defence. Do not "simplify" the preimage by
dropping `request_signature` in favour of the timestamp; the timestamp permits
replay inside its window, which is exactly what this closes.

The two headers are `x-hologram-cluster-responder` and
`x-hologram-cluster-response-signature`.

One property worth naming because it is easy to miss and easy to break: the
request signature already covers the request's method, path and canonical query,
so binding to it means the response is transitively bound to *which* request was
asked, without restating those fields in the response preimage.

The requester does **not** enforce a clock window on a response. Freshness comes
entirely from `request_signature`; the timestamp is informational, and a
verifier that started refusing responses on skew would add a failure mode
without adding a defence.

`responder_node_id` and the signature travel in headers. The requester performs
**two distinct checks**: first that the signature verifies against the node id
the response carries, and second that this identity is acceptable. Collapsing
them yields a check that proves only self-consistency and nothing about who
answered.

## What the requester does with a verified identity

- **No pairing known for this endpoint.** Verify self-consistency, record the
  pairing, proceed. This is trust-on-first-use, and the code says so: it is not
  proof the endpoint belongs to whom the operator thinks. An operator who needs
  that uses `admission = "allowlist"` with `cluster.trusted_keys`.
- **A pairing is known and the reply disagrees.** Refuse the exchange, log, and
  let the existing per-peer backoff apply by treating it as a transport-class
  failure. This is the forgery #183 names: a substituted host can no longer pose
  as the peer this node has been talking to.

  Two rules here are security-relevant and must not be "tidied" later. A
  conflicting reply **never overwrites** the stored pairing — first observation
  wins — because an attacker who could overwrite it would simply replace the
  pairing and then satisfy it. And a conflict **never evicts** the peer: eviction
  on a hostile signal would let an attacker who can answer on an endpoint remove
  a legitimate peer from this node's table, turning an authentication check into
  a denial-of-service lever. Refuse the exchange and keep the peer.
- **The reply is unsigned, or its signature fails.** Refuse. Required, not
  optional — the cluster protocol is untagged, so there is no migration window
  to design around.

**Outbound proofs stay bound to the dialled origin.** The pairing is an added
check, not a new dependency. Phase 1's property is therefore untouched, and a
wrong pairing cannot silently break authentication the way node-id binding
could.

The pairing store must **not** live in `cluster-peers.json`. That file is
deliberately endpoints-only — untrusted routing hints — and putting identities
in it would rebuild the "unauthenticated input reaches a trust decision" shape
that produced Phase 1's Critical finding. Pairings are held **in memory only**:
one exchange re-establishes a pairing after a restart, and nothing persisted
means no stale-trust file to revoke.

## Components

`proof.rs` gains `sign_response`, `verify_response`, and the two headers, beside
the request scheme — one file defines how this cluster signs anything.

`authorize_proof` begins returning the **verified request proof** rather than
only the caller's node id, because a handler needs the request's signature to
bind its reply. This is a signature change on the function every cluster request
already passes through.

The two signing handlers (`join_cluster`, `list_cluster_objects`) currently
return `Json<T>`, which serializes after the handler returns. Signing needs the
bytes, so both call one shared helper that serializes once, digests, signs,
attaches headers, and returns a `Response`. Neither hand-rolls it.

`membership.rs` gains a `PairingTable` mirroring `PeerTable`: a pure map whose
one method returns `New`, `Matches`, or `Conflict`. `run()` owns it beside
`PeerTable` and passes it to both requester call sites.

## Testing

Unit: a golden vector for the response preimage; each bound field altered in turn
and refused — responder, status, body, and especially `request_signature`, since
that is the replay defence; a reply signed by a key other than the one it claims;
the three `PairingTable` outcomes.

The two tests that pin this document's actual claims:

- An unsigned or invalidly signed reply is refused. This is a requirement, so it
  is asserted rather than left to hold by code order.
- A reply that is internally consistent but carries a different identity than
  previously observed for that endpoint is refused.

Both use the in-process axum stub peer already established in this repository,
which is controllable in a way that orchestrating interception between real
daemons is not.

All eight `tests/cluster_e2e.rs` tests must keep passing unedited.

## Rollout

A third breaking wire change, free while the cluster protocol is untagged. It
ships as its own pull request stacked on #178's branch rather than folded into
it: that branch is green and reviewed, and enlarging it would put a security
change through a review already completed. It must merge after #178.

## Recorded, deliberately not built

Under Phase 2 an iroh connection is authenticated by EndpointId, so a
message-level response proof is redundant there. The clean seam is for
`ClusterNetwork` to declare whether a response arrived transport-authenticated,
letting a requester accept that in lieu of a proof. That is one field and real
value, and it is speculative until iroh exists — so it is recorded here and the
trait does not grow the field yet.

## Out of scope

Making a peer honest about *content*: a signed inventory proves the responder
said it, not that it is true. A peer can still claim ids it lacks; the gain is
knowing exactly who lied.

Transport confidentiality: this is message-level authentication, and a network
observer still sees everything over plain HTTP. HTTPS remains that story.

Admission: a verified identity says *who*, not *whether they may*.

Epoch enforcement (#184), which additionally needs sender-side refresh and retry.

Object body signatures, for the content-addressing reason given above.
