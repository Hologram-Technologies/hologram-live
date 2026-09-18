# ADR 021: Object storage runs behind a registry provider boundary

## Status

Accepted.

## Decision

`RegistryProvider` has two implementations, selected by `[registry].provider`.
`local` is the default and keeps a stock install self-contained. `kappa` speaks
to an external kappa-registry instance over its OCI blob and manifest surface.

Object identity does not change. Hologram Live addresses objects as
`blake3:<64 hex>`, and kappa-registry's kappa-label grammar treats blake3 as a
first-class axis of exactly that shape, so the same identifier is valid on both
sides and no translation exists to get wrong.

An object is one blob plus one sidecar OCI manifest. The blob holds the bytes
and its media type; the manifest holds kind, filename, and creation time as
annotations, sets `subject` to the blob it describes, and is tagged with the
object's kappa transliterated to `blake3_<hex>` so a point lookup is one
deterministic request. The manifest's own address is server-assigned and
deliberately not part of our identity contract: it is only ever reached through
its tag or through referrers.

Search is implemented in Hologram Live over upstream's paginated tag listing.
Upstream's `blobs/_meta` endpoint is an object-type index, not general metadata
search: `blob_put_meta` and `meta_query` address different tables, and the only
key ever written is `object-type`. A selective query therefore walks tag pages
client-side, bounded by `registry.max_scan_pages`; reaching that bound sets
`truncated` rather than silently returning a short page.

That walk reads one manifest per tag, because kind, filename and creation time
live in the manifest's annotations. Measured through the daemon against
kappa-registry 2af8656, one search cost 0.78 s at 459 objects and 6.96 s at
5,000, every time. The provider therefore keeps decoded records by tag. A tag is
the hash of the bytes, so the blob a record describes never changes; only its
annotations can. Three rules follow, and the tests hold each of them:

- The tag listing is never cached, so an object written by anyone (the CLI
  pushing to the registry, a second daemon on the same store) appears on the
  next search.
- A put or a rename through this provider replaces the record at once.
- A record expires after five minutes, which bounds how long a rewrite this
  daemon cannot see stays invisible. The cache holds at most 50,000 records.

Warm, the same searches take 5 ms and 29 ms. What remains is the listing itself:
one request per thousand tags per search.

## Alternatives considered

**Authoring a new object API.** Rejected. kappa-registry already defines a
content-addressed object contract that our identifiers satisfy unchanged, and a
third such API would fragment the ecosystem for no gain.

**Pointing every sidecar manifest at one synthetic namespace root**, so a single
referrers call could enumerate every object with annotations inline. Rejected.
It depends on referrers pagination that was never verified, it makes one index
grow without bound, and its failure mode is baked into persisted data: recovery
would mean rewriting every manifest. Blob-as-subject fails cheaply instead,
because an index can be added behind the search seam without touching stored
data.

**An embedded search index.** Rejected as a new primary dependency for a need
that has not been demonstrated. The scan is bounded and reports truncation.

**Renaming by writing a new manifest and deleting the old one.** Rejected in
favour of replacing the sidecar under the same tag, which removes the
non-atomic window entirely rather than documenting it.

## Consequences

- Storage moves without changing module routes, operation ids, or clients. A
  conformance suite executes one behavioural contract against both providers,
  because the seam's guarantee is only real if they are observably identical.
- Search is bounded by `registry.max_scan_pages`, and reaching that bound sets
  `truncated` rather than silently returning a short page.
- Cursors are opaque and provider-scoped, so neither side is pinned by the
  other's internals.
- Creation time is load-bearing: it is preserved across re-puts of identical
  content, because it resolves duplicate records.
- Selecting a provider that cannot be constructed fails at startup rather than
  on first request, and an unknown provider name is refused outright. Neither
  case falls back to local storage, because an operator who asked for a remote
  registry must never silently keep writing to this machine.
