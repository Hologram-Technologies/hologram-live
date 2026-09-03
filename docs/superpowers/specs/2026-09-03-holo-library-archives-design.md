# Design: `.holo` library archives

- Status: approved, not yet implemented
- Date: 2026-09-03
- Scope: the library-archive contract only. Remote content resolution, a host-side
  SDK crate, and linkable cross-archive calls are explicitly out of scope; see
  "Adjacent work" below.

## Context

`.holo` is currently understood as an executable format: an archive names ordered
layers, one of which is the primary whose exit code is the application's exit
code. Packaging a reusable artifact — a model, a view bundle, a wasm module meant
to be consumed rather than run — has no stated contract.

The format already permits it. Upstream `uor-hologram` declares
`AppManifest.primary` as `Option<u32>` and documents `None` as "a non-executable
archive (a degenerate tensor-only / library artifact has no app exit code;
'running' it is opening a session)". The canonical encoding reserves
`NO_PRIMARY = u32::MAX` for it, and `AppManifest::validate` accepts it. Live
already produces such archives: the inference-model test in `src/compile.rs`
compiles a manifest with no `primary` and asserts `directory.primary_layer` is
`None`, and `features/fixtures/view-app/hologram.json` is a view-only archive with
no `primary`.

What is missing is not representation. It is *declaration* and *honest reporting*.

Three pieces of reserved-but-unwired code mark the gap:

- `ApplicationPlanReport::require_single_primary()` (`src/application_plan.rs`) —
  the only producer of a missing-primary blocker, with zero callers anywhere in
  the repository.
- `ApplicationPlan::primary()` (`src/application_plan.rs`) — zero callers.
- `ResolutionSource::ConfiguredResolver(String)` (`src/application_plan.rs`) —
  declared and rendered, never constructed. Relevant to adjacent work D, not to
  this design.

The consequences today:

1. A source manifest that omits `primary` by mistake compiles successfully,
   inspects clean, and produces an archive that cannot run. Nothing catches it.
2. `ApplicationPlanReport::runnable()` never consults `primary_layer`. For an
   archive whose layers all have available providers — a wasm-only archive with
   `primary` omitted — `hologram holo plan` and `GET /api/v1/holo/{kappa}/plan`
   report `runnable: true`. Execution then fails at
   `prepare_and_start_with_admitted_grants` with
   `LIVE_CAPABILITY_MISSING`. The failure is clean and rolls back correctly, but
   the plan report — whose entire purpose is a payload-free, truthful
   explanation — is wrong.
3. Where a provider happens to be unavailable (inference-model archives, view-only
   archives), the report is non-runnable for an unrelated reason. "This is a
   library" and "a provider is missing" are currently indistinguishable.

ADR 016 governs the shape of the fix: one complete current contract, explicit
declarations over inference, readers reject missing fields, invalid data fails
close to its boundary with a typed error.

## Decision

### 1. Declaration

The source manifest gains one optional boolean:

```json
{ "schema_version": 4, "library": true, "layers": [ ... ] }
```

`CompileManifest` gains `#[serde(default)] pub library: bool` alongside the
existing `primary: Option<u32>`.

`validate_compile_manifest` enforces the biconditional `library == primary.is_none()`:

- `library: true` with a `primary` present is `LiveError::Config`.
- an absent `primary` without `library: true` is `LiveError::Config`.

Both are raised before any layer content is built.

The declaration survives into the archive without new machinery: `compile_manifest_with_options`
already calls `writer.set_metadata(source)` with the raw `hologram.json` bytes.

### 2. What does not change

- **Canonical `AppManifest`.** `primary: None` remains the single canonical
  signal, encoded through upstream's existing `NO_PRIMARY` sentinel. Application
  κ is unaffected.
- **Application directory.** Stays at schema version 2. Because `library` is
  exactly `primary.is_none()`, recording it in the directory would be pure
  redundancy, and ADR 006 is explicit that the directory is a normalized
  projection that "cannot override" the manifest and is "queryable metadata
  rather than a second source of truth". `primary_layer: null` is already the
  queryable signal.
