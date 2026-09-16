# ADR 023: Applications manage named artifacts through mediated, consented authority

- Status: proposed
- Date: 2026-09-16

## Context

ADR 022 made `.holo` artifacts addressable by reference and acquirable with
`hologram pull`. Every layer is re-hashed on write (`ObjectStore::cache_addressed`),
and shared payloads transfer once. The operation exists only as a CLI command.

An application that helps a person find, acquire, check and remove models (a
model hub) cannot use that machinery today:

- A portable View reaches only `/_hologram/intent` on its own origin, which
  invokes its own primary (ADR 018).
- Desktop starts every session with `EffectiveGrant::local_baseline()`
  (`apps/desktop/src-tauri/src/view_surface.rs`), so the primary holds no
  storage, channel or network authority.
- `hologram:host/store@1.0.0` reads only exact admitted roots.
  `hologram:host/store-write@1.0.0` writes exact addresses within a quota. Neither
  can enumerate, and neither knows what an artifact is.
- `hologram:host/network-fetch@1.0.0` returns at most 1 MiB in 1.5 seconds
  (ADR 021), and a Component invocation ends after 2 seconds with at most 1 MiB
  of input and output. Model layers are gigabytes.
- A pull leaves no record. Layers are cached as bare addressed blobs, so nothing
  can later answer "which artifacts are here, from where, at which manifest
  digest".

Widening existing interfaces would be wrong. Granting store roots or fetch
scopes broad enough for a hub is ambient authority by another name, and moving
multi-gigabyte transfers into a guest defeats the bounded Component limits.
Adding CORS or loopback access for Views would bypass capability admission
entirely.

## Decision

### Authority

The upstream capability view gains one ordered set, `artifact_scopes`. A scope
has exactly the byte form `<registry-host[:port]>/<namespace-prefix>`, using the
reference grammar of ADR 022. Containment is exact on host and port and
segment-bounded on the namespace (`models` admits `models/qwen`, not
`models-x`). Each scope carries a fixed, ordered operation set drawn from
`list`, `pull`, `verify`, `remove`.

Canonical encoding follows the ADR 020 precedent:

- An empty set leaves `CapabilitySet` bytes unchanged, so every existing
  capability object and application identity keeps its κ.
- A non-empty set appends a tagged `ART1` extension.
- Older readers observe no artifact authority. Newer readers reject malformed
  or unsorted extensions.

Child applications may only narrow scopes and operations.

Human-authored `capabilities.json` uses schema 3 for `artifact_scopes`. Schemas
1 and 2 compile to identical bytes when the field is absent.

Admission errors, traces, audit rows and run reports expose only
capability-object identities and scope counts, never scope strings. This is the
same redaction ADR 020 applies to endpoints.

### Guest contract

`hologram:guest/component-artifacts@1` is a distinct Component profile. Its
world imports exactly `hologram:host/artifacts@1.0.0`:

```wit
package hologram:host@1.0.0;

/// Mediated access to named artifacts in the local content store.
interface artifacts {
  record artifact {
    reference: string,
    manifest-digest: option<string>,
    archive-kappa: string,
    layers: u32,
    bytes: u64,
    recorded-at-millis: u64,
  }

  record page {
    artifacts: list<artifact>,
    next-cursor: option<string>,
  }

  enum job-kind { pull, verify }
  enum job-state { running, succeeded, failed, cancelled }

  record job {
    id: string,
    kind: job-kind,
    state: job-state,
    layers-done: u32,
    layers-total: u32,
    bytes-done: u64,
    /// Verify only: the first layer whose bytes do not produce its address.
    mismatch: option<string>,
    /// Redacted, stable failure code; never a URL, path or scope.
    error: option<string>,
  }

  list: func(cursor: option<string>, limit: u16) -> result<page, string>;
  pull: func(reference: string) -> result<string, string>;
  verify: func(archive-kappa: string) -> result<string, string>;
  status: func(job-id: string) -> result<job, string>;
  cancel: func(job-id: string) -> result<_, string>;
  remove: func(archive-kappa: string) -> result<_, string>;
}
```

The provider refuses to build its linker unless the admitted request retains at
least one artifact scope. Every call re-checks the reference or recorded
artifact against the admitted scopes and operations before entering the host,
and again at the store boundary.

### Host semantics

- **Record.** A successful pull writes one artifact record object, the existing
  `PullReport` plus `recorded_at_millis`, with kind `artifact` through
  `ObjectStore::put`. `list` enumerates records whose reference falls within an
  admitted scope. Records confer no authority and create no execution route;
  ADR 022's "pulling confers no authority" is unchanged.
- **Pull.** A pull runs as a host job on a blocking worker using
  `artifact_pull::pull` and the configured `[registry]` client. The call returns
  a job id immediately. `status` reports progress from `PullProgress`. Layers
  keep being re-hashed on write, and completeness is still confirmed before
  success.
- **Verify.** A verify job re-hashes every layer named by the recorded
  artifact's manifest with `ObjectStore::verify`. It reports `succeeded` only
  when every layer is present and matches its address. Otherwise it reports
  `failed`, with the first mismatch or `missing_layer`.
