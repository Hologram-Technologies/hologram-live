# Design: named artifact distribution and the pull/serve/chat surface

- Status: approved, not yet implemented
- Date: 2026-09-15
- Depends on: `2026-09-15-kappa-registry-provider-design.md`. Pulling is
  inherently remote, so it needs that adapter to exist first.
- Scope: a reference grammar for named `.holo` artifacts, a deduplicating pull,
  and the CLI surface over it. Publishing (`push`), a curated model index, and
  new inference engines are out of scope; see "Adjacent work".

## Context

Hologram Live can compile, import, inspect, plan, verify, and run `.holo`
archives, and it can execute inference through three engines — `echo`,
`weightc`, and `ollama` (`src/inference/mod.rs`). What it cannot do is obtain an
archive by name. Every existing path takes a filesystem path or a `blake3:`
kappa, so acquiring an artifact is a manual, out-of-band step.

The requested experience is the one ollama and hipfire established:

```bash
hologram pull qwen3.5:4b
hologram serve qwen3.5:4b
hologram chat qwen3.5:4b
```

Two pieces already exist and are easy to overlook. `ModelCatalog::resolve`
(`src/models.rs`) already resolves a blake3 id *or* a name. And `.holo` already
has fat/thin packaging over kappa-addressed layers, with `ARCHITECTURE.md`
recording that "importing a fat archive verifies and caches its layer content by
kappa, which lets the catalog-backed runtime produce the same logical plan for a
later thin archive".

That second property is the whole design. A thin `.holo` plus its kappa-addressed
payload blobs, expressed as the layers of one OCI manifest, *is* a
content-deduplicated distribution format. Two artifacts sharing weights or a
tokenizer transfer them once. `ObjectStore::cache_addressed_bounded`
(`src/store.rs`) already verifies each blob against its kappa on write and
enforces a byte ceiling; it is the fetch-and-verify primitive, already written
and tested. This design adds naming and transport around machinery that exists.

### What was verified

A second throwaway spike against a live kappa-registry confirmed:

- Blobs and manifests are accepted under dotted, nested namespaces, so
  `qwen3.5:4b` maps to `/v2/models/qwen3.5/manifests/4b`.
- A three-layer manifest (thin archive plus two payload layers) is accepted,
  resolves by tag, and returns all three layers.
- Every layer is independently fetchable by kappa and round-trips byte-exactly.
- **Deduplication holds.** A second artifact published with a layer already
  present succeeded without re-uploading it, and `HEAD` on the shared layer
  returned `200` beforehand. A pull can skip what it already has.
- `tags/list` enumerates every tag of one named artifact
  (`{"name":"models/qwen3.5","tags":["4b","4b-instruct"]}`).

Two findings shaped the design rather than confirming it:

- **The registry does not validate layer presence.** A manifest referencing a
  blob that was never uploaded was accepted with `201`. Dangling manifests are
  publishable.
- **Tags are mutable.** Re-PUTting an existing tag moved it from a three-layer
  manifest to a two-layer one.

## Decision

### 1. Reference grammar

```
[host[:port]/]namespace/name[:tag]
```

A bare `qwen3.5:4b` expands against the configured default host and namespace.
`tag` defaults to `latest`. Following OCI, the colon separates repository from
tag — `qwen3.5:4b` is repository `qwen3.5`, tag `4b` — because the OCI tag
grammar (`[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}`) excludes colons. The spike
confirmed the dotted repository segment resolves correctly.

`run` and `serve` already accept paths and kappas, so resolution precedence is
stated explicitly and path-first, ensuring no existing invocation changes
meaning:

1. Parses as `blake3:<64 hex>` → catalog lookup. Existing behaviour.
2. Exists on the filesystem → local archive. Existing behaviour.
3. Otherwise → artifact reference, resolved through the registry.

Two ambiguities get explicit rules rather than silent guessing:

- A file literally named `qwen3.5:4b` wins, because rule 2 precedes rule 3.
  `--ref` forces registry interpretation.
- A Windows path such as `C:\models\a.holo` would parse as `host:port` under
  rule 3, but rule 2 matches first. `--file` forces filesystem interpretation.

A reference is not a capability. See section 5.

### 2. Artifact representation

One OCI manifest per named artifact version:

- `artifactType`: `application/vnd.hologram.artifact.v1+json`
- One layer with `mediaType: application/vnd.hologram.holo` and annotation
  `dev.hologram.role=archive` — the thin `.holo` itself.
- Zero or more layers with annotation `dev.hologram.role=layer` — the
  kappa-addressed payload blobs the thin archive references.
- Annotations `dev.hologram.kind`, `dev.hologram.name`, `dev.hologram.tag`.

A fat archive may be published as a single `role=archive` layer with no payload
layers. It is valid and simpler, and it forfeits deduplication. Thin is the
recommended publishing form and the one the pull path optimises for.

### 3. Pull

`hologram pull <ref>`:

1. Resolve the reference to `host`, `namespace/name`, `tag`.
2. `GET /v2/{namespace}/{name}/manifests/{tag}`, recording the **manifest
   digest** returned in `docker-content-digest`.
