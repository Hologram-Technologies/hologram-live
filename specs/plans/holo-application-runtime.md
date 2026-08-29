# `.holo` application compiler and runtime plan

## Status

- State: active
- Created: 2026-08-25
- Format target: strict `.holo` v4 reads and writes
- Active execution tracker: [`specs/SPRINT.md`](../SPRINT.md)
- Current delivery: M4.2 deterministic Python Component build randomness
- Previous delivery: M4.2 clean Python rootfs equality complete
- Next runtime milestone: M3, real multi-layer providers
- Tracking rule: check an item only after its acceptance criteria and listed verification pass

This is the living implementation plan for turning `.holo` archives into complete Hologram applications. It records the strict current v4 baseline, the recommended application-runtime milestone, an interactive manifest generator, and every prioritized follow-on area: capabilities, multi-layer providers, compiler completion, isolation, installation and content lifecycle, trust, and conformance.

## Product principles

- [x] Keep one `.holo` application format; v4 includes `InferenceModel` without renumbering prior layer kinds and rejects all other physical versions.
- [x] Keep the canonical `AppManifest` as application identity and execution truth.
- [x] Keep the application-directory extension a verified projection, never a second manifest.
- [x] Keep physical archive identity distinct from canonical application identity.
- [x] Resolve content by κ; do not make filenames or catalog metadata authoritative.
- [x] Reject missing capabilities and unsupported providers explicitly; never simulate execution success.
- [x] Boot ordered layers transactionally and unwind partial starts in reverse order.
- [x] Keep execution providers behind typed boundaries so Wasm, views, tensors, and root filesystems do not leak engine details into the archive loader.
- [x] Make every interactive workflow available non-interactively for automation and CI.

## Completed baseline

- [x] Read and write verified `.holo` v4 archives using the pinned upstream format implementation; reject all other versions at Live boundaries.
- [x] Compile `hologram.json` into a canonical `AppManifest` plus κ-addressed layer payloads.
- [x] Emit self-contained fat archives by default.
- [x] Emit thin archives with `hologram compile --thin` while preserving identical canonical manifest bytes.
- [x] Embed and verify the application-directory v1 projection.
- [x] Import, list, inspect, verify, and remove archive variants from the local catalog.
- [x] Cache verified fat-archive payloads by κ without replacing user-facing object metadata.
- [x] Resolve primary Wasm content for a thin archive from the local κ cache.
- [x] Execute a self-contained local archive with `hologram run application.holo` without starting the service.
- [x] Compile and execute a source directory or `hologram.json` in memory with `hologram run <PROJECT>`.
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
- [x] Support schema-v4 child application archives and delegated capability
  files through paired flags or a repeatable interactive prompt.
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

- [x] Introduce an explicit identity record containing archive object κ, archive footer fingerprint, and canonical application-manifest κ.
- [x] Add `application_kappa` to inspection and compile reports without renaming the existing physical archive `kappa` field silently.
- [x] Prove in tests that fat and thin variants have different archive IDs but the same application κ.
- [x] Make logs, errors, resident records, and audit events identify which identity they report.

### `ApplicationPlan`

- [x] Add a runtime-owned `ApplicationPlan` decoded from the canonical `AppManifest`.
- [x] Preserve manifest layer order and primary-layer position in the plan.
- [x] Represent each resolved layer with its position, kind, content κ, entrypoint, kind-specific auxiliary value, bytes, and resolution source.
- [x] Distinguish embedded, local-store, and future synchronized resolution sources.
- [x] Resolve and validate the required capability-set object before preparing providers.
- [x] Resolve every layer payload before any layer starts, rather than resolving only the primary Wasm layer.
- [x] Resolve child application and delegated-capability references recursively.
- [x] Detect child-application cycles.
- [x] Apply explicit maximum closure depth, application count, object count,
  aggregate layer count, and cumulative resolved-byte limits.
- [x] Deduplicate equal κ references while retaining every logical edge and layer position.
- [x] Reject a declared embedded κ whose bytes do not re-hash to that κ.
- [x] Reject unresolved closure members with an error that names the missing κ and referring manifest edge.
- [x] Keep the application directory out of planning decisions except as an already-verified inspection index.

### Provider interface

- [x] Define a provider trait keyed by closed `LayerKind` values.
- [x] Separate provider `prepare`, `start`, `invoke` or attach, and `stop` phases.
- [ ] Give providers only the resolved layer, effective capability grant, resource budget, and explicit host interfaces they need.
- [x] Make unsupported kinds fail during planning or preparation before any layer starts.
- [x] Require providers to report resident bytes, lifecycle state, and typed failure details.
- [x] Avoid exposing Wasmtime, weightc, desktop WebView, or microVM types in shared planning APIs.
- [x] Decide and document whether provider methods are async and `Send` on each supported platform.

### Transactional lifecycle

- [x] Introduce explicit planned, preparing, running, stopping, stopped, and failed states.
- [x] Prepare and start layers in manifest order.
- [x] If a layer fails, stop every previously started layer in reverse order.
- [x] Stop all layers in reverse order during normal unload.
- [x] Route typed application completion or provider-observed exit status from the manifest’s primary exit-bearing layer.
- [x] Do not invent exit semantics for tensor, view, or inference-model layers.
- [x] Define how an observed non-primary layer failure affects a running application; the autonomous provider notification mechanism lands with the first provider that needs it.
- [x] Make repeated load and unload requests idempotent where safe.
- [x] Preserve bounded mailboxes and backpressure for resident applications.
- [ ] Emit structured lifecycle traces and audit events for plan, prepare, start, rollback, and stop.

### Planning interface

- [x] Add `hologram holo plan <PATH|KAPPA>` for a read-only explanation of identities, resolution sources, layer order, providers, capabilities, children, and blockers.
- [x] Make `holo plan` useful when execution is unsupported; inspection must not require a provider.
- [x] Add equivalent native API and JSON/HTTP representations without exposing engine-specific internals.
- [x] Keep `hologram run <PATH|KAPPA>` output compatible while routing both direct and resident preparation through `ApplicationPlan`.
- [x] Accept project directories and `hologram.json` manifests in `hologram run`, with file and UTF-8 text inputs.
- [x] Expose fixed desktop import/verify/download/direct-run commands and an Applications input/output panel for watched builds and existing archives.
- [x] Give the Desktop-owned local transport enough room for documented rootfs
  archives, with one bounded restart/retry for a transport-size failure.

### M1 acceptance criteria

- [x] The existing one-layer Wasm direct and resident scenarios pass through `ApplicationPlan` with no behavior regression.
- [x] A multi-layer manifest is fully resolved before returning the expected unsupported-provider error.
- [x] A missing non-primary layer prevents all layer starts.
- [x] A synthetic provider failure proves reverse-order rollback.
- [x] A cyclic child graph fails deterministically without recursion overflow.
- [x] Fat and thin variants produce equivalent logical plans when the local store contains the required content.
- [x] Unit, BDD, API round-trip, docs, Clippy, release build, and smoke gates pass.
- [x] ADR 004 and ADR 007 are amended if implementation details refine their accepted decisions.

