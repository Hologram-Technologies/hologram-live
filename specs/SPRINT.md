# Current sprint: M3.1 guest-contract negotiation

## Sprint status

- State: complete
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M3.1 — Wasm provider migration](plans/holo-application-runtime.md#m31-wasm-provider-migration)
- Goal: finish the core-Wasm v1 entry and completion contracts, then define the
  canonical version-negotiation and capability-gated host boundary required
  before implementing a Component Model guest ABI
- Exit signal: the canonical contract selector, compatibility rules, first WIT
  world, host-import admission table, and typed diagnostics are accepted without
  advertising Component Model or WASI execution prematurely

This short-lived tracker replaces the completed M2 tracker, which remains in
Git history. Durable requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
accepted decisions stay in [`adrs/`](adrs/).

## Evidence reviewed

- [x] `AppManifest` already binds an `entry` string into every Wasm layer and
  therefore into canonical application identity.
- [x] `ResolvedLayer` and `LayerPrepareContext` preserve that entry through
  planning and provider selection.
- [x] Slice 1 removed the original `holo_run` runtime hard-code: direct and
  resident preparation now resolve the canonical manifest entry.
- [x] The compiler and non-interactive app generator now default an omitted
  Wasm entry to `holo_run`, matching the implemented `(i32, i32) -> i64`
  transform and separate `holo_alloc` export.
- [x] Core-Wasm guest contract v1 has no imports, no WASI, one byte input and
  one byte output per invocation, and a fresh instance for each input.
- [x] Slice 2 separates `LayerInvocation` output bytes from typed `Returned` or
  provider-observed `Exited` completion.
- [x] The current `.holo` manifest has no independent Wasm ABI-version field;
  adding one requires an upstream-compatible format decision rather than an
  undocumented entry-name convention.
- [x] The pinned upstream `Layer.aux` string is canonical, identity-bearing,
  kind-specific, and already encoded for every Wasm layer, but current upstream
  validation requires it to be empty for portable layers.
- [x] Older upstream validators fail closed on a non-empty Wasm `aux`, so using
  that slot for explicit future contract identifiers cannot silently select the
  legacy runtime.

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
  completion; a trap is a typed protocol failure. The public result represents
  completion additively, while any guest-visible numeric exit contract requires
  an explicitly versioned future ABI.
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

- [x] Add an internal provider completion type that keeps byte outputs distinct
  from application completion or exit status.
- [x] Define which layer kinds are exit-bearing and keep View, Tensor, and
  InferenceModel layers explicitly non-exit-bearing.
- [x] Define parent behavior when the root primary completes and when a
  non-primary lifecycle-managed layer fails after startup.
- [x] Preserve core-Wasm v1 semantics as successful completion without a
  fabricated numeric process status.
- [x] Decide the additive Protobuf/JSON representation for a future explicit
  exit status, including legacy decode defaults.
- [x] Amend ADR 010 with the primary completion and non-primary failure rules.
- [x] Add provider, runtime, native round-trip, HTTP/OpenAPI, CLI, and BDD coverage.

Slice 2 evidence (2026-08-25): provider invocation now carries output bytes and
`LayerCompletion` independently. Only an exit-bearing root primary can supply
the application outcome; Wasm reports `Returned`, the Python OCI provider
reports an exit code only after observing successful child-process exits, and
View, Tensor, and InferenceModel layers cannot become the application primary.
The additive public model exposes `returned`, `exited { code }`, and a
legacy-decode-only `unknown` across native JSON, HTTP/OpenAPI, and
Protobuf/gRPC. ADR 010 defines direct cleanup, resident-call completion, child
and non-primary ownership, and the deferred provider-notification mechanism for
autonomous dependency failure. Full verification passes formatting,
source-size and product-boundary gates, all-target checks, 154 library tests, 21
CLI tests, Clippy with warnings denied, all 11 BDD scenarios / 115 steps, the
optimized release build, release smoke, and the 13-page Astro documentation
build.

## Slice 3 — Version negotiation and host-interface design

- [x] Decide where a future Wasm ABI identifier is canonically represented;
  do not overload the callable entry name.
- [x] Define compatibility negotiation between core-Wasm v1 and a component
  model / WIT contract.
- [x] Define the first Hologram WIT world with one byte input and one byte output.
- [x] Inventory every proposed WASI or Hologram host import and map it to a
  specific admitted capability field.
- [x] Define unavailable-import and under-granted-import diagnostics before
  linking any host interface.
- [x] Record the accepted design in a dedicated guest-contract ADR.
- [x] Compile the checked-in WIT through Wasmtime bindgen and guard that the v1
  world remains import-free.
- [x] Keep component-model implementation and Python/WASI proof in M3.1a after
  the design and resource limits are accepted.

Slice 3 evidence (2026-08-25): ADR 011 selects the existing canonical
`Layer.aux` slot as the Wasm guest-contract identifier. Empty remains the
byte-compatible alias for `hologram:guest/core-wasm@1`; explicit
`hologram:guest/component@1` archives fail closed on older runtimes. Exact-major
registry negotiation never falls back across contracts. The checked-in
`hologram:application@1.0.0` WIT world exports one stateless binary `run` and
imports nothing. The host-interface inventory maps storage, channels, network,
and scalar limits to current canonical fields while withholding clocks, random,
environment, process, secrets, inference, and raw ambient WASI authority until
the capability schema can represent them. A Wasmtime-bindgen conformance test
parses the checked-in world and guards its import-free boundary. Unknown
contracts, malformed worlds, and under-granted imports have distinct typed
preparation failures. Upstream validation, runtime implementation, enforced
resource limits, and Python/WASI proof remain explicitly routed to M3.1a.
Full verification passes formatting, source-size and product-boundary gates,
all-target checks, 154 library tests, 21 CLI tests, the WIT conformance test,
Clippy with warnings denied, all 11 BDD scenarios / 115 steps, the optimized
release build, release smoke, and the 13-page Astro documentation build.

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

- [x] `DISC-014` — **Resolved in Slice 2** — Core-Wasm v1 reports `returned`
  separately from byte outputs and does not fabricate a numeric status. The
  additive public type can carry a real provider-observed exit code; a future
  guest-visible exit contract remains part of versioned ABI design.
- [x] `DISC-015` — **Resolved in Slice 3 design** — Use the existing canonical
  Wasm `Layer.aux` tag for namespaced contract identifiers, with empty as the
  legacy core-Wasm v1 alias. Coordinate the validation change upstream before
  emitting a non-empty tag; do not encode versioning in filenames or entries.
- [ ] `DISC-016` — **Next gate** — Provider receipt of scalar budgets does not yet
  enforce fuel, memory, output size, or deadlines. Land the minimum component
  limits in M3.1a before advertising Python/WASI execution and retain broader
  provider hardening in M5.
- [ ] `DISC-017` — **Next** — The pinned upstream validator rejects every
  non-empty Wasm `aux`. Land the namespaced contract-tag validation upstream
  and pin it here before the compiler can emit Component Model archives.
