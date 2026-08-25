# `.holo` application compiler and runtime plan

## Status

- State: active
- Created: 2026-08-25
- Format target: `.holo` v4 with v2/v3 read compatibility
- Next delivery: M1, provider and lifecycle ADR
- Next runtime milestone: M1, application planning and provider boundary
- Tracking rule: check an item only after its acceptance criteria and listed verification pass

This is the living implementation plan for turning `.holo` archives into complete Hologram applications. It records the current v4 baseline, compatibility requirements, the recommended application-runtime milestone, an interactive manifest generator, and every prioritized follow-on area: capabilities, multi-layer providers, compiler completion, isolation, installation and content lifecycle, trust, and conformance.

## Product principles

- [x] Keep one append-only `.holo` application format; v4 adds `InferenceModel` without renumbering prior layer kinds and retains v2/v3 reads.
- [ ] Keep the canonical `AppManifest` as application identity and execution truth.
- [ ] Keep the application-directory extension a verified projection, never a second manifest.
- [ ] Keep physical archive identity distinct from canonical application identity.
- [ ] Resolve content by κ; do not make filenames or catalog metadata authoritative.
- [ ] Reject missing capabilities and unsupported providers explicitly; never simulate execution success.
- [ ] Boot ordered layers transactionally and unwind partial starts in reverse order.
- [ ] Keep execution providers behind typed boundaries so Wasm, views, tensors, and root filesystems do not leak engine details into the archive loader.
- [x] Make every interactive workflow available non-interactively for automation and CI.

## Completed baseline

- [x] Read v2/v3 and read/write verified `.holo` v4 archives using the pinned upstream format implementation.
- [x] Compile `hologram.json` into a canonical `AppManifest` plus κ-addressed layer payloads.
- [x] Emit self-contained fat archives by default.
- [x] Emit thin archives with `hologram compile --thin` while preserving identical canonical manifest bytes.
- [x] Embed and verify the application-directory v1 projection.
- [x] Import, list, inspect, verify, and remove archive variants from the local catalog.
- [x] Cache verified fat-archive payloads by κ without replacing user-facing object metadata.
- [x] Resolve primary Wasm content for a thin archive from the local κ cache.
- [x] Execute a self-contained local archive with `hologram run application.holo` without starting the service.
- [x] Load, run, list, and unload a resident Wasm archive through the service.
- [x] Return typed errors for invalid archives, missing content, unsupported layer kinds, and unloaded applications.
- [x] Document the binary layout, logical layer model, fat/thin packaging, and current execution behavior.

## Delivery order

1. M0: interactive manifest generation. Completed in the first implementation slice.
2. M1: `ApplicationPlan`, closure resolution, and the provider/lifecycle boundary.
3. M2: capability decoding, grants, and child attenuation.
4. M3: multi-layer execution—Wasm migration, then View, Tensor, and Rootfs providers.
5. M4: compiler completion and deterministic source-to-layer transformations.
6. M5: execution resource isolation and cancellation.
7. M6: application installation, archive variants, cache ownership, and garbage collection.
8. M7: signatures, publisher identity, and trust policy.
9. M8: conformance, fuzzing, portability, and release gates.

M0 may land before M1 because it is isolated. M2 must land before executing child applications. M1 is the architectural prerequisite for M2 and all new providers. M4 view bundling must land before the View provider is considered production-ready. M5 is required before providers are enabled for untrusted applications. M6 and M7 can begin after M1 exposes stable application identity.

## M0 — Interactive `hologram.json` generator

### Command design

- [x] Add `hologram app init [DIRECTORY]` as the friendly interactive command.
- [x] Keep the existing top-level `hologram init` dedicated to service configuration.
- [x] Default `DIRECTORY` to the current directory.
- [x] Detect whether stdin and stderr are terminals before prompting.
- [x] Refuse to prompt in non-interactive environments unless all required choices are supplied as flags.
- [x] Add non-interactive flags for layer kind, layer path, entrypoint, architecture, surface, primary layer, and capability file.
- [x] Support repeated layer flags or a repeatable interactive “add another layer” step.
- [x] Show the resulting compile and run commands after generation.
- [x] Return a machine-readable creation report when global `--json` is active.

### Generated files and safety

