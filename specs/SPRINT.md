# Current sprint: `.holo` application planning foundation

## Sprint status

- State: active
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M1 — Canonical application plan and provider boundary](plans/holo-application-runtime.md#m1--canonical-application-plan-and-provider-boundary)
- Goal: make every `.holo` application explainable and fully resolved before execution, then route the existing Wasm paths through one provider lifecycle
- Exit signal: `hologram --json holo plan <PATH|KAPPA> | jq` explains identity, resolution, provider availability, and blockers, while existing Wasm and Python demo behavior remains intact

This file is the short-lived execution tracker. Durable requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
decisions stay in [`adrs/`](adrs/). Add newly discovered work here immediately,
triage it into **Now**, **Next**, or **Later**, and move lasting decisions or
requirements into the appropriate durable document before closing the sprint.

## Why this is next

The archive, compiler, inspection, Wasm runtime, experimental Python rootfs
path, and inference-model metadata are real. Execution still has two structural
shortcuts in `src/holo.rs`: it resolves only the primary layer, and it requires
exactly one layer at primary position zero. Direct and resident execution also
prepare Wasm separately. Connecting View, Tensor, `hologram-ai`, or microVM
providers before removing those shortcuts would duplicate resolution and
lifecycle behavior in every backend.

The next dependency chain is therefore:

```text
identity + ADR
      -> read-only ApplicationPlan + complete non-child resolution
      -> holo plan CLI/native/HTTP surfaces
      -> provider lifecycle + Wasm migration
      -> capabilities, children, and additional providers
```

## Evidence reviewed

- [x] Reconciled the current implementation with the M0–M8 runtime plan.
- [x] Reviewed ADRs 004, 006, 007, 008, and 009.
- [x] Confirmed `HoloInspection` exposes archive identity and footer fingerprint but not top-level application identity.
- [x] Confirmed `extract_primary_layer` resolves only one primary layer and rejects multi-layer applications.
- [x] Confirmed no `holo.plan` operation exists in the protocol, gRPC schema, HTTP module, or CLI.
- [x] Confirmed the application directory is a verified inspection projection and must not become planning truth.
- [x] Confirmed direct Python rootfs execution is a shipped experimental demo and needs explicit regression coverage during the refactor.

## Scope guardrails

### In this sprint

- Canonical identity vocabulary and reporting.
- A runtime-owned plan for the root application and every non-child layer.
- Embedded and local-store content resolution with explicit resolution sources.
- Read-only planning through local paths and catalog object IDs.
- A closed provider registry and transactional lifecycle boundary.
- Migration of existing direct and resident Wasm execution through the plan.
- Preservation of the existing experimental direct Python rootfs path.

### Deliberately deferred

- Capability schema and grant enforcement: M2.
- Recursive child execution and attenuation: M2. This sprint must list child
  references and report them as blockers rather than silently ignoring them.
- View, Tensor, `hologram-ai`, and production rootfs providers: M3.
- WASI Component Model and portable Python: M3.1a/M4.
- Production resource budgets and cancellation: M5. Planning limits in this
  sprint protect resolution only and are not advertised as execution isolation.
- Installation/GC, signatures/trust, and fuzz/release hardening: M6–M8.

## Delivery slices

Each slice should be independently reviewable. Do not check a slice until its
tests and acceptance notes pass.

### Slice 1 — ADR 010 and identity plumbing

- [x] Write ADR 010 for `ApplicationPlan`, resolver ownership, provider method
  async/`Send` requirements, lifecycle phases, rollback, and the difference
  between an explanatory plan report and a strict executable plan.
- [x] Define one identity record with explicit fields for:
  - physical archive object κ (BLAKE3 of the complete file);
  - archive footer fingerprint;
  - canonical application-manifest κ.
- [x] Document which identity appears in logs, errors, resident records, run
  results, and audit events. Do not silently repurpose the existing `kappa`.
- [x] Add `application_kappa` additively to inspection, compile reports,
  Protobuf/gRPC, HTTP/OpenAPI, and JSON CLI output.
- [x] Add archive object κ to compile output so a freshly written file reports
  all three identities without requiring import.
- [x] Prove fat and thin variants have different archive object κ values and
  footer fingerprints but the same `application_kappa`.
- [x] Amend ADR 007's v3-only wording to acknowledge v4 writes and ADR 009.

Slice 1 acceptance:

- [x] Existing inspection clients remain decodable through additive/defaulted
  fields.
- [x] `hologram --json compile ... | jq` returns all three identities.
- [x] `hologram --json holo inspect ... | jq` returns the same application κ
  for fat and thin variants.

Slice 1 evidence (2026-08-25): unit tests cover identity equivalence and legacy
Protobuf decoding; direct fat/thin CLI output passed the documented `jq`
assertions; `just verify` passed formatting, file-size, check, 118 unit tests,
7 BDD scenarios, Clippy, release build, and smoke; the Astro documentation build
also passed.

### Slice 2 — `ApplicationPlan` and closure resolution

- [ ] Add runtime-owned identity, resolved-object, resolved-layer, blocker, and
  `ApplicationPlan` types outside the CLI and transport modules.
- [ ] Decode and validate the canonical `AppManifest` exactly once per planning
  attempt.
- [ ] Preserve every layer's manifest position, closed kind, content κ,
  entrypoint, auxiliary value, and primary status.
- [ ] Resolve the capability-set object and every non-child layer before any
  provider preparation begins.
- [ ] Record resolution source as `embedded` or `local_store`; reserve a typed
  extension point for configured registry/peer resolvers without adding network
  access now.
- [ ] Deduplicate equal κ payloads while retaining all logical layer edges.
- [ ] Re-hash every resolved payload and reject mismatches.
- [ ] Add explicit root-plan limits for layer count, resolved object count, and
  cumulative bytes. Record child depth limits in ADR 010 but enforce them when
  M2 adds recursive resolution.
- [ ] Return blockers that name the missing κ, referring manifest edge/layer,
  unavailable provider, unsupported child closure, or exceeded limit.
- [ ] Keep `HoloDirectory` out of execution decisions; it may only be attached
  to the report after its existing verification succeeds.
- [ ] Add unit fixtures for multi-layer order, duplicate κ references, missing
  non-primary content, forged cached bytes, thin-cache resolution, no-primary
  service applications, and declared child references.

Slice 2 acceptance:

- [ ] No provider starts until every required non-child object resolves and
  verifies.
- [ ] A missing non-primary layer prevents execution and identifies both the κ
  and layer position.
- [ ] Fat and cache-resolved thin variants produce equivalent logical plans.
- [ ] Planning an unsupported provider succeeds as an explanation with
  `runnable = false`; strict execution returns the corresponding typed error.

### Slice 3 — `hologram holo plan`

- [ ] Add read-only `holo.plan` operation metadata and routing semantics.
- [ ] Add request/response types to the native protocol and Protobuf schema.
- [ ] Add `hologram holo plan <PATH|KAPPA>` with the same local-path/catalog-ID
  selection used by `holo inspect` and `hologram run`.
- [ ] Add an HTTP/OpenAPI plan representation for cataloged applications.
- [ ] Report identities, packaging, capability κ, ordered layers, resolution
  sources, provider availability, child references, limits, `runnable`, and
  stable typed blockers without exposing payload bytes or engine internals.
- [ ] Ensure global `--json` emits one jq-safe document for runnable and blocked
  plans; command/protocol errors retain the global typed JSON error contract.
- [ ] Add protocol round-trip, CLI, HTTP, OpenAPI, and BDD coverage.

Slice 3 acceptance:

- [ ] A local fat Wasm file plans without starting the service.
- [ ] A cataloged thin archive reports local-cache resolution accurately.
- [ ] View, Tensor, rootfs, and inference-model layers can be inspected in a
  plan even when their provider is unavailable.
- [ ] The plan operation is advertised by module discovery and route
  explanation.

### Slice 4 — Provider lifecycle and Wasm migration

- [ ] Define the closed provider registry keyed by `LayerKind`; absence is a
  typed availability result, not a fallback implementation.
- [ ] Separate `prepare`, `start`, `invoke`/attach, and `stop` phases as decided
  by ADR 010.
- [ ] Define planned, preparing, running, stopping, stopped, and failed states.
- [ ] Start layers in manifest order and stop them in reverse order.
- [ ] Roll back every previously started layer in reverse order after a later
  preparation/start failure.
- [ ] Make repeated load idempotent and define safe repeated-unload behavior.
- [ ] Require providers to report lifecycle state, resident bytes, and typed
  failures without leaking Wasmtime or platform UI/microVM types into the
  shared plan.
- [ ] Move direct and resident Wasmtime paths behind the provider boundary.
- [ ] Remove the one-layer/primary-zero execution special case.
- [ ] Preserve the core-Wasm v1 contract, one-output-per-input behavior,
  bounded resident mailbox, and existing `HoloRunResult` compatibility.
- [ ] Preserve the experimental direct Python rootfs demo through a clearly
  named compatibility provider/adapter; do not promote it to production.
- [ ] Add synthetic providers that record call order and fail at every
  prepare/start/stop boundary.
- [ ] Add structured lifecycle traces for plan, prepare, start, rollback, and
  stop using the identity vocabulary from Slice 1.

Slice 4 acceptance:

- [ ] Existing direct and resident Wasm BDD scenarios pass through
  `ApplicationPlan` with no output regression.
- [ ] A synthetic later-layer failure proves reverse-order rollback.
- [ ] Normal unload proves reverse-order stopping.
- [ ] Provider preparation never occurs for a plan with unresolved content or
  blockers.
- [ ] The NumPy/pandas `.holo` demo still executes directly with its documented
  support warning.

## Sprint-wide completion gates

- [ ] Public planning behavior has BDD coverage.
- [ ] Unit and negative tests cover resolution, identity, limits, provider
  ordering, rollback, and idempotency.
- [ ] Native protocol, gRPC, JSON/HTTP, CLI, module discovery, and route
  explanation agree.
- [ ] Errors name the application identity, layer position/provider, and κ
  involved.
- [ ] README, website docs, architecture, actual capabilities, ADRs, and the
  durable runtime plan are current.
- [ ] `cargo fmt`, source-size gate, `cargo check`, full tests, Clippy, BDD,
  optimized build, and release smoke test pass.
- [ ] Completed durable items are checked in
  `plans/holo-application-runtime.md`; unfinished discoveries remain here with
  an explicit disposition.
- [ ] Changes land as reviewable commits/PR slices without unrelated files.

## Discovery log

Use the next ID, include concrete evidence, and choose one disposition:
**Now** (required for this sprint), **Next** (the following milestone), or
**Later** (durable backlog). Do not expand sprint scope merely because an item
was discovered.

- [ ] `DISC-001` — **Now** — Top-level application identity is missing from
  `HoloInspection` and compile reports. Evidence: `src/protocol.rs` and
  `src/cli/compile.rs`. Routed to Slice 1 / M1 identity model.
- [ ] `DISC-002` — **Now** — `extract_primary_layer` resolves only one layer and
  requires primary position zero. Evidence: `src/holo.rs`. Routed to Slices 2
  and 4 / M1 planning and lifecycle.
- [ ] `DISC-003` — **Now** — There is no `holo.plan` operation in protocol,
  Protobuf, HTTP, module discovery, or CLI. Routed to Slice 3 / M1 planning
  interface.
- [ ] `DISC-004` — **Now** — ADR 007 still describes new archives as v3 even
  though ADR 009 and the implementation use v4 writes with v2/v3 reads. Routed
  to Slice 1 documentation reconciliation.
- [ ] `DISC-005` — **Now** — The experimental Python OCI direct executor sits
  beside the Wasm executor and could regress during provider extraction.
  Evidence: `src/holo.rs` and ADR 008. Routed to Slice 4 compatibility tests.
- [ ] `DISC-006` — **Next** — Recursive child planning cannot define execution
  ownership safely until M2 fixes effective grants and attenuation. This sprint
  lists child edges and emits an explicit blocker; M2 owns recursive execution.
- [ ] `DISC-007` — **Later** — Resolution limits are necessary now, but CPU,
  memory, deadline, cancellation, and concurrency budgets remain M5 and must
  not be implied by this sprint's planner limits.

Template for new discoveries:

```text
- [ ] DISC-### — Now|Next|Later — Concise work item. Evidence: path, test,
  error, or command. Routed to: sprint slice or durable milestone/ADR.
```

## Expected next sprint

After this tracker closes, begin M2 capability source validation, effective
grants, and child attenuation. Once M1/M2 make lifecycle and authority explicit,
the first new provider should be selected between the `hologram-ai` inference
adapter and the portable View composition proof; do not choose by bypassing the
provider registry established here.
