# Current sprint: M3.1 typed Wasm entry and exit contracts

## Sprint status

- State: active
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M3.1 — Wasm provider migration](plans/holo-application-runtime.md#m31-wasm-provider-migration)
- Goal: make the manifest-declared Wasm entry authoritative under an explicit,
  backward-compatible core-Wasm v1 contract, then define typed application
  completion before introducing a new guest ABI
- Exit signal: direct and resident execution invoke the declared entry, invalid
  contracts fail during provider preparation, and v1 completion semantics are
  explicit across provider and public result boundaries

This short-lived tracker replaces the completed M2 tracker, which remains in
Git history. Durable requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
accepted decisions stay in [`adrs/`](adrs/).

## Evidence reviewed

- [x] `AppManifest` already binds an `entry` string into every Wasm layer and
  therefore into canonical application identity.
- [x] `ResolvedLayer` and `LayerPrepareContext` preserve that entry through
  planning and provider selection.
- [x] `src/holo_wasm.rs` ignores the resolved entry and hard-codes the exported
  function name `holo_run` during validation and invocation.
- [x] The compiler and non-interactive app generator default an omitted Wasm
  entry to `_start`, while the implemented byte-transform contract requires
  `(i32, i32) -> i64` and a separate `holo_alloc` export.
- [x] Core-Wasm guest contract v1 has no imports, no WASI, one byte input and
  one byte output per invocation, and a fresh instance for each input.
- [x] `LayerInvocation` currently returns output bytes and elapsed time but no
  typed completion or exit status.
- [x] The current `.holo` manifest has no independent Wasm ABI-version field;
  adding one requires an upstream-compatible format decision rather than an
  undocumented entry-name convention.

## Contract guardrails

- Archive-controlled `entry` selects only an export inside the already selected
  Wasm module; it cannot select a host function, provider, filesystem path, or
  ambient authority.
- Core-Wasm v1 keeps fixed `memory` and `holo_alloc` exports. The manifest entry
  selects the sole callable export with signature `(i32, i32) -> i64`.
- Core-Wasm v1 imports nothing. WASI and future Hologram host interfaces remain
  unavailable until a versioned contract links each import from an admitted
  capability grant.
- Existing archives whose entry is `holo_run` remain byte-for-byte compatible.
- A missing, empty, or wrongly typed declared entry fails with
  `LIVE_PROTOCOL_ERROR` during provider preparation, before any layer starts.
- V1 does not fabricate a process exit code. A returned byte value is successful
  completion; a trap is a typed protocol failure. Exit status must be additive
  and versioned before it appears in public results.
- Direct and resident providers must share the same contract parser,
  validation, invocation, limits, and errors.

## Slice 1 — Manifest-declared core-Wasm v1 entry

- [x] Introduce one named core-Wasm v1 contract boundary in `holo_wasm`.
- [x] Thread `ResolvedLayer.entry` through direct compilation and invocation.
- [x] Thread the same entry through resident actor compilation and invocation.
- [x] Resolve and type-check the declared export during provider preparation.
- [x] Name the declared entry and contract version in typed missing/signature
  diagnostics.
- [x] Keep public helper compatibility by retaining `holo_run` as the default
  only for APIs that do not receive a manifest layer.
- [x] Change compiler and `hologram app init` Wasm defaults from `_start` to
  `holo_run` and reject an explicitly empty entry.
- [x] Add direct, resident, custom-entry, wrong-entry, and legacy-default unit
  coverage.
- [x] Add BDD proof that a manifest using a non-`holo_run` export compiles and
  executes through the real CLI.

### Slice 1 acceptance

- [x] An archive declaring `entry: "transform"` invokes `transform`, not
  `holo_run`, in direct and resident modes.
- [x] An archive declaring a missing or wrongly typed entry is rejected before
  its provider starts.
- [x] The existing `holo_run` fixture and previously compiled compatible
  archives keep working without migration.
- [x] Generated minimal manifests describe an executable contract rather than
  defaulting to an unrelated WASI-style name.

Slice 1 evidence (2026-08-25): the runtime now owns one `core-wasm-v1`
boundary that validates fixed memory and allocator exports plus the callable
export named by the canonical layer entry. Direct and resident providers carry
that entry through preparation and invocation; compatibility helpers and newly
generated manifests default to `holo_run`. Compiler validation rejects empty or
unsafe diagnostic entry names, and missing or wrongly typed exports fail with
`LIVE_PROTOCOL_ERROR` before a resident record is created. Contract docs make
v1's import-free, fresh-instance, one-output-per-input, and no-numeric-exit
semantics explicit. Unit coverage proves custom, missing, wrongly typed,
legacy-default, nonzero-primary, fat/thin-plan, and direct/resident behavior.
Full verification passes formatting, source-size and product-boundary gates,
all-target checks, 151 library tests, 21 CLI tests, Clippy with warnings denied,
all 11 BDD scenarios / 112 steps, the optimized release build, release smoke,
and the 13-page Astro documentation build.

## Slice 2 — Typed completion and exit model

- [ ] Add an internal provider completion type that keeps byte outputs distinct
  from application completion or exit status.
- [ ] Define which layer kinds are exit-bearing and keep View, Tensor, and
  InferenceModel layers explicitly non-exit-bearing.
- [ ] Define parent behavior when the root primary completes and when a
  non-primary lifecycle-managed layer fails after startup.
- [ ] Preserve core-Wasm v1 semantics as successful completion without a
  fabricated numeric process status.
- [ ] Decide the additive Protobuf/JSON representation for a future explicit
  exit status, including legacy decode defaults.
- [ ] Amend ADR 010 with the primary completion and non-primary failure rules.
- [ ] Add provider, runtime, native round-trip, HTTP, CLI, and BDD coverage.

## Slice 3 — Version negotiation and host-interface design

- [ ] Decide where a future Wasm ABI identifier is canonically represented;
  do not overload the callable entry name.
- [ ] Define compatibility negotiation between core-Wasm v1 and a component
  model / WIT contract.
- [ ] Define the first Hologram WIT world with one byte input and one byte output.
- [ ] Inventory every proposed WASI or Hologram host import and map it to a
  specific admitted capability field.
- [ ] Define unavailable-import and under-granted-import diagnostics before
  linking any host interface.
- [ ] Record the accepted design in a dedicated guest-contract ADR.
- [ ] Keep component-model implementation and Python/WASI proof in M3.1a after
  the design and resource limits are accepted.

## Documentation and conformance

- [x] Update the module-level contract documentation in `src/holo_wasm.rs`.
- [x] Amend ADR 004 and ADR 007 with manifest entry and v1 completion semantics.
- [x] Update README, website CLI and `.holo` guides, architecture, security, and
  actual-capabilities inventory.
- [x] Keep `specs/plans/holo-application-runtime.md` checkboxes and next delivery
  synchronized with completed slices.
- [x] Prove fat and cache-resolved thin archives select the same callable entry.
- [x] Run formatting, source-size/product-boundary gates, all-target checks,
  unit and CLI tests, Clippy, BDD, optimized build, release smoke, and docs.
- [x] Land each completed slice as a reviewable commit with a clean worktree.

## Deferred discoveries

- [ ] `DISC-014` — **Next** — Core-Wasm v1 cannot express a numeric exit status
  separately from its packed byte-output pointer. Route to Slice 2 and the
  guest-contract ADR; do not reinterpret output bytes.
- [ ] `DISC-015` — **Next** — The canonical manifest has no Wasm ABI-version
  field. Route to Slice 3 and upstream format review; do not encode versioning
  in filenames or entry strings.
- [ ] `DISC-016` — **Later** — Provider receipt of scalar budgets does not yet
  enforce fuel, memory, output size, or deadlines. Route to M5 before enabling
  untrusted host interfaces or advertising Python/WASI execution.