- [x] Generate a minimal schema-v1 `hologram.json` accepted by the same parser used by `hologram compile`.
- [x] Offer Wasm, tensor, rootfs, view, and inference-model layer templates without claiming unsupported runtime execution.
- [x] Accept an existing capability-file path but defer generating `capabilities.json` until M2 defines and validates its canonical source schema; do not scaffold a placeholder that will become invalid.
- [x] Generate paths relative to `hologram.json`; never persist absolute workstation paths by default.
- [x] Write files atomically.
- [x] Refuse to overwrite an existing manifest unless `--force` is explicit.
- [x] Do not leave a partial scaffold if any write or validation step fails.
- [x] Validate the generated manifest before reporting success.
- [x] Keep packaging selection out of `hologram.json`; explain `compile` versus `compile --thin` after generation.

### M0 acceptance criteria

- [x] A new user can generate a one-layer Wasm manifest interactively and compile it without editing JSON.
- [x] A CI job can generate the same manifest with flags and no terminal prompts.
- [x] Rootfs and view prompts require `arch` and `surface`, respectively.
- [x] Existing files survive aborted, invalid, and non-`--force` runs unchanged.
- [x] Automated CLI and BDD tests cover prompt decisions, flags, validation, overwrite protection, and JSON output.
- [x] A BDD scenario generates, compiles, and directly executes the Wasm fixture.
- [x] README and website CLI documentation include the interactive and non-interactive workflows.

## M1 — Canonical application plan and provider boundary

### Identity model

- [ ] Introduce an explicit identity record containing archive object κ, archive footer fingerprint, and canonical application-manifest κ.
- [ ] Add `application_kappa` to inspection and compile reports without renaming the existing physical archive `kappa` field silently.
- [ ] Prove in tests that fat and thin variants have different archive IDs but the same application κ.
- [ ] Make logs, errors, resident records, and audit events identify which identity they report.

### `ApplicationPlan`

- [ ] Add a runtime-owned `ApplicationPlan` decoded from the canonical `AppManifest`.
- [ ] Preserve manifest layer order and primary-layer position in the plan.
- [ ] Represent each resolved layer with its position, kind, content κ, entrypoint, kind-specific auxiliary value, bytes, and resolution source.
- [ ] Distinguish embedded, local-store, and future synchronized resolution sources.
- [ ] Resolve and validate the required capability-set object before preparing providers.
- [ ] Resolve every layer payload before any layer starts, rather than resolving only the primary Wasm layer.
- [ ] Resolve child application and delegated-capability references recursively.
- [ ] Detect child-application cycles.
- [ ] Apply explicit maximum closure depth, object count, and cumulative resolved-byte limits.
- [ ] Deduplicate equal κ references while retaining every logical edge and layer position.
- [ ] Reject a declared embedded κ whose bytes do not re-hash to that κ.
- [ ] Reject unresolved closure members with an error that names the missing κ and referring manifest edge.
- [ ] Keep the application directory out of planning decisions except as an already-verified inspection index.

### Provider interface

- [ ] Define a provider trait keyed by closed `LayerKind` values.
- [ ] Separate provider `prepare`, `start`, `invoke` or attach, and `stop` phases.
- [ ] Give providers only the resolved layer, effective capability grant, resource budget, and explicit host interfaces they need.
- [ ] Make unsupported kinds fail during planning or preparation before any layer starts.
- [ ] Require providers to report resident bytes, lifecycle state, and typed failure details.
- [ ] Avoid exposing Wasmtime, weightc, desktop WebView, or microVM types in shared planning APIs.
- [ ] Decide and document whether provider methods are async and `Send` on each supported platform.

### Transactional lifecycle

- [ ] Introduce explicit planned, preparing, running, stopping, stopped, and failed states.
- [ ] Prepare and start layers in manifest order.
- [ ] If a layer fails, stop every previously started layer in reverse order.
- [ ] Stop all layers in reverse order during normal unload.
- [ ] Route application exit status from the manifest’s primary exit-bearing layer.
- [ ] Do not invent exit semantics for tensor or view layers.
- [ ] Define how a non-primary layer failure affects a running application.
- [ ] Make repeated load and unload requests idempotent where safe.
- [ ] Preserve bounded mailboxes and backpressure for resident applications.
- [ ] Emit structured lifecycle traces and audit events for plan, prepare, start, rollback, and stop.