3. For each layer, consult the local store first. `ObjectStore::get_cached`
   already answers "do I have this kappa" without a network call. Fetch only
   what is missing.
4. Write each fetched layer through `cache_addressed_bounded`, which verifies
   the content against its kappa and enforces the byte ceiling. A layer whose
   bytes do not match its kappa fails the pull.
5. **Verify every referenced layer is present before declaring success**, since
   the registry does not. A manifest with an unfetchable layer fails with
   `LiveError::NotFound` naming the missing kappa, and the partial pull leaves
   no catalog entry.
6. Import the archive through the existing `.holo` import path, which re-derives
   and validates the application directory.

The pull is idempotent and resumable at no extra cost: every step is
content-addressed, so a re-run fetches only what is still missing, and an
interrupted pull leaves verified blobs that the next run reuses.

**Tags are mutable, so pull always re-resolves.** A name is never treated as a
cache key. The resolved manifest digest is recorded in the catalog entry, which
is what makes a pull reproducible and lets `hologram pull` report whether a tag
moved since last time. A reference may also be given as a manifest digest
directly, which pins exactly.

### 4. Command surface

| Command | Behaviour |
| ------- | --------- |
| `hologram pull <ref>` | Section 3. Reports layers fetched, layers already present, and bytes transferred. |
| `hologram serve [<ref>]` | Positional optional. Absent → today's behaviour exactly. Present → ensure pulled, then load as resident before the listener binds, reusing the existing `config.holo.resident` path. |
| `hologram chat <ref>` | Interactive session against the configured `InferenceEngine`. |
| `hologram run <ref\|path> [prompt]` | Existing command, extended by precedence rule 3. |

`hologram chat` currently takes a `send` subcommand. It gains an optional
positional with `args_conflicts_with_subcommands`: a leading token matching a
known subcommand is a subcommand, anything else is a reference. `chat send`
keeps working. The residual ambiguity — an artifact named `send` — is resolved
by `--ref`. The unambiguous alternative, `chat with <ref>`, was rejected for
discarding the terseness that motivates the feature.

`--json` is preserved end to end. Progress is inherently streaming, so human
progress renders on stderr while stdout carries the result document; under
`--json`, progress is emitted as JSONL events rather than a rendered bar. No
command loses its machine-readable contract.

### 5. Pulling confers no authority

A pulled archive traverses the identical admission path as a local one. It
receives the ordinary local baseline of ADR 020 — no storage roots, no channels,
no network endpoint scopes — and development grants remain restricted to direct
local files and loopback service configuration. `serve <ref>` making an archive
resident does not elevate it; resident execution continues to draw its effective
grant from trusted host context, never from the archive or its origin.

Provenance is not trust. Having been pulled from a configured registry is not
evidence about an archive's contents, and the capability model must not weaken
because acquisition became convenient. The audit record for a pulled archive
names its reference, resolved manifest digest, and archive kappa, so a decision
can be explained without implying the origin granted anything.

### 6. Testing

- Reference-grammar unit tests: expansion of bare names, tag defaulting, the
  three precedence rules, and both documented ambiguities.
- Pull tests over a fake registry: full fetch, partial fetch with some layers
  cached, resumption after interruption, kappa mismatch rejection, and the
  missing-layer failure of step 5.
- Mutable-tag test proving a moved tag is re-resolved rather than served from a
  name cache, and that the recorded manifest digest changes.
- An admission test proving a pulled archive receives the same baseline grant as
  an equivalent local file, failing if pulling ever confers authority.
- Integration tests against a live kappa-registry, skipped cleanly when absent,
  following `scripts/check-python-private-registry.sh`.
- A cucumber scenario covering `pull` then `run` by name.

## Consequences

`ACTUAL_CAPABILITIES.md` gains named artifact acquisition. `ARCHITECTURE.md`
gains the distribution format. The README CLI table gains four entries. An ADR
records the reference grammar and the pull-confers-no-authority rule, which is
the load-bearing security decision.

No new dependencies. Transport is the kappa provider from the dependent design;
verification and caching are existing `ObjectStore` primitives.

## Adjacent work

- **`hologram push <ref>`.** The natural inverse, and cheap once pull exists.
  Deliberately deferred: publishing raises signing, ownership, and mutable-tag
  policy questions that deserve their own cycle.
- **A curated index.** hipfire ships 82 curated entries. Valuable for discovery,
  but it introduces curation, distribution, and trust problems. The grammar here
  admits an index as an extra resolution step without changing.
- **Layer-presence validation upstream.** The registry accepting manifests with
  absent layers is arguably an upstream defect. This design compensates
  client-side; an upstream issue is worth filing alongside the two build defects
  recorded in the dependent design.
- **Range requests and parallel layer fetch.** `HEAD` advertises
  `accept-ranges: bytes`. Large-model pulls would benefit from chunked, parallel,
  resumable transfers. Correctness first; optimise when a real artifact is slow.