## M2 — Capability enforcement and child attenuation

### Capability source schema

- [x] Define the schema accepted by source `capabilities.json` using the upstream canonical `CapabilitySet` realization.
- [x] Reject malformed or non-canonical capability input during `compile --check` and `compile`.
- [x] Preserve the capability-set κ in `AppManifest.requires`.
- [x] Provide clear diagnostics that point to the invalid capability entry and source file.

### Runtime grants

- [x] Define where effective grants come from for direct local execution, local service execution, remote execution, and child applications.
  - [x] Direct local execution uses the deny-by-default baseline or an explicit trusted `--development-grant` file.
  - [x] Local service execution uses the baseline or loopback-only `holo.development_grant` host configuration.
  - [x] Remote callers cannot attach self-asserted grants; absent trusted remote authority remains denied.
  - [x] Child execution receives only an admitted attenuation of the parent grant.
- [x] Fail before provider preparation unless the effective grant admits the application’s `requires` set.
- [x] Pass only the effective grant—not the untrusted request—to providers and host interfaces.
- [x] Add an explicit local-development grant mode without making it the production default.
- [x] Include capability decisions in structured audit records without leaking secrets.

### Child applications

- [x] Add source-manifest syntax for child application references and delegated capability documents.
- [x] Resolve child applications through the same κ closure resolver as layers.
- [x] Enforce that every delegated child grant is a subset of the parent’s effective grant.
- [x] Reject capability amplification before starting the child.
- [x] Define parent/child lifecycle ownership, exit propagation, and rollback behavior.
- [x] Apply closure and resource limits across the entire application tree, not independently per child.

Compiler evidence (updated 2026-08-26): source-manifest schema v4 accepts child entries
that pair a verified, self-contained child `.holo` archive with a canonical
delegated-capability document. Fat parents embed the canonical child manifest,
its verified closure blobs, and the delegated capability object. Thin parents
omit those payloads while preserving the same canonical parent application κ.
`hologram app init` exposes the same model through repeatable paired flags and
interactive prompts. Runtime planning now iteratively resolves canonical child
manifests, delegated and requested capability objects, and nested layers under
one tree-wide budget. It reports application count and maximum depth through
the plan API. Strict plans retain distinct delegated and requested capability
objects for runtime admission. Admission proves parent grant → delegation →
child request for every edge before provider preparation. The runtime then
prepares and starts the tree depth-first in manifest order, passes each child
only its admitted delegated grant, and rolls back or stops the exact reverse
order. Only the root primary is invoked; child primaries are lifecycle-managed
dependencies until an explicit child invocation contract is introduced.

### M2 acceptance criteria

- [x] Insufficient grants fail with `LIVE_AUTHORIZATION_DENIED` before any provider starts.
- [x] Sufficient grants produce the same plan and behavior as the previous Wasm fixture.
- [x] Child attenuation succeeds; attempted amplification fails deterministically.
- [x] Capability checks are covered by unit, BDD, audit, native API, and HTTP/OpenAPI tests.
- [x] Security and `.holo` documentation distinguish requested, granted, delegated, and enforced capabilities.

### M2.1 Strict capability objects

- [x] Require canonical `CapabilitySet` bytes for requested and delegated
  capability objects.
- [x] Use the canonical empty set as the only deny-all representation.
- [x] Reject zero-byte and other malformed objects after κ verification.
- [x] Keep source compilation and trusted grant decoding on the same canonical
  contract.
- [x] Document the strict rule in `.holo`, security, and capability docs.

## M3 — Real multi-layer providers

### M3.1 Wasm provider migration

- [x] Move the current Wasmtime implementation behind the provider trait.
- [x] Preserve direct and resident execution behavior and typed guest-contract errors.
- [x] Use the manifest entrypoint instead of assuming one hard-coded function where the contract permits it.
- [x] Name and document the import-free `core-wasm-v1` contract, including its
  fixed memory/allocator exports, manifest-selected callable export, fresh
  instances, one-output-per-input behavior, and lack of numeric exit status.
- [x] Align compiler and app-generator defaults with the executable v1
  `holo_run` compatibility entry while permitting another declared export.
- [x] Introduce a typed provider completion model that does not conflate byte
  output, successful completion, and a future explicit exit status.
- [x] Require `returned` or provider-observed `exited { code }` completion
  through JSON/HTTP and Protobuf/gRPC.
- [x] Select canonical Wasm `Layer.aux` as the namespaced guest-contract
  identifier, retaining empty as the byte-compatible core-Wasm v1 alias.
- [x] Define exact-major contract negotiation, the import-free Component Model
  v1 WIT world, capability-to-import mapping, and fail-closed diagnostics in
  ADR 011.
- [x] Preserve one-output-per-input compatibility until a versioned guest-contract upgrade lands.
- [x] Remove the runtime’s “exactly one layer at primary position zero” special case.

### M3.1a Component-model and Python/WASI proof

- [x] Define a versioned, import-free Hologram WIT world beginning with one byte input and one byte output.
- [x] Require explicit canonical `hologram:guest/core-wasm@1` and
  `hologram:guest/component@1` selectors without changing the codec.
- [x] Add required source-manifest schema v4 `contract` and `app init --contract`.
- [x] Expose the normalized contract through verified directory, inspect,
  plan, JSON/HTTP, OpenAPI, and Protobuf/gRPC surfaces.
- [x] Select providers by exact `(LayerKind, contract)`; the selector slice
  proved Component v1 failed closed before its provider landed, and the current
  provider remains isolated from core Wasm.
- [x] Add a Wasmtime Component Model provider without weakening core-Wasm guest-contract v1 compatibility.
- [ ] Link WASI and Hologram host interfaces only when admitted by the effective capability grant.
- [x] Prove a dependency-free Python application bundled with pinned CPython can execute directly and resident.
- [x] Prove a locked pure-Python dependency is included without reading the developer's ambient virtual environment.
- [x] Resolve componentize-py through an exact URL/SHA-256 wheel for every
  server-release host, disable index/source fallback, report the artifact, and
  fail unsupported hosts closed.
- [x] Report unsupported WASI imports and dependency-bearing portable locks as typed preparation/compile diagnostics.
- [x] Add component fuel, memory, input/output, deadline, and cancellation limits before advertising Component execution.

Python Component proof (2026-08-26): source profile `wasi-component` invokes
pinned `componentize-py 0.25.0 --stub-wasi` through an isolated `uvx`
environment, emits an 18.3 MiB import-free Wasm layer, and preserves the
existing `module:function` byte entrypoint through a generated private adapter.
`examples/python-component-hello` executed directly and through catalog
import/load/run/unload with bundled CPython 3.14.0. The opt-in
`just python-component-holo-demo` gate repeats both paths. That initial slice
rejected every non-project package in `uv.lock`; the follow-up below admits a
bounded portable subset. Reproducible Component output remains open; its
non-canonical build provenance and exact tool-artifact selection are now
implemented below.

