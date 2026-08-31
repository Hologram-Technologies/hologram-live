# Current sprint: M3.2 portable View bundle and provider

## Sprint status

- State: active
- Started: 2026-08-30
- Last reviewed: 2026-08-30
- Durable milestone: [M3 — Real multi-layer providers](plans/holo-application-runtime.md#m3--real-multi-layer-providers)
- Decision: [ADR 018](adrs/018-portable-view-bundle-and-surface.md)
- Goal: compile complete portable frontends into canonical View payloads and
  attach them through a narrow Desktop-owned provider boundary
- Exit signal: a composed Wasm + View application attaches in Desktop,
  exchanges one bounded intent, retains root-primary completion, rolls back and
  stops in reverse order, and fails headless with an unavailable-surface error

## Canonical bundle slice

- [x] Replace the raw single-HTML placeholder with a versioned View bundle
  containing a fixed `index.html` entry and lexically ordered asset bytes.
- [x] Make a View source path name a directory and reject missing entries,
  symlinks, special files, non-portable paths, case collisions, and size-limit
  violations.
- [x] Exclude timestamps, permissions, ownership, directory creation order,
  host paths, and MIME guesses from canonical bytes.
- [x] Accept only the `portable` surface and fail every other selector closed.
- [x] Update the generator default and enforced View fixture to use `ui/`.
- [x] Record the bundle, surface, lifecycle, authority, and headless behavior in
  ADR 018.

## Provider slice

- [x] Add the lightweight host-neutral `hologram-view-surface` registry and
  attachment trait without pulling server or Tauri dependencies across the
  boundary.
- [ ] Add a Desktop-owned registry of available portable surface handles.
- [x] Add a View provider that validates during `prepare`, attaches during
  `start`, and detaches idempotently during reverse-order `stop` or rollback.
- [ ] Serve assets from an opaque per-application/layer origin without
  `file://`, workstation paths, or a localhost API redirect.
- [ ] Define and implement one bounded versioned intent/message round trip
  without exposing general Tauri commands or ambient host APIs.
- [x] Return an explicit unavailable-surface blocker for direct headless
  execution before any layer starts.
- [ ] Keep View non-exit-bearing and preserve root-primary completion.

## Verification and delivery

- [x] Unit-test deterministic construction, strict decoding, missing entry,
  unsupported surface, noncanonical order, trailing bytes, and symlinks.
- [x] Verify fat compilation embeds a decodable bundle and fat/thin packages
  preserve equal application manifests.
- [x] Exercise directory compilation through the enforced fat-archive BDD
  fixture.
- [x] Add BDD coverage for the explicit headless unavailable-surface failure.
- [x] Unit-test registry publication, prepare-without-attach, start attachment,
  decoded immutable assets, single detach, and idempotent repeated stop.
- [ ] Add Desktop coverage for attachment, messaging, rollback, and shutdown.
- [x] Update README and website `.holo` and Desktop docs for the bundle and
  provider contract.
- [x] Pass formatting, workspace tests, Clippy, BDD, docs, release build, and
  binary smoke gates for the canonical bundle slice.
- [x] Pass the same full repository and documentation gates for the
  host-neutral provider foundation.
- [ ] Pass Desktop tests and build with the attachment/provider slice.
- [ ] Commit, open and merge the PR, remove this sprint's worktree, and leave
  the primary checkout synchronized on `main`.

## Next prioritized work

- [ ] Register the Tauri portable surface adapter and opaque asset origin.
- [ ] Complete the bounded intent/message and composed-application slice.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
- [ ] Link WASI and Hologram Component host interfaces only when admitted by
  the effective capability grant.

---

# Previous sprint: M4.2 deterministic Python Components

## Sprint status

- State: complete
- Started: 2026-08-27
- Last reviewed: 2026-08-30
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Decision: [ADR 013](adrs/013-python-component-build-provenance.md)
- Discovery: `DISC-017d`
- Goal: make source-compiled Python Component layers byte-identical without
  rewriting a completed Wasm snapshot
- Exit signal: Hologram pins one patched componentizer wheel for every
  standalone-server release host, two clean builds per host agree on layer,
  application, and archive identity, and completed provenance reports
  `reproducible: true`

## Deterministic componentizer boundary

- [x] Confirm upstream `componentize-py 0.25.0` exposes no build-randomness
  control and constructs a private `WasiCtxBuilder` with three random defaults.
- [x] Reject fixed-offset or length-dependent rewriting of the finished Wasm
  snapshot because controlled outputs differ in both content and layout.
- [x] Define a minimal patch at pinned upstream revision `c0949b1` that supplies
  fixed secure bytes, insecure bytes, insecure seed, wall/monotonic clocks, and
  `PYTHONHASHSEED` during pre-initialization, then epoch-normalizes every
  compiler-owned preopened tree after generated bindings are complete.
- [x] Remove componentize-py's implicit virtualenv/pipenv/host-site fallback so
  the snapshot sees only Hologram's complete explicit staging paths.
- [x] Mount every pre-initialization input read-only so CPython cannot create
  bytecode caches and restore host-time directory mtimes after normalization.
- [x] Test CPython hash seeding and deterministic debug/system allocation;
  these controls leave the same 20 host-identity bytes in the snapshot and do
  not close reproducibility by themselves.
- [x] Approve a build-only Wasmtime metadata policy that maps each distinct
  host `(device, inode)` pair to a distinct, context-local guest identity in
  deterministic observation order. The policy exists only in the private
  preinitializer; it is absent from Hologram's runtime and emitted component.
- [x] Add a checksum-verified source-preparation script and immutable
  `componentizer-v*` release workflow. It applies the complete versioned patch
  set
  to the exact componentize-py revision, vendors the SHA-256-pinned
  `wasmtime-wasi 46.0.1` crate, and builds the five native server hosts with
  pinned Rust, maturin-action, maturin, and WASI SDK inputs.
- [x] Build the patch as five immutable wheel assets matching the server
  release matrix and publish their SHA-256 manifest under
  [`componentizer-v0.25.0-hologram.5`](https://github.com/Hologram-Technologies/hologram-live/releases/tag/componentizer-v0.25.0-hologram.5).
- [x] Replace every upstream componentizer URL/hash pin with the corresponding
  Hologram distribution asset; retain fail-closed host selection.
- [x] Report the patch identity and deterministic build-randomness contract in
  planned and completed non-canonical provenance.

## Reproducibility evidence

- [x] Reduce local drift from generated-section ordering plus snapshot state to
  exactly 20 bytes in one linear-memory data segment. Fixed-seed Rust maps
  removed the former large ordering delta; deterministic randomness, clocks,
  timestamps, CPython hash seed, and allocator selection do not remove the
  remaining host metadata.
- [x] Prove two independent local compiles have identical component layer,
  application, archive, footer, and complete-file identities using the exact
  locked release patch set. Two isolated uvx caches produced byte-identical
  19,554,774-byte archives with complete-file SHA-256
  `04d9b1b62ef98336d02ddcd76e13981aa43f636c2c984a616fc7ba6af9907048`,
  layer
  `blake3:abb209bfdd3b932910b0bfede3aeb8be477adeff07c6b8feaaafbc41e6e085f8`,
  application
  `blake3:f03f47117b4d3db6e55b559fe953d0ad60fa86604b8c5a781eaa6dbff7356fef`,
  archive
  `blake3:585b3a7b0fd048b005f474aa7887798fa7646859019d5277e9866b01e914fb98`,
  and footer
  `9a2893ff163aa67d694e2286af87cc417571e9684e6c8f8fa33c069d11b055b7`.
  The resulting archive executed successfully with bundled CPython 3.14.0.
- [x] Record the exact local arm64 macOS wheel SHA-256
  `06b3896b922e77bd6257b2b773348f62b37327fa9ea043b61054f70620904f5b`.
  Its clean locked build took 20m31s; the release workflow therefore builds
  the shared CPython WASI inputs once before fanning out to five wheel jobs.
- [x] Add one `jq`-friendly reproducibility command with JSON on stdout and
  progress on stderr.
- [x] Run two clean builders for macOS arm64/x86_64, Linux arm64/x86_64, and
  Windows x86_64 and compare all canonical and physical identities.
- [x] Keep completed provenance `reproducible: false` until the full clean-host
  matrix passes; cached wheel reuse is not acceptance evidence.
- [x] After the matrix passes, set completed provenance to `reproducible: true`
  while keeping `compile --check` honest about its unobserved output.

Local locked-dependency evidence (2026-08-28): `just
python-component-repro` used separate Hologram and uv caches for two macOS
arm64 builds, executed both archives, and matched component layer
`blake3:e454be4792df969408170829351ef3ce7f1bf87c67c5cbbbc126c3a1b690c41c`,
application
`blake3:f1e614a90cc39d67d8e7540c2346312f6db14838fe7769fd04d8f5bddb6f7e85`,
archive
`blake3:1412cfbf19e0d84eb775927252e0858bf693f72e1af8958e5d0d63c00e7bc366`,
footer `7d0da0fea43b3a67b03f46518ff8750d6393f788655221c92179a5f66f9b2968`,
and complete-file SHA-256
`24f63245fe22ebc105f451b851f88bcefe4099535f1312795f1d10594b39b7e9`.
This validates the command and one host locally; provenance remains false
until the ten clean GitHub runners pass.

First clean-host evidence (run `33196484166`, 2026-08-28): all ten independent
builders compiled and executed the locked dependency example successfully,
and both macOS host pairs matched every identity. The aggregate correctly
failed because equal-length component bytes differed between replicas on both
Linux architectures and Windows x86_64; their build contracts matched. The
next diagnostic run retains each `.holo` artifact so the remaining component
byte differences can be localized instead of weakening the gate.

Retained-archive evidence (run `33198288139`, 2026-08-28) reproduced the same
host pattern while every independent compile and execution succeeded. Extracting
the component layer localized the replica delta to only 17 bytes on Linux
x86_64, 19 bytes on Linux arm64, and 7 bytes on Windows x86_64, all in the
preinitialized filesystem metadata region. PR #34 added lexical guest-directory
enumeration to deterministic metadata mode and merged as `dec6a00`. Untagged
release run `33200329304` then built the shared CPython/WASI payload, all five
native wheels, `SHA256SUMS`, and `PATCHSET.sha256` successfully. Publishing the
immutable `.2` tag in run `33203476950` rebuilt and published all seven assets.
A fresh download verified every wheel and the patch-set manifest; the compiler
now pins the `.2` host wheels and contract
`hologram:componentizer/preinitialization-determinism@2`. The ten-runner
acceptance matrix and provenance claim remain open until the new release passes.

Pinned `.2` local proof (2026-08-28): two isolated macOS arm64 builds executed
successfully and matched layer
`blake3:d647d38b165f9f11462791e5bc0df53b97c9f597e805b254eeada2224af72df8`,
application
`blake3:1a35dac18db1dcfa7697e4b67afd5214580c87205942f40c909f7e660a67e010`,
archive
`blake3:2c0cafa298460003ed25ca585e815c3e77c464c2fa9fe38c1cbd53afc22bbadc`,
footer `cb60b3fea1cca459c0197fd0ff51e3b9b9d275c8ad0a56e0e3f0b26cea0e2e05`,
and complete-file SHA-256
`d150fa30cb5492473c5eacc797b5906512f81b99b47823012ebc5101d7f4c9fb`.

Pinned `.2` matrix result (run `33206743619`, 2026-08-28): all ten clean
archives compiled and executed. Both macOS pairs matched completely; Linux
arm64/x86_64 and Windows x86_64 still differed at equal component lengths.
Sorting directory streams did not remove lazy first-observation order from the
metadata mapper. PR #35 now pre-registers every preopened-tree host identity in
lexical mount/path order before guest execution and includes a regression test
for opposite host file-creation orders. Provenance remains false while the
six-patch distribution and replacement matrix are pending.

Six-patch build evidence (run `33209572217`, 2026-08-28): after PR #35 merged
as `533dd4c`, a workflow dispatch from merged `main` passed the shared
CPython/WASI build, every native wheel builder, and both checksum manifests.
Immutable tag `componentizer-v0.25.0-hologram.3` points to that exact commit.
Tagged run `33211899065` rebuilt all five wheels and published the seven public
assets. A fresh download verified every wheel against `SHA256SUMS`, all six
local patches against `PATCHSET.sha256`, and patch-manifest digest
`d281c2667a893fffa7e7d64c3b34d6ef22d9f40b9b89ab643475705bd0eba9c7`.
PR #33 now pins the `.3` assets and determinism contract `@3`; the replacement
matrix remains open and completed provenance is still false.

Pinned `.3` local proof (2026-08-28): two isolated macOS arm64 builds executed
successfully and matched layer
`blake3:cadb16f50a4cef8fd992838fb20c5acb44b2a94e84b0f9a5a56212c32545d716`,
application
`blake3:86d4be4b4900263bde7c38e245379e41a20fa78562d966abf2e5298eae51d805`,
archive
`blake3:344d1e3d84e6c5a217eb63cdfef5a14ebe11ff5034ec7a59b5e47a7a6e025ba8`,
footer `d47bbff76be502f6003211f9b14e7ba46478b40936abba158e5ddd1fab3adde0`,
and complete-file SHA-256
`67efc1a326e380a2fb6e35da7dc002396f0baeb1de4ffb7bf1261d9e680054d3`.

Pinned `.3` matrix result (run `33214553697`, 2026-08-28): all eight Linux and
macOS replicas compiled and executed, but both Windows replicas failed before
componentization. On Windows, `cap-std` exposes device/file identity only from
an open handle; preregistration's path-derived directory-entry metadata
panicked on `dev()`. PR #36 merged as `370c92b` with handle-derived identity
queries and a release-wheel invocation smoke test on every platform.
Fresh-source application, the unit regression, the locked vendored feature
build, and PR CI passed. Merged-main release run `33217328768` then built all
five wheels, invoked the packaged componentizer successfully on every platform
(including Windows), and generated both manifests. Annotated immutable `.4`
tag release run `33219475061` then rebuilt and published the validated source;
provenance remains false until the replacement matrix passes.

Handle-portable `.4` release evidence (run `33219475061`, 2026-08-28): all
five native wheel builders installed and invoked their packaged componentizer,
including Windows, before the immutable
`componentizer-v0.25.0-hologram.4` release was published. A fresh public
download verified every wheel against `SHA256SUMS`, all six repository patches
against `PATCHSET.sha256`, and patch-manifest digest
`1160ed7bd742dd55d798aae7baa2047897d0b188d251af63cbae5f25381c775f`.
PR #33 now pins the exact `.4` assets and contract `@4`; provenance remains
false until the local proof and replacement ten-runner matrix pass.

Pinned `.4` local proof (2026-08-28): two isolated macOS arm64 builds executed
successfully and matched layer
`blake3:37f149dae0f4ddfc95e7e424bdde2825b5978465fc21e56b8a59b41099110a49`,
application
`blake3:cff358ff9052748487822aa98f8d9b51701ffc6e028e7171b253ddb730529176`,
archive
`blake3:dfa39f441e209997de1fd802d8ba1c2ed5c4d73ab4142a3d96dcd57d1b771d31`,
footer `e77557c01644073652f746a82cd9bf6732c970275028f2f020b7cf726eea09e2`,
and complete-file SHA-256
`7fbb256c51c2d2a2f22bcd997a0cebde038f14c83270466ba042caeaf30f6470`.
The replacement ten-runner matrix is now the remaining acceptance gate.

Pinned `.4` matrix result (run `33221589694`, 2026-08-28): all ten clean
archives compiled and executed. Both macOS pairs matched, but Linux
arm64/x86_64 and Windows x86_64 produced equal-length components with different
identities. Retained bytes localized the Linux x86_64 delta to three 32-bit
nanosecond fields beside stable epoch seconds. Access/modification timestamps
were normalized on disk, but host status/creation time remained observable on
Linux and Windows. A seventh build-only patch now preserves timestamp
availability while mapping every exposed filesystem timestamp to epoch zero;
fresh-source application and its focused regression pass. Provenance remains
false pending a corrected release and replacement matrix.

Timestamp-normalized `.5` release evidence (2026-08-28): PR #37 merged as
`903c671`, and untagged merged-main run `33224125002` passed the shared
CPython/WASI stage, all five wheel builders, each packaged-tool invocation, and
both checksum manifests. Tagged run `33225747320` repeated those gates and
published `componentizer-v0.25.0-hologram.5`. A fresh public download verified
all five wheels, all seven local patches, and patch-manifest digest
`8262cb4562428132c29dc4a46780178a5e0f4d7fa1c41549e2f15c76f7dec8ad`.
PR #33 now pins the `.5` assets and determinism contract `@5`; provenance
remains false until the replacement ten-runner matrix passes.

Pinned `.5` local proof (2026-08-28): two isolated macOS arm64 builds executed
successfully and matched layer
`blake3:624884be7f65be8cb3ff4f7c8c9f9109bc33b81456feb8ea74653bd3e1c454b3`,
application
`blake3:bdf89554364b8df2ec40160880194e4bac7244bdbbb7ebc5285a9f8b9144aac0`,
archive
`blake3:b52177ef4d463218037802aa47fa15a62c428b5f666122fe7f1b522869cbcbc2`,
footer `245b38faa865018c52fb9592d47aa56959b98cf3de437c99462ef5b01145b709`,
and complete-file SHA-256
`3207dbf510698d48108064470ac26f17eecb120aa1d00ab78e61d23b0d94e691`.
This closes the local gate; the five-host result follows.

Five-host acceptance (run `33227358037`, 2026-08-28): all ten isolated clean
builders compiled and executed successfully. The aggregate reported
`target_local_equality: true`; both replicas matched every canonical and
physical identity plus the build contract within all five supported hosts.
Completed Component provenance now reports `reproducible: true` with no
blocker, while offline `compile --check` remains false because output is
unobserved.

## Verification and delivery

- [x] Validate the source preparation and workflow statically with ShellCheck,
  actionlint, locked offline Cargo metadata, and `git diff --check`.
- [x] Run the merged workflow without a release tag. Run `33140574673` proved
  the shared CPython/WASI build in 12m44s and exposed two release-portability
  defects before publication: both macOS runners use a BSD `sha256sum` that
  lacks GNU `--check`, and Windows CRLF checkout prevents LF vendored-source
  patches from matching.
- [x] Merge the portable digest verifier/LF checkout fix and rerun all five
  native wheel jobs from `main` before creating a `componentizer-v*` tag.
  PR #29 merged as `951cc25`; untagged run `33142178976` passed the shared
  CPython/WASI build, Linux x86_64/arm64, macOS x86_64/arm64, Windows x86_64,
  and the wheel/checksum/patch-set manifest aggregation. The release job was
  correctly skipped because no `componentizer-v*` tag was present.
- [x] Publish the immutable tagged distribution. Run `33188965708` rebuilt all
  five wheels from tag `componentizer-v0.25.0-hologram.1`, generated
  `SHA256SUMS` and `PATCHSET.sha256`, and published all seven assets. A fresh
  download verified every wheel against `SHA256SUMS` and every local patch
  against `PATCHSET.sha256`.
- [x] Publish the directory-ordering correction. Untagged run `33200329304`
  passed all five hosts, and tagged run `33203476950` published
  `componentizer-v0.25.0-hologram.2`. A fresh download verified all wheel
  hashes and patch-set digest
  `ce542742dfdd624bb25380bf042638a4e7caa5edb7e7560f0f8809343999c37c`.
- [x] Publish the metadata-preregistration correction. Untagged run
  `33209572217` passed every build host; tagged run `33211899065` published
  `componentizer-v0.25.0-hologram.3`; and a fresh download verified every
  wheel plus patch-set digest
  `d281c2667a893fffa7e7d64c3b34d6ef22d9f40b9b89ab643475705bd0eba9c7`.
- [x] Publish the Windows handle-portability correction. Untagged run
  `33217328768` invoked all five packaged wheels successfully; tagged run
  `33219475061` published `componentizer-v0.25.0-hologram.4`; and a fresh
  download verified all wheel hashes and patch-set digest
  `1160ed7bd742dd55d798aae7baa2047897d0b188d251af63cbae5f25381c775f`.
- [x] Publish the timestamp-normalization correction. Untagged run
  `33224125002` passed all five packaged-wheel invocations; tagged run
  `33225747320` published `componentizer-v0.25.0-hologram.5`; and a fresh
  public download verified all wheels and patch-set digest
  `8262cb4562428132c29dc4a46780178a5e0f4d7fa1c41549e2f15c76f7dec8ad`.
- [x] Add exact five-host distribution-selection and planned-provenance tests
  for the immutable release and patch contract.
- [x] Add a fail-closed clean-host comparator and execute every compiled proof
  archive before accepting its report.
- [ ] Add a direct patch-application regression test independent of the release
  workflow's existing `git apply --check` step.
- [x] Pass formatting, workspace tests/checks, Clippy, BDD, release/smoke,
  documentation, desktop, and component clean-build gates.
- [x] Update README, website Python guidance, ADR 013, and the durable runtime
  plan with the released tool boundary and exact commands.
- [x] Commit, merge the PR or PR sequence, remove only this sprint's temporary
  worktree, and leave the primary checkout clean on synchronized `main`.

## Next prioritized work

- [x] Resolve the pre-initialization filesystem-identity decision and prove
  local byte equality before publishing or pinning a distribution.
- [x] Publish the deterministic componentizer distribution after the local
  equality and five-host workflow gates pass.
- [x] Pin the five immutable release assets in the compiler and report the
  patch identity in provenance.
- [x] Close `DISC-017d` with the clean five-host equality matrix.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.

---

# Previous sprint: M4.2 clean Python rootfs equality

## Sprint status

- State: complete
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Decision: [ADR 017](adrs/017-normalized-python-rootfs-archive.md)
- Goal: prove that uncached rootfs compilation is byte-identical on independent
  clean builders for both supported Linux target architectures
- Exit signal: the release gate compares two clean replicas each for
  `linux/amd64` and `linux/arm64`, all target-local identities match, and any
  generated filesystem differences have been eliminated

## Builder contract

- [x] Distinguish the five standalone-server release hosts from the Linux
  container-engine contract required to compile a rootfs.
- [x] Compare independent builder replicas within one target architecture;
  never require an amd64 artifact and an arm64 artifact to share identity.
- [x] Add `compile --no-build-cache` and pass Docker `--no-cache` without
  weakening the normal cached developer path.
- [x] Record `builder.cache_disabled` in non-canonical completed provenance.
- [x] Keep `compile --check` offline and report that no build cache was disabled.

## Reproducibility evidence

- [x] Add `just python-rootfs-repro` with one JSON document on stdout and
  progress on stderr so every result can be queried with `jq`.
- [x] Compare image ID, rootfs layer κ/size, application κ, archive κ/size, and
  footer fingerprint.
- [x] Add two independent clean GitHub runners per architecture and a
  target-aware aggregate comparison artifact.
- [x] Make the clean-builder matrix a prerequisite of every server release.
- [x] Run the local two-build uncached probe and record its identities here.
- [x] Run the first clean GitHub matrix and record the failure evidence here.
- [x] Identify the first matrix's unstable generated layer and replace its
  cross-stage directory copy with a canonical runtime tar artifact.
- [x] Rerun both clean target comparisons and record the passing workflow
  evidence here.

Local evidence (2026-08-26): the first uncached comparison exposed unstable
timestamps in every generated Docker layer. A two-stage recipe reduced that
to the local-project `uv_cache.json`, whose nanosecond source timestamp also
changed its `RECORD` hash. The final recipe installs only locked dependencies,
runs the already-staged source through `PYTHONPATH`, normalizes the runtime
tree to epoch zero, and copies only that tree onto the pinned base. Two
uncached macOS-arm64/Linux-arm64-engine builds then matched exactly: image ID
`sha256:a4d4ad759567e43ebec5bcc84d5dae5a52a0a5f3fcce74cd7fe1e756f97e2271`,
rootfs layer κ
`blake3:64f53c4cf1f721a7efa857e3397589034eea565adb89dc93ce3db8799062f538`,
application κ
`blake3:9b20b3cb7f6a9fcabcd9888b54a05bad6b7f9c50a396ecfdf5cbdd4aae30b451`,
archive κ
`blake3:e31387403074e0e7546de124012764c6b389222d881d50d66e48447260ca0048`,
and footer fingerprint
`f4638af9a5d3e5c95d3c1170b558e82796095cec5773e9e2cfb16a5c5f0c9e25`.
The resulting archive executed NumPy/pandas successfully and returned three
rows, mean `20.0`, and sum `60.0` from an isolated current configuration.

Clean-runner evidence (workflow run `33031626335`, 2026-08-26): all four
uncached builds completed with Docker client/server `28.0.4` and the same
digest-bound base. Each architecture still disagreed across its two replicas:
amd64 emitted rootfs layer κ values beginning `blake3:164e` and
`blake3:49f9`, while arm64 emitted `blake3:ad32` and `blake3:fece`. This ruled
out tool and base selection drift and isolated the remaining instability to
BuildKit's serialization of `COPY --from=builder` directory trees.

The replacement builder writes `/app` and `/hologram` as one lexically sorted
GNU tar with epoch-zero timestamps and numeric root ownership. Hologram copies
that artifact from a stopped builder container and feeds it to the final
digest-bound image through local `ADD`, so no foreign-architecture process is
executed and BuildKit no longer chooses directory traversal order. Two local
uncached arm64 builds now match at image ID
`sha256:1f55f44f41af891e3464b056f6b0beefbf6be9d736611de1505d17fb9a8cd754`,
rootfs layer κ
`blake3:7466d21d435ec4d2a7da0efdd9e974f26ff58a471d579abfa04b3f6df4077b8b`,
application κ
`blake3:fdfc49a149a89b0fcce848515dd4aa3d5e85f9d6028ad2d28ae4efc23d943f2e`,
archive κ
`blake3:5c97573cf6df8a2fe4e538db32f1d5f3edddc24a049e0f7a776c0bdcd8ac7438`,
and footer fingerprint
`7e9e6308320a9d8428eee9700fb98bef3baa127db66dfb3ac3e94c6bfdd576a3`.
Passing clean-runner evidence (workflow run `33035209550`, 2026-08-26): the
rebased PR repeated two uncached builds for each target and the aggregate
comparison passed. Linux/amd64 matched at image ID
`sha256:778bc5f5e4c66392b798ff8b6ad6178e42c80efff6a8ff44b18fbfc44573d31f`,
rootfs layer κ
`blake3:cae3233e0f062c839b0517de631e4d77774cdda3df341fc152b8776b919bb6c9`,
application κ
`blake3:2e4f56c955af32b361194b20b0d1c98bf55dfeacc933c995ec2e489570892ed1`,
and archive κ
`blake3:6b0c6b9b21aae5bfe30dfa5494a25b90b5dde598978928fe0182fe26fe63a068`.
Linux/arm64 matched the local proof's image, rootfs, application, and archive
identities exactly. Completed builds now report `reproducible: true`; an
offline check of a mutable base remains false only until compilation resolves
and binds its immutable digest.

## Verification and delivery

- [x] Add focused CLI/provenance and report-comparison tests.
- [x] Pass formatting, workspace tests/checks, Clippy, BDD, release/smoke,
  documentation, and desktop gates.
- [x] Update README, website Python guidance, ADR 017, and the durable runtime
  plan with the rootfs-builder boundary and exact commands.
- [x] Commit, open and merge PR
  [#26](https://github.com/Hologram-Technologies/hologram-live/pull/26), remove
  only its worktree, and leave the primary checkout clean on synchronized
  `main`.

Repository evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests (including 204
library tests, 23 CLI tests, and four provenance/comparator tests), Clippy with
warnings denied, 13 BDD scenarios with 135 steps, the optimized server build,
and isolated smoke. `just docs` regenerated OpenAPI and built all 13 pages.
The Tauri release gate reported zero npm vulnerabilities and produced the
macOS application and arm64 DMG. The docs audit continues to report the
existing one low and two high findings.

## Next prioritized work

- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
- [ ] Continue M4 deterministic compiler work after both Python profiles have
  clean-build evidence.

---

# Previous sprint: M4.2 normalized Python rootfs archives

## Sprint status

- State: complete
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Decision: [ADR 017](adrs/017-normalized-python-rootfs-archive.md)
- Goal: remove Docker-export metadata and storage-layout variation from Python
  rootfs layer identity
- Exit signal: the current schema-3 rootfs bundle has one canonical archive
  representation, repeated exports are byte-identical, cold-load execution
  works, and the remaining uncached cross-host proof is precisely tracked

## Contract boundary

- [x] Replace the experimental rootfs envelope with bundle schema 3, magic
  `HOLOPYR2`, and provider `normalized-docker-archive-zstd-v1`.
- [x] Accept exactly one Docker image with the expected content-derived tag.
- [x] Re-address config and layer bytes as `blobs/sha256/<digest>` while
  preserving semantic layer order.
- [x] Emit only the canonical manifest and referenced blobs with lexical member
  order and fixed tar headers.
- [x] Set `SOURCE_DATE_EPOCH=0` for the build and disable injected provenance.
- [x] Reject unsafe paths, duplicate or non-file members, missing references,
  oversized archives, and image-ID mismatches.
- [x] Keep build provenance non-canonical and `reproducible: false` until the
  current clean Linux builder matrix passes for both rootfs target architectures.

## Tests and evidence

- [x] Prove differing input member order, JSON key order, timestamps, modes, and
  ownership normalize to identical bytes.
- [x] Cover duplicate members, unexpected tags, and canonical member sets.
- [x] Compile the locked NumPy/pandas application twice and prove equal layer,
  application, archive, and footer identities.
- [x] Remove the generated local image tag, cold-load from the `.holo`, and
  recover three rows, mean `20.0`, and sum `60.0`.
- [x] Hand uncached equality to the current sprint, with release-binary hosts
  separated from the Docker-compatible Linux builder contract.
- [x] Pass formatting, workspace tests/checks, Clippy, BDD, release/smoke,
  documentation, and desktop gates.

Local evidence (2026-08-26): Docker client 29.2.1/server 29.4.0 emitted two
identical normalized exports with rootfs layer κ
`blake3:6ac835129125e3f997a211611c96094e606fdbf332073c02fe2a9f906a7c07f7`,
application κ
`blake3:104da1166bf688727352e966097e1d0ce837c4ad3873199e4d6038d5ac0b24b0`,
archive κ
`blake3:3e302dff5f62ed341d5ce9b65296167bffb93d948330947db366c17d9726aff0`,
and fingerprint
`d01c6246d6efb6909262eea1df0489a575086dab67da236174f2f520b932db2c`.
The cold-load direct run completed successfully after removing the local tag.

Repository evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests, Clippy with warnings
denied, 12 BDD scenarios with 123 steps, the optimized server build, and the
isolated smoke test. `just docs` regenerated OpenAPI and built all 13 static
pages. `npm --prefix apps/desktop ci` reported zero vulnerabilities and the
release build produced the sidecar, frontend bundle, macOS application, and
arm64 DMG. The docs dependency audit continues to report the existing one low
and two high findings.

## Documentation and delivery

- [x] Record the representation and remaining proof boundary in ADR 017.
- [x] Update README, architecture, security, actual-capability, and website
  documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Commit, open and merge PR #20, remove only its worktree, and leave the
  primary checkout clean on synchronized `main`.

## Next prioritized work

- [ ] Complete `DISC-019b` with an uncached supported-host equality matrix and
  eliminate any differing generated filesystem content.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.

---

# Previous sprint: strict pre-release contract

## Sprint status

- State: complete
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M8 — Conformance and release hardening](plans/holo-application-runtime.md#m8--conformance-and-release-hardening)
- Decision: [ADR 016](adrs/016-strict-pre-release-contract.md)
- Goal: remove speculative compatibility paths before the first public release
- Exit signal: one explicit current format is enforced across compiler,
  runtime, configuration, persistence, RPC, fixtures, and documentation; all
  verification gates pass and the change is merged

## Contract boundary

- [x] Accept physical `.holo` version 4 only.
- [x] Require exactly one verified application directory for every application
  archive.
- [x] Accept source-manifest schema version 4 only.
- [x] Require explicit Wasm entry and canonical guest contract.
- [x] Require canonical capability objects; reject the zero-byte sentinel.
- [x] Accept exactly one current Python rootfs bundle schema; ADR 017 now sets
  that contract to schema 3.
- [x] Accept configuration schema version 2 only without automatic rewriting.
- [x] Require complete history, resident, and run records.
- [x] Keep OpenAI and Ollama compatibility APIs as supported integrations.

## Implementation

- [x] Add a Live-owned physical-version gate at inspect, import/cache, compile-
  child, and planning boundaries.
- [x] Replace optional application-directory derivation with required
  verification.
- [x] Remove source-schema feature gates and Wasm contract normalization.
- [x] Remove capability, rootfs, configuration, history, and RPC decode
  fallbacks.
- [x] Update generated examples and fixtures to the current manifest schema.
- [x] Finish strict current-archive test helpers and remove stale assertions.
- [x] Confirm public archive, manifest, configuration, persistence, and RPC
  boundaries return typed errors for noncurrent or incomplete input.

## Tests and evidence

- [x] Add focused rejection tests for physical version, source schema,
  configuration schema, missing application directory, missing Wasm contract,
  malformed capability objects, and incomplete result records.
- [x] Pass Rust formatting, unit tests, checks, and Clippy.
- [x] Pass public-boundary BDD and isolated smoke tests.
- [x] Pass desktop and documentation builds.
- [x] Record the exact verification commands and outcomes here.

Verification evidence (2026-08-26): `just verify` passed formatting, source-size
and product-boundary checks, locked workspace check/tests, Clippy with warnings
denied, 12 BDD scenarios with 123 steps, the optimized server build, and the
isolated smoke test. `just docs` regenerated OpenAPI and built all 13 static
pages. `npm --prefix apps/desktop ci && npm --prefix apps/desktop run build`
produced the release sidecar, frontend bundle, macOS application, and arm64 DMG.

## Documentation and delivery

- [x] Record the decision in ADR 016 and supersede conflicting ADR clauses.
- [x] Update README, architecture, security, actual-capability, and website
  documentation.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized.
- [x] Commit, open and merge the PR, remove only this worktree, and leave the
  primary checkout clean on synchronized `main`.

## Next prioritized work

- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.

---

# Previous sprint: M4.1 rootfs base-digest binding

## Sprint status

- State: ready for review
- Started: 2026-08-26
- Last reviewed: 2026-08-26
- Durable milestone: [M4 — Compiler completion](plans/holo-application-runtime.md#m4--compiler-completion)
- Goal: prevent a mutable Python rootfs base tag from moving between selection
  and Docker execution while preserving offline `compile --check`
- Exit signal: a real NumPy/pandas compile resolves the requested tag to the
  registry manifest digest, uses that exact reference in `FROM`, records both
  identities in provenance, executes successfully, passes all gates, and is
  merged

Rootfs provenance remains governed by ADR 014 and the binding decision in ADR
015. The later strict-contract decision in ADR 016 supersedes experimental
format compatibility from this period.

## Acceptance boundary

- Keep `compile --check` offline. It may report a digest-pinned request as
  already resolved, but it must not contact Docker or invent a digest for a
  mutable tag.
- Resolve mutable bases from Docker's original raw registry manifest and
  accept only schema 2.
- Compute the SHA-256 registry identity from the exact manifest bytes and
  preserve registry host/port and repository path when constructing
  `repository@sha256:digest`.
- Put the resolved reference into the Dockerfile before the image build so tag
  movement after resolution cannot redirect `FROM`.
- Preserve the requested source value and add the resolved build value to the
  non-canonical provenance report; do not rewrite `hologram.json` or `.holo`
  identity.
- Keep `reproducible: false` until the emitted OCI representation is normalized
  and equal layer κ values are proven across clean hosts.

## Runtime implementation

- [x] Add registry-manifest resolution for mutable rootfs base references.
- [x] Bypass registry resolution for valid lowercase SHA-256 digest references.
- [x] Generate Docker's `FROM` from the resolved immutable reference.
- [x] Add `base_image.resolved_reference` while retaining the requested
  `base_image.reference` and optional local `observed_image_id`.
- [x] Report mutable resolution as deferred during offline checks and remove
  that blocker after a completed digest-bound build.
- [x] Reject base strings that could be parsed as Docker command options.

## Tests and evidence

- [x] Prove raw schema-2 manifest bytes produce the expected SHA-256 reference.
- [x] Prove repository parsing handles a registry port and a tag.
- [x] Prove malformed, empty, and unsupported registry manifests fail closed.
- [x] Prove an already pinned reference returns unchanged without registry
  access.
- [x] Prove the generated Dockerfile uses the resolved reference.
- [x] Prove `compile --check` reports the request but omits a resolution for the
  mutable NumPy/pandas example.
- [x] Compare the resolver's
  `sha256:7a8b475003c4fe15a2cd4e55e5cfc2f3560bdc9333d624f24cdd6d4340fd7a17`
  with Docker's reported `python:3.12-slim` registry digest.
- [x] Compile the NumPy/pandas example with the digest-bound Dockerfile and
  confirm completed provenance reports both requested and resolved references.
- [x] Run the resulting fat archive through the direct rootfs provider and
  recover three rows, mean `20.0`, and sum `60.0`.
- [x] Run formatting, workspace tests/checks, Clippy, BDD, release/smoke, and
  documentation gates.

## Documentation and delivery

- [x] Record the digest-binding decision and threat boundary in ADR 015.
- [x] Update README and website guidance for offline checks, automatic real-
  build resolution, and the remaining OCI normalization blocker.
- [x] Keep `specs/plans/holo-application-runtime.md` synchronized with this
  milestone and its evidence.
- [x] Commit, open and merge the PR, remove only this worktree, and return the
  primary checkout to clean synchronized `main`.

## Next prioritized work

- [ ] `DISC-019b` — Define a normalized OCI/rootfs representation and prove
  byte-identical layer κ values across clean supported hosts.
- [ ] `DISC-017d` — Supply deterministic Python Component build randomness and
  prove clean supported-host equality.
- [ ] Add authenticated private-registry integration coverage without exposing
  credentials in build provenance.
