# Current sprint: `.holo` application planning foundation

## Sprint status

- State: complete
- Started: 2026-08-25
- Completed: 2026-08-25
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

At sprint start, execution had two structural shortcuts in `src/holo.rs`: it
resolved only the primary layer and required exactly one layer at primary
position zero, while direct and resident paths prepared Wasm separately. This
sprint removed those shortcuts through complete planning and one transactional
provider lifecycle. View, Tensor, `hologram-ai`, and production microVM
providers can now extend that boundary without duplicating archive resolution.

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

- [x] Add runtime-owned identity, resolved-object, resolved-layer, blocker, and
  `ApplicationPlan` types outside the CLI and transport modules.
- [x] Decode and validate the canonical `AppManifest` exactly once per planning
  attempt.
- [x] Preserve every layer's manifest position, closed kind, content κ,
  entrypoint, auxiliary value, and primary status.
- [x] Resolve the capability-set object and every non-child layer before any
  provider preparation begins.
- [x] Record resolution source as `embedded` or `local_store`; reserve a typed
  extension point for configured registry/peer resolvers without adding network
  access now.
- [x] Deduplicate equal κ payloads while retaining all logical layer edges.
- [x] Re-hash every resolved payload and reject mismatches.
- [x] Add explicit root-plan limits for layer count, resolved object count, and
  cumulative bytes. Record child depth limits in ADR 010 but enforce them when
  M2 adds recursive resolution.
- [x] Return blockers that name the missing κ, referring manifest edge/layer,
  unavailable provider, unsupported child closure, or exceeded limit.
- [x] Keep `HoloDirectory` out of execution decisions; it may only be attached
  to the report after its existing verification succeeds.
- [x] Add unit fixtures for multi-layer order, duplicate κ references, missing
  non-primary content, forged cached bytes, thin-cache resolution, no-primary
  service applications, and declared child references.

Slice 2 acceptance:

- [x] No provider starts until every required non-child object resolves and
  verifies.
- [x] A missing non-primary layer prevents execution and identifies both the κ
  and layer position.
- [x] Fat and cache-resolved thin variants produce equivalent logical plans.
- [x] Planning an unsupported provider succeeds as an explanation with
  `runnable = false`; strict execution returns the corresponding typed error.

Slice 2 evidence (2026-08-25): planner fixtures cover multi-layer ordering,
shared-κ deduplication, required-capability resolution, embedded/local-store
sources, thin/fat equivalence, forged local bytes, missing secondary content,
root limits, service-only applications, and child blockers. An execution test
uses malformed primary Wasm plus missing layer 1 and receives the layer-1
`LIVE_NOT_FOUND`, proving provider compilation did not begin. `just verify`
passed formatting, file-size, check, 126 unit tests, 7 BDD scenarios, Clippy,
release build, and smoke; the Astro documentation build also passed.

### Slice 3 — `hologram holo plan`

- [x] Add read-only `holo.plan` operation metadata and routing semantics.
- [x] Add request/response types to the native protocol and Protobuf schema.
- [x] Add `hologram holo plan <PATH|KAPPA>` with the same local-path/catalog-ID
  selection used by `holo inspect` and `hologram run`.
- [x] Add an HTTP/OpenAPI plan representation for cataloged applications.
- [x] Report identities, packaging, capability κ, ordered layers, resolution
  sources, provider availability, child references, limits, `runnable`, and
  stable typed blockers without exposing payload bytes or engine internals.
- [x] Ensure global `--json` emits one jq-safe document for runnable and blocked
  plans; command/protocol errors retain the global typed JSON error contract.
- [x] Add protocol round-trip, CLI, HTTP, OpenAPI, and BDD coverage.

Slice 3 acceptance:

- [x] A local fat Wasm file plans without starting the service.
- [x] A cataloged thin archive reports local-cache resolution accurately.
- [x] View, Tensor, rootfs, and inference-model layers can be inspected in a
  plan even when their provider is unavailable.
- [x] The plan operation is advertised by module discovery and route
  explanation.

Slice 3 evidence (2026-08-25): local-path and catalog-κ BDD steps validate the
payload-free JSON contract without starting providers; unit tests cover blocked
View/Tensor/rootfs/inference-model reports and cataloged thin archives resolving
both capabilities and layers from `local_store`. Native/Protobuf round trips,
module/OpenAPI assertions, route explanation, the catalog HTTP endpoint, and
release-smoke `jq` checks pass. `just verify` passed formatting, file-size,
check, 127 unit tests, 7 BDD scenarios / 59 steps, Clippy, optimized build, and
release smoke; the final added provider-matrix test brings the unit total to
128 and passed with Clippy. The Astro documentation build also passed.

### Slice 4 — Provider lifecycle and Wasm migration

- [x] Define the closed provider registry keyed by `LayerKind`; absence is a
  typed availability result, not a fallback implementation.
- [x] Separate `prepare`, `start`, `invoke`/attach, and `stop` phases as decided
  by ADR 010.
