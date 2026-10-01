# Epoch enforcement: detecting divergent ownership views

## Status

Designed, not implemented. Implements #184. Depends on Phase 1 (#178), which
put the epoch on the wire as observability only, and records why the first
enforcement attempt was reverted.

## Goal

Two admitted peers whose ownership views disagree can *detect* it, today.
Phase 1's guarantee, stated exactly, is that ownership converges under stable
membership and mutual reachability — and that a partition (or a slow
convergence window) may transiently produce two owners with nothing noticing.
The epoch is the detection mechanism. This phase wires it so a mismatch is
refused where refusal is safe, deferred where it is not, and visible in both
cases.

## Why the first attempt deadlocked

`2026-09-24-distributed-p2p-clustering-design.md` specified: receiver compares
epochs, refuses a mismatch with `409` plus its own epoch, sender refreshes and
retries. Implemented literally, every request in a forming cluster 409s
permanently. The mechanism of convergence is the join exchange itself —
refusing requests on epoch mismatch refuses exactly the traffic that would
have made the epochs converge. Any enforcement design that does not start
from this fact reproduces the deadlock.

## Decision 1: the epoch digests the ownership candidate set

Today `ownership::epoch` digests the *admitted* set. That is local trust, not
the view ownership reads. `AppState::cluster_owner` computes candidates as
*directory ∩ admitted, plus self* — so two nodes can share an admitted set,
hold different directories, answer different owners, and agree on the current
epoch all the while. An epoch that cannot detect that is observability
theatre.

The epoch's input becomes the candidate set itself — the node ids ownership
can name, sorted and digested as today. Consequences, each intended:

- **Directory pruning changes the epoch.** Correct: the candidate set changed.
- **`last_seen` and other record content do not.** Only ids are digested; a
  heartbeat refresh is not a view change.
- **It converges later than an admitted-set digest** (directory convergence
  follows admission convergence by a round) and detects more (the issue's
  "most diagnostic" option). Requirement 3's test exists to prove the
  convergence half of that trade is bounded.

Admission-view divergence alone (receiver has not admitted the dialler) never
reaches the epoch check: `authorize_cluster_request` refuses it first with
403. Epoch enforcement therefore detects what nothing else does — divergence
*among mutually admitted* peers.

## Decision 2: enforcement by route, because routes differ

**`POST /api/v1/cluster/join`: never enforced.** It is the convergence
mechanism; the deadlock above is what enforcing it costs. The request keeps
carrying the sender's epoch as observability, and a receiver logs a mismatch
at debug — useful when reading a forming cluster's logs, never a refusal.

**`GET /api/v1/cluster/objects` and `/objects/{id}`: enforced.** These carry
no role in forming membership, so refusal cannot deadlock it. After
`authorize_cluster_request` has passed — never before; a requester that has
not proved itself learns nothing, not even an epoch — the receiver compares
the request's epoch with its own and answers `409 Conflict` carrying its own
epoch in the `x-hologram-cluster-epoch` response header. A request carrying
**no** epoch header is served as today: pre-#184 senders do not send one on
object routes, and a cluster migrates node by node. Enforcement tightens only
for senders that claim a view.

## Decision 3: the sender defers to the next round, and says so once

`replication::signed_get` sets the epoch, computed from the candidate set at
round time. A 409 answers ends that peer's round as a **deferral**, a third
outcome beside success and failure:

- it is not a per-object failure (nothing about one object is wrong),
- it is not a transport/auth failure (the peer answered fine),
- and `record_replication` is **not** called, so the peer stays due and the
  next heartbeat round retries — the issue's "tolerate a mismatch and retry
  next round rather than abort". The retry is bounded by construction: one
  attempt per heartbeat per diverged peer, no tighter loop to bound further.

Logging is by transition, not by event: the first deferral against a peer
logs at info, repeats log at debug, and the first round without a mismatch
logs convergence at info. `PeerState` carries one `bool` for this. A
persistently diverged pair therefore leaves a quiet, findable trace instead
of a log line per round forever.

`contact_peer` needs no 409 handling: joins are never refused on epoch, by
decision 2.

The refresh-and-retry pattern for *future* owner-routed requests (#137's
inference/Holo forwarding) is recorded here so it is not reinvented wrong:
recompute the candidate view, re-derive the owner, retry **once**, then
surface the conflict to the caller. A caller-visible 409 there is correct —
an inference request has someone waiting to be told; an anti-entropy round
does not.

## Decision 4: the ownership surface carries the epoch it answered from

`GET /api/v1/nodes/owner` and `/api/v1/nodes/placement` gain an
`x-hologram-cluster-epoch` **response header**: the digest of the candidate
set the answer was computed from. Additive, no body shape change, and
thematically exact — the same header name the cluster protocol uses. This is
what lets a caller (and the convergence test) distinguish "same owner because
converged" from "same owner by coincidence of two different views".

## Testing

Unit:

- the candidate-set epoch: a pruned record changes it, a heartbeat refresh
  does not, an admission change does, record field noise does not;
- receiver: a mismatched epoch on an object route gets 409 with the
  receiver's epoch, *after* admission (an unadmitted request with a
  mismatched epoch gets 403 and learns nothing); a missing header is served;
  a join with a mismatched epoch is never refused on that ground;
- sender: a 409 from the inventory fetch defers the round — no
  `record_replication`, retry due next heartbeat, mismatch logging
  transitions fire once each way.

Integration (`tests/cluster_e2e.rs`), the issue's requirement 3: two daemons
started simultaneously, each seeded with the other, must reach (owner, epoch)
agreement — polled from *both* sides via the new header — within a bounded
deadline. Asserted, not assumed; and a node joining an already-converged pair
converges its epoch to theirs within a bounded deadline. The existing
placement-agreement test continues to pass, now with the stronger header
available.

## Out of scope

Leases and fencing (#180): the epoch *detects* a partition's double-owner,
nothing here prevents one. Owner-routed request forwarding itself (#137) —
this phase establishes the retry pattern it will use. Authenticated cluster
responses (#183); the 409 body is unsigned like every cluster response today.
Changing what `Admission` admits, or how the directory converges.

## Decisions recorded

- The epoch's input moves from the admitted set to the ownership candidate
  set (directory ∩ admitted ∪ self). No interop break: the header was
  observability-only, so no receiver ever compared the old value.
- Joins are never epoch-enforced; object routes are, after admission, with
  missing-header tolerance for mixed-version clusters.
- A mismatched replication round defers to the next heartbeat rather than
  consuming the replication interval or aborting other peers.
- Owner/placement answers carry the epoch they were computed from.
- Owner-routed forwarding (#137) gets refresh-recompute-retry-once, then a
  caller-visible conflict.
