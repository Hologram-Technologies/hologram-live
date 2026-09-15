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