### Planning interface

- [ ] Add `hologram holo plan <PATH|KAPPA>` for a read-only explanation of identities, resolution sources, layer order, providers, capabilities, children, and blockers.
- [ ] Make `holo plan` useful when execution is unsupported; inspection must not require a provider.
- [ ] Add equivalent native API and JSON/HTTP representations without exposing engine-specific internals.
- [ ] Keep `hologram run <PATH|KAPPA>` output compatible while routing both direct and resident preparation through `ApplicationPlan`.

### M1 acceptance criteria

- [ ] The existing one-layer Wasm direct and resident scenarios pass through `ApplicationPlan` with no behavior regression.
- [ ] A multi-layer manifest is fully resolved before returning the expected unsupported-provider error.
- [ ] A missing non-primary layer prevents all layer starts.
- [ ] A synthetic provider failure proves reverse-order rollback.
- [ ] A cyclic child graph fails deterministically without recursion overflow.
- [ ] Fat and thin variants produce equivalent logical plans when the local store contains the required content.
- [ ] Unit, BDD, API round-trip, docs, Clippy, release build, and smoke gates pass.
- [ ] ADR 004 and ADR 007 are amended if implementation details refine their accepted decisions.

## M2 — Capability enforcement and child attenuation

### Capability source schema

- [ ] Define the schema accepted by source `capabilities.json` using the upstream canonical `CapabilitySet` realization.
- [ ] Reject malformed or non-canonical capability input during `compile --check` and `compile`.
- [ ] Preserve the capability-set κ in `AppManifest.requires`.
- [ ] Provide clear diagnostics that point to the invalid capability entry and source file.

### Runtime grants

- [ ] Define where effective grants come from for direct local execution, local service execution, remote execution, and child applications.
- [ ] Fail before provider preparation unless the effective grant admits the application’s `requires` set.
- [ ] Pass only the effective grant—not the untrusted request—to providers and host interfaces.
- [ ] Add an explicit local-development grant mode without making it the production default.
- [ ] Include capability decisions in structured audit records without leaking secrets.

### Child applications

- [ ] Add source-manifest syntax for child application references and delegated capability documents.
- [ ] Resolve child applications through the same κ closure resolver as layers.
- [ ] Enforce that every delegated child grant is a subset of the parent’s effective grant.
- [ ] Reject capability amplification before starting the child.
- [ ] Define parent/child lifecycle ownership, exit propagation, and rollback behavior.
- [ ] Apply closure and resource limits across the entire application tree, not independently per child.

### M2 acceptance criteria

- [ ] Insufficient grants fail with `LIVE_AUTHORIZATION_DENIED` before any provider starts.
- [ ] Sufficient grants produce the same plan and behavior as the previous Wasm fixture.
- [ ] Child attenuation succeeds; attempted amplification fails deterministically.
- [ ] Capability checks are covered by unit, BDD, audit, and native API tests.
- [ ] Security and `.holo` documentation distinguish requested, granted, delegated, and enforced capabilities.

## M3 — Real multi-layer providers

### M3.1 Wasm provider migration

- [ ] Move the current Wasmtime implementation behind the provider trait.
- [ ] Preserve direct and resident execution behavior and typed guest-contract errors.
- [ ] Use the manifest entrypoint instead of assuming one hard-coded function where the contract permits it.
- [ ] Preserve one-output-per-input compatibility until a versioned guest-contract upgrade lands.
- [ ] Remove the runtime’s “exactly one layer at primary position zero” special case.

### M3.1a Component-model and Python/WASI proof

- [ ] Define a versioned Hologram WIT world beginning with one byte input and one byte output.
- [ ] Add a Wasmtime Component Model provider without weakening core-Wasm guest-contract v1 compatibility.
- [ ] Link WASI and Hologram host interfaces only when admitted by the effective capability grant.
- [ ] Prove a dependency-free Python application bundled with pinned CPython can execute directly and resident.
- [ ] Prove a locked pure-Python dependency is included without reading the developer's ambient virtual environment.
- [ ] Report unsupported WASI modules, imports, and dependencies as typed preparation diagnostics.
- [ ] Add component fuel, memory, input/output, deadline, and cancellation limits before advertising Python/WASI execution.

