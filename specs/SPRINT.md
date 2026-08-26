# Current sprint: M3.1a canonical guest-contract selection

## Sprint status

- State: active
- Started: 2026-08-25
- Last reviewed: 2026-08-25
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: make the canonical Wasm guest-contract selector flow from source
  manifest through the upstream application identity, inspection, planning, and
  provider boundary without enabling Component Model execution prematurely
- Exit signal: legacy core-Wasm archives keep identical canonical bytes and κ,
  explicit Component v1 archives compile and plan as unsupported, and unknown
  source or canonical identifiers fail closed before provider preparation

The completed M3.1 tracker remains in Git history. Durable requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
the accepted negotiation design is
[`adrs/011-holo-guest-contract-negotiation.md`](adrs/011-holo-guest-contract-negotiation.md).

## Evidence reviewed

- [x] Live pins upstream revision `fdd1190` from
  `feature/inference-model-layer`, not upstream `main`.
- [x] Upstream already canonicalizes every layer's `aux` string, so contract
  selection can become application identity without a codec change.
- [x] Upstream currently rejects every non-empty Wasm `aux` as
  `PortableLayerHasArch`; older readers therefore fail closed.
- [x] Live already preserves `aux` through `PlannedLayer`, `ResolvedLayer`, and
  `ProviderContext`, but does not normalize or expose it as a Wasm contract.
- [x] Provider registration is keyed only by `LayerKind`; the existing Wasmtime
  provider would otherwise accept a Component contract as core Wasm.
- [x] Source-manifest schema v3 has no `contract` field and must remain readable.

## Contract guardrails

- Empty canonical Wasm `aux` normalizes to
  `hologram:guest/core-wasm@1` without changing bytes or κ.
- Explicit `hologram:guest/core-wasm@1` and
  `hologram:guest/component@1` are canonical identity-bearing tags.
- Contract identifiers are exact and closed; no filename, entry-name, content
  sniffing, implicit downgrade, or major-version fallback is permitted.
- `entry` remains the callable selection and is never parsed as a contract.
- Component v1 remains unavailable in this slice. Its provider and resource
  limits land together in the next slice.
- Direct and resident planning must produce the same normalized contract and
  provider decision.

## Slice 1 — Upstream canonical validation

- [x] Add public constants for the two accepted explicit Wasm contracts.
- [x] Retain `Layer::wasm` as the empty-tag compatibility constructor.
- [x] Add an explicit-contract Wasm constructor without changing canonical
  encoding.
- [x] Accept empty, core-v1, and component-v1 Wasm tags.
- [x] Reject unknown Wasm tags with a distinct manifest error.
- [x] Continue rejecting non-empty `aux` for TensorPlan layers.
- [x] Prove legacy canonical bytes and κ are unchanged.
- [x] Prove explicit contract tags survive canonical encode/decode.
- [x] Land upstream PR 142 against `feature/inference-model-layer` at
  `c5e33ec`.

## Slice 2 — Live source, inspection, and planning

- [x] Pin the merged upstream contract-tag revision.
- [x] Add source-manifest schema v4 `contract` for Wasm layers only.
- [x] Keep omitted `contract` byte-compatible with schemas v1-v3.
- [x] Add `hologram app init --contract` and the equivalent interactive prompt.
- [x] Expose the normalized contract in the verified application directory,
  inspect, and plan results with legacy decode defaults.
- [x] Normalize empty Wasm tags to core-v1 in one runtime-owned helper.
- [x] Key provider selection by both layer kind and normalized contract.
- [x] Keep core-Wasm direct and resident execution working unchanged.
- [x] Report Component v1 as a typed unavailable blocker before provider
  preparation; reject unknown source identifiers as configuration and unknown
  canonical identifiers as invalid archives; never route either to core.
- [x] Add compiler, identity, inspection, plan, direct, resident, CLI, OpenAPI,
  and BDD coverage.

## Documentation and conformance

- [x] Update ADR 011 with the final upstream API and migration behavior.
- [x] Update README, architecture, security, actual capabilities, and website
  `.holo`, CLI, architecture, and security pages.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Run upstream formatting and focused workspace tests.
- [x] Run Live formatting, boundary gates, all-target tests, Clippy, BDD,
  optimized build, release smoke, WIT conformance, and docs.
- [ ] Land both reviewable PRs and return both repositories to clean `main`.

## Deferred discoveries

- [ ] `DISC-016` — **Next slice gate** — Enforce component memory, fuel,
  input/output, deadline, and cancellation limits before advertising execution.
- [ ] `DISC-018` — **Later** — Define a capability-gated WASI profile only after
  the import-free Component v1 provider works directly and resident.
