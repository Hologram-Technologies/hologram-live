# ADR 022: Named artifacts are distributed as multi-layer OCI manifests

## Status

Accepted.

## Decision

`.holo` archives are addressable by a docker-style reference,
`[host[:port]/]namespace/name[:tag]`, where a bare `name:tag` expands against
the configured default registry and an omitted tag means `latest`.

An artifact version is one OCI manifest whose layers are a thin `.holo` archive
plus the kappa-addressed payload blobs it references. This reuses machinery that
already existed: thin archives resolve their payloads by kappa, and
`ObjectStore::cache_addressed` already verifies content against its address on
write. Two artifacts sharing weights transfer them once.

Resolution precedence is kappa, then existing filesystem path, then registry
reference. Path-first is required so that adding reference support cannot change
what an existing `run` invocation means.

**Pulling confers no authority.** A pulled archive receives the ordinary ADR 020
local baseline — no storage roots, no channels, no network endpoint scopes — and
resident execution continues to draw its effective grant from trusted host
context, never from the archive or its origin. A pulled archive is executed
directly from bytes, by exactly the path a local `.holo` file takes, because the
simplest way to keep that guarantee true is to give a pulled archive no separate
route into execution.

## Alternatives considered

**Pointing `run` at a registry directly, without a local pull step.** Rejected:
it would make execution depend on network availability and would bypass the
content-addressed store where verification already lives.

**Trusting the registry's manifest.** Rejected on evidence. The registry accepts
manifests referencing blobs that were never uploaded — verified against a live
instance, which returned `201` for a manifest citing an absent layer — so a
successful manifest fetch is not proof the artifact is complete. The client
verifies every layer is present before reporting success.

**Caching resolution by name.** Rejected on evidence. Tags are mutable; a
re-`PUT` was observed moving a tag between manifests. Every pull re-resolves and
records the resolved manifest digest.

**Importing a pulled archive into the catalog before running it.** Rejected for
this change. It would give a pulled archive a second, distinct route into
execution, which is precisely where a provenance-implies-trust shortcut would
later be tempting.

## Consequences

- Acquisition is idempotent and resumable at no extra cost, because every step
  is content-addressed. A re-run transfers only what is still missing.
- A dangling manifest fails the pull with a typed error naming the missing
  layer, rather than producing an artifact that cannot run.
- Reproducibility comes from the recorded manifest digest, not the tag.
- Provenance is never authority. The admission tests exist to keep it that way:
  they assert the reference type and the pull report carry no capability
  vocabulary at all, so a grant-shaped field cannot be added quietly.
- `serve <ref>` and `chat <ref>` are deliberately not part of this change. Both
  overload commands that already mean something else, and `chat` needs an
  optional positional beside an existing subcommand. They deserve their own
  cycle once `pull` and `run` have proven the resolution path in practice.
