# ADR 020: Network authority is endpoint-scoped before mediation

- Status: accepted; capability schema implemented, fetch profile shipped by ADR 021
- Date: 2026-09-01

## Context

The upstream `CapabilitySet` historically carried `network_fetch` and
`network_announce` booleans. A boolean cannot express which authority, port, or
path an application may contact, cannot be meaningfully narrowed for a child,
and would turn any linked socket or HTTP client into ambient network access.
Live therefore never linked a network import for those fields.

Capability objects are content-addressed canonical bytes. Reinterpreting an
existing `true` flag as a wildcard endpoint would silently widen old authority;
changing the encoding of ordinary no-network sets would unnecessarily change
existing application identities.

## Decision

### Canonical endpoint scope

The upstream capability view replaces both booleans with ordered sets of
`NetworkEndpointScope` values:

- `network_fetch_endpoints`
- `network_announce_endpoints`

One scope has exactly the byte form
`https://<lowercase-dns-host>:<explicit-port>/<path-prefix>`. Only HTTPS is
representable. User information, implicit or zero ports, uppercase or invalid
hosts, query strings, fragments, percent escapes, repeated slashes, dot
segments, non-ASCII bytes, and noncanonical port spelling are rejected. The
path alphabet is limited to `/`, ASCII alphanumerics, and `-._~`.

Host and port containment are exact. A child path must equal its parent's path
or extend it on a segment boundary: `/v1` admits `/v1/models`, but not `/v10`.
Every child scope must be contained by some parent scope. Source lists are
strictly lexically sorted and duplicate-free; upstream canonicalization also
sorts and deduplicates defensively.

### Compatibility and fail-closed migration

The canonical `CapabilitySet` retains its legacy flag byte at zero. If both
endpoint sets are empty, its bytes are unchanged, preserving existing
no-network capability and application identities. Scoped sets append a tagged
`NEP1` extension. An older reader observes the zero flags and therefore grants
no network authority. A new reader rejects legacy nonzero flags rather than
mapping them to a wildcard.

Human-authored `capabilities.json` uses schema 2 for endpoint lists. Schema 1
remains accepted only when its legacy network booleans are absent or false, so
existing deny-all and storage/channel-only sources continue to compile to the
same canonical bytes. A schema-1 `true` value fails during compilation with a
replacement diagnostic.

Endpoint contents are authority and may reveal internal topology. Admission
errors, traces, audit rows, and run reports expose only capability-object
identities and endpoint counts, never endpoint strings.

### No network interface in this capability slice

This decision makes authority sufficiently specific; it does not add a guest
network interface. Raw WASI sockets, DNS, HTTP, listen sockets, and host client
handles remain unavailable from the capability schema alone. ADR 021's
mediated-fetch contract uses a new fixed guest-contract selector and links only
after the request has been admitted and a nonempty fetch scope retained.

The ADR 021 mediator parses the requested URL into the same canonical
origin/path model, disable automatic redirects or reauthorize every redirect,
resolve DNS through host policy, reject resolved addresses forbidden by that
policy on every connection, re-check authority after resolution, and enforce
bounded request bytes, response bytes, duration, and concurrency. Credentials,
cookies, proxy settings, ambient certificate identities, and host environment
must not be inherited. Announce/listen semantics require a separate decision
and profile; fetch authority cannot be reused for them.

## Consequences

- Network authority can be attenuated across parent/child application trees.
- Existing canonical no-network objects retain their κ values.
- Legacy ambient flags fail closed on new runtimes and new scoped sets fail
  closed on old runtimes.
- Source schema 2 is required to request network authority; schema 1 remains a
  compatibility input only for no-network requests.
- No application can perform a network operation merely because this schema
  exists.
- ADR 021 adds a bounded mediated-fetch profile; announce/listen and raw sockets
  remain unavailable.

## Verification

Upstream tests cover canonical syntax, malformed inputs, exact-origin and
segment-boundary containment, stable sorting/deduplication, legacy no-network
round trips, legacy ambient-flag rejection, boundary checks, `no_std`, and the
workspace consumers.

Live tests cover schema-1 compatibility and rejection, schema-2 compilation,
ordering and syntax diagnostics, canonical archive decoding, empty baseline
denial, matching and narrowed development grants, sibling-path denial,
parent/child attenuation, redacted audit output, and unchanged provider
preparation behavior. The normal format, workspace, test, Clippy, BDD,
documentation, release, smoke, and clean Component reproducibility gates remain
required.