### M3.2 View provider

- [ ] Define a versioned, deterministic view-bundle payload rather than treating one HTML file as an entire application UI.
- [ ] Define supported surfaces beginning with `portable` and the desktop attachment contract.
- [ ] Attach view layers when their target surface becomes available.
- [ ] Keep views non-exit-bearing and route application exit through the primary Wasm or rootfs layer.
- [ ] Define the intent/message boundary between the view and its application without granting ambient desktop authority.
- [ ] Make direct headless execution report an explicit unavailable-surface capability when a required view cannot attach.
- [ ] Demonstrate a composed Wasm + View `.holo` application in Hologram Desktop.

### M3.3 Inference-model provider

- [x] Upgrade the archive/space boundary to additive `.holo` v4 while retaining v2/v3 reads.
- [x] Represent `InferenceModel` as a non-exit-bearing service layer with required entry and engine tags.
- [x] Include model entry, engine, content κ, and embedded size in verified inspection output.
- [x] Add `hologram ai inspect` as a metadata-only operation that never initializes an engine.
- [x] Keep `hologram run` honest with a typed missing-provider error for model-only applications.
- [ ] Consume and validate the deterministic R4 bundle through the `hologram-ai` facade rather than duplicating its schema.
- [ ] Add `hologram ai compile` and `hologram ai infer` by delegating source acquisition, compilation, loading, and sessions to `hologram-ai`.
- [ ] Route a selected archive model into Chat and the OpenAI/Ollama compatibility modules.
- [ ] Define model residency, cancellation, resource budgets, and session reuse at the provider boundary.
- [ ] Prove a real pinned model can compile in `hologram-ai`, import into Live, and answer a prompt end to end.

### M3.3a Tensor provider

- [ ] Define the TensorPlan payload contract and supported port metadata.
- [ ] Adapt the existing weightc engine boundary as the first provider instead of embedding unstable upstream CPU code.
- [ ] Map manifest entrypoints to inference sessions.
- [ ] Define tensor input/output typing, model residency, cancellation, and session reuse.
- [ ] Keep a missing weightc implementation or unsupported artifact as a typed provider-capability error.
- [ ] Add a tensor-only, no-primary session workflow without pretending it has an application exit code.

### M3.4 Rootfs provider

- [ ] Define the rootfs payload and boot descriptor contract, including architecture validation.
- [ ] Use the mvm/microVM boundary rather than booting an untrusted rootfs in the host process.
- [ ] Add an explicit rootfs provider selector and adapter: local OCI for trusted development, `mvm`/hologram-sandbox for production isolation.
- [ ] Extend the `mvm` guest protocol to mount or restore the verified Python rootfs payload and invoke the byte-oriented launcher.
- [ ] Provide an HVF path or a remote Linux execution target before advertising `mvm` rootfs execution on Apple Silicon.
- [ ] Add a resident framed Python worker for repeated local calls so container startup is paid once per loaded application.
- [ ] Cache completed Python rootfs bundles by source, lock, toolchain, base digest, and target so unchanged recompiles avoid image export and compression.
- [ ] Refuse an architecture mismatch before VM creation.
- [ ] Define block, network, console, shutdown, exit-code, snapshot, and cleanup semantics.
- [ ] Enforce capabilities and resource budgets at the VM boundary.
- [ ] Prove crash containment and cleanup with failure-injection tests.

### M3 acceptance criteria

- [ ] Wasm remains fully compatible behind the provider boundary.
- [ ] Wasm + View validates ordered multi-layer startup and reverse-order shutdown in the desktop.
- [ ] Tensor execution uses a real weightc artifact and reports typed ports/results.
- [ ] Inference-model execution uses a real `hologram-ai` archive and reports typed completions/status.
- [ ] Rootfs execution is not marked supported until microVM isolation and cleanup gates pass.
- [ ] Provider availability appears in `holo plan`, module capabilities, health, and documentation.

## M4 — Compiler completion

### Source transformations

