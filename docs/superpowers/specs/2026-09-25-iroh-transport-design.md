# Phase 2a: the iroh transport

## Status

Designed, not implemented. Implements #179. Depends on the Phase 1 work in #178,
whose `ClusterNetwork` trait this is the second implementation of.

Scope is deliberately the **transport only**. Replacing object replication with
`iroh-blobs` is Phase 2b and is not designed here.

## Goal

A Hologram server with no routable address joins a cluster: dialled by key,
hole-punched where the network allows it, relayed where it does not.

Phase 1 made clustering correct. It did not make a NAT'd server reachable, and
that is the gap this closes.

## What Phase 1 already provides

Nothing here is new machinery; this phase is a second implementation of an
existing seam.

- `ClusterNetwork` + `NetworkRegistry` route an address to whichever network
  claims its scheme, so a cluster may run HTTP and iroh peers at once.
- `node.key`'s 32 secret bytes construct the iroh `SecretKey` directly, so the
  EndpointId a peer dials **is** the `node_id` it has already admitted. There is
  no second identity and no mapping table.
- The request proof authenticates the *request*, not the connection, so it rides
  any transport unchanged. iroh authenticating its own connections is additional
  assurance, not a replacement.
- `ClusterRequest::max_response_bytes` obliges every implementation to abort an
  oversize body **while reading**. Phase 1 added that contract precisely so a
  second network would inherit it rather than rediscover the memory-exhaustion
  path it closed.

## Addressing

An iroh address is written `iroh:ed25519:<64 hex>`; `IrohNetwork::accepts`
matches the `iroh:` prefix. The form states the fact that matters: with iroh, the
address *is* the identity.

`recipient_for` moves from a free function to a **trait method**. Phase 1 made it
free-standing and unable to accept a node id, as the structural fix for an
escalation path where a peer's claim about a third party could re-aim a proof.
That rigidity was about *provenance* — never trust a peer's claim — not about the
function's shape. Each network now derives the recipient from the address it is
about to dial: HTTP yields the normalized origin, iroh yields the bare node id
(scheme stripped, because the recipient names an identity and `iroh:` is routing).

The receiver needs no change. Phase 1's `recipient_names_self` already accepts
either this node's `node_id` or its normalized `advertise_endpoint`, and the
node-id branch was kept specifically for this phase; the whole-branch review
confirmed it was load-bearing rather than dead.

## Serving

Under ALPN `hologram/cluster/1`, an accepted bidirectional stream becomes a
single duplex via `tokio::io::join(recv, send)` — verified: iroh 1.2's
`RecvStream` implements `tokio::io::AsyncRead` and `SendStream` implements
`AsyncWrite`. `hyper_util::rt::TokioIo` wraps it for
`hyper::server::conn::http1`, which serves the **existing axum cluster router**.
Handlers, extractors, OpenAPI annotations and tests carry over untouched, and
there is no second wire format to keep in sync.

Because the same router is served, **the request proof and admission layers
apply identically over iroh** — a dialled connection being transport-
authenticated does not exempt it from carrying a proof or from admission. iroh's
connection authentication is additional assurance on top, never a substitute, and
an implementation that skipped the proof layer for iroh peers would be a defect.

**The iroh listener serves only the cluster router, never the application
router.** This is the highest-severity requirement in this document. Mounting the
full router would place every administrative and inference route behind a proof
layer they were never designed for, reachable by anyone who can dial the
endpoint. The cluster surface is three routes; that is what is served, and a test
asserts it rather than a comment claiming it.

## Dialling

Outbound uses **hyper's client** (`hyper::client::conn::http1::handshake`) over
the same duplex rather than hand-written HTTP/1 bytes: one protocol
implementation for both directions, and no hand-rolled framing to get subtly
wrong.

`IrohNetwork::send` enforces `max_response_bytes` chunk-wise while reading, as
`HttpNetwork` does. The existing call-site checks remain as backstops.

## Configuration

```toml
[cluster]
seeds = ["https://seed.example:11435", "iroh:ed25519:<64 hex>"]
transport = "http"        # p2p feature only: http | iroh | both
discovery = "none"        # p2p feature only: none | mdns | dns
relays = []               # p2p feature only; self-hosted relays belong here
```

`validate_cluster_endpoint` currently **rejects** `iroh:` addresses. An earlier
document claimed seeds could already carry them; the whole-branch review caught
that as false. This phase is where it becomes true.

`advertise_endpoint` becomes genuinely optional when `transport` includes iroh —
a node with no routable address advertises only its EndpointId. That is the
point of the phase.

`transport = "both"` is retained despite doubling the listener surface and the
test matrix, because node-by-node migration was the entire justification for
building `NetworkRegistry`, and dropping it would strand that work.

An `iroh:` address in `seeds` while the feature is **off** must produce a clear
configuration error. Silently ignoring it would leave a node quietly not joining
the cluster its configuration says it should.

## Discovery and relays: the default follows the deployment mode

An earlier draft of this document said simply "default to off". That was reasoned
entirely from the operator-run cluster and is wrong for the other mode this
product targets.

For an **operator cluster**, off is right, and the reasoning below stands.

For a **volunteer mesh** — nodes that come and go, such as a screensaver that
starts a node while a machine is idle — off is actively wrong. Zero-configuration
internet-wide reach is the whole point of that mode: an end user installing a
screensaver cannot be asked to configure a relay endpoint, and a node whose
address nobody can resolve cannot join a public mesh at all. Shipping off as the
universal default would make that mode unusable out of the box, and the person it
fails is precisely the one who will never open `live.toml`.

So the default belongs to the mode, not to the transport:

