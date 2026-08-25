# Current sprint: `.holo` capability enforcement and child attenuation

## Sprint status

- State: active
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M2 — Capability enforcement and child attenuation](plans/holo-application-runtime.md#m2--capability-enforcement-and-child-attenuation)
- Goal: turn a `.holo` application's requested capabilities into a validated,
  content-addressed request and admit it only under an explicit trusted grant
- Exit signal: insufficient authority returns `LIVE_AUTHORIZATION_DENIED` before
  provider preparation, while child grants can only attenuate parent authority
- Current focus: Slice 3 child closure and attenuation

This is the short-lived execution tracker. Durable requirements remain in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
decisions remain in [`adrs/`](adrs/). The completed M1 tracker remains available
in Git history at commit `1a9b755`.

## Why this is next

M1 now resolves a complete non-child application plan and starts every provider
through a transactional lifecycle. The remaining authority shortcut is that the
resolved `AppManifest.requires` payload is opaque and is handed to providers as
if a request were a grant. M2 must separate these concepts before adding child
execution or host interfaces:

```text
source capabilities.json
        -> canonical upstream CapabilitySet bytes + κ
        -> resolved requested capabilities
trusted execution context
        -> effective grant
request admitted by grant
        -> provider preparation with the effective grant only
        -> attenuated child grants
```

## Evidence reviewed

- [x] `CompileManifest.requires` currently reads and embeds arbitrary bytes.
- [x] An omitted `requires` currently becomes an empty opaque blob rather than
  a canonical empty `CapabilitySet`.
- [x] `ApplicationPlan.required_capabilities` currently stores opaque bytes.
- [x] `LayerPrepareContext.required_capabilities` passes those untrusted bytes
  directly to providers.
- [x] Upstream `CapabilitySet::canonicalize` and
  `CapabilitySet::to_capabilities` provide the canonical realization boundary.
- [x] Upstream `Capabilities::admits` implements subset and bounded-budget
  attenuation, including the `0 = unbounded` convention.
- [x] Child edges already carry application and delegated-capability κ values,
  but M1 deliberately reports them as execution blockers.
- [x] The service audit log has a stable JSONL boundary but does not yet record
  application capability decisions.

## Scope guardrails

### In this sprint

- A documented JSON source schema that compiles to the upstream canonical
  `CapabilitySet` realization.
- Compile/check diagnostics with source paths and field/index locations.
- Distinct requested-capability and effective-grant runtime types.
- Explicit, safe grant sources for direct and local-service execution.
- Authorization before provider preparation and effective-grant-only provider
  contexts.
- Recursive child resolution, bounded traversal, attenuation, and lifecycle
  ownership.
- Capability decision traces/audit events and end-to-end interfaces/tests.

### Deliberately deferred

- Remote identity, policy distribution, and remote grant issuance. M2 defines
  the interface and rejects absent remote authority; it does not invent a PKI.
- Enforcing CPU, memory, deadline, and concurrency budgets inside engines: M5.
- Linking granular WASI/host interfaces from grants: M4/M5. Providers receive
  the effective grant in M2 so those boundaries can enforce it later.
- Signatures, revocation, transparency, and publisher trust: M7.

## Delivery slices

### Interruption — Desktop watched application projects

- [x] Add/remove local source directories containing `hologram.json` through
  the Tauri adapter and persist them with the reusable watch engine.
- [x] Recursively watch and debounce relevant file changes without writing
  build output into the source directory.
- [x] Compile and import through the existing CLI/service boundaries; retain
  the last successful archive when a later build fails.
- [x] Replace obsolete watched variants after successful changed builds while
  leaving a final immutable archive when a watch is removed.
- [x] Add a desktop Applications navigation item, watched-project status list,
  real `.holo` catalog listing, and verified inspection detail.
- [x] Add focused watcher tests, frontend/desktop builds, documentation, and
  full verification evidence.
- [x] Extract watch persistence, event filtering, debounce, and build-state
  orchestration from `src-tauri` into the Tauri-independent
  `hologram-application-watch` workspace crate; retain only path authorization,
  fixed sidecar invocation, and UI events in the desktop adapter.
- [x] Make server build, install, CI, and release commands select the root
  `hologram-live` package and `hologram` binary explicitly so hosted builds do
  not depend on Tauri or Node.js.

Acceptance:

- [x] Adding `features/fixtures/wasm-app/` produces an inspectable catalog entry.
- [x] A referenced-file edit triggers one debounced rebuild and refreshes the
  Applications view.
- [x] Invalid source shows a project error without discarding its last good
  archive.
- [x] All list and inspect data comes from `holo list` / `holo inspect`, not a
  frontend reconstruction of archive metadata.

Interruption evidence (2026-08-25): the packaged Tauri app selected
`features/fixtures/wasm-app`, compiled/imported it to a verified v4 catalog row,
and rendered its archive/application/capability identities, Wasm layer, section
offsets, and embedded-blob counts from `holo inspect`. A source edit changed the
archive κ after one debounced rebuild; malformed `hologram.json` showed Failed
while the prior κ remained inspectable; restoring the manifest returned Ready;
and an edit after Stop watching left the final archive unchanged. Watcher unit
tests, ANSI-diagnostic coverage, desktop Clippy, production frontend/Tauri
builds, the 13-page documentation build, and full repository verification all
pass (135 library tests, 15 CLI tests, 9 BDD scenarios / 80 steps, optimized
build, and release smoke).

Boundary follow-up (2026-08-25): `apps/desktop/src-tauri/src/holo_watch.rs` is
now a thin adapter over `crates/hologram-application-watch`. The extracted
crate's five tests include a real debounced register/build/persist/remove cycle;
the server and desktop workspaces compile independently, and server recipes and
release workflows explicitly build `--package hologram-live --bin hologram`.
The server's declared MSRV is corrected from 1.88 to Wasmtime 46's actual Rust
1.94 floor, while development and release jobs consistently install 1.97.1.
The new product-boundary gate rejects Tauri or application-watch dependencies
in the resolved standalone-server graph.

### Slice 1 — Canonical capability source

- [x] Add a versioned, deny-unknown-fields `capabilities.json` source schema for
  storage roots, channels, network flags, and scalar budgets.
- [x] Parse every κ reference as a canonical `blake3:<64 lowercase hex>` label
  and reject duplicates or unstable ordering.
- [x] Compile source JSON into upstream `CapabilitySet::canonicalize()` bytes;
  never embed the source JSON as the runtime object.
- [x] Make omitted `requires` compile to a canonical empty capability set.
- [x] Validate capabilities during both `compile --check` and `compile` and
  include the source path plus field/index in diagnostics.
- [x] Preserve the resulting capability κ in `AppManifest.requires` for fat and
  thin archives.
- [x] Add positive, malformed JSON, unknown field/version, invalid κ,
  duplicate/order, and fat/thin identity tests.
- [x] Document the source-to-canonical distinction and provide a minimal file.

Slice 1 acceptance:

- [x] `hologram --json compile --check hologram.json | jq` validates the
  capability document without writing an archive.
- [x] Inspecting/planning a newly compiled archive resolves a decodable upstream
  `CapabilitySet`, not JSON or arbitrary bytes.
- [x] Equivalent canonical source produces the same capability κ.

Slice 1 evidence (2026-08-25): `src/holo_capability.rs` maps the closed JSON
schema into upstream `CapabilitySet` canonical bytes and proves round-trip
canonicality. Compiler tests cover explicit and omitted requests, source
diagnostics, fat/thin κ stability, and embedded canonical decoding. Targeted
tests and the Astro documentation build pass. Full verification passes the
source-size gate, all-target check, 126 library tests, 15 CLI tests, Clippy, all
7 BDD scenarios / 59 steps, the optimized release build, and release smoke.

### Slice 2 — Effective grants and authorization boundary

- [x] Add typed requested capabilities and an `EffectiveGrant` that records its
  trusted source without treating archive content as authority.
- [x] Decode/validate the requested canonical capability object during planning.
- [x] Define a deny-by-default grant for ordinary direct execution and local
  service execution.
- [x] Add an explicit local-development grant mode and CLI/config surface; make
  its scope and warning visible in JSON and human output.
- [x] Authorize with upstream `Capabilities::admits` after resolution and before
  any provider `prepare` call.
- [x] Return `LIVE_AUTHORIZATION_DENIED` with application/request/grant identity
  and a non-secret capability summary when admission fails.
- [x] Replace provider `required_capabilities` with the trusted effective grant.
- [x] Preserve current behavior for applications requesting the empty set.
- [x] Add synthetic-provider proof that denial occurs before preparation.

Slice 2 acceptance:

- [x] Empty-requirement Wasm and Python demos continue to run without an unsafe
  grant flag.
- [x] A non-empty request is denied by default before provider preparation.
- [x] The same request runs under an explicit sufficient development grant.
- [x] An insufficient explicit grant remains denied.

Slice 2 evidence (2026-08-25): planning now canonical-decodes requests into a
typed value; execution constructs authority only from the built-in baseline,
the direct CLI development file, or loopback-only service configuration. Both
direct and resident paths authorize before provider preparation and providers
receive only the effective grant. Successful raw results carry request/grant κ,
trusted source, and allow outcome across JSON and Protobuf/gRPC; denial returns
the typed authorization error and a non-secret summary. Unit coverage proves an
insufficient explicit grant remains denied and synthetic provider preparation
does not run. The BDD suite passes all 9 scenarios / 80 steps, including direct
baseline, direct development, and resident service development sources. Full
verification passes formatting, the source-size gate, all-target checks, 135
library tests, 15 CLI tests, Clippy with warnings denied, the optimized release
build, and release smoke; the Astro documentation build also passes.

### Slice 3 — Child closure and attenuation

- [ ] Add source-manifest child syntax with application and delegated capability
  references, plus interactive-generator support.
- [ ] Resolve child manifests, delegated grants, required capabilities, and
  layers through the same verified κ resolver.
- [ ] Enforce total depth, application count, object count, and byte limits over
  the complete tree.
- [ ] Detect cycles by application κ and return a deterministic path diagnostic.
- [ ] Require parent effective grant to admit each delegated child grant.
- [ ] Require the delegated grant to admit the child's requested capabilities.
- [ ] Reject amplification before preparing the child or any later provider.
- [ ] Define manifest-order child startup, reverse-order rollback/stop, and exit
  propagation in an ADR amendment.
- [ ] Remove the M1 child blocker only when the complete child plan is admitted.

Slice 3 acceptance:

- [ ] Narrow child delegation starts and stops with its parent.
- [ ] Amplification and under-granted child requests fail deterministically
  before child provider preparation.
- [ ] Cyclic and over-limit graphs fail without recursion overflow.

### Slice 4 — Audit, interfaces, and conformance

- [ ] Emit capability-decision records for allow/deny with principal, grant
  source, application κ, requested κ, effective-grant κ, and outcome.
- [ ] Do not log tokens, source documents, payload bytes, or secret values.
- [ ] Expose grant mode and authorization outcomes consistently through native,
  Protobuf/gRPC, JSON/HTTP, CLI, and desktop-consumed surfaces.
- [ ] Add unit, native round-trip, HTTP/OpenAPI, BDD, security-negative, and
  release-smoke coverage.
- [ ] Update README, website `.holo` documentation, architecture, actual
  capabilities, and durable plan checkboxes.

## Sprint-wide completion gates

- [ ] Requested, granted, delegated, and enforced capabilities are distinct in
  code, errors, JSON, and documentation.
- [ ] No archive-controlled byte string becomes authority.
- [ ] No provider prepares before closure resolution and authorization finish.
- [ ] Capability behavior is deterministic across fat and cache-resolved thin
  archives.
- [ ] `cargo fmt`, source-size gate, check, full tests, Clippy, BDD, optimized
  build, release smoke, and website build pass.
- [ ] Durable plan/ADR checkboxes and discoveries are reconciled before closure.
- [ ] Work lands in reviewable commits without unrelated changes.

## Discovery log

- [x] `DISC-009` — **Now** — The upstream `KappaLabel::from_bytes` checks only
  width and ASCII, so source validation must additionally enforce the exact
  `blake3:<64 lowercase hex>` grammar. Routed to Slice 1.
- [x] `DISC-010` — **Now** — Existing M1 fixtures use arbitrary capability
  bytes; runtime decoding will require converting fixtures to canonical empty
  or explicit capability sets while retaining a legacy-invalid negative test.
  Routed to Slices 1–2.
- [x] `DISC-011` — **Now** — Direct execution has no trusted principal or grant
  input today. The explicit development mode must be a deliberate CLI/API value,
  not inferred from loopback networking. Routed to Slice 2 and an ADR.
- [ ] `DISC-012` — **Next** — Provider receipt of scalar budgets does not itself
  enforce them. Engine and host-interface enforcement remains M5 and must be
  reported honestly. Routed to M5.
- [x] `DISC-013` — **Now** — The desktop exposes files but has no local source
  project boundary, so users cannot turn directory changes into real cataloged
  `.holo` archives or inspect them in the application UI. Keep filesystem watch
  authority in Tauri and route outputs through compile/import/list/inspect.
  Routed to: desktop watched-project interruption and M4 development loop.

Template:

```text
- [ ] DISC-### — Now|Next|Later — Work item and concrete evidence.
  Routed to: slice/milestone/ADR.
```