- [ ] Normalize `.wat` source into WebAssembly binary during compilation so `WasmCodemodule` content is portable Wasm bytes.
- [ ] Validate Wasm binaries and their selected guest-contract version before writing an archive.
- [ ] Define deterministic view-bundle construction with stable file ordering, normalized paths, and reproducible bytes.
- [ ] Validate tensor and rootfs payload metadata without claiming to compile formats the selected provider cannot consume.
- [ ] Keep source-language compilation and archive assembly as explicit stages with actionable diagnostics.

### Python source compilation

- [x] Add source-manifest schema v2 with a typed source recipe while retaining schema-v1 prebuilt `path` compatibility.
- [x] Add `hologram app init --template python` and non-interactive flags for project, entrypoint, lock file, and execution profile. Interactive Python-specific prompting remains a UX follow-up.
- [ ] Support a portable `wasi-component` profile that emits a `WasmCodemodule`, not a new layer kind.
- [x] Require `uv.lock` and resolve it for the declared Linux target in a clean OCI build root for the experimental rootfs provider.
- [ ] Pin and record the Python runtime, component toolchain, target ABI, dependency artifacts, and hashes.
- [x] Stage only `pyproject.toml`, the declared lock file, `src/`, and the generated launcher for the experimental rootfs provider; reject absolute/escaping paths and symlinks.
- [ ] Diagnose native dependencies that lack a compatible WASI build and recommend the explicit rootfs profile.
- [ ] Promote the experimental `rootfs` Python profile to supported status only after digest pinning, reproducibility, and the microVM provider are ready. The current direct OCI provider is demo-only.
- [ ] Normalize file order, paths, permissions, timestamps, generated bindings, and source epoch for reproducible layer κ values.
- [ ] Produce a dependency inventory and build provenance without making it part of canonical application identity unless the schema explicitly says so.
- [ ] Keep fat/thin archive packaging independent from source-language compilation.

### Manifest features

- [ ] Add child applications and delegated capabilities to `hologram.json`.
- [ ] Validate `primary` against exit-bearing layer kinds before reading large payloads.
- [ ] Validate duplicate, missing, unreadable, and unsupported layer paths with source locations.
- [ ] Decide whether hybrid archives with only some embedded blobs are supported and document the decision.
- [ ] Preserve deterministic manifest and section ordering across machines.
- [ ] Ensure source metadata does not introduce absolute paths, timestamps, or other accidental nondeterminism.

### Compiler UX

- [x] Add `hologram compile --check hologram.json` to validate the manifest and source paths without writing an archive. Printing the complete application plan remains a follow-up.
- [ ] Report canonical application κ, physical archive κ, footer fingerprint, packaging profile, layers, and embedded-byte totals.
- [ ] Add human-readable and JSON diagnostics with stable error codes.
- [ ] Integrate M0’s generator with the compiler parser rather than maintaining a second schema model.
- [ ] Consider `hologram app add-layer` only after `app init` proves the interaction model.

### M4 acceptance criteria

- [ ] Repeated compilation of the same sources produces byte-identical archives for the same packaging profile.
- [ ] `.wat` and equivalent `.wasm` input produce the intended portable executable payload.
- [ ] A generated multi-layer manifest passes `--check` and compiles without manual JSON edits.
- [ ] Golden tests cover every source layer kind, child applications, fat, thin, and any supported hybrid profile.
- [ ] The same locked Python project and pinned toolchain produce byte-identical layer payloads and equal application κ values across clean builds.
- [ ] A Python project with a portable dependency executes through the component provider; an incompatible native wheel fails before archive emission with an actionable diagnostic.
- [ ] The Python rootfs profile is not marked executable until architecture validation, microVM isolation, resource limits, and cleanup tests pass.

## M5 — Execution isolation and operational controls

- [ ] Define a versioned resource-budget structure shared by planning and providers.
- [ ] Configure Wasmtime fuel or epoch interruption.
- [ ] Limit Wasm linear memory, table growth, instance count, and compiled-module cache size.
- [ ] Enforce per-invocation wall-clock deadlines.
- [ ] Enforce maximum input and output byte sizes before allocation.
- [ ] Propagate client cancellation into direct and resident execution.
- [ ] Bound concurrent runs per application and globally.
- [ ] Define behavior for queued work during unload and shutdown.
- [ ] Prevent guest traps, panics, or provider crashes from poisoning the runtime registry.
- [ ] Apply equivalent CPU, memory, disk, network, and lifetime budgets to tensor and rootfs providers.
- [ ] Expose budget exhaustion through typed errors, traces, metrics, and audit events.