- operator cluster: `discovery = "none"`, `relays = []`
- volunteer mesh: discovery and relays on, with publication documented as
  **inherent to participating** rather than as an opt-in risk — because it is.
  Joining a public mesh by dialling keys requires those keys to be resolvable.

That is a disclosure obligation, not a default-safety one, and it should be
stated where a participant sees it rather than buried in a config reference.

The mode mechanism itself is **not designed here**: churn-tolerant membership is
a separate epic, and this phase implements only the operator-cluster default plus
the configuration keys a mode would set. This section exists so that the default
is not mistaken for a decision that already covers both modes.

### Why off is right for an operator cluster

`discovery = "none"` with no relays. Enabling the feature never publishes
anything; an operator opts into `mdns` for a LAN cluster, or `dns` plus relays for
internet-wide reach. The reasoning, recorded because the default will be
questioned:

- **Enabling a transport is not consent to publish.** `--features p2p` says "I
  want to reach peers by key". Publishing an EndpointId and candidate addresses
  to third-party infrastructure is a different decision with different
  consequences.
- **What is published is not trivial.** pkarr/DNS discovery publishes a signed
  record mapping a node key to its direct IPs and ports, held by a third party
  and queryable by anyone holding the key — infrastructure inventory. Relays
  additionally observe connection metadata, though payloads stay end-to-end
  encrypted.
- **The error costs are asymmetric.** Defaulting off and needing traversal costs
  one config line, announces itself immediately, and is fully recoverable.
  Defaulting on and needing privacy means publication already happened, is not
  visible locally, and cannot be undone.
- **It matches this repository.** `registry.provider` defaults to `local`, the
  inference engines are off by default, `admission` fails closed, and
  `validate_cluster_endpoint` refuses everything but HTTPS and loopback. A
  default that reached outward would be the odd one out.
- **It does not cost the goal.** A NAT'd server can still join with a configured
  relay, or over mDNS on a LAN. Only the zero-configuration internet-wide variant
  needs the opt-in.

Relays need not be n0's; iroh relays can be self-hosted, and an empty default
makes that choice visible rather than pre-made.

**Required diagnostic.** A node with the feature on, `transport` including iroh,
and no discovery, no relays and no direct-address seeds is a detectable startup
condition. It must log what to enable. Phase 1 established the cost of the
alternative: an endpoint-spelling mismatch that refused every request forever
while logging nothing consumed an entire fix round.

## Dependencies

Measured, not estimated:

| | |
| --- | --- |
| iroh's tree, `default-features = false, features = ["tls-ring"]` | 332 lock entries, 225 on normal build edges |
| Net-new **lock entries** against this repository's 765 | **75** |
| `aws-lc`, `openssl`, `veilid`, `topcoat`, `rekindle` | none |

`dlopen2` appears in the lockfile and is **never compiled** — it is not in the
normal-edge build graph. Recorded here because `DEPENDENCIES.md` states the
daemon has no dynamic native plugin loader, and a reviewer grepping the lock will
otherwise reasonably conclude that boundary was broken.

Those figures come from resolving iroh alone in a scratch project and
differencing the lockfiles; the exact compiled count inside this daemon will be
lower still, since more of the tree is already shared. Confirm the real number
during implementation rather than quoting this table.

Off-by-default is what makes 75 net-new crates acceptable, following the
`llamacpp`, `candle` and `burn` precedent in ADRs 033 and 034. The stock daemon
graph is unchanged for anyone who does not opt in. `DEPENDENCIES.md` gains a
section for the feature.

`scripts/check-kappa-pin.sh` is extended to run its forbidden-crate check with
`--features p2p`. Without that, the gate most likely to catch a problem in this
phase would be blind to it.

Two things to verify during implementation rather than assume: that
`default-features = false` still yields a working endpoint, and that the compiled
graph stays free of `aws-lc` and `openssl` once iroh is a real dependency rather
than a scratch measurement.

## Testing

Unit: address parsing and `accepts`; `recipient_for` returning the bare node id
for an iroh address and the normalized origin for HTTP; the registry routing a
mixed seed list.

Feature-gated integration: **two in-process iroh endpoints over loopback**, no
relay and no discovery, serving the cluster router over one and dialling from the
other. Asserts that a join succeeds, the request proof verifies, and the response
ceiling is honoured. This is the test that proves the stack composes.

Three requirements get their own tests because prose does not enforce them:

- the iroh listener serves **only** the cluster router — a request to an
  application route over iroh does not reach a handler
- with the feature **off**, nothing changes; the stock build is what ships
- an `iroh:` seed with the feature off is a clear configuration error

**A claim this phase will not make.** A loopback iroh test proves the transport
and the plumbing. It does **not** prove hole punching or relay fallback, which
need real NAT and cannot be honestly exercised in CI. The limitation is stated
here so that a green suite is not read as coverage it does not have — Phase 1
shipped a placement test that passed on topology luck, and the lesson is cheaper
learned once.

## Out of scope

`iroh-blobs` and verified blob streaming (Phase 2b). `iroh-gossip`. Replica sets.
Signed cluster responses (#183) — an iroh connection is transport-authenticated,
so that work applies mainly to HTTP peers and its value shrinks here.

Churn-tolerant membership for ephemeral peers is a separate epic and explicitly
not addressed by this phase: this document assumes the small, stable, operator-run
cluster Phase 1 was built for. The transport itself is indifferent to churn, which
is why it can land first.

## Decisions recorded

- `recipient_for` becomes a trait method; the provenance guarantee is preserved
  because the network derives the recipient from the address being dialled.
- The iroh listener serves only the cluster router, enforced by test.
- Discovery and relays default to off, with the required startup diagnostic.
- `transport = "both"` is kept for node-by-node migration.
- 75 net-new crates, accepted because the feature is off by default.