Locked dependency follow-up (2026-08-26): the compiler now admits registry
packages only through HTTPS, SHA-256-pinned Python 3 `*-none-any.whl` artifacts
already present in `uv.lock`. It chooses one qualifying wheel deterministically,
installs exact direct references into a private target with uv configuration,
indexes, dependency re-resolution, source builds, and cache links disabled,
then scans the result for symlinks and native payload suffixes before exposing
that target to componentization. `examples/python-component-dependency` bundles
`six==1.17.0` and executes directly and resident. A separate proof places a
poisoned `six.py` on ambient `PYTHONPATH` and in a fake virtual environment;
the guest still reports the locked `six-1.17.0`. Native/source-only and
non-registry packages fail during `compile --check` with rootfs guidance.

This does not satisfy deterministic-output acceptance. Two clean compiles of
the locked example produced different component sizes, application κ values,
and archive κ values. Pinned `componentize-py 0.25.0 --stub-wasi` documents that
it bakes a build-time PRNG seed into the component and exposes no seed override.
The versioned provenance slice below now records the toolchain and artifacts.
A deterministic replacement or upstream control remains the next Python
compiler slice.

Build-provenance follow-up (2026-08-26): compile and check results now expose
`build_provenance` schema v1 with `canonical: false`. Each Python Component row
names its manifest layer, compiler, CPython runtime, componentizer release and
source revision, guest contract, target ABI, build host, normalized SHA-256
input hashes, and every selected wheel URL/hash. A completed build adds the
observed uvx/uv versions and component layer κ/length; `--check` omits those
unobserved fields and performs no tool download. The report is not written into
archive metadata, directories, manifests, or content blobs, so build evidence
does not affect canonical or physical `.holo` identity. Source files are
hashed and staged in lexical normalized-path order; nested symlinks and special
files fail before componentization. ADR 013 defines the schema and identity
boundary.

Deterministic output remains unclaimed. Inspection of componentize-py release
revision `c0949b1` confirmed that its pre-initializer owns a private WASI
context with no seed injection. Two controlled non-stubbed builds also differed
in both byte length (18,308,535 vs 18,320,373) and SHA-256, ruling out the stub
adapter as the only source and making fixed-offset output patching unsafe.
Cache reuse is not clean-build reproducibility. The machine-readable report
therefore carries `reproducible: false` and the blocker until a deterministic
componentizer and cross-platform equality gate land.

Exact-artifact follow-up (2026-08-26): version-only uvx resolution has been
removed. The compiler maps the five server-release hosts (macOS arm64/x86_64,
Linux arm64/x86_64, and Windows x86_64) to the upstream componentize-py 0.25.0
wheel URL and PyPI SHA-256, supplies that direct reference with `--no-index
--no-build`, and records it under `componentizer.distribution` in both planned
and completed provenance. An unmapped host returns
`LIVE_CAPABILITY_MISSING`; it cannot fall back to an index or source archive.
The exact arm64 macOS wheel compiled the dependency-free proof and preserved
direct and resident execution. This closes distribution selection, not output
determinism: uvx/host-Python bundling and deterministic pre-initialization
randomness remain separate follow-ups.

Deterministic-componentizer work (started 2026-08-27): upstream revision
`c0949b1` still creates a private WASI context with independently randomized
secure bytes, insecure bytes, and insecure seed, and has no CLI or library seed
control. Hologram now carries a reviewed seven-patch source set under
`tools/componentize-py/`: it supplies
separate fixed-domain streams for both byte interfaces, fixes the insecure
seed, fixes wall and monotonic clocks at epoch zero, and sets
`PYTHONHASHSEED=0` before CPython pre-initialization. The same patch traverses
compiler-owned preopened trees lexically and fixes access/modification times at
epoch zero after generated bindings are complete. This is a build-tool change,
not finished-component rewriting. A dedicated release workflow uses
`scripts/prepare-componentizer-source.sh` to apply the exact patch set, verifies
the published `wasmtime-wasi 46.0.1` crate against SHA-256
`e9f65ef30a2c5478873cdb619085a7a649d3ce41cc3eaf298a7ce3dee96a8e11`,
and builds the same five native host wheels as the standalone-server matrix.
Rust, maturin-action, maturin, and WASI SDK inputs are pinned, and the release
contains wheel and patch-set SHA-256 manifests. The compiler now selects the
five host wheels from immutable release `componentizer-v0.25.0-hologram.5`,
verifies each exact SHA-256, and reports `reproducible: false` until the full
clean-host equality gate passes.

The Hologram distribution also removes upstream CLI discovery of virtualenv,
pipenv, and host-Python site-packages. Hologram supplies the complete staged
source and locked wheel closure explicitly, so appending uvx's own tool
environment would both weaken isolation and expose non-scratch filesystem
metadata to pre-initialization. All remaining preopened inputs are read-only;
this prevents CPython bytecode-cache writes from changing epoch-normalized
directory metadata during the snapshot.

Local equality investigation (2026-08-27) removed generated-section ordering
drift with fixed-seed Rust maps and reduced the remaining difference to exactly
20 bytes in one preinitialized linear-memory data segment. Fixed WASI entropy
and clocks, epoch timestamps, direct CPython hash seeding, and deterministic
allocator selection left those bytes unchanged. They originated at the host
filesystem-identity boundary used by Wasmtime's metadata hash. The approved
build-only policy now maps each distinct host `(device, inode)` pair to a
distinct, context-local guest identity in deterministic observation order.
Before guest execution, that private preinitializer now registers every
identity by walking preopened trees in lexical mount/path order, so runtime
metadata-call order cannot change the mapping. This policy does not alter
Hologram's runtime or the import-free emitted component.

The exact locked release patch set built an arm64 macOS wheel in 20m31s with
SHA-256
`06b3896b922e77bd6257b2b773348f62b37327fa9ea043b61054f70620904f5b`.
Two independent local compiles using separate isolated uvx caches produced
byte-identical 19,554,774-byte archives with layer
`blake3:abb209bfdd3b932910b0bfede3aeb8be477adeff07c6b8feaaafbc41e6e085f8`,
application
`blake3:f03f47117b4d3db6e55b559fe953d0ad60fa86604b8c5a781eaa6dbff7356fef`,
archive
`blake3:585b3a7b0fd048b005f474aa7887798fa7646859019d5277e9866b01e914fb98`,
footer
`9a2893ff163aa67d694e2286af87cc417571e9684e6c8f8fa33c069d11b055b7`,
and complete-file SHA-256
`04d9b1b62ef98336d02ddcd76e13981aa43f636c2c984a616fc7ba6af9907048`.
The resulting archive also executed successfully under Component v1 with
bundled CPython 3.14.0. The release workflow builds the expensive shared CPython
WASI inputs once before fanning out to the five wheel jobs. This closes the
local equality gate, not the release claim. Do not pin the patched wheels or
report `reproducible: true` until the immutable release exists and the five-host
clean-build matrix passes.