### M5 acceptance criteria

- [ ] Infinite loops terminate at the configured budget without hanging the service.
- [ ] Memory and output bombs fail before destabilizing the host.
- [ ] Cancellation and timeout release resident queue capacity and provider resources.
- [ ] Load tests demonstrate bounded memory, queues, and concurrency.
- [ ] Security documentation defines default budgets and operator overrides.

## M6 — Application installation and content lifecycle

### Application catalog

- [ ] Add an installed-application record keyed by canonical application κ.
- [ ] Group fat, thin, and any hybrid archive variants beneath the same application record.
- [ ] Preserve each physical archive’s κ, fingerprint, filename, origin, and verification status.
- [ ] Let users list applications separately from physical archive variants.
- [ ] Define install, update, activate, deactivate, and uninstall semantics without mutating canonical identity.

### Content ownership and garbage collection

- [ ] Track which installed application closures reference each cached κ.
- [ ] Add pinning for active or explicitly retained applications.
- [ ] Do not delete shared content while any installed application or pin references it.
- [ ] Add a read-only garbage-collection plan and `--dry-run` before destructive collection.
- [ ] Make interrupted installation and collection recoverable.
- [ ] Define retention for imported archive bytes independently from extracted layer content.
- [ ] Surface cache size, referenced bytes, reclaimable bytes, and last-access information.

### Resolution beyond local cache

- [ ] Add a resolver chain that can include embedded content, local cache, configured registry, and future peer synchronization.
- [ ] Preserve κ verification at every resolver boundary.
- [ ] Add explicit offline behavior and prohibit accidental network access in the default local mode.
- [ ] Deduplicate concurrent fetches and support cancellation and bounded retries.

### M6 acceptance criteria

- [ ] Fat and thin variants appear as one application with two physical packages.
- [ ] Removing one variant does not remove shared content required by the other.
- [ ] Garbage-collection dry runs are deterministic and explain every retained object.
- [ ] A thin application can resolve through a configured resolver and then run offline from the verified cache.

## M7 — Certificates, signatures, and trust policy

- [ ] Write a threat model covering archive corruption, malicious publishers, replay, dependency substitution, and compromised resolvers.
- [ ] Decide whether signatures bind canonical application identity, physical archive bytes, or both through separate attestations.
- [ ] Define a versioned Certificates-section payload and canonical signed message.
- [ ] Include publisher key identity, signature algorithm, scope, and required verification material.
- [ ] Reject ambiguous, duplicate, unsupported, or malformed certificate records.
- [ ] Add a trust-store configuration with explicit local-development behavior.
- [ ] Define policy levels for unsigned, signed-but-untrusted, and trusted applications.
- [ ] Verify trust policy during install/load before provider preparation.
- [ ] Add `hologram holo sign`, `holo verify --trust`, and machine-readable verification reports.
- [ ] Define key rotation, expiration, and revocation behavior without rewriting application identity.
- [ ] Record trust decisions in audit events.

### M7 acceptance criteria

- [ ] Tampering with a signed manifest, layer, or bound archive variant fails verification.
- [ ] A valid signature from an untrusted key is distinguishable from an invalid signature.
- [ ] Trust policy is consistent across direct, resident, local, and remote execution.
- [ ] Golden signature fixtures work across supported operating systems.
- [ ] Security and operator documentation explain signing and trust-store management.

## M8 — Conformance and release hardening

### Golden and compatibility fixtures

- [ ] Maintain golden v3/v4 archives for Wasm, tensor, rootfs, view, inference-model, multi-layer, child-app, fat, thin, signed, and legacy-without-directory cases.
- [ ] Prove fat/thin application-identity equivalence and physical-fingerprint difference.
- [ ] Test supported v2/v3 read compatibility and v4 write behavior.
- [ ] Verify Live-produced archives with upstream Hologram tooling and upstream-produced archives with Live.
- [ ] Add round-trip tests that ensure unknown extension bytes survive tooling that claims to preserve them.

### Negative and fuzz testing

