# Current sprint: M3.1d exact Python Component tool artifact

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M3.1a — Component-model and Python/WASI proof](plans/holo-application-runtime.md#m31a-component-model-and-pythonwasi-proof)
- Goal: remove mutable version-only componentizer resolution from Python
  Component builds while preserving the honest boundary between supply-chain
  pinning and reproducible output
- Exit signal: every server-release host selects one reviewed
  `componentize-py 0.25.0` wheel URL/SHA-256, uvx cannot consult an index or
  build a source distribution, provenance reports the selected artifact,
  unsupported hosts fail closed, and direct/resident execution still passes

The completed dependency admission and provenance trackers remain in Git
history. Durable requirements stay in
[`plans/holo-application-runtime.md`](plans/holo-application-runtime.md).
Dependency admission is ADR 012; provenance identity and artifact reporting
are ADR 013.

## Acceptance boundary

- This slice pins the componentizer distribution artifact. It does not claim
  that uvx, its host interpreter, or generated component bytes are
  reproducible.
- Keep `build_provenance.schema_version: 1` additive, non-canonical, and
  outside every archive, manifest, directory, and content blob.
- Select artifacts only for the five published server targets: macOS arm64 and
  x86_64, Linux arm64 and x86_64, and Windows x86_64.
- Use upstream wheel URLs and PyPI-published SHA-256 digests. Do not admit the
  source distribution or fall back to a package index.
- Continue reporting `reproducible: false` until controlled componentizer
  randomness and clean cross-host equality are proven.

## Compiler and provenance

- [x] Map each server-release OS/architecture pair to one exact
  `componentize-py 0.25.0` wheel.
- [x] Pass a PEP 508 direct URL with `#sha256=` to isolated uvx.
- [x] Disable registry lookup and source builds for the componentizer
  invocation.
- [x] Add the selected distribution URL/hash to the declared componentizer in
  both check and completed provenance.
- [x] Return typed `LIVE_CAPABILITY_MISSING` for an unmapped host.
- [x] Keep runner observation separate: `compile --check` does not execute uvx,
  while a real compile records the observed uvx version.
- [x] Keep the exact artifact report outside canonical `.holo` identity.

## Tests and release evidence

- [x] Cover all five release host mappings and validate wheel suffix/hash
  shape.
- [x] Cover an unsupported host and its typed diagnostic.
- [x] Exercise `compile --check --json` and assert the artifact URL/hash is
  selectable with jq.
- [x] Execute the exact direct-reference form with `--no-index --no-build` on
  macOS arm64.
- [x] Compile the dependency-free Python Component with the exact wheel and
  execute it directly and resident.
- [x] Run formatting, source-size, all-target tests, Clippy, BDD, optimized
  build, release smoke, all Component demos, WIT conformance, and docs.

## Documentation and delivery

- [x] Update README, website Python/CLI/security pages, architecture, security,
  actual capabilities, and ADR 013.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with proven
  and still-open acceptance.
- [ ] Land the reviewable PR, remove the feature worktree, and return the
  primary repository to clean, synchronized `main`.

## Reproducibility investigation retained from M3.1c

- [x] Confirm componentize-py 0.25.0 exposes no deterministic seed option.
- [x] Inspect release revision `c0949b1`: pre-initialization owns a private
  `WasiCtxBuilder`, so the caller cannot inject deterministic random.
- [x] Prove non-stubbed outputs also differ in length and SHA-256, ruling out
  the stub adapter as the only source.
- [x] Reject output byte patching because snapshot layout and total length
  differ.
- [x] Reject cache reuse as clean-build reproducibility evidence.
- [ ] Obtain or maintain a componentizer with explicit deterministic
  pre-initialization randomness.
- [ ] Prove byte-identical component, application κ, and archive κ values
  across clean macOS, Linux, and Windows builds.

## Deferred discoveries

- [ ] `DISC-017d` — Patch or replace the componentizer so its build-time WASI
  context receives deterministic randomness, then make clean supported-host
  output equality a release gate.
- [x] `DISC-017e` — Pin and report the exact componentizer distribution
  artifact instead of relying on version plus upstream source revision.
- [ ] `DISC-017f` — Decide whether uvx and its host Python must become an
  independently distributed, digest-pinned Hologram toolchain bundle before
  build provenance can graduate to a signed attestation.
- [ ] `DISC-018` — Define capability-gated WASI only when an application needs
  real host interfaces; do not broaden the import-free base world.