First merged-workflow dry run (2026-08-27): run `33140574673` completed the
shared CPython/WASI build in 12m44s, then found two host portability defects
before any tag or release existed. Both macOS architectures rejected GNU-only
`sha256sum --check`, and Windows converted the upstream checkout to CRLF while
the vendored crate remained LF, preventing its patch from applying. The
follow-up uses a value-comparing portable SHA-256 helper and forces LF for patch
inputs/upstream source.

Portable five-host workflow proof (2026-08-27): PR #29 merged those fixes as
`951cc25`, and untagged run `33142178976` then passed from merged `main`. The
shared CPython/WASI stage completed in 10m31s; Linux x86_64/arm64, macOS
x86_64/arm64, and Windows x86_64 each built and uploaded a patched wheel; and
the final job generated the wheel SHA-256 and patch-set manifests. The immutable
release job was correctly skipped without a `componentizer-v*` tag. This closes
workflow portability, but not publication, compiler pinning, or the required
two-clean-builds-per-host equality proof.

Immutable distribution publication (2026-08-28): annotated tag
`componentizer-v0.25.0-hologram.1` points to validated commit `8d65bed`, and
tagged run `33188965708` rebuilt and uploaded all five native wheels plus
`SHA256SUMS` and `PATCHSET.sha256`. The resulting non-draft, non-prerelease
[GitHub release](https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.1)
contains exactly those seven assets. A fresh download verified every wheel
against the published wheel manifest and every repository patch against the
published patch-set manifest. Publication is complete; compiler URL/hash
pinning, provenance patch identity, and two clean builds per supported host
remain required before changing the reproducibility claim.

Compiler pinning follow-up (2026-08-28): the closed five-host mapping no longer
references PyPI's upstream wheels. Every host selects its exact asset under
`componentizer-v0.25.0-hologram.1` and the SHA-256 independently verified from
the published manifest; unsupported hosts still fail closed. Planned and
completed Component provenance now includes `componentizer.patch_set` with the
immutable release tag/URL, `PATCHSET.sha256` URL and digest, and contract
`hologram:componentizer/preinitialization-determinism@1`. The old missing-seed
blocker is replaced by the truthful remaining two-clean-builds-per-host gate.
The report stays non-canonical and `reproducible: false` until that matrix
passes.

Clean-component gate implementation (2026-08-28): `just
python-component-repro` now compiles the locked `six` example with independent
Hologram and uv caches, executes every archive, and emits one JSON report on
stdout. It records component layer κ/size, capabilities κ, application κ,
archive κ/size, footer fingerprint, complete-file SHA-256, and the exact build
contract. The reusable `component-reproducibility` workflow runs one build on
two independent runners for each of the five native release hosts and rejects
missing hosts, missing replicas, contract drift, or target-local identity
drift. Server releases depend on the aggregate job. Completed provenance stays
false until the first ten-runner matrix passes.

First matrix result (2026-08-28): run `33196484166` compiled and executed all
ten clean proofs. Both macOS arm64 and x86_64 replica pairs matched completely.
Linux arm64, Linux x86_64, and Windows x86_64 produced equal-length components
whose layer, application, archive, footer, and complete-file identities
differed between replicas despite identical reported build contracts. The
aggregate failed as designed. Diagnostic matrix artifacts now retain the
`.holo` files and expose both mismatched identity sets for byte localization;
the reproducibility claim remains false.

Directory-order correction and immutable `.2` publication (2026-08-28):
retained-archive run `33198288139` repeated the host pattern while every one of
the ten archives executed correctly. Extracting the component layer localized
the replica differences to 17 bytes on Linux x86_64, 19 bytes on Linux arm64,
and 7 bytes on Windows x86_64, all in preinitialized filesystem metadata. PR
#34 (`dec6a00`) added lexical guest-directory enumeration whenever the private
deterministic metadata policy is enabled. Untagged run `33200329304` proved
the updated five-patch source on every native host. Annotated tag
`componentizer-v0.25.0-hologram.2` then triggered tagged run `33203476950`,
which rebuilt all five wheels, generated both manifests, and published the
non-draft, non-prerelease
[immutable release](https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.2).
A fresh download verified all five wheels against `SHA256SUMS`; the independently
computed `PATCHSET.sha256` digest is
`ce542742dfdd624bb25380bf042638a4e7caa5edb7e7560f0f8809343999c37c`.
The compiler now pins those `.2` assets and reports contract
`hologram:componentizer/preinitialization-determinism@2`. Completed provenance
remains false until the new ten-runner acceptance matrix passes.

Pinned `.2` local proof (2026-08-28): two macOS arm64 compiles used separate
Hologram and uv caches, executed successfully, and matched component layer
`blake3:d647d38b165f9f11462791e5bc0df53b97c9f597e805b254eeada2224af72df8`,
application
`blake3:1a35dac18db1dcfa7697e4b67afd5214580c87205942f40c909f7e660a67e010`,
archive
`blake3:2c0cafa298460003ed25ca585e815c3e77c464c2fa9fe38c1cbd53afc22bbadc`,
footer `cb60b3fea1cca459c0197fd0ff51e3b9b9d275c8ad0a56e0e3f0b26cea0e2e05`,
and complete-file SHA-256
`d150fa30cb5492473c5eacc797b5906512f81b99b47823012ebc5101d7f4c9fb`.
The 19,548,031-byte archives both returned the expected locked `six==1.17.0`
response. This closes the new-release local gate only; the five-host matrix is
still authoritative.

Pinned `.2` matrix result (2026-08-28): run `33206743619` compiled and executed
all ten clean proofs. Both macOS replica pairs matched every canonical and
physical identity, but Linux arm64, Linux x86_64, and Windows x86_64 again
produced equal-length components with different identities. This proves
lexically sorted guest directory streams alone are insufficient: the metadata
mapper still assigns identities lazily according to the guest's first metadata
calls, whose order is not stable on those hosts. Completed provenance therefore
remains false. PR #35 moves identity assignment before guest execution: it
walks every preopened tree in lexical mount/path order, registers each host
`(device, inode)` pair once, and makes runtime metadata-call order irrelevant.
Its unit regression test creates equivalent trees in opposite host creation
orders and proves the same path-to-guest-identity sequence. The next gate is a
five-host build of that six-patch source, followed by a new immutable release
and another two-replica matrix.