- **Remove.** Remove deletes the artifact record only. Blobs remain, because
  other artifacts may share them and the store has no collector. Remove never
  reports freed bytes it did not free.
- **Jobs.** Jobs are scoped to the application session that started them.
  `status` and `cancel` accept only that session's job ids. Stopping the session
  cancels its running jobs (ADR 019). Cancellation keeps already verified blobs.
- **Host ceilings,** not guest budgets:
  - two concurrent jobs per session;
  - eight concurrent jobs per process;
  - 256 records per `list` page;
  - 512 bytes per reference.

  Each call remains inside the ordinary 2-second Component deadline, because
  long work happens in jobs.

### Consent in Desktop

Desktop no longer passes `local_baseline()` unconditionally to View sessions.
On **Open application**, when the archive's requested capabilities contain
artifact scopes, Desktop shows a consent sheet naming each registry, namespace
and operation in plain words. It then starts the session with either the
approved grant or the baseline.

- **Scope of approval.** Approval is keyed by
  `(application_kappa, requested_capabilities_kappa)`. It is stored in Desktop
  state, not in the archive. Any change to the application or its request asks
  again.
- **Inspector.** The Applications inspector lists approvals and can revoke them.
  Revocation stops the running session.
- **Audit.** Every decision is written to `audit.jsonl` with the new grant
  source `desktop_user_consent`, carrying identities only.
- **Other hosts.** The headless daemon and CLI are unchanged. They still grant
  artifact authority only through the existing loopback-only development grant
  file.

Consent grants only artifact scopes. Storage roots, channels and network
endpoints requested by the same archive are still refused under the Desktop
baseline. Extending consent to them requires a separate decision.

## Alternatives considered

- **Broad store roots plus fetch scopes.** Rejected. The guest would need
  authority over arbitrary blobs and the registry host. It could not transfer
  layers under the ADR 021 ceilings, and enumeration would still be missing.
- **An HTTP route the View calls directly.** Rejected. It needs CORS or loopback
  access from a View origin. That bypasses capability admission and audit, and
  it contradicts ADR 018.
- **Importing pulled archives into the catalog so existing `holo.list` works.**
  Rejected for the reason ADR 022 gives: a second route into execution.
- **Guest-driven pull through `network-fetch`.** Rejected. Multi-gigabyte layers
  cannot cross a 1 MiB, 2-second boundary, and the guest would hold fetch
  authority it does not need.
- **Automatic grant from provenance** (a pulled or signed archive gets
  authority). Rejected. Provenance is never authority (ADR 022).

## Consequences

- A model hub, or any artifact manager, can ship as an ordinary `.holo` with a
  portable View, using the same pull and verification code as the CLI.
- Existing capability objects, application identities and Desktop behaviour
  for applications that request no artifact scopes are unchanged.
- Requires a coordinated upstream change in `Hologram-Technologies/hologram`
  (`crates/hologram-space`: capability extension, contract selector, accepted
  selector list) and a pin bump in this repository.
- Pull records become the first queryable account of what the store holds by
  name. A future garbage collector can use them to decide blob reachability.
- `artifact_pull::pull` holds each layer in memory. That is acceptable for the
  CLI, but a host job running beside a View makes the cost visible. Streaming
  layer writes are recommended before this ships and are tracked separately.
- PrismPM-built applications compose an empty capability request today. They
  cannot request artifact scopes until its composer accepts one. The request
  can be added after composition only if acceptance is re-run.

## Open questions

1. Should the consent sheet support approving a narrower scope than requested?
   The proposal is all or nothing, for simplicity.
2. Should `list` also surface artifacts pulled by the CLI before records
   existed? The proposal is no; records begin with this change.
3. Should `remove` be allowed while a job for the same archive is running? The
   proposal is to refuse, with a stable conflict code.
4. Does push (open PR #71) belong in the same operation set, or in a separate
   decision? The proposal is a separate decision, because publishing is
   outward-facing authority.

## Verification

**Upstream tests:**
- canonical `ART1` syntax, sorting and deduplication;
- empty-set byte identity with existing objects;
- older-reader denial;
- scope and operation containment;
- child narrowing;
- the new selector;
- `no_std`.

**Live unit and integration tests:**
- schema 3 compilation, with schema 1 and 2 byte identity;
- pre-link denial without scopes;
- per-call scope and operation denial;
- redacted errors and audit rows;
- record creation and paging;
- pull success, a dangling manifest, and a corrupt layer (refused on write);
- verify success, a planted flipped byte (reports the mismatch), and a missing
  layer;
- job isolation across sessions;
- cancellation on session stop;
- concurrency ceilings;
- remove keeping shared blobs.

**Desktop tests:**
- the consent sheet appears only when scopes are requested;
- approval keying;
- a changed request asks again;
- revocation stops the session;
- `desktop_user_consent` audit rows.

**BDD suite** `features/suites/artifact-authority` tagged `@status:enforced`.

**Gates:** `just verify` and the upstream workspace gates remain required.
Each new denial path is armed by a planted defect that makes its test fail,
then restored.