- [ ] Fuzz headers, section tables, manifests, directories, content labels, certificate records, and view bundles.
- [ ] Cover truncation, overlapping sections, duplicate sections, forged κ labels, unknown discriminants, oversized counts, and malformed UTF-8.
- [ ] Cover missing closure members, child cycles, excessive depth, excessive cumulative bytes, and duplicate logical references.
- [ ] Prove parser and planner failures are panic-free and allocation-bounded.
- [ ] Add lifecycle failure injection at every prepare, start, invoke, stop, and rollback boundary.

### Portability and product gates

- [ ] Run compile, inspect, plan, and direct Wasm execution on macOS, Linux, and Windows.
- [ ] Run architecture-specific rootfs planning tests on supported host architectures.
- [ ] Add desktop Wasm + View coverage once the View provider lands.
- [ ] Keep unit tests, Clippy, formatting, source-size, BDD, optimized build, and isolated smoke tests mandatory.
- [ ] Add performance baselines for archive verification, closure resolution, Wasm preparation, resident invocation, and cache lookup.
- [ ] Define backwards-compatibility and migration checks before changing source-manifest, directory, guest-contract, or certificate schema versions.

### M8 acceptance criteria

- [ ] The release matrix exercises every provider that is advertised as available.
- [ ] Malformed or hostile fixtures cannot panic, escape limits, or partially start an application.
- [ ] Cross-tool and cross-platform golden files remain stable in CI.
- [ ] Documentation and actual capability reports are generated or checked against the same provider availability source.

## Cross-cutting API and guest-contract work

- [ ] Define version negotiation for the core-Wasm guest contract.
- [ ] Move beyond fixed anonymous one-input/one-output execution with typed, named ports.
- [ ] Define application exit status separately from byte outputs.
- [ ] Add structured logs and diagnostics without treating stdout as a protocol.
- [ ] Define streaming output only after provider cancellation and backpressure are in place.
- [ ] Define stateful sessions as an explicit API rather than silently changing per-run fresh-instance behavior.
- [ ] Preserve v1 guest compatibility while introducing any v2 contract.

## Open decisions to record as ADRs

- [ ] Provider trait async and platform-bound requirements.
- [ ] View-bundle canonical encoding and surface protocol.
- [ ] Direct-execution capability grant source and safe defaults.
- [ ] Child lifecycle and exit propagation.
- [ ] TensorPlan payload/port schema and weightc adapter contract.
- [ ] Rootfs image format, architecture naming, and microVM contract.
- [ ] Resource-budget schema and default limits.
- [ ] Installed-application record and garbage-collection ownership model.
- [ ] Certificate payload, signed message, and trust policy.
- [ ] Guest-contract v2 versioning, typed ports, sessions, and streaming.
- [ ] Hologram WIT world versioning and the relationship between core-Wasm v1 and component-model applications.
- [ ] Python dependency resolver, supported lock formats, toolchain pinning, and build-provenance schema.

## Per-milestone definition of done

- [ ] Public behavior has BDD coverage.
- [ ] Unit and negative tests cover the new invariants and failure paths.
- [ ] Native protocol, JSON/HTTP API, CLI, and desktop behavior agree where the capability is exposed.
- [ ] Error paths use stable typed errors and name the failing application, layer, provider, or κ.
- [ ] Security-sensitive decisions have an ADR and audit coverage.
- [ ] README, website documentation, architecture, and actual-capabilities inventory are current.
- [ ] `cargo fmt`, source-size gate, `cargo check`, tests, Clippy, BDD, release build, and smoke test pass.
- [ ] Changes are committed as a reviewable milestone without unrelated workspace modifications.

## Immediate next slice

- [x] Implement M0 `hologram app init` as a small standalone commit.
- [ ] Draft the M1 provider and lifecycle ADR before runtime refactoring.
- [ ] Add `application_kappa` to compile and inspection results.
- [ ] Introduce the read-only `ApplicationPlan` and full non-child layer resolution.
- [ ] Add `hologram holo plan` over local paths and catalog κ values.
- [ ] Route existing direct and resident Wasm execution through the plan.
- [ ] Add synthetic-provider rollback tests.
- [ ] Extend closure resolution to child applications after M2 grant semantics are fixed.