Six-patch release validation (2026-08-28): PR #35 merged as `533dd4c`. A fresh
upstream checkout accepted all six patches, its preregistration regression test
passed, and the vendored componentizer passed a locked feature build. Untagged
release run `33209572217` then built the shared CPython/WASI payload, all five
native wheels, `SHA256SUMS`, and `PATCHSET.sha256` from merged `main`. The
immutable `componentizer-v0.25.0-hologram.3` tag points to that exact commit.
Tagged run `33211899065` rebuilt all five wheels and published the seven-asset,
non-draft, non-prerelease release. A fresh public download verified every wheel
against `SHA256SUMS`, every local patch against `PATCHSET.sha256`, and the
patch-manifest SHA-256
`d281c2667a893fffa7e7d64c3b34d6ef22d9f40b9b89ab643475705bd0eba9c7`.
PR #33 now pins those exact `.3` URLs and hashes under determinism contract
`hologram:componentizer/preinitialization-determinism@3`. Completed provenance
remains false until the replacement ten-runner matrix passes.

Pinned `.3` local proof (2026-08-28): two macOS arm64 compiles used separate
Hologram and uv caches, executed successfully, and matched component layer
`blake3:cadb16f50a4cef8fd992838fb20c5acb44b2a94e84b0f9a5a56212c32545d716`,
application
`blake3:86d4be4b4900263bde7c38e245379e41a20fa78562d966abf2e5298eae51d805`,
archive
`blake3:344d1e3d84e6c5a217eb63cdfef5a14ebe11ff5034ec7a59b5e47a7a6e025ba8`,
footer `d47bbff76be502f6003211f9b14e7ba46478b40936abba158e5ddd1fab3adde0`,
and complete-file SHA-256
`67efc1a326e380a2fb6e35da7dc002396f0baeb1de4ffb7bf1261d9e680054d3`.
Both 19,547,588-byte archives returned the expected locked `six==1.17.0`
response. This closes the `.3` local gate only; the five-host matrix remains
authoritative.

Pinned `.3` matrix result (2026-08-28): run `33214553697` successfully compiled
and executed both replicas on Linux arm64/x86_64 and macOS arm64/x86_64. Both
Windows replicas failed before componentization because `cap-std` exposes
device/file identity there only for metadata queried from an open handle;
path-derived directory-entry metadata panicked when preregistration requested
`dev()`. PR #36 (`370c92b`) now opens each file or directory before registering
its identity and adds a release-wheel smoke step that invokes the built
componentizer on every platform. Fresh-source patching, the preregistration
unit test, the locked vendored feature build, and PR CI passed. Merged-main
release run `33217328768` then built all five wheels, invoked the packaged
componentizer successfully on every platform (including Windows), and
generated both manifests. Annotated immutable `.4` tag release run
`33219475061` then rebuilt and published the same validated source.
Completed provenance remains false until the replacement matrix passes.

Handle-portable `.4` publication (2026-08-28): annotated tag
`componentizer-v0.25.0-hologram.4` points to merged fix commit `370c92b`.
Tagged run `33219475061` rebuilt the shared CPython/WASI payload and all five
native wheels, invoked the packaged componentizer successfully on every host,
generated both checksum manifests, and published the non-draft,
non-prerelease seven-asset release. A fresh public download verified every
wheel against `SHA256SUMS`, all six repository patches against
`PATCHSET.sha256`, and patch-manifest SHA-256
`1160ed7bd742dd55d798aae7baa2047897d0b188d251af63cbae5f25381c775f`.
PR #33 now pins the exact `.4` assets and determinism contract
`hologram:componentizer/preinitialization-determinism@4`. The local two-build
proof and ten-runner acceptance matrix remain the final gates before completed
provenance may report reproducible output.

Pinned `.4` local proof (2026-08-28): two isolated macOS arm64 compiles
executed successfully and matched component layer
`blake3:37f149dae0f4ddfc95e7e424bdde2825b5978465fc21e56b8a59b41099110a49`,
application
`blake3:cff358ff9052748487822aa98f8d9b51701ffc6e028e7171b253ddb730529176`,
archive
`blake3:dfa39f441e209997de1fd802d8ba1c2ed5c4d73ab4142a3d96dcd57d1b771d31`,
footer `e77557c01644073652f746a82cd9bf6732c970275028f2f020b7cf726eea09e2`,
and complete-file SHA-256
`7fbb256c51c2d2a2f22bcd997a0cebde038f14c83270466ba042caeaf30f6470`.
Both 19,547,588-byte archives returned the expected locked `six==1.17.0`
response. This closes the `.4` local gate; the five-host matrix remains
authoritative.

Pinned `.4` matrix result (run `33221589694`, 2026-08-28): all ten clean
archives compiled and executed. Both macOS architecture pairs matched every
identity, while Linux arm64/x86_64 and Windows x86_64 produced equal-length
components with different identities. Retained-archive comparison localized
each Linux x86_64 delta to three 32-bit nanosecond fields beside stable epoch
seconds in preinitialized linear memory. The private filesystem policy had
normalized settable access/modification times but still exposed host
status/creation timestamps, which differ per clean workspace on Linux and
Windows. A seventh build-only patch now preserves timestamp availability while
mapping every exposed access, modification, and status/creation value to epoch
zero. Its focused regression passes after fresh-source application; completed
provenance remains false pending a corrected release and replacement matrix.

Timestamp-normalized `.5` publication (2026-08-28): PR #37 merged the seventh
patch as `903c671`. Untagged merged-main release run `33224125002` rebuilt the
shared CPython/WASI payload and all five native wheels, invoked each packaged
componentizer successfully, and generated both checksum manifests. Annotated
tag `componentizer-v0.25.0-hologram.5` points to that exact merge commit;
tagged run `33225747320` repeated every build and smoke test and published the
non-draft, non-prerelease seven-asset release. A fresh public download verified
all five wheels against `SHA256SUMS`, all seven repository patches against
`PATCHSET.sha256`, and patch-manifest SHA-256
`8262cb4562428132c29dc4a46780178a5e0f4d7fa1c41549e2f15c76f7dec8ad`.
PR #33 pins those exact `.5` assets and contract
`hologram:componentizer/preinitialization-determinism@5`; completed provenance
remains false pending the replacement matrix.

Pinned `.5` local proof (2026-08-28): two isolated macOS arm64 compiles
executed successfully and matched component layer
`blake3:624884be7f65be8cb3ff4f7c8c9f9109bc33b81456feb8ea74653bd3e1c454b3`,
application
`blake3:bdf89554364b8df2ec40160880194e4bac7244bdbbb7ebc5285a9f8b9144aac0`,
archive
`blake3:b52177ef4d463218037802aa47fa15a62c428b5f666122fe7f1b522869cbcbc2`,
footer `245b38faa865018c52fb9592d47aa56959b98cf3de437c99462ef5b01145b709`,
and complete-file SHA-256
`3207dbf510698d48108064470ac26f17eecb120aa1d00ab78e61d23b0d94e691`.
Both 19,547,588-byte archives returned the expected locked `six==1.17.0`
response. The five-host matrix is the remaining acceptance gate.