- **Physical format.** v4 only. No new extension, no schema bump, no migration.
  ADR 016's strict contract is untouched.
- **`HoloPlan` schema.** No new field, and therefore no OpenAPI or gRPC change.

The `library` marker is a compile-time assertion. It exists to catch the typo,
which is the only failure inference cannot catch.

### 3. Plan reporting

**Delete `require_single_primary()`.** It bundles a stale second restriction —
`self.layers.len() != 1` yields "multi-layer lifecycle is not connected yet" —
but multi-layer applications work today (`examples/wasm-view/hologram.json` ships
two layers, and ARCHITECTURE.md describes "ordered multi-layer applications whose
primary is not position zero"). Wiring it as written would break working
archives. Delete `ApplicationPlan::primary()` in the same pass; it is also dead.

**Add one `PlanBlocker` variant**, `LibraryArchive`:

- `kind()` returns `"library_archive"`. This string is the client discriminator
  and is deliberately distinct from `"execution_shape_unsupported"`, so an
  intentional library is never confused with a broken execution shape.
- `error_code()` returns `"LIVE_CAPABILITY_MISSING"`, matching what the runtime
  already raises for this condition.
- `message()` names the archive as a library and states that library archives are
  not executable.

The planner pushes it in exactly one place inside `explain_application`, when the
**root** manifest's `primary` is `None`.

**`runnable()` is not modified.** It already returns false when `blockers` is
non-empty, so pushing the blocker fixes the report for free and lands on the
pattern ARCHITECTURE.md already describes: "successful non-runnable reports with
typed blockers".

Because `explain_application` runs before `registry.evaluate(&mut report)` in both
production callers, `library_archive` occupies `blockers[0]`. Since
`into_application_plan()` converts the *first* blocker into the returned error,
the specific reason wins over an incidental provider-availability one.

### 4. Enforcement

`into_application_plan()` converts the first blocker into a typed error, and it
has exactly two production call sites:

- `HoloRuntime::load_for` (`src/holo.rs`) — the resident path.
- `HoloExecutor::start_session_internal` (`src/holo.rs`) — sessions, and
  therefore `execute_internal`, which delegates to it.

So `hologram run`, `holo load`, and `start_session` are all enforced by the single
blocker. There are no per-command checks to keep in sync, and the HTTP and gRPC
surfaces inherit the behavior because they call the same executor.

**Root only.** The blocker reads the root's `primary_layer`. Child applications
with no primary remain legal, because composition is the library consumption
path. This already works: `application_primary_layer` demands a child primary only
when that child carries a View layer, and only the root primary is ever invoked.
A library composed as a `children` entry prepares, starts, is never invoked, and
stops in reverse order — unchanged by this design. `ResolvedChild.primary_layer`
stays `Option` and stays unexamined.

**Defense in depth.** With the blocker in place, no `ApplicationPlan` with
`primary_layer: None` can be constructed, so the primary check at the top of
`prepare_and_start_with_admitted_grants` becomes unreachable. Keep it, but retype
it from `LiveError::Capability` to `LiveError::Conflict`, matching the file's
existing convention for "the runtime lost a planner invariant". Reaching it now
indicates a bug, not user error, and the error type should say so.

### 5. Error contract

| Situation | Outcome |
|---|---|
| `library: true` with `primary` present | `LIVE_CONFIG_INVALID` at compile, before layers build |
| `primary` absent without `library: true` | `LIVE_CONFIG_INVALID` at compile, before layers build |
| `holo plan` on a library | HTTP 200, `runnable: false`, `primary_layer: null`, blocker kind `library_archive` at index 0 |
| `hologram run` / `holo load` / `start_session` on a library | `LIVE_CAPABILITY_MISSING`, message naming the archive as a library |
| Library composed as a `children` entry | Prepares, starts, never invoked — unchanged |
| `compile` / `import` / `inspect` on a library | Unchanged |

## Consequences

- A mistyped manifest fails at compile with a typed error instead of producing an
  archive that dies at start.
- The plan report stops claiming a library is runnable, and distinguishes "this is
  a library" from "a provider is unavailable".
- Refusing to execute a library now gives the true reason rather than an
  incidental one. Today, running a model-only archive reports a missing inference
  provider, which misleads: installing that provider would not make the archive
  runnable. Provider availability stays visible in the plan report.
- Library archives gain a stated contract: compile, import, inspect, plan, and
  compose as a child. They are inert — never prepared, never started.
- Two pieces of dead code (`require_single_primary`, `ApplicationPlan::primary`)
  are removed.
- `features/fixtures/view-app/hologram.json` must declare `"library": true`. It is
  the only in-repo manifest without a `primary`; all six examples declare
  `"primary": 0`.
- No format, wire, or schema version changes.

## Testing

The headline bug is expressible red-first against current `main`, but the test
must build `AppManifest` directly rather than going through `compile`: the
`library` field does not exist yet, and `CompileManifest` is
`deny_unknown_fields`, so a source manifest carrying the marker cannot parse
until the field lands. Construct a manifest with `primary: None` and a single
wasm layer, embed the content, and assert `plan_bytes(&bytes)?.runnable ==
false`. That is genuinely red today — `plan_bytes` evaluates through
`direct_registry` with a real `Engine`, so the wasm provider is available, no
blocker is produced, and the report claims `runnable: true`. The existing
`model_only_execution_reports_the_missing_inference_provider` test in `holo.rs`
is the pattern to follow.

Compile-level tests for the marker are written after the field exists.

**Unit**, inline `#[cfg(test)]` per existing convention:

- `compile.rs` — `library: true` with no primary compiles; `library: true` with a
  primary is `LIVE_CONFIG_INVALID`; no primary without `library: true` is
  `LIVE_CONFIG_INVALID`; the existing inference-model test gains the marker.
- `application_plan.rs` — a library root with available providers yields a single
  `library_archive` blocker at index 0 and `runnable() == false`; a library *as a child* yields no
  such blocker and the plan builds (the regression guard for the root-only rule);
  a multi-layer archive still plans runnable (guards the `require_single_primary`
  deletion).
- `holo.rs` — `load` and `execute` on a library archive return
  `LIVE_CAPABILITY_MISSING` with a library-specific message.

**Two existing `holo.rs` tests change, deliberately.** Both build `AppManifest`
directly, so the source-manifest rule does not touch them — but both assert on an
error message that this design replaces, because `library_archive` occupies
`blockers[0]` and `into_application_plan()` converts the first blocker:

- `load_rejects_a_view_only_archive` asserts the error contains `"view"`. It must
  now assert the library-archive error. A view-only archive is a library; a View
  layer is non-exit-bearing and can never be a primary, so this archive could
  never have run.
- `model_only_execution_reports_the_missing_inference_provider` asserts the
  execute error contains `"ai.default (uor-r4)"` and `"inference provider"`. It
  must now assert the library-archive error. Its *plan-level* assertions survive
  unchanged: they use `.any(...)`, and the `provider_unavailable` blocker is still
  produced and still reported alongside `library_archive`.

Both changes are improvements in diagnosis, not losses of coverage. Provider
availability remains visible in the plan report; what changes is which reason is
returned when execution is refused.

**BDD**, one `@status:enforced` scenario in `features/suites/s2_holo_exec`, per
`features/README.md`'s rule that a scenario accompanies a new public boundary:
compile a library archive, plan it, observe `runnable: false` with
`library_archive`, then run it and observe the typed error.

**Verification:** `just fmt`, `just check`, `just clippy`, `just test`, `just bdd`.

## Alternatives considered

- **Infer library from an absent `primary`; add no source field.** Smallest
  possible change and aligned with upstream's encoding, but a manifest that forgets
  `primary` silently becomes a library. That is exactly the "hides malformed
  inputs" failure ADR 016 exists to prevent. Rejected.
- **Accept `library: true` but do not require it.** Two ways to say one thing,
  contradicting ADR 016's "one complete current contract", and the marker would
  carry no enforcement weight. Rejected.
- **Record `library` in the application directory.** Redundant with
  `primary_layer: null` under the enforced biconditional, and adds a field that can
  disagree with the manifest — against ADR 006's projection rule. Rejected.
- **Allow `start_session` on a library** (prepare and start layers without
  invoking, per ADR 019). Matches upstream's "'running' it is opening a session"
  phrasing, but until cross-archive linking exists nothing can call into a started
  library, so it would hold provider resources with no reachable use, and
  `RunningApplication::invoke` has no no-primary semantics. Rejected as YAGNI; the
  typed blocker makes enabling it later a non-format change.
- **Add a `LiveError` variant for library archives.** `LIVE_CAPABILITY_MISSING` is
  imprecise — nothing is missing a capability. But a new variant ripples into HTTP
  status mapping, gRPC status, and OpenAPI for a cosmetic gain, and the blocker's
  `kind` already carries the precision. Rejected; revisit if the imprecision proves
  confusing in practice.
- **Wire `require_single_primary()` as written.** Its multi-layer restriction is
  stale and would break working archives. Rejected.

## Adjacent work

These were identified during design and are deliberately excluded.

### D — remote content resolution (next design pass)

Layer *references* cannot leave the manifest: every layer κ is folded into the
canonical operand list, so removing one changes the application κ. Layer *content
bytes* already can, via existing fat/thin packaging — identity is stable across
both — but today thin archives resolve only from the local store. The working
consumption path exists now: publish a fat library, `import` it once (which
verifies and caches its layers by κ), and later thin archives resolve those κs
locally.

Remote resolution is the next design pass. Known starting points:

- `ResolutionSource::ConfiguredResolver(String)` is the reserved seam. It is
  already rendered as `configured:{name}` in the plan report and in blocker
  messages, and is never constructed.
- Resolution funnels through one injection point: the `resolve_local` closure
  (`F: FnMut(&str) -> Result<Option<Vec<u8>>>`) threaded through the planner.
- The principal cost is that this closure is **synchronous** while a registry or
  peer resolver is network I/O. Either the planner becomes async — and it sits on
  many call paths — or a blocking bridge is introduced.
- Open questions for that pass: resolver trust and configuration, failure
  semantics when a resolver is unreachable, whether resolution is permitted during
  planning or only at import, and interaction with the existing content store
  provider boundary that anticipates a Kappa Registry backend.

Note that `src/holo_fetch.rs` is guest-mediated fetch under ADR 021 and is a
different concern from content resolution.

### B — linkable cross-archive calls

`AppManifest` has four fields — `primary`, `requires`, `layers`, `children` — and
no import edge. `children` is composition with nested lifetimes, not symbol
resolution. `Layer.content` is a payload κ and cannot address another application.
Adding an import edge changes the canonical `parts()` encoding, hence every
application κ, hence the physical version — an upstream change to the `hologram`
repository, not something Live can decide. An archive extension could carry
imports without touching canonical bytes, but then imports sit outside application
identity and two archives with different imports share an application κ. That is a
design fork requiring an upstream conversation. Note also that the current guest
contracts are documented as the "import-free" Component Model ABI, and
ARCHITECTURE.md states the Wasm boundary "links no WASI or ambient host
interface"; cross-archive calls contradict that boundary directly.

### C — host-side SDK crate

The format core (`holo_format`, `holo_contract`, `holo_directory`,
`holo_capability`, `application_plan`, `compile`) imports only `crate::error`, its
peers, upstream `hologram`, and serde/std — no tokio, axum, config, or server. Two
couplings block a clean split: `utoipa::ToSchema` is derived in `error.rs` and
`protocol.rs` and imported by `application_plan.rs`, putting an OpenAPI concern
beneath the archive contract; and `compile.rs` pulls in `holo_python` and
`holo_python_component`, which spawn subprocesses and import `crate::holo_provider`
— so the builder depends on the runtime. A move plus two targeted decouplings, not
a rewrite.
