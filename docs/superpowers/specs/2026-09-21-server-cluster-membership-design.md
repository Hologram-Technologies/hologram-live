# Server cluster membership design

## Goal

A running Hologram server can advertise a reachable origin and join one or more
existing Hologram servers. One configured seed is enough to discover the live
set; membership keeps converging after startup and stale nodes expire.

## Boundary

This is Hologram control-plane membership, not a second replication engine.
The OCI feature's Kappa-backed volume remains the data plane for blobs,
manifests, tags, and content reconciliation. Hologram never retries a mutation
against another node because doing so would make Docker upload-session
ownership and commit state ambiguous.

## Protocol

An enabled node periodically sends its `NodeRecord` to
`POST /api/v1/cluster/join` on every known peer. The response carries the
receiver's identity and its current node directory. Newly learned, valid
origins enter the next heartbeat round, bounded by `cluster.max_peers`.

The request has a millisecond timestamp and a keyed BLAKE3 proof over
`timestamp + newline + exact request bytes`. Every node derives the key from a
dedicated secret named by `cluster.token_env`. When that environment variable
is absent, a node generates 256 random bits, stores them at
`<state_dir>/cluster.token`, and reuses the file across restarts. The file is
owner-only on Unix. Operators distribute the same secret to joining nodes over
a separate secure channel; it is never logged or exchanged by this protocol.
The receiver rejects malformed
proofs and timestamps outside a 30-second clock window before parsing or
persisting the node record. The secret itself is never transmitted, and config
validation rejects reuse of the user authentication token.

Only HTTPS origins and loopback HTTP origins are accepted. Origins may not
carry paths, query strings, fragments, or embedded credentials. The normal
HTTP body limit bounds join payloads; node identifiers, advertised operations,
and peer counts receive additional bounds.

## Lifecycle

`cluster.advertise_endpoint` defaults to `http://127.0.0.1:11435`, matching the
stock listener. The membership task starts after the listener binds, heartbeats
immediately, retains failed seeds for later retry, and stops with the server.
The persisted node directory survives restarts. Entries older than
`cluster.node_ttl_secs` are pruned, while the local node is always retained.

The CLI maps repeatable `hologram serve --join URL` flags and
`--advertise URL` onto the same configuration used by background/service
startup. A service installation normally stores those values under `[cluster]`
in `live.toml`.