Rootfs-provenance follow-up (2026-08-26): the same schema-v1,
`canonical: false` envelope now covers source-compiled Python rootfs layers.
`compile --check` hashes `pyproject.toml`, `uv.lock`, and the normalized source
tree and reports the target, build host, requested base/digest-pin status,
compiler, pinned uv, and planned Docker builder without contacting Docker. A
completed build additionally records observed Docker client/server versions,
the resolved and locally observed base identities when available, final image
ID, rootfs layer κ, and byte sizes. ADR 014 keeps these observations outside
the archive and explicitly distinguishes them from canonical identity or
byte-reproducible OCI output. The NumPy/pandas demo asserts both completed
provenance and execution.

Rootfs base-binding follow-up (2026-08-26): real compilation now resolves a
mutable base through Docker Buildx's raw registry-manifest path, requires
schema 2, computes the manifest SHA-256, and places the resulting immutable
reference in Docker's `FROM` instruction before the build starts. Offline
`compile --check` continues to report the requested mutable value without
inventing a resolution; already digest-pinned inputs bypass registry lookup.
Completed provenance preserves the request and adds `resolved_reference` for
the input Docker actually consumed. Unit coverage proves digest construction,
registry-port parsing, pinned bypass, invalid-manifest rejection, and
Dockerfile binding. The real NumPy/pandas compile resolved
`python:3.12-slim` to
`sha256:7a8b475003c4fe15a2cd4e55e5cfc2f3560bdc9333d624f24cdd6d4340fd7a17`,
matched Docker's reported digest, emitted a 105,790,579-byte fat archive, and
executed with three rows, mean `20.0`, and sum `60.0`. ADR 015 records the
selection/binding boundary.

Rootfs archive-normalization follow-up (2026-08-26): bundle schema 3 now
re-addresses the exact Docker image config and ordered layer bytes under
`blobs/sha256/<digest>`, emits only those blobs plus a canonical manifest, and
writes lexical GNU-tar members with fixed mode, ownership, and timestamp before
fixed-level Zstandard compression. The build receives source epoch zero and
omits injected provenance. Unsafe, duplicate, missing, non-file, oversized, and
image-ID-mismatched exports fail before archive assembly. Two local exports of
the locked NumPy/pandas image produced identical layer, application, archive,
and footer identities; removing the local tag proved the normalized archive
can cold-load and execute. ADR 017 defines the current-only contract. Clean
uncached equality is now proven across both Linux target architectures.

The first two-replica Linux matrix (workflow run `33031626335`) completed all
four builds with Docker 28.0.4 and the same digest-bound base but produced a
different image and rootfs κ on every runner. The target-local differences
remained after application timestamps were normalized, identifying BuildKit's
cross-stage directory serialization as the unstable boundary. The compiler
now has the builder create one sorted GNU tar of `/app` and `/hologram` with
epoch-zero timestamps and numeric root ownership. It copies that tar from a
stopped builder container and uses local `ADD` in the final digest-bound
image, avoiding both foreign-architecture execution and engine-selected tree
ordering. Two local uncached arm64 builds match through the image, rootfs,
application, archive, and footer identities. Workflow run `33035209550`
repeated that proof on two clean Linux runners for each target: both amd64
reports matched and both arm64 reports matched. Completed builds therefore
report `reproducible: true`; offline checking of a mutable base remains false
only until compilation resolves and binds its immutable digest.

### M3.2 View provider

- [ ] Define a versioned, deterministic view-bundle payload rather than treating one HTML file as an entire application UI.
- [ ] Define supported surfaces beginning with `portable` and the desktop attachment contract.
- [ ] Attach view layers when their target surface becomes available.
- [ ] Keep views non-exit-bearing and route application exit through the primary Wasm or rootfs layer.
- [ ] Define the intent/message boundary between the view and its application without granting ambient desktop authority.
- [ ] Make direct headless execution report an explicit unavailable-surface capability when a required view cannot attach.
- [ ] Demonstrate a composed Wasm + View `.holo` application in Hologram Desktop.

### M3.3 Inference-model provider

- [x] Upgrade the archive/space boundary to `.holo` v4 and reject every other physical version.
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

- [x] Wasm remains fully compatible behind the provider boundary.
- [ ] Wasm + View validates ordered multi-layer startup and reverse-order shutdown in the desktop.
- [ ] Tensor execution uses a real weightc artifact and reports typed ports/results.
- [ ] Inference-model execution uses a real `hologram-ai` archive and reports typed completions/status.
- [ ] Rootfs execution is not marked supported until microVM isolation and cleanup gates pass.
- [ ] Provider availability appears in `holo plan`, module capabilities, health, and documentation.

## M4 — Compiler completion

### Desktop development loop and watched projects

- [x] Let Hologram Desktop add a local application directory containing
  `hologram.json` without granting the service ambient filesystem access.
- [x] Watch the selected directory recursively, debounce relevant changes, and
  compile a fat `.holo` archive outside the source tree.
- [x] Import each successful build through the existing catalog boundary so
  `holo list` and `holo inspect` remain the source of truth for the frontend.
- [x] Replace the watched project's prior catalog variant after a changed build
  while preserving content-addressed identity when the output is unchanged.
- [x] Persist watched directory registrations across desktop restarts and show
  compiling, ready, and failed states with actionable diagnostics.
- [x] Add an Applications view that lists real cataloged `.holo` archives and
  renders their verified inspection metadata, directory, layers, and sections.
- [x] Keep watched-project path authorization local to the desktop adapter;
  remote authorities and archive contents cannot request arbitrary host
  directories. Keep reusable registration, persistence, filtering, debounce,
  and build-state orchestration in a Tauri-independent workspace crate outside
  `src-tauri`.
- [x] Run cataloged rootfs applications by verifying and downloading their
  immutable archive to a κ-derived cache path, then invoking the direct
  provider rather than claiming resident rootfs support.

Watched-project acceptance:

- [x] Adding the Wasm fixture directory creates a cataloged `.holo` visible in
  the desktop Applications list.
- [x] Editing a referenced source file rebuilds and refreshes the inspected
  archive without writing generated output into the watched directory.
- [x] A compile failure leaves the last successful archive inspectable and
  reports the new failure on the watched project.
- [x] Removing a watch stops future builds without silently deleting the last
  immutable cataloged archive.

Watched-project evidence (2026-08-25): the packaged desktop application added
the Wasm fixture through the native picker, showed the resulting verified v4
archive from the catalog, and inspected its three identities, capabilities,
logical layer, physical sections, and embedded blobs. A temporary source edit
changed the archive κ after the debounce; an invalid manifest reported Failed
while retaining that last good κ; restoring valid source recovered Ready; and
a source edit after removing the watch did not rebuild or delete the final
archive. The implementation persists registrations in the Tauri application
configuration, stores generated archives in its cache, ignores dependency/build
trees, and exposes only fixed compile/import/list/inspect commands. Five watch
engine tests and the desktop adapter test, Clippy with warnings denied,
production frontend and packaged Tauri builds, the Astro site build, and the
complete repository verification pass.

