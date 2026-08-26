# Current sprint: M3.1b locked pure-Python Component dependencies

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: package the exact platform-independent wheels selected by `uv.lock`
  into the existing import-free Python Component v1 build path without reading
  the developer Python environment
- Exit signal: a locked `six` dependency executes directly and resident, a
  poisoned ambient `PYTHONPATH` cannot replace it, and native/source-only locks
  fail before componentization with a typed diagnostic

The completed M3.1a dependency-free tracker remains in Git history. Durable
requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md), and
the accepted negotiation design is
[`adrs/011-holo-guest-contract-negotiation.md`](adrs/011-holo-guest-contract-negotiation.md).

## Packaging policy

- Keep source profile `wasi-component` and the import-free
  `hologram:guest/component@1` contract. This slice adds compiler inputs, not a
  new layer kind, runtime provider, or host capability.
- Traverse the editable project's runtime dependency closure while excluding
  unreferenced development/optional records. Require each reached package to
  use a registry source and contain an HTTPS, SHA-256-pinned Python 3
  platform-independent wheel whose filename ends in `-none-any.whl`.
- Choose one qualifying wheel deterministically, preferring the exact `py3`
  tag and then lexical URL order. Never resolve a version outside `uv.lock`.
- Install exact wheel URLs into a private target with `uv --no-config pip
  install --no-index --no-deps --require-hashes --only-binary :all:` for Python
  3.14. Use copy mode so the component build does not depend on uv cache links.
- Withhold `VIRTUAL_ENV`, `PYTHONPATH`, `PYTHONHOME`, pip/uv index overrides,
  and Python user-site lookup from both dependency installation and
  componentization.
- Reject Git/path dependencies, missing or non-SHA-256 artifacts, native wheel
  tags, source-only packages, installed symlinks, and installed native library
  suffixes with `LIVE_CAPABILITY_MISSING` and rootfs guidance.
- Keep pinned `componentize-py 0.25.0 --stub-wasi`; dependencies enter only
  through the private `--python-path`. Runtime filesystem, network, clocks,
  environment, arguments, stdio, DNS, secrets, and process control stay absent.
- Do not claim byte-for-byte reproducibility yet. Two clean proof compiles
  produced different component/application κ values because the pinned tool's
  `--stub-wasi` mode bakes a build-time PRNG seed into the component and offers
  no deterministic seed option.

## Compiler and validation

- [x] Parse external package records from the declared `uv.lock` without
  consulting the developer environment.
- [x] Traverse only the editable project's runtime dependency closure and
  reject missing or ambiguous dependency references.
- [x] Admit registry packages only through qualifying locked universal wheels.
- [x] Install exact HTTPS wheel URLs under SHA-256 enforcement with indexes,
  transitive re-resolution, source builds, and configuration discovery off.
- [x] Add the private dependency target to componentization without changing
  canonical layer kind, entry, or contract.
- [x] Reject native/source-only and non-registry packages during `compile
  --check`, before wheel download or archive emission.
- [x] Reject requirement-injection names, installed symlinks, and native
  payload suffixes with typed diagnostics.
- [x] Preserve dependency-free Component and rootfs source compatibility.

## Runtime and conformance

- [x] Add a locked `six==1.17.0` teaching project.
- [x] Execute its `.holo` directly and through catalog import/load/run/unload.
- [x] Prove the guest observes `six-1.17.0` with a poisoned ambient `six.py` on
  `PYTHONPATH` and a fake `VIRTUAL_ENV`.
- [x] Keep the dependency-free direct/resident proof passing.
- [x] Cover universal-wheel admission, native-wheel rejection, non-registry
  rejection, unsafe names, and installed native payloads with unit tests.
- [ ] Add cross-platform proof on Linux and Windows before broadening the
  portable-wheel claim beyond the current runtime matrix.

## Documentation and delivery

- [x] Update README and website Python/CLI/format documentation with the
  portable dependency policy, example, and native fallback.
- [x] Record the observed non-reproducibility and keep deterministic component
  output unchecked in the durable runtime plan.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Run formatting, focused tests, all-target tests, Clippy, BDD, optimized
  build, release smoke, Component demos, WIT conformance, and docs.
- [ ] Land the reviewable PR, remove the feature worktree, and return the
  primary repository to clean `main`.

## Deferred discoveries

- [ ] `DISC-017c` — Define deterministic Component Python builds despite the
  pinned `--stub-wasi` build-time PRNG seed; record toolchain/artifact
  provenance without changing canonical identity accidentally.
- [ ] `DISC-018` — Define capability-gated WASI only when an application needs
  real host interfaces; do not broaden the import-free base world.