- [x] Define planned, preparing, running, stopping, stopped, and failed states.
- [x] Start layers in manifest order and stop them in reverse order.
- [x] Roll back every previously started layer in reverse order after a later
  preparation/start failure.
- [x] Make repeated load idempotent and define safe repeated-unload behavior.
- [x] Require providers to report lifecycle state, resident bytes, and typed
  failures without leaking Wasmtime or platform UI/microVM types into the
  shared plan.
- [x] Move direct and resident Wasmtime paths behind the provider boundary.
- [x] Remove the one-layer/primary-zero execution special case.
- [x] Preserve the core-Wasm v1 contract, one-output-per-input behavior,
  bounded resident mailbox, and existing `HoloRunResult` compatibility.
- [x] Preserve the experimental direct Python rootfs demo through a clearly
  named compatibility provider/adapter; do not promote it to production.
- [x] Add synthetic providers that record call order and fail at every
  prepare/start/stop boundary.
- [x] Add structured lifecycle traces for plan, prepare, start, rollback, and
  stop using the identity vocabulary from Slice 1.

Slice 4 acceptance:

- [x] Existing direct and resident Wasm BDD scenarios pass through
  `ApplicationPlan` with no output regression.
- [x] A synthetic later-layer failure proves reverse-order rollback.
- [x] Normal unload proves reverse-order stopping.
- [x] Provider preparation never occurs for a plan with unresolved content or
  blockers.
- [x] The NumPy/pandas `.holo` demo still executes directly with its documented
  support warning.

Slice 4 evidence (2026-08-25): the async provider registry and coordinator have
synthetic prepare/start/stop failure injection, manifest-order startup,
reverse-order normal stop and rollback, rollback diagnostics, lifecycle states,
and idempotent stop tests. Direct and resident Wasmtime run behind the same
boundary; a real two-Wasm-layer archive with primary position 1 executes both
ways, while the existing missing-secondary fixture still fails before Wasm
compilation. The native resident response reports lifecycle state additively.
All 135 unit/bin tests and 7 BDD scenarios / 59 steps pass. The 105.8 MB locked
NumPy/pandas archive compiled and executed through `python-oci-direct`,
returning the expected columns, three rows, mean 20, and sum 60.

## Sprint-wide completion gates

- [x] Public planning behavior has BDD coverage.
- [x] Unit and negative tests cover resolution, identity, limits, provider
  ordering, rollback, and idempotency.
- [x] Native protocol, gRPC, JSON/HTTP, CLI, module discovery, and route
  explanation agree.
- [x] Errors name the application identity, layer position/provider, and κ
  involved.
- [x] README, website docs, architecture, actual capabilities, ADRs, and the
  durable runtime plan are current.
- [x] `cargo fmt`, source-size gate, `cargo check`, full tests, Clippy, BDD,
  optimized build, and release smoke test pass.
- [x] Completed durable items are checked in
  `plans/holo-application-runtime.md`; unfinished discoveries remain here with
  an explicit disposition.
- [x] Changes land as reviewable commits/PR slices without unrelated files.

## Discovery log

Use the next ID, include concrete evidence, and choose one disposition:
**Now** (required for this sprint), **Next** (the following milestone), or
**Later** (durable backlog). Do not expand sprint scope merely because an item
was discovered.

- [x] `DISC-001` — **Now** — Top-level application identity is missing from
  `HoloInspection` and compile reports. Evidence: `src/protocol.rs` and
  `src/cli/compile.rs`. Routed to Slice 1 / M1 identity model.
- [x] `DISC-002` — **Now** — `extract_primary_layer` resolves only one layer and
  requires primary position zero. Evidence: `src/holo.rs`. Routed to Slices 2
  and 4 / M1 planning and lifecycle.
- [x] `DISC-003` — **Now** — There is no `holo.plan` operation in protocol,
  Protobuf, HTTP, module discovery, or CLI. Routed to Slice 3 / M1 planning
  interface.
- [x] `DISC-004` — **Now** — ADR 007 still describes new archives as v3 even
  though ADR 009 and the implementation use v4 writes with v2/v3 reads. Routed
  to Slice 1 documentation reconciliation.
- [x] `DISC-005` — **Now** — The experimental Python OCI direct executor sits
  beside the Wasm executor and could regress during provider extraction.
  Evidence: `src/holo.rs` and ADR 008. Routed to Slice 4 compatibility tests.
- [ ] `DISC-006` — **Next** — Recursive child planning cannot define execution
  ownership safely until M2 fixes effective grants and attenuation. This sprint
  lists child edges and emits an explicit blocker; M2 owns recursive execution.
- [ ] `DISC-007` — **Later** — Resolution limits are necessary now, but CPU,
  memory, deadline, cancellation, and concurrency budgets remain M5 and must
  not be implied by this sprint's planner limits.
- [ ] `DISC-008` — **Next** — Lifecycle traces now carry archive/application
  identity, phase, layer, and provider fields; durable audit-event emission
  still needs an audit schema tied to M2 grant decisions. Routed to M2 audit
  coverage rather than adding authority-free events in M1.

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