Architecture follow-up (2026-08-25): the watcher engine moved to
`crates/hologram-application-watch`; the Tauri file now only resolves desktop
configuration/cache paths, invokes fixed sidecar commands, and emits UI events.
The extracted engine has five focused tests, including a complete debounced
register/build/persist/remove cycle. Server builds and releases explicitly
select the root `hologram-live` package and `hologram` binary, preserving a
Tauri- and Node-independent cloud deployment boundary. The server manifest now
declares Rust 1.94, the actual floor imposed by Wasmtime 46, and release jobs
install the repository's pinned Rust 1.97.1 toolchain consistently. A permanent
product-boundary gate inspects the resolved server graph and rejects Tauri or
application-watch dependencies.

Python follow-up (2026-08-25): the packaged desktop application compiled and
cataloged the locked `examples/python-hello` project as a verified 57.0 MiB v4
archive, inspected its `python_hello_holo:main` rootfs layer, and rendered
`{"message":"Hello, Grace!","name":"Grace","runtime":"python"}` from the Run
panel. Desktop sidecars use a 256 MiB local RPC limit, and the watched import
performs one restart/retry only for a transport-size failure without weakening
the server default. Full
repository verification, Desktop Clippy, documentation, and packaged macOS
application/DMG builds pass.

### Source transformations

- [ ] Normalize `.wat` source into WebAssembly binary during compilation so `WasmCodemodule` content is portable Wasm bytes.
- [ ] Validate Wasm binaries and their selected guest-contract version before writing an archive.
- [ ] Define deterministic view-bundle construction with stable file ordering, normalized paths, and reproducible bytes.
- [ ] Validate tensor and rootfs payload metadata without claiming to compile formats the selected provider cannot consume.
- [ ] Keep source-language compilation and archive assembly as explicit stages with actionable diagnostics.

### Python source compilation

- [x] Add typed source recipes and prebuilt `path` layers to the required source-manifest schema v4.
- [x] Add `hologram app init --template python` and non-interactive flags for project, entrypoint, lock file, and execution profile. Interactive Python-specific prompting remains a UX follow-up.
- [x] Keep a minimal locked standard-library Python project as a fast teaching example alongside the NumPy/pandas dependency proof.
- [x] Support a portable `wasi-component` profile that emits a `WasmCodemodule`, not a new layer kind.
- [x] Require `uv.lock` and resolve it for the declared Linux target in a clean OCI build root for the experimental rootfs provider.
- [x] Record the already-pinned Python runtime, component toolchain, target ABI,
  dependency artifact URLs, and hashes in a versioned build-provenance report.
- [x] Stage only `pyproject.toml`, the declared lock file, `src/`, and the generated launcher for the experimental rootfs provider; reject absolute/escaping paths and symlinks.
- [x] Diagnose native, source-only, non-registry, or unpinned dependencies that
  lack a portable wheel and recommend the explicit rootfs profile.
- [ ] Promote the experimental `rootfs` Python profile to supported status only
  after normalized reproducible output and the microVM provider are ready. Base
  inputs are now digest-bound, but the current direct OCI provider is demo-only.
- [x] Normalize rootfs archive member order, blob paths, permissions,
  ownership, timestamps, manifest encoding, compression, and source epoch for
  identical Docker image config/layer inputs.
- [x] Normalize or eliminate clean-build differences inside generated image
  config/layer bytes and prove equal κ values across two independent clean
  Linux builders for each supported rootfs target architecture.
- [x] Diagnose the first clean matrix's target-local mismatch and replace
  `COPY --from=builder` directory serialization with a canonical sorted runtime
  tar consumed by the final image.
- [x] Add `compile --no-build-cache`, record its use in rootfs provenance, and
  emit a `jq`-ready local identity comparison.
- [x] Add a server-release matrix with two clean runners per Linux target and
  compare image, layer, application, archive, and footer identities within
  each architecture.
- [x] Produce a dependency inventory and build provenance without making it part of canonical application identity unless the schema explicitly says so.
- [x] Report planned and completed Python rootfs evidence without requiring
  Docker during `compile --check` or claiming the emitted OCI bytes are
  reproducible.
- [x] Resolve mutable Python rootfs bases to a schema-2 registry manifest
  digest, bind the digest-qualified reference into `FROM`, and report requested
  and resolved identities without mutating the source manifest.
- [ ] Keep fat/thin archive packaging independent from source-language compilation.

### Manifest features

- [x] Add child applications and delegated capabilities to `hologram.json`.
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
- [x] A Python project with a portable dependency executes through the component provider; an incompatible native wheel fails before archive emission with an actionable diagnostic.
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

### Strict pre-release baseline (ADR 016)

- [x] Admit physical `.holo` v4 and source-manifest schema v4 only.
- [x] Require one verified application directory per application archive.
- [x] Require explicit Wasm entry/contract, canonical capability objects, and
  complete configuration, history, resident, and run records.
- [x] Remove speculative migrations and decode defaults from production code,
  tests, examples, smoke coverage, and documentation.
- [x] Verify the server, public BDD surface, static documentation, and packaged
  desktop application on the current macOS release host.

Evidence (2026-08-26): `just verify` passed the locked workspace tests, Clippy,
12 BDD scenarios/123 steps, optimized server build, and isolated smoke test;
`just docs` built 13 static pages; the Tauri release build produced the macOS
application and arm64 DMG. Cross-platform coverage remains in the portability
work below.

### Golden and conformance fixtures

- [ ] Maintain golden v4 archives for Wasm, tensor, rootfs, view, inference-model, multi-layer, child-app, fat, thin, and signed cases.
- [ ] Prove fat/thin application-identity equivalence and physical-fingerprint difference.
- [x] Test rejection of non-v4 archives and v4 read/write behavior.
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
- [x] Keep unit tests, Clippy, formatting, source-size, BDD, optimized build, and isolated smoke tests mandatory.
- [ ] Add performance baselines for archive verification, closure resolution, Wasm preparation, resident invocation, and cache lookup.
- [ ] Define an explicit version transition only when a shipped release creates a real compatibility requirement.

### M8 acceptance criteria

- [ ] The release matrix exercises every provider that is advertised as available.
- [ ] Malformed or hostile fixtures cannot panic, escape limits, or partially start an application.
- [ ] Cross-tool and cross-platform golden files remain stable in CI.
- [ ] Documentation and actual capability reports are generated or checked against the same provider availability source.

## Cross-cutting API and guest-contract work

- [x] Define version negotiation for the core-Wasm and Component Model guest contracts.
- [x] Carry required canonical guest-contract selection from authoring through
  identity, inspection, planning, and provider lookup.
- [ ] Move beyond fixed anonymous one-input/one-output execution with typed, named ports.
- [x] Define application completion and exit status separately from byte outputs.
- [ ] Add structured logs and diagnostics without treating stdout as a protocol.
- [ ] Define streaming output only after provider cancellation and backpressure are in place.
- [ ] Define stateful sessions as an explicit API rather than silently changing per-run fresh-instance behavior.
- [ ] Introduce any future guest contract as an explicit, independently selected version.

