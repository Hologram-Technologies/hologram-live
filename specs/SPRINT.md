# Current sprint: M3.1c Python Component build provenance

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: make Python Component inputs, selected artifacts, toolchain, target, and
  output explainable through a versioned report without changing `.holo`
  identity, while determining the honest boundary for reproducible output
- Exit signal: `compile --check --json` reports stable planned provenance, a
  completed compile adds observed tools and layer identity, nested source
  symlinks fail closed, and the remaining deterministic-output blocker is
  machine-readable and backed by a controlled experiment

The completed dependency-packaging tracker remains in Git history. Durable
requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md).
Dependency admission is ADR 012; this sprint's provenance boundary is ADR 013.

## Provenance policy

- Use additive `build_provenance` schema version 1 in compile/check results.
  Mark it `canonical: false` and do not embed it in archive metadata,
  application directories, manifests, or content blobs.
- Index reports by manifest layer. The first schema covers Python
  `wasi-component` source layers; prebuilt layers and rootfs provenance remain
  outside this slice.
- Record the Hologram compiler, CPython runtime, componentizer version/release
  revision, target ABI, guest contract, and build host.
- Hash `pyproject.toml`, `uv.lock`, and a domain-separated source-tree view with
  SHA-256. The tree view sorts normalized UTF-8 `/` paths and includes path,
  length, and file bytes while ignoring directory order, timestamps, and
  permissions.
- Record every selected runtime dependency name, version, HTTPS wheel URL, and
  SHA-256 from the lock closure.
- During a real build, record observed uvx/uv versions plus the component layer
  κ and byte length. During `--check`, omit fields that were not observed and
  do not download or execute the toolchain.
- Stage source regular files in lexical order and reject nested symlinks and
  special files before componentization.
- Keep `reproducible: false` until two clean builds on every supported host
  produce byte-identical layers. Never substitute cache reuse for that proof.

## Reproducibility investigation

- [x] Confirm pinned componentize-py 0.25.0 exposes no deterministic seed
  option.
- [x] Inspect release source revision `c0949b1`: pre-initialization owns a
  private `WasiCtxBuilder`, so the caller cannot inject deterministic random.
- [x] Compile identical dependency-free input twice without `--stub-wasi` to
  test whether only the stub adapter caused variation.
- [x] Record that non-stubbed outputs also differed: 18,308,535 vs 18,320,373
  bytes and distinct SHA-256 values.
- [x] Reject output byte patching as unsafe because snapshot layout and total
  length differ, not merely one fixed seed field.
- [x] Reject caching as reproducibility evidence because a clean build still
  generates a new output.
- [ ] Obtain or maintain a componentizer with explicit deterministic
  pre-initialization randomness.
- [ ] Prove byte-identical component, application κ, and archive κ values
  across clean macOS, Linux, and Windows builds.

## Compiler and report

- [x] Add the versioned non-canonical report to compile and check library
  results.
- [x] Expose the report from `hologram --json compile` and `compile --check` so
  it can be selected or saved with `jq`.
- [x] Add normalized source input and source-tree SHA-256 values.
- [x] Add complete selected portable-wheel inventory with exact artifact URLs
  and hashes.
- [x] Add declared compiler/runtime/componentizer, Component target and
  contract, and build-host evidence.
- [x] Add observed uvx/uv versions and output layer κ/length only after a real
  build.
- [x] Keep provenance outside all canonical and physical archive bytes.
- [x] Stage source files deterministically and reject nested symlinks/special
  files.

## Tests and conformance

- [x] Cover stable tree hashing across file-creation order and sensitivity to
  content changes.
- [x] Cover nested source-symlink rejection.
- [x] Cover planned provenance through a real `compile --check --json` CLI
  invocation without downloading tools.
- [x] Prove completed provenance and direct/resident execution with the pinned
  Component toolchain.
- [x] Run formatting, focused tests, all-target tests, Clippy, BDD, optimized
  build, release smoke, Component demos, WIT conformance, and docs.

## Documentation and delivery

- [x] Record the identity and schema decision in ADR 013 and reconcile ADR 012.
- [x] Update README and website Python, CLI, architecture, format, and security
  documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with proven
  and still-open acceptance.
- [ ] Land the reviewable PR, remove the feature worktree, and return the
  primary repository to clean `main`.

## Deferred discoveries

- [ ] `DISC-017d` — Add deterministic random injection to the pinned
  componentizer or select a maintained deterministic replacement; then make
  clean cross-platform output equality a release gate.
- [ ] `DISC-017e` — Pin and report the exact componentizer distribution
  artifact rather than relying on version plus upstream release revision.
- [ ] `DISC-018` — Define capability-gated WASI only when an application needs
  real host interfaces; do not broaden the import-free base world.