## Open decisions to record as ADRs

- [x] Provider trait async and platform-bound requirements.
- [ ] View-bundle canonical encoding and surface protocol.
- [x] Direct-execution capability grant source and safe defaults.
- [x] Child lifecycle ownership and the current root-primary-only exit boundary.
- [ ] TensorPlan payload/port schema and weightc adapter contract.
- [ ] Rootfs image format, architecture naming, and microVM contract.
- [x] Resource-budget defaults and capability tightening for Component v1;
  cross-provider priority/concurrency policy remains future work.
- [ ] Installed-application record and garbage-collection ownership model.
- [ ] Certificate payload, signed message, and trust policy.
- [ ] Guest-contract v2 versioning, typed ports, sessions, and streaming.
- [x] Hologram WIT world versioning and the relationship between core-Wasm v1 and component-model applications.
- [x] Python dependency resolver, supported `uv.lock` inputs, portable-wheel
  admission, and toolchain pinning (ADR 012).
- [x] Python build-provenance schema and non-canonical identity boundary (ADR 013).
- [x] Exact platform componentizer distribution selection and fail-closed host
  coverage (ADR 013).
- [x] Observational Python rootfs build provenance and its non-canonical
  identity boundary (ADR 014).
- [x] Resolve mutable rootfs base tags to a registry manifest digest and bind
  the selected digest to the build.
- [x] Normalize the current Python rootfs Docker archive representation under
  ADR 017.
- [x] Prove byte-identical rootfs layer κ values across uncached clean Linux
  builder replicas for both supported target architectures.
- [ ] Deterministic Python Component output and clean-build equality proof.

## Per-milestone definition of done

- [x] Public behavior has BDD coverage.
- [x] Unit and negative tests cover the new invariants and failure paths.
- [x] Native protocol, JSON/HTTP API, CLI, and desktop behavior agree where the capability is exposed.
- [x] Error paths use stable typed errors and name the failing application, layer, provider, or κ.
- [x] Security-sensitive decisions have an ADR and audit coverage.
- [x] README, website documentation, architecture, and actual-capabilities inventory are current.
- [x] `cargo fmt`, source-size gate, `cargo check`, tests, Clippy, BDD, release build, and smoke test pass.
- [x] Changes are committed as a reviewable milestone without unrelated workspace modifications.

## Immediate next slice

- [x] Implement M0 `hologram app init` as a small standalone commit.
- [x] Draft the M1 provider and lifecycle ADR before runtime refactoring.
- [x] Add `application_kappa` to compile and inspection results.
- [x] Introduce the read-only `ApplicationPlan` and full non-child layer resolution.
- [x] Add `hologram holo plan` over local paths and catalog κ values.
- [x] Route existing direct and resident Wasm execution through the plan.
- [x] Add synthetic-provider rollback tests.
- [x] Extend closure resolution to child applications with bounded, iterative κ
  traversal.
- [x] Admit parent grant → delegated grant → child request chains before any
  provider preparation.
- [x] Prepare and start the admitted child tree in depth-first manifest order,
  invoke only the root primary, and roll back or stop in exact reverse order.
- [x] Complete M2 capability audit records, interface coverage, and conformance
  tests.
- [x] Define and enforce the M3.1 core-Wasm v1 manifest-entry contract without
  adding host imports or weakening existing archives.
- [x] Define typed provider completion and application exit semantics without
  fabricating a core-Wasm v1 process status.
- [x] Decide the canonical guest-contract version negotiation and
  capability-gated host-interface mapping before implementing Component Model
  or WASI support.
- [x] Execute import-free Component v1 directly and resident through its exact
  provider with fresh-store memory/fuel limits, byte ceilings, deadline, and
  cancellation interruption.
- [x] Package and execute dependency-free Python against Component v1 without
  introducing ambient WASI.
- [x] Package and execute a SHA-256-locked pure-Python wheel without consulting
  ambient Python paths, and fail native/source-only locks before emission.
- [x] Define and emit a versioned, non-canonical Python Component provenance
  report with stable source hashes, selected artifacts, observed tools, target,
  output identity, and an explicit reproducibility status.
- [x] Pin the exact componentizer wheel URL/SHA-256 for all five server release
  hosts, report it, disable registry/source fallback, and fail unpinned hosts.
- [x] Emit planned/completed Python rootfs provenance with hashed inputs,
  requested/resolved/observed image identities, observed Docker versions,
  output layer κ, and explicit reproducibility blockers.
- [x] Replace Docker-save byte identity with ADR 017's normalized schema-3
  rootfs archive and prove repeated local export plus cold-load execution.
- [x] Run uncached rootfs builds on two independent clean Linux builders for
  each supported target architecture, compare config/layer/application/archive
  identities, and close every observed difference before setting
  `reproducible: true`.
- [ ] Supply deterministic componentizer randomness and prove byte-identical
  layer, application, and archive κ values across clean supported-host builds
  before claiming reproducible output.
  - [x] Define an exact-revision patch and five-host immutable wheel release
    workflow without post-processing completed Wasm bytes.
  - [x] Publish the patched wheels and SHA-256 manifest under an immutable
    `componentizer-v*` release.
  - [x] Pin those five distributions in the compiler and record the patch
    identity in build provenance.
  - [x] Add a jq-friendly local comparator and a two-replica five-host release
    matrix that executes every proof archive.
  - [x] Retain mismatched archives, localize the clean-host delta to 7–19 bytes
    in preinitialized filesystem metadata, and merge deterministic lexical guest
    directory enumeration in PR #34 (`dec6a00`).
  - [x] Prove the updated five-patch source builds all five native wheels and
    both checksum manifests in untagged release run `33200329304`.
  - [x] Publish and pin the immutable
    `componentizer-v0.25.0-hologram.2` release.
  - [x] Run the `.2` ten-runner matrix and retain the truthful failure: macOS
    pairs match, while Linux and Windows prove lazy first-observation metadata
    identity assignment remains unstable (`33206743619`).
  - [x] Replace lazy assignment with lexical pre-registration of every
    preopened-tree identity before guest execution, with an opposite-creation-
    order regression test (PR #35).
  - [x] Build and publish the six-patch componentizer distribution in dry run
    `33209572217` and tagged release run `33211899065`, then independently
    verify all public wheel and patch checksums.
  - [x] Run the handle-portable `.4` release and replacement matrix, retain its
    truthful failure, and localize the remaining Linux/Windows delta to
    status/creation timestamp nanoseconds (`33221589694`).
  - [x] Add a seventh build-only patch that epoch-normalizes every timestamp
    exposed by the private WASI filesystem, including status/creation time,
    with fresh-source and unit-regression coverage.
  - [ ] Compare two clean builds per host before changing the reproducibility
    claim.
